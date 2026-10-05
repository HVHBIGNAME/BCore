//! Immutable terrain cache for isolated component fixtures only.
//! Live generation retains mutable chunks in GenerationWorld and never consults
//! this cache when satisfying a source's status dependencies.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use crate::{ChunkPos, GeneratedChunk, WorldGenerator};

type Key = (i64, i32, i32);
type Slot = Arc<OnceLock<Arc<GeneratedChunk>>>;
const CAPACITY: usize = 64;

#[derive(Default)]
struct TerrainCache {
    entries: VecDeque<(Key, Slot)>,
}

impl TerrainCache {
    fn slot(&mut self, key: Key) -> Slot {
        if let Some(index) = self.entries.iter().position(|(k, _)| *k == key) {
            let entry = self.entries.remove(index).unwrap();
            let slot = entry.1.clone();
            self.entries.push_back(entry);
            return slot;
        }
        let slot = Arc::new(OnceLock::new());
        if self.entries.len() == CAPACITY {
            if let Some(index) = self
                .entries
                .iter()
                .position(|(_, slot)| slot.get().is_some())
            {
                self.entries.remove(index);
            } else {
                // All retained entries are being generated. This request gets
                // an unretained slot rather than evicting an in-flight build.
                return slot;
            }
        }
        self.entries.push_back((key, slot.clone()));
        slot
    }
}

pub(crate) fn get(generator: WorldGenerator, pos: ChunkPos) -> Arc<GeneratedChunk> {
    static CACHE: OnceLock<Mutex<TerrainCache>> = OnceLock::new();
    let slot = CACHE
        .get_or_init(|| Mutex::new(TerrainCache::default()))
        .lock()
        .expect("terrain cache poisoned")
        .slot((generator.seed(), pos.x, pos.z));
    // Never hold the cache mutex during terrain generation or a Rayon join.
    slot.get_or_init(|| Arc::new(generator.generate_chunk_before_features(pos)))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    #[test]
    fn mutable_feature_chunks_do_not_change_cached_terrain() {
        let mut cache = TerrainCache::default();
        let slot = cache.slot((42, -2, 3));
        let mut chunk = slot
            .get_or_init(|| Arc::new(GeneratedChunk::new(ChunkPos::new(-2, 3))))
            .clone();
        Arc::make_mut(&mut chunk).set(15, -63, 0, block::DIAMOND_ORE);
        let cached = cache.slot((42, -2, 3));
        assert!(Arc::ptr_eq(&slot, &cached));
        assert_eq!(cached.get().unwrap().get(15, -63, 0), Some(block::AIR));
        assert!(!Arc::ptr_eq(&slot, &cache.slot((43, -2, 3))));
    }

    #[test]
    fn cache_is_bounded_and_keeps_in_flight_entries() {
        let mut cache = TerrainCache::default();
        let first = cache.slot((0, 0, 0));
        for i in 1..CAPACITY as i32 + 10 {
            cache.slot((0, i, 0));
        }
        assert_eq!(cache.entries.len(), CAPACITY);
        assert!(Arc::ptr_eq(&first, &cache.slot((0, 0, 0))));
        first
            .set(Arc::new(GeneratedChunk::new(ChunkPos::new(0, 0))))
            .unwrap();
        cache.slot((0, 1000, 0));
        assert_eq!(cache.entries.len(), CAPACITY);
        assert!(!cache.entries.iter().any(|(key, _)| *key == (0, 0, 0)));
    }
}
