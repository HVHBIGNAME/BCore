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
use std::sync::{Mutex, OnceLock};

use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;

use crate::chunk::ChunkColumn;
use crate::chunk_store::ChunkStore;

/// The default world seed. `/seed` reports this.
pub const DEFAULT_SEED: i64 = 0x1A93_2A57_9B13_2D98u64 as i64;

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
    payloads: Mutex<HashMap<(i32, i32), Vec<u8>>>,
    /// Set once the first persistence error has been reported.
    warned: Mutex<bool>,
}

impl World {
    /// A world that generates terrain and persists it under `world/`.
    pub fn new(seed: i64) -> Self {
        Self {
            generator: WorldGenerator::new(seed),
            store: Some(ChunkStore::new()),
            payloads: Mutex::new(HashMap::new()),
            warned: Mutex::new(false),
        }
    }

    /// A world rooted at a specific directory (used by tests).
    pub fn with_store(seed: i64, store: ChunkStore) -> Self {
        Self {
            generator: WorldGenerator::new(seed),
            store: Some(store),
            payloads: Mutex::new(HashMap::new()),
            warned: Mutex::new(false),
        }
    }

    /// A world that never touches the disk (used by tests and benchmarks).
    pub fn in_memory(seed: i64) -> Self {
        Self {
            generator: WorldGenerator::new(seed),
            store: None,
            payloads: Mutex::new(HashMap::new()),
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

        let generated = self.generator.generate_chunk(ChunkPos::new(x, z));
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
        ChunkColumn::from_generated(&self.generator.generate_chunk(ChunkPos::new(x, z)))
    }

    /// The encoded `map_chunk` payload for `(x, z)`, cached across calls.
    pub fn chunk_payload(&self, x: i32, z: i32) -> Vec<u8> {
        if let Some(hit) = self
            .payloads
            .lock()
            .expect("world payload lock")
            .get(&(x, z))
        {
            return hit.clone();
        }
        let (column, _origin) = self.chunk(x, z);
        let payload = column.encode_payload(x, z);
        self.payloads
            .lock()
            .expect("world payload lock")
            .insert((x, z), payload.clone());
        payload
    }

    /// Drop cached payloads (used when a test wants to force a re-read).
    pub fn clear_cache(&self) {
        self.payloads.lock().expect("world payload lock").clear();
    }

    /// How many payloads are currently cached.
    pub fn cached_payloads(&self) -> usize {
        self.payloads.lock().expect("world payload lock").len()
    }

    /// A safe spawn position: the terrain surface at `(x, z)`, plus one block.
    pub fn spawn_position(&self, x: f64, z: f64) -> (f64, f64, f64) {
        let y = self.generator.spawn_y(x.floor() as i32, z.floor() as i32);
        (x, y, z)
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
    fn the_shared_world_is_a_singleton() {
        let a = shared();
        let b = shared();
        assert!(std::ptr::eq(a, b));
        assert_eq!(a.seed(), DEFAULT_SEED);
    }
}
