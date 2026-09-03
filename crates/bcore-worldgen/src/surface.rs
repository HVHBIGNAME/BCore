//! Vanilla-like overworld surface rules.

use crate::biome::BiomeId;
use crate::{block, MIN_Y};

/// A network block-state id (the default state of a block).
pub type BlockState = u32;

/// Extra ids for biomes commonly present in the vanilla report.  Registry ids
/// can be replaced by the integration layer without changing the rules API.
pub mod biome_ids {
    pub const BADLANDS: u32 = 2;
    pub const BEACH: u32 = 3;
    pub const DESERT: u32 = 14;
    pub const ERODED_BADLANDS: u32 = 18;
    pub const FROZEN_OCEAN: u32 = 22;
    pub const MUSHROOM_FIELDS: u32 = 34;
    pub const OCEAN: u32 = 35;
    pub const RIVER: u32 = 41;
    pub const SNOWY_PLAINS: u32 = 46;
    pub const SNOWY_SLOPES: u32 = 47;
    pub const WOODED_BADLANDS: u32 = 64;
}

const RED_SAND: BlockState = 123;
const TERRACOTTA: BlockState = 12_912;
const MYCELIUM: BlockState = 8_919;

fn is_badlands(biome: BiomeId) -> bool {
    matches!(
        biome,
        biome_ids::BADLANDS | biome_ids::ERODED_BADLANDS | biome_ids::WOODED_BADLANDS
    )
}
fn is_snowy(biome: BiomeId) -> bool {
    matches!(
        biome,
        biome_ids::SNOWY_PLAINS | biome_ids::SNOWY_SLOPES | biome_ids::FROZEN_OCEAN
    )
}
fn is_water_biome(biome: BiomeId) -> bool {
    matches!(
        biome,
        biome_ids::OCEAN | biome_ids::FROZEN_OCEAN | biome_ids::RIVER
    )
}

/// Returns the block placed at `y` for a column whose terrain top is `top_y`.
/// `height` is retained in the public API because surface rules can use the
/// sampled terrain height; it is currently equivalent to `top_y` for rules.
pub fn surface_block(
    biome: BiomeId,
    y: i32,
    height: i32,
    top_y: i32,
    sea_level: i32,
) -> BlockState {
    if y <= MIN_Y + 4 {
        return block::BEDROCK;
    }
    let depth = top_y - y;
    let effective_top = if height == 0 {
        top_y
    } else {
        height.max(top_y)
    };
    // 1.18+ deepslate replaces stone below Y=0.
    let stone = if y < 0 {
        block::DEEPSLATE
    } else {
        block::STONE
    };
    if is_water_biome(biome) && y >= effective_top && y < sea_level {
        return block::WATER;
    }
    if depth < 0 {
        return block::AIR;
    }

    if biome == biome_ids::DESERT {
        if depth < 3 {
            block::SAND
        } else {
            block::SANDSTONE
        }
    } else if is_snowy(biome) {
        if depth == 0 {
            block::SNOW_BLOCK
        } else if depth <= 4 {
            block::DIRT
        } else {
            stone
        }
    } else if is_badlands(biome) {
        if depth < 3 {
            RED_SAND
        } else {
            TERRACOTTA
        }
    } else if biome == biome_ids::MUSHROOM_FIELDS {
        if depth == 0 {
            MYCELIUM
        } else if depth <= 4 {
            block::DIRT
        } else {
            stone
        }
    } else if is_water_biome(biome) {
        if depth < 2 {
            block::SAND
        } else {
            block::GRAVEL
        }
    } else if depth == 0 {
        block::GRASS_BLOCK
    } else if depth <= 4 {
        block::DIRT
    } else {
        stone
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bedrock_is_floor() {
        assert_eq!(surface_block(0, -64, 70, 70, 63), block::BEDROCK);
    }
    #[test]
    fn desert_has_sandstone_under_three_sand() {
        assert_eq!(
            surface_block(biome_ids::DESERT, 71, 73, 73, 63),
            block::SAND
        );
        assert_eq!(
            surface_block(biome_ids::DESERT, 70, 73, 73, 63),
            block::SANDSTONE
        );
    }
    #[test]
    fn snowy_and_mushroom_tops() {
        assert_eq!(
            surface_block(biome_ids::SNOWY_PLAINS, 70, 70, 70, 63),
            block::SNOW_BLOCK
        );
        assert_eq!(
            surface_block(biome_ids::MUSHROOM_FIELDS, 70, 70, 70, 63),
            MYCELIUM
        );
    }
    #[test]
    fn ocean_fills_to_sea_and_has_bottom() {
        assert_eq!(
            surface_block(biome_ids::OCEAN, 62, 50, 50, 63),
            block::WATER
        );
        assert_eq!(surface_block(biome_ids::OCEAN, 49, 50, 50, 63), block::SAND);
    }
}
