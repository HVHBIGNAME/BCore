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
//! Fresh generation shares status claims, structure starts and neighbouring
//! feature writes for the lifetime of this world. Saved columns are admitted into
//! the same canonical region before fresh generation. Native generation history
//! and world registration are not stored in BCC v6.
//!
//! Persistence errors stop admission/loading. An unreadable or incompatible
//! saved chunk is never treated as an absent chunk.

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
        if let Err(error) = self.world.try_chunk_payload(x, z) {
            eprintln!("[bcore] queued chunk ({x}, {z}) failed: {error}");
        }
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
use bcore_worldgen::generation::{
    graph::layer_positions,
    request::{NativeChunkRequests, NativeRequestError},
    ChunkPyramid, ChunkStatus, ReturnedChunk, StageState,
};
use bcore_worldgen::{
    block, generation::ChunkProgress, GenerationCoverage, GenerationError, GenerationResult,
    GenerationWorld, WorldGenerator,
};

use crate::chunk::ChunkColumn;
use crate::chunk_store::{ChunkStore, ChunkStoreError};

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

/// Loading/persistence errors are distinct from generation-only failures.
#[derive(Debug, Clone)]
pub enum WorldError {
    Generation(GenerationError),
    Native(NativeRequestError),
    Store(Arc<ChunkStoreError>),
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generation(error) => write!(f, "{error}"),
            Self::Native(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WorldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Generation(error) => Some(error),
            Self::Native(error) => Some(error),
            Self::Store(error) => Some(error.as_ref()),
        }
    }
}

impl From<GenerationError> for WorldError {
    fn from(error: GenerationError) -> Self {
        Self::Generation(error)
    }
}

impl From<ChunkStoreError> for WorldError {
    fn from(error: ChunkStoreError) -> Self {
        Self::Store(Arc::new(error))
    }
}

impl From<NativeRequestError> for WorldError {
    fn from(error: NativeRequestError) -> Self {
        match error {
            NativeRequestError::Generation(error) => Self::Generation(error),
            error => Self::Native(error),
        }
    }
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
    generation: Arc<GenerationWorld>,
    native_requests: OnceLock<Result<NativeChunkRequests, NativeRequestError>>,
    store: Option<ChunkStore>,
    /// Encoded `map_chunk` payloads, keyed by chunk position.
    payloads: [PayloadShard; PAYLOAD_SHARDS],
    /// Coalesce concurrent cache misses before loading/generating the same chunk.
    generation_locks: [Mutex<()>; PAYLOAD_SHARDS],
    in_flight: Mutex<HashSet<(i32, i32)>>,
    payload_errors: Mutex<HashMap<(i32, i32), WorldError>>,
    entities: crate::entity::EntityTracker,
    warned_generation: OnceLock<()>,
}

impl World {
    /// A world that generates terrain and persists it under `world/`.
    pub fn new(seed: i64) -> Self {
        Self::open(seed, ChunkStore::new())
            .unwrap_or_else(|error| panic!("cannot admit the default world: {error}"))
    }

    /// Low-level world over a chunk store. Live server admission uses [`Self::open`].
    pub fn with_store(seed: i64, store: ChunkStore) -> Self {
        Self::with_optional_store(seed, Some(store))
    }

    /// Admit an existing authenticated world or initialize an empty directory.
    /// Validation precedes generation, queue startup, and any chunk writes.
    pub fn open(seed: i64, store: ChunkStore) -> Result<Self, ChunkStoreError> {
        store.prepare_world(seed)?;
        let store = ChunkStore::at(std::fs::canonicalize(store.root())?);
        Ok(Self::with_store(seed, store))
    }

    pub fn prepare_store(&self) -> Result<(), ChunkStoreError> {
        if let Some(store) = self.store() {
            store.prepare_world(self.seed())?;
        }
        Ok(())
    }

    /// A world that never touches the disk (used by tests and benchmarks).
    pub fn in_memory(seed: i64) -> Self {
        Self::with_optional_store(seed, None)
    }

    fn with_optional_store(seed: i64, store: Option<ChunkStore>) -> Self {
        Self {
            inner: Arc::new(WorldInner {
                generator: WorldGenerator::new(seed),
                generation: Arc::new(GenerationWorld::new(seed)),
                native_requests: OnceLock::new(),
                store,
                payloads: std::array::from_fn(|_| RwLock::new(HashMap::new())),
                generation_locks: std::array::from_fn(|_| Mutex::new(())),
                in_flight: Mutex::new(HashSet::new()),
                payload_errors: Mutex::new(HashMap::new()),
                entities: Default::default(),
                warned_generation: OnceLock::new(),
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

    /// Native-style fresh-world admission/cache/main-executor route. The first
    /// caller becomes its owner thread; subsequent admission and pumping must run
    /// there. `try_generate` remains the generation-only compatibility route.
    pub fn native_requests(&self) -> Result<&NativeChunkRequests, NativeRequestError> {
        self.inner
            .native_requests
            .get_or_init(|| NativeChunkRequests::new(self.inner.generation.clone()))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Returns the actual object selected by the four-slot native request cache.
    /// Use `ReturnedChunk::view` for an inert snapshot or `prepare_save` for its
    /// explicit save boundary, then `ChunkColumn::from_generated` on that output.
    /// This route shares the world's production generation claims and storage.
    pub fn request_native_chunk(
        &self,
        x: i32,
        z: i32,
        status: ChunkStatus,
        create: bool,
    ) -> Result<Option<ReturnedChunk>, WorldError> {
        let pos = ChunkPos::new(x, z);
        NativeChunkRequests::validate_position(pos)?;
        if let Some(requests) = self.inner.native_requests.get() {
            requests.as_ref().map_err(Clone::clone)?.check_admission()?;
        }
        if create {
            self.hydrate_saved_neighbors(pos)?;
        }
        let requests = self.native_requests()?;
        if self.inner.generation.is_saved_chunk(pos)? {
            requests.admit_saved_chunk(pos)?;
        }
        Ok(requests.get_chunk(pos, status, create)?)
    }

    /// The chunk store, if this world persists chunks.
    pub fn store(&self) -> Option<&ChunkStore> {
        self.inner.store.as_ref()
    }

    /// Load a chunk from disk, or generate (and save) it.
    ///
    /// Returns the column and where it came from.
    pub fn chunk(&self, x: i32, z: i32) -> (ChunkColumn, ChunkOrigin) {
        self.try_chunk(x, z).unwrap_or_else(|error| {
            panic!(
                "cannot load or generate chunk ({x}, {z}) for seed {}: {error}",
                self.seed()
            )
        })
    }

    /// Load or generate, releasing the shard lock before returning either error.
    pub fn try_chunk(&self, x: i32, z: i32) -> Result<(ChunkColumn, ChunkOrigin), WorldError> {
        let pos = ChunkPos::new(x, z);
        NativeChunkRequests::validate_position(pos)?;
        let _generation = self.inner.generation_locks[payload_shard(x, z)]
            .lock()
            .expect("chunk generation lock");
        if let Some(store) = &self.inner.store {
            if let Some(column) = store.load(x, z)? {
                if self
                    .inner
                    .generation
                    .chunk_access(pos, ChunkStatus::Full)?
                    .is_none()
                {
                    self.inner
                        .generation
                        .admit_saved_chunks(vec![column.into_saved_data(pos)])?;
                }
                let chunk = self
                    .inner
                    .generation
                    .chunk_snapshot(pos)?
                    .expect("admitted level chunk");
                return Ok((ChunkColumn::from_generated(&chunk), ChunkOrigin::Loaded));
            }
        }

        self.hydrate_saved_neighbors(pos)?;
        let result = self.generate_retained(x, z)?;
        let column = if self.inner.store.is_some() {
            // Generation queues are still pending; no world clock has unpacked them.
            self.prepare_generated_chunk_save(x, z, 0)?
        } else {
            ChunkColumn::from_generated(&result.chunk)
        };

        if let Some(store) = &self.inner.store {
            store.save(x, z, &column)?;
        }
        Ok((column, ChunkOrigin::Generated))
    }

    fn hydrate_saved_neighbors(&self, pos: ChunkPos) -> Result<(), WorldError> {
        let Some(store) = &self.inner.store else {
            return Ok(());
        };
        let radius = ChunkPyramid::Generation
            .step(ChunkStatus::Full)
            .accumulated
            .radius();
        let mut saved = Vec::new();
        for source in layer_positions(pos, radius) {
            if self
                .inner
                .generation
                .progress(source)?
                .is_some_and(|progress| {
                    progress
                        .stages
                        .iter()
                        .any(|stage| stage.state != StageState::Pending)
                })
            {
                continue;
            }
            if let Some(column) = store.load(source.x, source.z)? {
                saved.push(column.into_saved_data(source));
            }
        }
        self.inner.generation.admit_saved_chunks(saved)?;
        Ok(())
    }

    /// Generate a chunk without consulting or touching the disk.
    pub fn generate(&self, x: i32, z: i32) -> ChunkColumn {
        self.try_generate(x, z)
            .unwrap_or_else(|error| {
                panic!(
                    "cannot generate chunk ({x}, {z}) for seed {}: {error}",
                    self.seed()
                )
            })
            .0
    }

    /// Generate using this world's retained history and return its actual coverage.
    /// Publication is a snapshot; saving or explicit lookup materializes pending BEs.
    pub fn try_generate(
        &self,
        x: i32,
        z: i32,
    ) -> Result<(ChunkColumn, GenerationCoverage), GenerationError> {
        let result = self.generate_retained(x, z)?;
        Ok((ChunkColumn::from_generated(&result.chunk), result.coverage))
    }

    fn generate_retained(&self, x: i32, z: i32) -> Result<GenerationResult, GenerationError> {
        let result = self.inner.generation.generate_chunk(ChunkPos::new(x, z))?;
        if !result.coverage.is_complete() {
            self.inner.warned_generation.get_or_init(|| {
                eprintln!(
                    "[bcore] world seed {} has partial native generation coverage at ({x}, {z}): \
                     {} missing feature streams, {} missing stage entries; \
                     World::try_generate returns the detailed coverage",
                    self.seed(),
                    result.coverage.missing_features().count(),
                    result.coverage.missing_stages.len(),
                );
            });
        }
        Ok(result)
    }

    /// Explicit level-save lookup on an already retained generated chunk. This
    /// materializes supported pending BEs but performs no generation or disk I/O.
    /// `ChunkStore::save` itself remains an inert BCC snapshot encoder.
    pub fn prepare_generated_chunk_save(
        &self,
        x: i32,
        z: i32,
        game_time: i64,
    ) -> Result<ChunkColumn, GenerationError> {
        let saved = self
            .inner
            .generation
            .prepare_chunk_save(ChunkPos::new(x, z), game_time)?;
        Ok(ChunkColumn::from_generated(&saved.chunk))
    }

    /// Null-player lazy-inventory access on this world's retained level chunk.
    /// Coordinates are absolute blocks. Saving uses prepare_generated_chunk_save;
    /// this does not admit unloaded chunks, select player context or create menus.
    pub fn unpack_generated_chest(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<Option<bcore_worldgen::block_entity::ChestInventory>, GenerationError> {
        self.inner.generation.unpack_chest(
            ChunkPos::new(x.div_euclid(16), z.div_euclid(16)),
            (x.rem_euclid(16) as usize, y, z.rem_euclid(16) as usize),
        )
    }

    /// Live generation claims; saved admission has Partial data and zero attempts.
    pub fn generation_progress(
        &self,
        x: i32,
        z: i32,
    ) -> Result<Option<ChunkProgress>, GenerationError> {
        self.inner.generation.progress(ChunkPos::new(x, z))
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
        if self.cached_chunk(x, z).is_some() || self.payload_error(x, z).is_some() {
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
        self.try_chunk_payload(x, z).unwrap_or_else(|error| {
            panic!(
                "cannot encode chunk ({x}, {z}) for seed {}: {error}",
                self.seed()
            )
        })
    }

    pub fn try_chunk_payload(&self, x: i32, z: i32) -> Result<Vec<u8>, WorldError> {
        if let Some(hit) = self.cached_chunk(x, z) {
            return Ok(hit.payload.as_ref().clone());
        }
        match self.try_chunk(x, z) {
            Ok((column, _origin)) => Ok(self.cache_column(x, z, &column).payload.as_ref().clone()),
            Err(error) => {
                self.inner
                    .payload_errors
                    .lock()
                    .expect("payload error lock")
                    .insert((x, z), error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn payload_error(&self, x: i32, z: i32) -> Option<WorldError> {
        self.inner
            .payload_errors
            .lock()
            .expect("payload error lock")
            .get(&(x, z))
            .cloned()
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
        self.inner
            .payload_errors
            .lock()
            .expect("payload error lock")
            .remove(&(x, z));
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
        self.inner
            .payload_errors
            .lock()
            .expect("payload error lock")
            .clear();
    }

    /// How many payloads are currently cached.
    pub fn cached_payloads(&self) -> usize {
        self.inner
            .payloads
            .iter()
            .map(|shard| shard.read().expect("world payload shard lock").len())
            .sum()
    }

    /// Reserved queued jobs, including the dispatcher job currently running.
    pub fn pending_payloads(&self) -> usize {
        self.inner
            .in_flight
            .lock()
            .expect("generation reservation lock")
            .len()
    }

    /// A safe spawn position: the terrain surface at `(x, z)`, plus one block.
    pub fn spawn_position(&self, x: f64, z: f64) -> (f64, f64, f64) {
        self.try_spawn_position(x, z)
            .unwrap_or_else(|error| panic!("cannot select spawn for seed {}: {error}", self.seed()))
    }

    pub fn try_spawn_position(&self, x: f64, z: f64) -> Result<(f64, f64, f64), WorldError> {
        let bx = x.floor() as i32;
        let bz = z.floor() as i32;
        let (column, _) = self.try_chunk(bx.div_euclid(16), bz.div_euclid(16))?;
        let y = column
            .surface_y(bx.rem_euclid(16) as usize, bz.rem_euclid(16) as usize)
            .unwrap_or(crate::chunk::SEA_LEVEL)
            + 1;
        Ok((x, y as f64, z))
    }

    /// A spawn on the nearest column whose actual vanilla-generated surface is
    /// land above sea level. The search retains the original chunk spiral, but
    /// never uses the approximate noise height for this decision or Y value.
    pub fn land_spawn(&self, x0: i32, z0: i32) -> (f64, f64, f64) {
        self.try_land_spawn(x0, z0).unwrap_or_else(|error| {
            panic!("cannot select land spawn for seed {}: {error}", self.seed())
        })
    }

    pub fn try_land_spawn(&self, x0: i32, z0: i32) -> Result<(f64, f64, f64), WorldError> {
        let (mut bx, mut bz) = (x0, z0);
        let actual_surface = |x: i32, z: i32| -> Result<Option<(i32, u32)>, WorldError> {
            let cx = x.div_euclid(16);
            let cz = z.div_euclid(16);
            let lx = x.rem_euclid(16) as usize;
            let lz = z.rem_euclid(16) as usize;
            let (chunk, _) = self.try_chunk(cx, cz)?;
            Ok(chunk
                .surface_y(lx, lz)
                .map(|y| (y, chunk.get(lx, y, lz).expect("surface block"))))
        };
        let is_land = |x: i32, z: i32| -> Result<bool, WorldError> {
            Ok(actual_surface(x, z)?.is_some_and(|(y, state)| {
                y >= crate::chunk::SEA_LEVEL && state != block::WATER && state != block::LAVA
            }))
        };
        if is_land(bx, bz)? {
            let y = actual_surface(bx, bz)?.expect("land surface").0 + 1;
            return Ok((bx as f64, y as f64, bz as f64));
        }
        let mut radius = 1;
        'outer: loop {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    if dx != radius && dx != -radius && dz != radius && dz != -radius {
                        continue;
                    }
                    let (tx, tz) = (bx + dx * 16, bz + dz * 16);
                    if is_land(tx, tz)? {
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
        let y = actual_surface(bx, bz)?
            .map(|(surface, _)| surface)
            .unwrap_or(crate::chunk::SEA_LEVEL)
            + 1;
        Ok((bx as f64, y as f64, bz as f64))
    }
}

/// Compatibility world for callers without a server owner. It never persists;
/// live connections use their `ServerState`'s world instead.
pub fn shared() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| World::in_memory(DEFAULT_SEED))
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
        // A second call reuses the live generated data without loading a file.
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
        let chunk = world.generate(cx, cz);
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
        assert!(
            a.store().is_none(),
            "compatibility world must not touch disk"
        );
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
    fn corrupt_queued_load_releases_its_reservation_without_generating_or_overwriting() {
        let saved = SavedChunk::new(block::STONE, 50);
        let world = saved.world(47);
        let path = saved.store.chunk_path(POSITION.0, POSITION.1);
        let original = std::fs::read(&path).unwrap();
        let mut corrupt = original.clone();
        let middle = corrupt.len() / 2;
        corrupt[middle] ^= 1;
        std::fs::write(&path, &corrupt).unwrap();
        world
            .reserve_generation(POSITION.0, POSITION.1)
            .unwrap()
            .run();
        assert_eq!(world.pending_payloads(), 0);
        assert!(matches!(
            world.payload_error(POSITION.0, POSITION.1),
            Some(WorldError::Store(error))
                if matches!(error.as_ref(), ChunkStoreError::ChecksumMismatch { .. })
        ));
        assert!(world.cached_chunk(POSITION.0, POSITION.1).is_none());
        assert!(world
            .generation_progress(POSITION.0, POSITION.1)
            .unwrap()
            .is_none());
        assert!(world.reserve_generation(POSITION.0, POSITION.1).is_none());
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);

        // Repair is an explicit operation; cache invalidation then permits retry.
        std::fs::write(&path, original).unwrap();
        world.clear_cache();
        world
            .reserve_generation(POSITION.0, POSITION.1)
            .unwrap()
            .run();
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

    #[test]
    fn rejected_generation_does_not_poison_other_positions_in_the_same_world_shard() {
        let saved = SavedChunk::new(block::STONE, 49);
        let world = saved.world(43);
        let invalid_x = (0..1024)
            .map(|offset| i32::MAX - offset)
            .find(|&x| payload_shard(x, POSITION.1) == payload_shard(POSITION.0, POSITION.1))
            .unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || run_generation_queue(rx));
        world
            .reserve_generation(invalid_x, POSITION.1)
            .unwrap()
            .send(&tx);
        world
            .reserve_generation(POSITION.0, POSITION.1)
            .unwrap()
            .send(&tx);
        drop(tx);
        worker
            .join()
            .expect("dispatcher must survive a rejected coordinate");
        assert!(world.inner.in_flight.lock().unwrap().is_empty());
        assert!(world
            .generation_progress(invalid_x, POSITION.1)
            .unwrap()
            .is_none());
        saved.assert_cached(&world, &wait_cached(&world));
    }
}
