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
/// `wx`/`wz`/`seed` are required for the bedrock-floor vertical gradient.
pub fn surface_block(
    biome: BiomeId,
    y: i32,
    height: i32,
    top_y: i32,
    sea_level: i32,
    wx: i32,
    wz: i32,
    seed: i64,
) -> BlockState {
    if bedrock_floor(wx, y, wz, seed) {
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

/// Vanilla bedrock floor: a `vertical_gradient` surface rule.
///
/// `y <= -64` is always bedrock; `y >= -59` never; in between the chance falls
/// linearly (`probability = (-59 - y) / 5`) against a positional random derived
/// from `randomName = "minecraft:bedrock_floor"`.
fn bedrock_floor(x: i32, y: i32, z: i32, seed: i64) -> bool {
    if y <= MIN_Y {
        return true;
    }
    if y >= MIN_Y + 5 {
        return false;
    }
    let probability = (MIN_Y + 5 - y) as f64 / 5.0;
    // randomFactory = fromHashOf("minecraft:bedrock_floor").forkPositional()
    let fork_seed = crate::simplex::fork_seed_for(seed);
    let bedrock_seed =
        crate::noise_perlin::java_string_hash("minecraft:bedrock_floor") as i64 ^ fork_seed;
    let mut r = crate::simplex::JavaRandom::new(bedrock_seed);
    let positional_seed = r.next_long();
    // at(x, y, z) = LegacyRandomSource(Mth.getSeed(x, y, z) ^ positionalSeed)
    let at_seed = mth_get_seed(x, y, z) ^ positional_seed;
    let mut at_rng = crate::simplex::JavaRandom::new(at_seed);
    (at_rng.next_float() as f64) < probability
}

/// Vanilla `Mth.getSeed`.
fn mth_get_seed(x: i32, y: i32, z: i32) -> i64 {
    let mut i = ((x as i64) * 3129871) ^ ((z as i64) * 116129781) ^ (y as i64);
    i = i
        .wrapping_mul(i)
        .wrapping_mul(42317861)
        .wrapping_add(i.wrapping_mul(11));
    i >> 16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bedrock_is_floor() {
        assert_eq!(
            surface_block(0, -64, 70, 70, 63, 0, 0, 1234),
            block::BEDROCK
        );
    }
    #[test]
    fn desert_has_sandstone_under_three_sand() {
        assert_eq!(
            surface_block(biome_ids::DESERT, 71, 73, 73, 63, 0, 0, 1234),
            block::SAND
        );
        assert_eq!(
            surface_block(biome_ids::DESERT, 70, 73, 73, 63, 0, 0, 1234),
            block::SANDSTONE
        );
    }
    #[test]
    fn snowy_and_mushroom_tops() {
        assert_eq!(
            surface_block(biome_ids::SNOWY_PLAINS, 70, 70, 70, 63, 0, 0, 1234),
            block::SNOW_BLOCK
        );
        assert_eq!(
            surface_block(biome_ids::MUSHROOM_FIELDS, 70, 70, 70, 63, 0, 0, 1234),
            MYCELIUM
        );
    }
    #[test]
    fn ocean_fills_to_sea_and_has_bottom() {
        assert_eq!(
            surface_block(biome_ids::OCEAN, 62, 50, 50, 63, 0, 0, 1234),
            block::WATER
        );
        assert_eq!(
            surface_block(biome_ids::OCEAN, 49, 50, 50, 63, 0, 0, 1234),
            block::SAND
        );
    }
}
