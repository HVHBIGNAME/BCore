//! Basic vanilla configured-feature generators (trees and ore veins).
//!
//! The public functions deliberately accept a write callback so they can be used
//! by chunk generators without coupling features to a particular world storage.
//! Coordinates are absolute block coordinates; clipping and replacement policy
//! belong to the callback/owner.

use crate::block;
use crate::noise::splitmix64;

/// Network block-state type used by the world generator.
pub type BlockState = u32;

/// Tree configurations represented by the vanilla configured features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKind {
    Oak,
    Birch,
    Spruce,
    Pine,
}

/// Ore configurations represented by the vanilla configured features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OreKind {
    Coal,
    Iron,
    Copper,
    Gold,
    Diamond,
    Emerald,
    Lapis,
}

/// Place a simple tree at `(x, y, z)`, where `y` is the ground block.
///
/// `rng` is a mutable SplitMix64 state. It is intentionally a plain integer:
/// bcore-worldgen has no random dependency and callers can seed features from
/// the world seed and absolute position. Heights and foliage dimensions mirror
/// the vanilla `configured_feature` tree placers.
pub fn place_tree(
    rng: &mut u64,
    world_write: &mut dyn FnMut(i32, i32, i32, BlockState),
    x: i32,
    y: i32,
    z: i32,
    kind: TreeKind,
) {
    let (log, leaves, height, foliage_radius) = match kind {
        // straight_trunk_placer: base_height 4, height_rand_a 2.
        TreeKind::Oak => (
            block::OAK_LOG,
            block::OAK_LEAVES,
            4 + next_range(rng, 0, 2),
            2 + next_range(rng, 0, 1),
        ),
        // straight_trunk_placer: base_height 5, height_rand_a 2.
        TreeKind::Birch => (
            block::BIRCH_LOG,
            block::BIRCH_LEAVES,
            5 + next_range(rng, 0, 2),
            2,
        ),
        // Spruce: base 5 + [0,2] + [0,1]; spruce foliage radius [2,3].
        TreeKind::Spruce => (
            block::SPRUCE_LOG,
            block::SPRUCE_LEAVES,
            5 + next_range(rng, 0, 2) + next_range(rng, 0, 1),
            2 + next_range(rng, 0, 1),
        ),
        // Pine: base_height 6, height_rand_a 4.
        TreeKind::Pine => (
            block::SPRUCE_LOG,
            block::SPRUCE_LEAVES,
            6 + next_range(rng, 0, 4),
            1,
        ),
    };

    for dy in 1..=height {
        world_write(x, y + dy, z, log);
    }

    match kind {
        TreeKind::Spruce | TreeKind::Pine => {
            // Wider toward the bottom, narrowing to a one-block tip.
            for dy in 0..height {
                let radius = if foliage_radius != 0 {
                    foliage_radius.min(2)
                } else if dy < 2 {
                    2
                } else {
                    ((height - dy + 1) / 3).max(0)
                };
                for dz in -radius..=radius {
                    for dx in -radius..=radius {
                        if dx.abs() + dz.abs() <= radius + 1 {
                            world_write(x + dx, y + height - dy, z + dz, leaves);
                        }
                    }
                }
            }
            world_write(x, y + height + 1, z, leaves);
        }
        TreeKind::Oak | TreeKind::Birch => {
            // Blob foliage placer: two broad layers and a smaller upper layer.
            for (dy, radius) in [
                (height - 1, foliage_radius),
                (height, foliage_radius),
                (height + 1, (foliage_radius - 1).max(1)),
            ] {
                for dz in -radius..=radius {
                    for dx in -radius..=radius {
                        if radius == 2 && dx.abs() == 2 && dz.abs() == 2 {
                            continue;
                        }
                        world_write(x + dx, y + dy, z + dz, leaves);
                    }
                }
            }
            world_write(x, y + height + 2, z, leaves);
        }
    }
}

/// Place a blob-shaped ore vein centered at `(x, y, z)`.
///
/// `size` is the configured-feature cluster size (vanilla examples are 4--17).
/// The callback receives exactly `size` writes, with duplicates removed. The
/// shape is deterministic for the supplied RNG state and uses the vanilla ore
/// state IDs from `crate::block`; emerald is from the same `blocks.json` report
/// because the current block module does not expose that constant.
pub fn place_ore(
    rng: &mut u64,
    world_write: &mut dyn FnMut(i32, i32, i32, BlockState),
    x: i32,
    y: i32,
    z: i32,
    kind: OreKind,
    size: usize,
) {
    if size == 0 {
        return;
    }
    let state = ore_state(kind);
    let mut points: Vec<(i32, i32, i32)> = Vec::with_capacity(size);
    points.push((x, y, z));
    while points.len() < size {
        let anchor = points[(next_u64(rng) as usize) % points.len()];
        let axis = (next_u64(rng) % 3) as i32;
        let step = if next_u64(rng) & 1 == 0 { -1 } else { 1 };
        let mut p = anchor;
        match axis {
            0 => p.0 += step,
            1 => p.1 += step,
            _ => p.2 += step,
        }
        if !points.contains(&p) {
            points.push(p);
        }
    }
    for (px, py, pz) in points {
        world_write(px, py, pz, state);
    }
}

#[inline]
fn next_u64(state: &mut u64) -> u64 {
    *state = splitmix64(*state);
    *state
}

fn next_range(state: &mut u64, min: i32, max: i32) -> i32 {
    min + (next_u64(state) % (max - min + 1) as u64) as i32
}

#[inline]
fn ore_state(kind: OreKind) -> BlockState {
    match kind {
        OreKind::Coal => block::COAL_ORE,
        OreKind::Iron => block::IRON_ORE,
        OreKind::Copper => block::COPPER_ORE,
        OreKind::Gold => block::GOLD_ORE,
        OreKind::Diamond => block::DIAMOND_ORE,
        // blocks.json: minecraft:emerald_ore default state id 9573.
        OreKind::Emerald => 9573,
        OreKind::Lapis => block::LAPIS_ORE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_is_deterministic_and_has_expected_trunk() {
        let mut a = Vec::new();
        let mut seed = 1234;
        place_tree(
            &mut seed,
            &mut |x, y, z, state| a.push((x, y, z, state)),
            10,
            64,
            -3,
            TreeKind::Birch,
        );
        let mut b = Vec::new();
        let mut seed = 1234;
        place_tree(
            &mut seed,
            &mut |x, y, z, state| b.push((x, y, z, state)),
            10,
            64,
            -3,
            TreeKind::Birch,
        );
        assert_eq!(a, b);
        assert!(a.iter().filter(|p| p.3 == block::BIRCH_LOG).count() >= 5);
        assert!(a.iter().any(|p| p.3 == block::BIRCH_LEAVES));
    }

    #[test]
    fn ore_size_and_state_are_deterministic() {
        let mut vein = Vec::new();
        let mut seed = 7;
        place_ore(
            &mut seed,
            &mut |x, y, z, state| vein.push((x, y, z, state)),
            0,
            0,
            0,
            OreKind::Emerald,
            8,
        );
        assert_eq!(vein.len(), 8);
        assert!(vein.iter().all(|p| p.3 == 9573));
        assert_eq!(vein[0], (0, 0, 0, 9573));
    }
}
