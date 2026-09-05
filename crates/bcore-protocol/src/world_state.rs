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

use std::collections::HashMap;
use std::sync::{mpsc, Mutex, OnceLock, RwLock};
use std::thread;

const PAYLOAD_SHARDS: usize = 32;
const MAX_CACHED_PAYLOADS: usize = 8192;
type PayloadShard = RwLock<HashMap<(i32, i32), Vec<u8>>>;

type GenerationJob = (i32, i32);
static GENERATION_QUEUE: OnceLock<mpsc::Sender<GenerationJob>> = OnceLock::new();
static GENERATION_IN_FLIGHT: OnceLock<Mutex<std::collections::HashSet<GenerationJob>>> =
    OnceLock::new();

fn generation_queue() -> &'static mpsc::Sender<GenerationJob> {
    GENERATION_QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<GenerationJob>();
        let rx = std::sync::Arc::new(Mutex::new(rx));
        for _ in 0..std::thread::available_parallelism().map_or(2, |n| n.get().min(8)) {
            let rx = rx.clone();
            thread::spawn(move || loop {
                let job = rx.lock().expect("generation queue lock").recv();
                let Ok((x, z)) = job else { break };
                let _ = shared().chunk_payload(x, z);
                if let Some(in_flight) = GENERATION_IN_FLIGHT.get() {
                    in_flight
                        .lock()
                        .expect("generation in-flight lock")
                        .remove(&(x, z));
                }
            });
        }
        tx
    })
}

fn generation_in_flight() -> &'static Mutex<std::collections::HashSet<GenerationJob>> {
    GENERATION_IN_FLIGHT.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
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

/// A seeded world with a chunk cache and optional disk persistence.
#[derive(Debug)]
pub struct World {
    generator: WorldGenerator,
    store: Option<ChunkStore>,
    /// Encoded `map_chunk` payloads, keyed by chunk position.
    payloads: [PayloadShard; PAYLOAD_SHARDS],
    /// Set once the first persistence error has been reported.
    warned: Mutex<bool>,
}

impl World {
    /// A world that generates terrain and persists it under `world/`.
    pub fn new(seed: i64) -> Self {
        Self {
            generator: WorldGenerator::new(seed),
            store: Some(ChunkStore::new()),
            payloads: std::array::from_fn(|_| RwLock::new(HashMap::new())),
            warned: Mutex::new(false),
        }
    }

    /// A world rooted at a specific directory (used by tests).
    pub fn with_store(seed: i64, store: ChunkStore) -> Self {
        Self {
            generator: WorldGenerator::new(seed),
            store: Some(store),
            payloads: std::array::from_fn(|_| RwLock::new(HashMap::new())),
            warned: Mutex::new(false),
        }
    }

    /// A world that never touches the disk (used by tests and benchmarks).
    pub fn in_memory(seed: i64) -> Self {
        Self {
            generator: WorldGenerator::new(seed),
            store: None,
            payloads: std::array::from_fn(|_| RwLock::new(HashMap::new())),
            warned: Mutex::new(false),
        }
    }

    /// The seed this world generates from.
    pub fn seed(&self) -> i64 {
        self.generator.seed()
    }

    /// The generator, for callers that need raw heights (e.g. spawn selection).
    pub fn generator(&self) -> WorldGenerator {
        self.generator
    }

    /// The chunk store, if this world persists chunks.
    pub fn store(&self) -> Option<&ChunkStore> {
        self.store.as_ref()
    }

    /// Report a persistence problem once, then stay quiet.
    fn warn_once(&self, context: &str, error: &dyn std::fmt::Display) {
        let mut warned = self.warned.lock().expect("world warn lock");
        if !*warned {
            *warned = true;
            eprintln!("[bcore] world persistence disabled for this run: {context}: {error}");
        }
    }

    /// Load a chunk from disk, or generate (and save) it.
    ///
    /// Returns the column and where it came from.
    pub fn chunk(&self, x: i32, z: i32) -> (ChunkColumn, ChunkOrigin) {
        if let Some(store) = &self.store {
            match store.load(x, z) {
                Ok(Some(column)) => return (column, ChunkOrigin::Loaded),
                Ok(None) => {}
                Err(e) => self.warn_once(&format!("cannot read chunk ({x}, {z})"), &e),
            }
        }

        let generated = self.generator.generate_chunk_vanilla(ChunkPos::new(x, z));
        let column = ChunkColumn::from_generated(&generated);

        if let Some(store) = &self.store {
            if let Err(e) = store.save(x, z, &column) {
                self.warn_once(&format!("cannot write chunk ({x}, {z})"), &e);
            }
        }
        (column, ChunkOrigin::Generated)
    }

    /// Generate a chunk without consulting or touching the disk.
    pub fn generate(&self, x: i32, z: i32) -> ChunkColumn {
        ChunkColumn::from_generated(&self.generator.generate_chunk_vanilla(ChunkPos::new(x, z)))
    }

    /// Return a cached payload without doing world generation.
    pub fn cached_payload(&self, x: i32, z: i32) -> Option<Vec<u8>> {
        self.payloads[payload_shard(x, z)]
            .read()
            .expect("world payload shard lock")
            .get(&(x, z))
            .cloned()
    }

    /// Queue generation for a chunk, returning immediately.
    pub fn request_payload(&self, x: i32, z: i32) {
        if self.cached_payload(x, z).is_some() {
            return;
        }
        let key = (x, z);
        let mut pending = generation_in_flight()
            .lock()
            .expect("generation in-flight lock");
        if pending.insert(key) {
            let _ = generation_queue().send(key);
        }
    }

    /// Queue generation for all requested chunks without blocking the caller.
    pub fn request_payloads<I: IntoIterator<Item = (i32, i32)>>(&self, chunks: I) {
        for (x, z) in chunks {
            self.request_payload(x, z);
        }
    }

    /// The encoded `map_chunk` payload for `(x, z)`, cached across calls.
    pub fn chunk_payload(&self, x: i32, z: i32) -> Vec<u8> {
        let shard = &self.payloads[payload_shard(x, z)];
        if let Some(hit) = shard.read().expect("world payload shard lock").get(&(x, z)) {
            return hit.clone();
        }
        let (column, _origin) = self.chunk(x, z);
        let payload = column.encode_payload(x, z);
        shard
            .write()
            .expect("world payload shard lock")
            .entry((x, z))
            .or_insert_with(|| payload.clone());
        if self.cached_payloads() > MAX_CACHED_PAYLOADS {
            self.clear_cache();
        }
        payload
    }

    /// Drop cached payloads (used when a test wants to force a re-read).
    pub fn clear_cache(&self) {
        for shard in &self.payloads {
            shard.write().expect("world payload shard lock").clear();
        }
    }

    /// How many payloads are currently cached.
    pub fn cached_payloads(&self) -> usize {
        self.payloads
            .iter()
            .map(|shard| shard.read().expect("world payload shard lock").len())
            .sum()
    }

    /// A safe spawn position: the terrain surface at `(x, z)`, plus one block.
    pub fn spawn_position(&self, x: f64, z: f64) -> (f64, f64, f64) {
        let y = self.generator.spawn_y(x.floor() as i32, z.floor() as i32);
        (x, y, z)
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
            let chunk = self.generator.generate_chunk_vanilla(ChunkPos::new(cx, cz));
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
            world.payloads[payload_shard(i as i32, 0)]
                .write()
                .expect("shard lock")
                .insert((i as i32, 0), vec![0u8; 4]);
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
