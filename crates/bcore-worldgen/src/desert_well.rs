//! Vanilla 26.1 DesertWellFeature. Placement modifiers and the configured-origin
//! guard belong to the caller; all writes and archaeology lookups stay ordered.
use crate::block_predicate::{catalog, offset, Direction, FeatureResult};
use crate::feature_world::{FeatureError, FeatureWorld, Pos};
use crate::simplex::WorldgenRandom;

fn read(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<u32> {
    world
        .get_block(pos)
        .ok_or_else(|| FeatureError::MissingData(format!("desert well block at {pos:?}")))
}

fn is_empty(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
    Ok(catalog().info(read(world, pos)?)?.is_air())
}

pub fn place(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
) -> FeatureResult<bool> {
    let sand = catalog().default_state("sand")?;
    let sandstone = catalog().default_state("sandstone")?;
    let slab = catalog().default_state("sandstone_slab")?;
    let water = catalog().default_state("water")?;
    let suspicious = catalog().default_state("suspicious_sand")?;

    let mut center = offset(origin, (0, 1, 0));
    // Keep the native short-circuit order, including the last air query at minY+2.
    while is_empty(world, center)? && center.1 > crate::MIN_Y + 2 {
        center = offset(center, (0, -1, 0));
    }
    if read(world, center)? != sand {
        return Ok(false);
    }
    for x in -2..=2 {
        for z in -2..=2 {
            if is_empty(world, offset(center, (x, -1, z)))?
                && is_empty(world, offset(center, (x, -2, z)))?
            {
                return Ok(false);
            }
        }
    }
    for y in -2..=0 {
        for x in -2..=2 {
            for z in -2..=2 {
                world.set_feature_block(offset(center, (x, y, z)), sandstone, 2);
            }
        }
    }
    world.set_feature_block(center, water, 2);
    for direction in Direction::HORIZONTAL {
        world.set_feature_block(direction.step(center), water, 2);
    }
    let below = offset(center, (0, -1, 0));
    world.set_feature_block(below, sand, 2);
    for direction in Direction::HORIZONTAL {
        world.set_feature_block(direction.step(below), sand, 2);
    }
    for x in -2..=2 {
        for z in -2..=2 {
            if x == -2 || x == 2 || z == -2 || z == 2 {
                world.set_feature_block(offset(center, (x, 1, z)), sandstone, 2);
            }
        }
    }
    for delta in [(2, 1, 0), (-2, 1, 0), (0, 1, 2), (0, 1, -2)] {
        world.set_feature_block(offset(center, delta), slab, 2);
    }
    for x in -1..=1 {
        for z in -1..=1 {
            let state = if x == 0 && z == 0 { sandstone } else { slab };
            world.set_feature_block(offset(center, (x, 4, z)), state, 2);
        }
    }
    for y in 1..=3 {
        for (x, z) in [(-1, -1), (-1, 1), (1, -1), (1, 1)] {
            world.set_feature_block(offset(center, (x, y, z)), sandstone, 2);
        }
    }
    // Util.getRandom uses this List.of order, not Direction.Plane.HORIZONTAL.
    let candidates = [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1)];
    for depth in [1, 2] {
        let (x, z) = candidates[random.next_int(candidates.len())];
        let pos = offset(center, (x, -depth, z));
        world.set_feature_block(pos, suspicious, 3);
        // getBlockEntity is unconditional: a rejected write may leave an existing
        // brushable (including suspicious gravel) whose loot is still changed.
        let packed = ((pos.0 as i64 & 0x3ff_ffff) << 38)
            | ((pos.2 as i64 & 0x3ff_ffff) << 12)
            | (pos.1 as i64 & 0xfff);
        world.set_feature_brushable_loot(pos, "minecraft:archaeology/desert_well", packed)?;
    }
    Ok(true)
}
