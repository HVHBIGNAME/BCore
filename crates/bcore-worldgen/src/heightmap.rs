//! Native placement predicates and the separate WG/final heightmap lifetimes.
use std::sync::OnceLock;

use crate::feature_world::FeatureHeightmap;
use crate::{GeneratedChunk, MAX_Y, MIN_Y};

const NOT_AIR: u8 = 1;
const BLOCKS_MOTION: u8 = 2;
const SUPPORTS_VEGETATION: u8 = 4;
const PROTECTED_BELOW_TRUNK: u8 = 8;
const DATA: &str = include_str!("../data/heightmaps_26_2.json");

fn state_flags() -> &'static [u8] {
    static FLAGS: OnceLock<Vec<u8>> = OnceLock::new();
    FLAGS.get_or_init(|| {
        let data: serde_json::Value =
            crate::block_predicate::parse_catalog(DATA).expect("authenticated block predicates");
        decode_state_flags(&data)
    })
}

fn decode_state_flags(data: &serde_json::Value) -> Vec<u8> {
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
}

pub(crate) fn supports_sapling(state: u32) -> bool {
    state_flags()[state as usize] & SUPPORTS_VEGETATION != 0
}

pub(crate) fn protected_below_trunk(state: u32) -> bool {
    state_flags()[state as usize] & PROTECTED_BELOW_TRUNK != 0
}

/// Air, void air and cave air in the shared block-state registry.
pub const fn is_air(state: u32) -> bool {
    matches!(
        state,
        bcore_registry::block_state::AIR
            | bcore_registry::block_state::VOID_AIR
            | bcore_registry::block_state::CAVE_AIR
    )
}

pub(crate) fn blocks_motion(state: u32) -> bool {
    state_flags()[state as usize] & BLOCKS_MOTION != 0
}

/// Heights name the first free Y above the relevant block; an empty map is MIN_Y.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PlacementHeights {
    pub world_surface: i32,
    pub ocean_floor: i32,
}

/// First-free WG heights, updated through CARVERS and retained during decoration.
/// Native ProtoChunk switches to the final heightmap set once its persisted
/// status reaches CARVERS; later feature writes must not change these two maps.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorldgenHeightmaps {
    pub world_surface: Vec<i32>,
    pub ocean_floor: Vec<i32>,
    frozen: bool,
}

impl WorldgenHeightmaps {
    pub(crate) fn capture(chunk: &GeneratedChunk, frozen: bool) -> Self {
        let mut world_surface = Vec::with_capacity(256);
        let mut ocean_floor = Vec::with_capacity(256);
        for z in 0..16 {
            for x in 0..16 {
                let heights = placement_heights(chunk, x, z);
                world_surface.push(heights.world_surface);
                ocean_floor.push(heights.ocean_floor);
            }
        }
        Self {
            world_surface,
            ocean_floor,
            frozen,
        }
    }

    pub(crate) fn update(&mut self, states: &[u32], x: usize, y: i32, z: usize, state: u32) {
        if self.frozen {
            return;
        }
        let flags = state_flags();
        let index = z * 16 + x;
        for (mask, heights) in [
            (NOT_AIR, &mut self.world_surface),
            (BLOCKS_MOTION, &mut self.ocean_floor),
        ] {
            update_column(heights, flags, mask, states, index, y, state);
        }
    }
}

/// Runtime maps retain first-free heights; `ChunkAccess.getHeight` subtracts one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHeightmaps {
    pub world_surface: Vec<i32>,
    pub ocean_floor: Vec<i32>,
    pub motion_blocking: Vec<i32>,
    pub motion_blocking_no_leaves: Vec<i32>,
}

fn runtime_flags() -> &'static [u8] {
    static FLAGS: OnceLock<Vec<u8>> = OnceLock::new();
    FLAGS.get_or_init(|| runtime_flags_for(crate::block_predicate::catalog(), state_flags().len()))
}

fn runtime_flags_for(catalog: &crate::block_predicate::FeatureCatalog, count: usize) -> Vec<u8> {
    let mut flags = vec![0; count];
    for definition in catalog.blocks.values() {
        let leaves = definition.class.ends_with("LeavesBlock");
        for state in definition.first..definition.first + definition.count {
            let info = catalog.info(state).expect("native heightmap state");
            let motion = info.blocks_motion() || info.fluid != catalog.fluids["minecraft:empty"];
            flags[state as usize] = u8::from(!info.is_air())
                | (u8::from(info.blocks_motion()) << 1)
                | (u8::from(motion) << 2)
                | (u8::from(motion && !leaves) << 3);
        }
    }
    flags
}

impl RuntimeHeightmaps {
    pub(crate) fn capture(chunk: &GeneratedChunk) -> Self {
        let flags = runtime_flags();
        let mut maps: [Vec<i32>; 4] = std::array::from_fn(|_| vec![MIN_Y; 256]);
        for index in 0..256 {
            let mut found = 0_u8;
            for y in (MIN_Y..=MAX_Y).rev() {
                let state = chunk.states()[(y - MIN_Y) as usize * 256 + index];
                let bits = flags[state as usize] & !found;
                for (map, heights) in maps.iter_mut().enumerate() {
                    if bits & (1 << map) != 0 {
                        heights[index] = y + 1;
                    }
                }
                found |= bits;
                if found == 15 {
                    break;
                }
            }
        }
        let [world_surface, ocean_floor, motion_blocking, motion_blocking_no_leaves] = maps;
        Self {
            world_surface,
            ocean_floor,
            motion_blocking,
            motion_blocking_no_leaves,
        }
    }

    /// WG requests on a converted holder use the corresponding runtime map.
    pub fn first_free(&self, kind: FeatureHeightmap, x: usize, z: usize) -> Option<i32> {
        if x >= 16 || z >= 16 {
            return None;
        }
        let map = match kind {
            FeatureHeightmap::WorldSurface | FeatureHeightmap::WorldSurfaceWg => {
                &self.world_surface
            }
            FeatureHeightmap::OceanFloor | FeatureHeightmap::OceanFloorWg => &self.ocean_floor,
            FeatureHeightmap::MotionBlocking => &self.motion_blocking,
            FeatureHeightmap::MotionBlockingNoLeaves => &self.motion_blocking_no_leaves,
        };
        Some(map[z * 16 + x])
    }

    pub(crate) fn update(&mut self, states: &[u32], x: usize, y: i32, z: usize, state: u32) {
        for (index, heights) in [
            &mut self.world_surface,
            &mut self.ocean_floor,
            &mut self.motion_blocking,
            &mut self.motion_blocking_no_leaves,
        ]
        .into_iter()
        .enumerate()
        {
            update_column(
                heights,
                runtime_flags(),
                1 << index,
                states,
                z * 16 + x,
                y,
                state,
            );
        }
    }
}

fn update_column(
    heights: &mut [i32],
    flags: &[u8],
    mask: u8,
    states: &[u32],
    index: usize,
    y: i32,
    state: u32,
) {
    if flags[state as usize] & mask != 0 {
        heights[index] = heights[index].max(y + 1);
    } else if y == heights[index] - 1 {
        heights[index] = (MIN_Y..y)
            .rev()
            .find(|&lower| {
                let block = states[(lower - MIN_Y) as usize * 256 + index];
                flags[block as usize] & mask != 0
            })
            .map_or(MIN_Y, |lower| lower + 1);
    }
}

pub(crate) fn placement_heights(chunk: &GeneratedChunk, x: usize, z: usize) -> PlacementHeights {
    assert!(x < 16 && z < 16, "invalid placement column");
    if let Some(heights) = chunk.runtime_heightmaps() {
        return PlacementHeights {
            world_surface: heights.world_surface[z * 16 + x],
            ocean_floor: heights.ocean_floor[z * 16 + x],
        };
    }
    column_heights(chunk.states(), state_flags(), z * 16 + x)
}

fn column_heights(states: &[u32], flags: &[u8], index: usize) -> PlacementHeights {
    let mut world_surface = MIN_Y;
    for y in (MIN_Y..=MAX_Y).rev() {
        let state = states[(y - MIN_Y) as usize * 256 + index];
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
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../data/heightmaps_26_1.json")).unwrap();
        let flags = decode_state_flags(&data);
        for sample in data["samples"].as_array().unwrap() {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(-2, 3));
            let base = sample["base_y"].as_i64().unwrap() as i32;
            for (dy, state) in sample["states"].as_array().unwrap().iter().enumerate() {
                chunk.set(8, base + dy as i32, 8, state.as_u64().unwrap() as u32);
            }
            let heights = column_heights(chunk.states(), &flags, 8 * 16 + 8);
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
                flags[below as usize] & SUPPORTS_VEGETATION != 0,
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

    #[test]
    fn runtime_predicates_match_the_independent_native_standing_tree_table() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../data/standing_blocks_26_1.json")).unwrap();
        let heightmaps =
            serde_json::from_str(include_str!("../data/heightmaps_26_1.json")).unwrap();
        let wg_flags = decode_state_flags(&heightmaps);
        let catalog = crate::block_predicate::FeatureCatalog::from_value(
            serde_json::from_str(include_str!("../data/base_feature_catalog_26_1.json")).unwrap(),
        )
        .unwrap();
        let flags = runtime_flags_for(&catalog, wg_flags.len());
        for row in data["ranges"].as_array().unwrap() {
            let start = row[0].as_u64().unwrap() as usize;
            let end = row[1].as_u64().unwrap() as usize;
            let no_leaves = row[2].as_u64().unwrap() & 2 != 0;
            for state in start..end {
                assert_eq!(flags[state] & 8 != 0, no_leaves, "native state {state}");
                assert_eq!(
                    flags[state] & 3,
                    wg_flags[state] & 3,
                    "native WG predicates {state}"
                );
            }
        }
    }

    #[test]
    fn runtime_maps_update_without_thawing_wg_maps() {
        let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
        chunk.set(8, 40, 8, block::STONE);
        chunk.capture_worldgen_heightmaps(true);
        chunk.set(8, 80, 8, block::OAK_LEAVES);
        chunk.set(8, 60, 8, block::WATER);
        let maps = chunk.runtime_heightmaps().unwrap();
        assert_eq!(maps.world_surface[136], 81);
        assert_eq!(maps.ocean_floor[136], 81);
        assert_eq!(maps.motion_blocking[136], 81);
        assert_eq!(maps.motion_blocking_no_leaves[136], 61);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().world_surface[136], 41);
        chunk.set(8, 80, 8, block::AIR);
        assert_eq!(chunk.runtime_heightmaps().unwrap().world_surface[136], 61);
        assert_eq!(chunk.runtime_heightmaps().unwrap().ocean_floor[136], 41);
        chunk.set(8, 60, 8, block::AIR);
        chunk.set(8, 40, 8, block::AIR);
        let maps = chunk.runtime_heightmaps().unwrap();
        assert_eq!(maps.world_surface[136], MIN_Y);
        assert_eq!(maps.motion_blocking_no_leaves[136], MIN_Y);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().world_surface[136], 41);
    }

    #[test]
    fn wg_update_boundary_matches_the_captured_native_status_sets() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../data/feature_dependencies_26_1.json")).unwrap();
        for status in data["statuses"].as_array().unwrap() {
            let index = status["index"].as_u64().unwrap() as usize;
            let updates_wg = status["heightmaps_after"]
                .as_array()
                .unwrap()
                .iter()
                .any(|name| name == "WORLD_SURFACE_WG");
            assert_eq!(
                updates_wg,
                index < crate::generation::ChunkStatus::Carvers.index()
            );
        }
    }

    #[test]
    fn initialized_wg_maps_follow_pre_carvers_writes_and_removals() {
        let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
        chunk.set(8, 40, 8, block::DIRT);
        chunk.capture_worldgen_heightmaps(false);
        chunk.set(8, 48, 8, block::WATER);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().world_surface[136], 49);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().ocean_floor[136], 41);
        chunk.set(8, 45, 8, block::OAK_LEAVES);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().ocean_floor[136], 46);
        chunk.set(8, 45, 8, block::AIR);
        chunk.set(8, 48, 8, block::AIR);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().world_surface[136], 41);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().ocean_floor[136], 41);
        chunk.set(8, 40, 8, block::AIR);
        assert_eq!(chunk.worldgen_heightmaps().unwrap().ocean_floor[136], MIN_Y);
    }

    #[test]
    fn feature_region_keeps_wg_heights_separate_from_live_decoration() {
        use crate::feature_world::{FeatureHeightmap, FeatureWorld};
        use crate::generation::ChunkStatus;
        use crate::ore::OreWorld;
        use crate::region::FeatureRegion;

        let pos = ChunkPos::new(0, 0);
        let mut region = FeatureRegion::shared(crate::WorldGenerator::new(0));
        let chunk = region.owned_chunk_mut(pos);
        chunk.set(15, 125, 6, block::GRASS_BLOCK);
        chunk.capture_worldgen_heightmaps(true);
        region.begin_source(
            pos,
            ChunkStatus::Features,
            [((0, 0), ChunkStatus::Carvers)].into(),
        );
        assert!(region.set_feature_block((15, 132, 6), block::OAK_LEAVES, 2));
        // This 126/133 distinction is the captured patch_grass_forest witness.
        assert_eq!(
            region.feature_height(FeatureHeightmap::WorldSurfaceWg, 15, 6),
            126
        );
        assert_eq!(
            region.feature_height(FeatureHeightmap::WorldSurface, 15, 6),
            133
        );
        assert_eq!(region.ocean_floor_wg(15, 6), 126);
        assert_eq!(
            region.feature_height(FeatureHeightmap::OceanFloor, 15, 6),
            133
        );
        assert!(region.set_feature_block((15, 125, 6), block::AIR, 2));
        assert_eq!(
            region.feature_height(FeatureHeightmap::WorldSurfaceWg, 15, 6),
            126
        );
        region.end_source();
    }
}
