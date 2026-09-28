//! The live world: terrain generation, chunk caching and disk persistence.
//!
//! [`World`] is the layer between the generator and the network. It owns the
//! seed, decides where a chunk's blocks come from, and caches the encoded
//! `map_chunk` payload so streaming the same chunk to a second player does not
//! re-encode it.
//!
//! # Chunk lifecycle
//!
//! ```text
//! stream request for (x, z)
//!        |
//!        +-- payload cache hit? --> reuse the encoded bytes
//!        |
//!        +-- on disk?            --> load, encode, cache
//!        |
//!        +-- otherwise           --> generate, save, encode, cache
//! ```
//!
//! Because generation is a pure function of `(seed, x, z)` and saving happens
//! immediately, the three paths are interchangeable: a chunk loaded from disk is
//! byte-identical to the one that would have been generated. That is what
//! `tests/chunk_persistence.rs` asserts.
//!
//! Persistence failures are **non-fatal**: a read-only world directory degrades
//! to pure generation (with a one-time warning) rather than dropping players.

use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex, OnceLock, RwLock};
use std::thread;

const PAYLOAD_SHARDS: usize = 32;
const MAX_CACHED_PAYLOADS: usize = 8192;
type PayloadShard = RwLock<HashMap<(i32, i32), CachedChunk>>;

#[derive(Debug, Clone)]
pub(crate) struct CachedChunk {
    pub payload: Arc<Vec<u8>>,
    pub entities: crate::entity::TrackedEntities,
}

struct GenerationJob {
    world: World,
    position: (i32, i32),
}

impl GenerationJob {
    fn send(self, queue: &mpsc::Sender<Self>) {
        if let Err(error) = queue.send(self) {
            let job = &error.0;
            eprintln!(
                "[bcore] cannot queue chunk {:?} for world seed {}: {error}",
                job.position,
                job.world.seed()
            );
        }
    }

    fn run(self) {
        let (x, z) = self.position;
        let _ = self.world.chunk_payload(x, z);
    }
}

impl Drop for GenerationJob {
    fn drop(&mut self) {
        // Release reservations on completion, send failure, queue drop or unwind.
        self.world
            .inner
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.position);
    }
}

static GENERATION_QUEUE: OnceLock<mpsc::Sender<GenerationJob>> = OnceLock::new();

fn run_generation_queue(queue: mpsc::Receiver<GenerationJob>) {
    for job in queue {
        let position = job.position;
        let seed = job.world.seed();
        if std::panic::catch_unwind(|| job.run()).is_err() {
            eprintln!("[bcore] queued chunk {position:?} failed for world seed {seed}");
        }
    }
}

fn generation_queue() -> &'static mpsc::Sender<GenerationJob> {
    GENERATION_QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<GenerationJob>();
        // A chunk already uses the Rayon pool. One dispatcher keeps nearest-first
        // jobs from competing with seven other whole-chunk parallel generations.
        thread::spawn(move || run_generation_queue(rx));
        tx
    })
}

fn payload_shard(x: i32, z: i32) -> usize {
    let mut hash = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (z as u64).rotate_left(32);
    hash ^= hash >> 33;
    (hash as usize) & (PAYLOAD_SHARDS - 1)
}

use bcore_core::ChunkPos;
use bcore_worldgen::{block, WorldGenerator};

use crate::chunk::ChunkColumn;
use crate::chunk_store::ChunkStore;

/// The default world seed. `/seed` reports this.
pub const DEFAULT_SEED: i64 = 0x0BC0_0E00_1234_5678u64 as i64;

/// How the blocks of a chunk were obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkOrigin {
    /// Freshly generated (and saved, if the store is writable).
    Generated,
    /// Read back from `world/chunks/`.
    Loaded,
}

/// A shared handle to a seeded world, its cache and optional disk persistence.
///
/// Clones share the same entity identities and queued work. Each constructor
/// creates a distinct instance, even when its seed or store matches another.
#[derive(Debug, Clone)]
pub struct World {
    inner: Arc<WorldInner>,
}

#[derive(Debug)]
struct WorldInner {
    generator: WorldGenerator,
    store: Option<ChunkStore>,
    /// Encoded `map_chunk` payloads, keyed by chunk position.
    payloads: [PayloadShard; PAYLOAD_SHARDS],
    /// Coalesce concurrent cache misses before loading/generating the same chunk.
    generation_locks: [Mutex<()>; PAYLOAD_SHARDS],
    in_flight: Mutex<HashSet<(i32, i32)>>,
    entities: crate::entity::EntityTracker,
    /// Set once the first persistence error has been reported.
    warned: Mutex<bool>,
}

impl World {
    /// A world that generates terrain and persists it under `world/`.
    pub fn new(seed: i64) -> Self {
        Self::with_store(seed, ChunkStore::new())
    }

    /// A world rooted at a specific directory (used by tests).
    pub fn with_store(seed: i64, store: ChunkStore) -> Self {
        Self::with_optional_store(seed, Some(store))
    }

    /// A world that never touches the disk (used by tests and benchmarks).
    pub fn in_memory(seed: i64) -> Self {
        Self::with_optional_store(seed, None)
    }

    fn with_optional_store(seed: i64, store: Option<ChunkStore>) -> Self {
        Self {
            inner: Arc::new(WorldInner {
                generator: WorldGenerator::new(seed),
                store,
                payloads: std::array::from_fn(|_| RwLock::new(HashMap::new())),
                generation_locks: std::array::from_fn(|_| Mutex::new(())),
                in_flight: Mutex::new(HashSet::new()),
                entities: Default::default(),
                warned: Mutex::new(false),
            }),
        }
    }

    /// Pre-encoded terrain for unit tests of chunk selection and packet batching.
    #[cfg(test)]
    pub(crate) fn flat_fixture(positions: impl IntoIterator<Item = (i32, i32)>) -> Self {
        let world = Self::in_memory(0);
        for (x, z) in positions {
            world.inner.payloads[payload_shard(x, z)]
                .write()
                .unwrap()
                .insert(
                    (x, z),
                    CachedChunk {
                        payload: Arc::new(crate::chunk::flat_chunk_payload(x, z)),
                        entities: Arc::from([]),
                    },
                );
        }
        world
    }

    /// The seed this world generates from.
    pub fn seed(&self) -> i64 {
        self.inner.generator.seed()
    }

    /// The generator, for callers that need raw heights (e.g. spawn selection).
    pub fn generator(&self) -> WorldGenerator {
        self.inner.generator
    }

    /// The chunk store, if this world persists chunks.
    pub fn store(&self) -> Option<&ChunkStore> {
        self.inner.store.as_ref()
    }

    /// Report a persistence problem once, then stay quiet.
    fn warn_once(&self, context: &str, error: &dyn std::fmt::Display) {
        let mut warned = self.inner.warned.lock().expect("world warn lock");
        if !*warned {
            *warned = true;
            eprintln!("[bcore] world persistence disabled for this run: {context}: {error}");
        }
    }

    /// Load a chunk from disk, or generate (and save) it.
    ///
    /// Returns the column and where it came from.
    pub fn chunk(&self, x: i32, z: i32) -> (ChunkColumn, ChunkOrigin) {
        let _generation = self.inner.generation_locks[payload_shard(x, z)]
            .lock()
            .expect("chunk generation lock");
        if let Some(store) = &self.inner.store {
            match store.load(x, z) {
                Ok(Some(column)) => return (column, ChunkOrigin::Loaded),
                Ok(None) => {}
                Err(e) => self.warn_once(&format!("cannot read chunk ({x}, {z})"), &e),
            }
        }

        let generated = self
            .inner
            .generator
            .generate_chunk_vanilla(ChunkPos::new(x, z));
        let column = ChunkColumn::from_generated(&generated);

        if let Some(store) = &self.inner.store {
            if let Err(e) = store.save(x, z, &column) {
                self.warn_once(&format!("cannot write chunk ({x}, {z})"), &e);
            }
        }
        (column, ChunkOrigin::Generated)
    }

    /// Generate a chunk without consulting or touching the disk.
    pub fn generate(&self, x: i32, z: i32) -> ChunkColumn {
        ChunkColumn::from_generated(
            &self
                .inner
                .generator
                .generate_chunk_vanilla(ChunkPos::new(x, z)),
        )
    }

    /// Return a cached payload without doing world generation.
    pub fn cached_payload(&self, x: i32, z: i32) -> Option<Vec<u8>> {
        self.cached_chunk(x, z)
            .map(|chunk| chunk.payload.as_ref().clone())
    }

    pub(crate) fn cached_chunk(&self, x: i32, z: i32) -> Option<CachedChunk> {
        self.inner.payloads[payload_shard(x, z)]
            .read()
            .expect("world payload shard lock")
            .get(&(x, z))
            .cloned()
    }

    /// Queue generation in this world, coalescing requests across its clones.
    /// The queued job keeps the world alive until it completes or is discarded.
    pub fn request_payload(&self, x: i32, z: i32) {
        if let Some(job) = self.reserve_generation(x, z) {
            job.send(generation_queue());
        }
    }

    fn reserve_generation(&self, x: i32, z: i32) -> Option<GenerationJob> {
        if self.cached_chunk(x, z).is_some() {
            return None;
        }
        let key = (x, z);
        let mut pending = self
            .inner
            .in_flight
            .lock()
            .expect("generation in-flight lock");
        if !pending.insert(key) {
            return None;
        }
        Some(GenerationJob {
            world: self.clone(),
            position: key,
        })
    }

    /// Queue generation for all requested chunks without blocking the caller.
    pub fn request_payloads<I: IntoIterator<Item = (i32, i32)>>(&self, chunks: I) {
        for (x, z) in chunks {
            self.request_payload(x, z);
        }
    }

    /// The encoded `map_chunk` payload for `(x, z)`, cached across calls.
    pub fn chunk_payload(&self, x: i32, z: i32) -> Vec<u8> {
        if let Some(hit) = self.cached_chunk(x, z) {
            return hit.payload.as_ref().clone();
        }
        let (column, _origin) = self.chunk(x, z);
        self.cache_column(x, z, &column).payload.as_ref().clone()
    }

    pub(crate) fn cache_column(&self, x: i32, z: i32, column: &ChunkColumn) -> CachedChunk {
        let payload = Arc::new(column.encode_payload(x, z));
        let cached = self.inner.payloads[payload_shard(x, z)]
            .write()
            .expect("world payload shard lock")
            .entry((x, z))
            .or_insert_with(|| CachedChunk {
                payload,
                entities: self
                    .inner
                    .entities
                    .for_chunk(self.seed(), (x, z), column.entities()),
            })
            .clone();
        if self.cached_payloads() > MAX_CACHED_PAYLOADS {
            self.clear_cache();
        }
        cached
    }

    /// Drop cached payloads (used when a test wants to force a re-read).
    pub fn clear_cache(&self) {
        for shard in &self.inner.payloads {
            shard.write().expect("world payload shard lock").clear();
        }
        self.inner.entities.prune();
    }

    /// How many payloads are currently cached.
    pub fn cached_payloads(&self) -> usize {
        self.inner
            .payloads
            .iter()
            .map(|shard| shard.read().expect("world payload shard lock").len())
            .sum()
    }

    /// A safe spawn position: the terrain surface at `(x, z)`, plus one block.
    pub fn spawn_position(&self, x: f64, z: f64) -> (f64, f64, f64) {
        let bx = x.floor() as i32;
        let bz = z.floor() as i32;
        let (column, _) = self.chunk(bx.div_euclid(16), bz.div_euclid(16));
        let y = column
            .surface_y(bx.rem_euclid(16) as usize, bz.rem_euclid(16) as usize)
            .unwrap_or(crate::chunk::SEA_LEVEL)
            + 1;
        (x, y as f64, z)
    }

    /// A spawn on the nearest column whose actual vanilla-generated surface is
    /// land above sea level. The search retains the original chunk spiral, but
    /// never uses the approximate noise height for this decision or Y value.
    pub fn land_spawn(&self, x0: i32, z0: i32) -> (f64, f64, f64) {
        let (mut bx, mut bz) = (x0, z0);
        let actual_surface = |x: i32, z: i32| {
            let cx = x.div_euclid(16);
            let cz = z.div_euclid(16);
            let lx = x.rem_euclid(16) as usize;
            let lz = z.rem_euclid(16) as usize;
            let (chunk, _) = self.chunk(cx, cz);
            chunk
                .surface_y(lx, lz)
                .map(|y| (y, chunk.get(lx, y, lz).expect("surface block")))
        };
        let is_land = |x: i32, z: i32| {
            actual_surface(x, z).is_some_and(|(y, state)| {
                y >= crate::chunk::SEA_LEVEL && state != block::WATER && state != block::LAVA
            })
        };
        if is_land(bx, bz) {
            let y = actual_surface(bx, bz).expect("land surface").0 + 1;
            return (bx as f64, y as f64, bz as f64);
        }
        let mut radius = 1;
        'outer: loop {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    if dx != radius && dx != -radius && dz != radius && dz != -radius {
                        continue;
                    }
                    let (tx, tz) = (bx + dx * 16, bz + dz * 16);
                    if is_land(tx, tz) {
                        bx = tx;
                        bz = tz;
                        break 'outer;
                    }
                }
            }
            radius += 1;
            if radius > 256 {
                break;
            }
        }
        let y = actual_surface(bx, bz)
            .map(|(surface, _)| surface)
            .unwrap_or(crate::chunk::SEA_LEVEL)
            + 1;
        (bx as f64, y as f64, bz as f64)
    }
}

/// The process-wide world, created on first use.
///
/// The play loop needs one shared world across every connection thread so two
/// players standing in the same chunk see the same blocks and the chunk is only
/// generated once.
pub fn shared() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| World::new(DEFAULT_SEED))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::block_state;

    fn temp_store(tag: &str) -> ChunkStore {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "bcore-world-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        ChunkStore::at(dir)
    }

    #[test]
    fn a_fresh_chunk_is_generated_then_loaded_from_disk() {
        let store = temp_store("origin");
        let world = World::with_store(42, store.clone());

        let (first, origin) = world.chunk(3, -4);
        assert_eq!(origin, ChunkOrigin::Generated);
        assert!(store.contains(3, -4), "generating must also save");

        let (second, origin) = world.chunk(3, -4);
        assert_eq!(origin, ChunkOrigin::Loaded, "second call reads the disk");
        assert_eq!(first, second, "disk round trip must be lossless");

        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn simultaneous_requests_generate_a_persisted_chunk_once() {
        let store = temp_store("concurrent-origin");
        let world = std::sync::Arc::new(World::with_store(DEFAULT_SEED, store.clone()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let world = world.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    world.chunk(0, 0)
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|(_, origin)| *origin == ChunkOrigin::Generated)
                .count(),
            1
        );
        assert!(results.iter().all(|(column, _)| *column == results[0].0));
        std::fs::remove_dir_all(store.root()).unwrap();
    }

    #[test]
    fn a_loaded_chunk_matches_a_freshly_generated_one() {
        let store = temp_store("match");
        let world = World::with_store(7, store.clone());
        let (saved, _) = world.chunk(-9, 12);
        let regenerated = world.generate(-9, 12);
        assert_eq!(saved, regenerated, "load and generate must agree");
        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn payloads_are_cached_and_stable() {
        let world = World::in_memory(1);
        assert_eq!(world.cached_payloads(), 0);
        let first = world.chunk_payload(0, 0);
        assert_eq!(world.cached_payloads(), 1);
        let second = world.chunk_payload(0, 0);
        assert_eq!(first, second);
        // Coordinates are in the payload header.
        assert_eq!(&first[0..4], &0i32.to_be_bytes());
        let other = world.chunk_payload(5, -3);
        assert_eq!(&other[0..4], &5i32.to_be_bytes());
        assert_eq!(&other[4..8], &(-3i32).to_be_bytes());
        assert_eq!(world.cached_payloads(), 2);
        world.clear_cache();
        assert_eq!(world.cached_payloads(), 0);
    }

    #[test]
    fn payload_cache_evicts_when_it_exceeds_the_cap() {
        let world = World::in_memory(1);
        // Push the cache over the cap directly (bypassing worldgen, which is
        // ~0.2s/chunk). The shards and shard function are in scope because this
        // test module is a child of the `world_state` module.
        for i in 0..=MAX_CACHED_PAYLOADS {
            world.inner.payloads[payload_shard(i as i32, 0)]
                .write()
                .expect("shard lock")
                .insert(
                    (i as i32, 0),
                    CachedChunk {
                        payload: Arc::new(vec![0u8; 4]),
                        entities: Arc::from([]),
                    },
                );
        }
        assert!(world.cached_payloads() > MAX_CACHED_PAYLOADS);
        // A cache miss must detect the overflow and clear the whole cache so the
        // encoded-chunk cache never grows without bound.
        let _ = world.chunk_payload(9999, 9999);
        assert!(world.cached_payloads() <= MAX_CACHED_PAYLOADS);
    }

    #[test]
    fn in_memory_worlds_never_create_files() {
        let world = World::in_memory(3);
        assert!(world.store().is_none());
        let (_, origin) = world.chunk(0, 0);
        assert_eq!(origin, ChunkOrigin::Generated);
        // A second call regenerates rather than loading, since nothing is saved.
        let (_, origin) = world.chunk(0, 0);
        assert_eq!(origin, ChunkOrigin::Generated);
    }

    #[test]
    fn generated_terrain_is_not_flat() {
        let world = World::in_memory(DEFAULT_SEED);
        // Sample surfaces across several chunks; they must not all be equal.
        let mut heights = Vec::new();
        for cx in 0..6 {
            let column = world.generate(cx, 0);
            heights.push(column.surface_y(8, 8).expect("solid ground"));
        }
        heights.sort_unstable();
        heights.dedup();
        assert!(
            heights.len() > 1,
            "terrain should vary between chunks, got {heights:?}"
        );
        // And it must not be the superflat surface.
        assert!(
            heights.iter().all(|&y| y != crate::chunk::FLAT_SURFACE_Y),
            "terrain must not sit at the superflat height"
        );
    }

    #[test]
    fn every_column_is_floored_with_bedrock() {
        let world = World::in_memory(DEFAULT_SEED);
        let column = world.generate(2, -5);
        for z in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    column.get(x, crate::chunk::MIN_Y, z),
                    Some(block_state::BEDROCK)
                );
            }
        }
    }

    #[test]
    fn spawn_position_sits_on_the_surface() {
        let world = World::in_memory(DEFAULT_SEED);
        let (x, y, z) = world.spawn_position(8.5, 8.5);
        assert_eq!((x, z), (8.5, 8.5));
        let column = world.generate(0, 0);
        let surface = column.surface_y(8, 8).expect("ground");
        // Spawn must be at or above the surface, never buried inside it.
        assert!(
            y >= surface as f64,
            "spawn y={y} is below the surface {surface}"
        );
    }

    #[test]
    fn land_spawn_uses_actual_vanilla_surface() {
        let world = World::in_memory(DEFAULT_SEED);
        let (x, y, z) = world.land_spawn(10, -3);
        let cx = (x as i32).div_euclid(16);
        let cz = (z as i32).div_euclid(16);
        let lx = (x as i32).rem_euclid(16) as usize;
        let lz = (z as i32).rem_euclid(16) as usize;
        let chunk = world
            .generator()
            .generate_chunk_vanilla(ChunkPos::new(cx, cz));
        let surface = chunk.surface_y(lx, lz).expect("spawn has a surface");
        let state = chunk.get(lx, surface, lz).expect("surface block");
        println!("land spawn: x={x} y={y} z={z}, surface_y={surface}, state={state}");
        assert_eq!(y, (surface + 1) as f64);
        assert!(surface >= crate::chunk::SEA_LEVEL);
        assert_ne!(state, block::WATER);
        assert_ne!(state, block::LAVA);
    }
    #[test]
    fn the_shared_world_is_a_singleton() {
        let a = shared();
        let b = shared();
        assert!(std::ptr::eq(a, b));
        assert_eq!(a.seed(), DEFAULT_SEED);
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use bcore_worldgen::generated_entity::GeneratedEntity;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Barrier;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const POSITION: (i32, i32) = (3, -4);

    struct SavedChunk {
        store: ChunkStore,
        column: ChunkColumn,
    }

    impl SavedChunk {
        fn new(state: u32, loot_seed: i64) -> Self {
            static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
            let parent = std::env::temp_dir().join("opencode");
            std::fs::create_dir_all(&parent).unwrap();
            let root = parent.join(format!(
                "bcore-world-queue-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).expect("new isolated fixture directory");
            let store = ChunkStore::at(root);
            let mut column = ChunkColumn::flat();
            assert!(column.set(2, 32, 3, state));
            assert!(column.add_entity(
                ChunkPos::new(POSITION.0, POSITION.1),
                GeneratedEntity::ChestMinecart {
                    block_pos: [POSITION.0 * 16 + 2, 33, POSITION.1 * 16 + 3],
                    loot_seed,
                }
            ));
            store.save(POSITION.0, POSITION.1, &column).unwrap();
            Self { store, column }
        }

        fn world(&self, seed: i64) -> World {
            World::with_store(seed, self.store.clone())
        }

        fn assert_cached(&self, world: &World, cached: &CachedChunk) {
            assert_eq!(
                *cached.payload,
                self.column.encode_payload(POSITION.0, POSITION.1)
            );
            assert_eq!(cached.entities.len(), self.column.entities().len());
            for (actual, expected) in cached.entities.iter().zip(self.column.entities()) {
                assert_eq!(&actual.generated, expected);
            }
            let expected =
                World::in_memory(world.seed()).cache_column(POSITION.0, POSITION.1, &self.column);
            assert_eq!(cached.entities[0].uuid, expected.entities[0].uuid);
            assert_eq!(
                self.store.load(POSITION.0, POSITION.1).unwrap(),
                Some(self.column.clone())
            );
            assert_eq!(self.store.saved_chunks().unwrap(), [POSITION]);
        }
    }

    impl Drop for SavedChunk {
        fn drop(&mut self) {
            // A failed test may still have queued work; leave its isolated inputs.
            if !thread::panicking() {
                std::fs::remove_dir_all(self.store.root())
                    .expect("remove isolated fixture directory");
            }
        }
    }

    fn wait_cached(world: &World) -> CachedChunk {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(cached) = world.cached_chunk(POSITION.0, POSITION.1) {
                if !world.inner.in_flight.lock().unwrap().contains(&POSITION) {
                    return cached;
                }
            }
            assert!(
                Instant::now() < deadline,
                "queue did not finish for seed {}",
                world.seed()
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn async_requests_use_each_worlds_seed_store_and_entities() {
        for (first_seed, second_seed) in [(11, 22), (7, 7)] {
            let first = SavedChunk::new(block::STONE, 41);
            let second = SavedChunk::new(block::DIRT, 42);
            let a = first.world(first_seed);
            let b = second.world(second_seed);
            let barrier = Barrier::new(2);
            thread::scope(|scope| {
                for world in [&a, &b] {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        world.request_payloads([POSITION, POSITION]);
                    });
                }
            });
            let a_cached = wait_cached(&a);
            let b_cached = wait_cached(&b);
            assert_eq!((a.seed(), b.seed()), (first_seed, second_seed));
            first.assert_cached(&a, &a_cached);
            second.assert_cached(&b, &b_cached);
            assert_ne!(a_cached.payload, b_cached.payload);
            assert!(!Arc::ptr_eq(&a_cached.entities, &b_cached.entities));
            assert_ne!(a_cached.entities[0].id, b_cached.entities[0].id);
        }
    }

    #[test]
    fn equal_seed_and_store_do_not_merge_distinct_instances() {
        let saved = SavedChunk::new(block::STONE, 43);
        let a = saved.world(17);
        let clone = a.clone();
        let b = saved.world(17);
        let a_job = a.reserve_generation(POSITION.0, POSITION.1).unwrap();
        assert!(clone.reserve_generation(POSITION.0, POSITION.1).is_none());
        let b_job = b.reserve_generation(POSITION.0, POSITION.1).unwrap();
        a_job.run();
        b_job.run();
        let a_cached = wait_cached(&a);
        let clone_cached = wait_cached(&clone);
        let b_cached = wait_cached(&b);
        assert!(Arc::ptr_eq(&a_cached.payload, &clone_cached.payload));
        assert!(Arc::ptr_eq(&a_cached.entities, &clone_cached.entities));
        assert!(!Arc::ptr_eq(&a_cached.entities, &b_cached.entities));
        assert_ne!(a_cached.entities[0].id, b_cached.entities[0].id);
        assert_eq!(a_cached.entities[0].uuid, b_cached.entities[0].uuid);
    }

    #[test]
    fn clones_coalesce_requests_and_preserve_entities_across_eviction() {
        let saved = SavedChunk::new(block::STONE, 44);
        let world = saved.world(23);
        let clone = world.clone();
        let job = world.reserve_generation(POSITION.0, POSITION.1).unwrap();
        let barrier = Barrier::new(8);
        thread::scope(|scope| {
            for _ in 0..8 {
                let clone = world.clone();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    clone.request_payloads([POSITION, POSITION]);
                });
            }
        });
        assert_eq!(world.inner.in_flight.lock().unwrap().len(), 1);
        assert!(world.cached_chunk(POSITION.0, POSITION.1).is_none());
        job.send(generation_queue());
        let original = wait_cached(&world);
        let copy = clone.cached_chunk(POSITION.0, POSITION.1).unwrap();
        assert!(Arc::ptr_eq(&original.payload, &copy.payload));
        assert!(Arc::ptr_eq(&original.entities, &copy.entities));
        drop(copy);
        let weak = Arc::downgrade(&original.entities);
        clone.clear_cache();
        assert_eq!(world.cached_payloads(), 0);
        clone.request_payload(POSITION.0, POSITION.1);
        let restored = wait_cached(&world);
        saved.assert_cached(&clone, &restored);
        assert!(Arc::ptr_eq(&original.entities, &restored.entities));
        assert_eq!(original.entities[0].id, restored.entities[0].id);
        drop((original, restored));
        world.clear_cache();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn failed_sends_and_discarded_jobs_release_reservations() {
        let saved = SavedChunk::new(block::STONE, 45);
        let world = saved.world(29);
        let (tx, rx) = mpsc::channel();
        drop(rx);
        world
            .reserve_generation(POSITION.0, POSITION.1)
            .unwrap()
            .send(&tx);
        assert!(world.inner.in_flight.lock().unwrap().is_empty());
        let (tx, rx) = mpsc::channel();
        world
            .reserve_generation(POSITION.0, POSITION.1)
            .unwrap()
            .send(&tx);
        assert!(world.inner.in_flight.lock().unwrap().contains(&POSITION));
        drop(rx);
        assert!(world.inner.in_flight.lock().unwrap().is_empty());
        assert!(world.cached_chunk(POSITION.0, POSITION.1).is_none());
        world.request_payload(POSITION.0, POSITION.1);
        saved.assert_cached(&world, &wait_cached(&world));
    }

    #[test]
    fn queued_job_keeps_world_alive_without_a_reference_cycle() {
        let saved = SavedChunk::new(block::STONE, 46);
        let world = saved.world(31);
        let weak = Arc::downgrade(&world.inner);
        let (tx, rx) = mpsc::channel();
        world
            .reserve_generation(POSITION.0, POSITION.1)
            .unwrap()
            .send(&tx);
        drop(world);
        assert!(weak.upgrade().is_some());
        rx.recv_timeout(Duration::from_secs(5)).unwrap().run();
        assert!(weak.upgrade().is_none());
        assert_eq!(
            saved.store.load(POSITION.0, POSITION.1).unwrap(),
            Some(saved.column.clone())
        );
    }

    #[test]
    fn worker_unwind_releases_reservation_and_continues_other_worlds() {
        let failed = SavedChunk::new(block::STONE, 47);
        let healthy = SavedChunk::new(block::DIRT, 48);
        let a = failed.world(37);
        let b = healthy.world(41);
        let bad_job = a.reserve_generation(POSITION.0, POSITION.1).unwrap();
        let poisoned = std::panic::catch_unwind(|| {
            let _guard = a.inner.payloads[payload_shard(POSITION.0, POSITION.1)]
                .write()
                .unwrap();
            panic!("inject a poisoned payload shard");
        });
        assert!(poisoned.is_err());
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || run_generation_queue(rx));
        bad_job.send(&tx);
        b.reserve_generation(POSITION.0, POSITION.1)
            .unwrap()
            .send(&tx);
        drop(tx);
        worker
            .join()
            .expect("dispatcher must survive a failed world job");
        assert!(a.inner.in_flight.lock().unwrap().is_empty());
        healthy.assert_cached(&b, &wait_cached(&b));
    }
}
