//! Live placement heights and sapling support, using extracted 26.1 predicates.
use std::sync::OnceLock;

use crate::{GeneratedChunk, MAX_Y, MIN_Y};

const NOT_AIR: u8 = 1;
const BLOCKS_MOTION: u8 = 2;
const SUPPORTS_VEGETATION: u8 = 4;
const PROTECTED_BELOW_TRUNK: u8 = 8;
const DATA: &str = include_str!("../data/heightmaps_26_1.json");

fn state_flags() -> &'static [u8] {
    static FLAGS: OnceLock<Vec<u8>> = OnceLock::new();
    FLAGS.get_or_init(|| {
        let data: serde_json::Value =
            serde_json::from_str(DATA).expect("invalid bundled block predicates");
        let count = data["state_count"].as_u64().unwrap() as usize;
        let mut flags = Vec::with_capacity(count);
        for row in data["ranges"].as_array().unwrap() {
            let start = row[0].as_u64().unwrap() as usize;
            let end = row[1].as_u64().unwrap() as usize;
            let value = row[2].as_u64().unwrap();
            assert!(start == flags.len() && end > start && end <= count && value <= 7);
            flags.resize(end, value as u8);
        }
        assert_eq!(flags.len(), count);
        for state in data["protected_trunk_states"].as_array().unwrap() {
            flags[state.as_u64().unwrap() as usize] |= PROTECTED_BELOW_TRUNK;
        }
        flags
    })
}

pub(crate) fn supports_sapling(state: u32) -> bool {
    state_flags()[state as usize] & SUPPORTS_VEGETATION != 0
}

pub(crate) fn protected_below_trunk(state: u32) -> bool {
    state_flags()[state as usize] & PROTECTED_BELOW_TRUNK != 0
}

/// Air, void air and cave air in the pinned 26.1 block-state registry.
pub const fn is_air(state: u32) -> bool {
    matches!(state, 0 | 15292 | 15293)
}

/// Heights name the first free Y above the relevant block; an empty map is MIN_Y.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PlacementHeights {
    pub world_surface: i32,
    pub ocean_floor: i32,
}

pub(crate) fn placement_heights(chunk: &GeneratedChunk, x: usize, z: usize) -> PlacementHeights {
    let flags = state_flags();
    let mut world_surface = MIN_Y;
    for y in (MIN_Y..=MAX_Y).rev() {
        let state = chunk.get(x, y, z).expect("invalid placement column");
        let bits = flags[state as usize];
        if world_surface == MIN_Y && bits & NOT_AIR != 0 {
            world_surface = y + 1;
        }
        if bits & BLOCKS_MOTION != 0 {
            return PlacementHeights {
                world_surface,
                ocean_floor: y + 1,
            };
        }
    }
    PlacementHeights {
        world_surface,
        ocean_floor: MIN_Y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{block, ChunkPos};

    #[test]
    fn air_ids_match_all_native_state_predicates() {
        for (id, &flags) in state_flags().iter().enumerate() {
            assert_eq!(is_air(id as u32), flags & NOT_AIR == 0, "state {id}");
        }
    }

    #[test]
    fn column_heights_and_sapling_support_match_26_1_predicates() {
        let data: serde_json::Value = serde_json::from_str(DATA).unwrap();
        for sample in data["samples"].as_array().unwrap() {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(-2, 3));
            let base = sample["base_y"].as_i64().unwrap() as i32;
            for (dy, state) in sample["states"].as_array().unwrap().iter().enumerate() {
                chunk.set(8, base + dy as i32, 8, state.as_u64().unwrap() as u32);
            }
            let heights = placement_heights(&chunk, 8, 8);
            assert_eq!(
                heights.world_surface,
                sample["world_surface"].as_i64().unwrap() as i32
            );
            assert_eq!(
                heights.ocean_floor,
                sample["ocean_floor"].as_i64().unwrap() as i32
            );
            let below = chunk
                .get(8, heights.ocean_floor - 1, 8)
                .unwrap_or(block::AIR);
            assert_eq!(
                supports_sapling(below),
                sample["sapling_survives"].as_bool().unwrap()
            );
        }
    }

    #[test]
    fn placement_heights_follow_writes_and_removals() {
        let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
        chunk.set(8, 40, 8, block::DIRT);
        assert_eq!(placement_heights(&chunk, 8, 8).ocean_floor, 41);
        chunk.set(8, 48, 8, block::OAK_LEAVES);
        assert_eq!(placement_heights(&chunk, 8, 8).ocean_floor, 49);
        chunk.set(8, 48, 8, block::AIR);
        assert_eq!(placement_heights(&chunk, 8, 8).ocean_floor, 41);
        assert_eq!(chunk.height_at(8, 8), MIN_Y);
    }
}
