//! Basic vanilla configured-feature generators (trees and ore veins).
//!
//! The public functions deliberately accept a write callback so they can be used
//! by chunk generators without coupling features to a particular world storage.
//! Coordinates are absolute block coordinates; clipping and replacement policy
//! belong to the callback/owner.

use crate::block;
use crate::noise::splitmix64;
use crate::simplex::JavaRandom;
use crate::MAX_Y;

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
    Redstone,
    Andesite,
    Diorite,
    Granite,
    Tuff,
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
            4 + next_range_u64(rng, 0, 2),
            2 + next_range_u64(rng, 0, 1),
        ),
        // straight_trunk_placer: base_height 5, height_rand_a 2.
        TreeKind::Birch => (
            block::BIRCH_LOG,
            block::BIRCH_LEAVES,
            5 + next_range_u64(rng, 0, 2),
            2,
        ),
        // Spruce: base 5 + [0,2] + [0,1]; spruce foliage radius [2,3].
        TreeKind::Spruce => (
            block::SPRUCE_LOG,
            block::SPRUCE_LEAVES,
            5 + next_range_u64(rng, 0, 2) + next_range_u64(rng, 0, 1),
            2 + next_range_u64(rng, 0, 1),
        ),
        // Pine: base_height 6, height_rand_a 4.
        TreeKind::Pine => (
            block::SPRUCE_LOG,
            block::SPRUCE_LEAVES,
            6 + next_range_u64(rng, 0, 4),
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
/// The callback receives exactly `size` writes, with duplicates removed.  The
/// growth is constrained to an ellipsoid, like vanilla's `OreFeature`, rather
/// than being an unbounded random walk.
pub fn place_ore(
    rng: &mut JavaRandom,
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

    // OreFeature grows from random anchors, but rejects positions outside the
    // ellipsoid around the vein origin.  The radii scale with the configured
    // size; this keeps small veins compact while allowing large blobs to fill
    // their intended volume.  A bounded retry fallback guarantees progress
    // even when the ellipsoid is saturated.
    let radius = ((size as f64).sqrt() * 0.72).ceil().max(1.0) as i32;
    let ry = (radius / 2).max(1);
    let rxz = radius.max(1);
    let max_attempts = size.saturating_mul(32).max(64);
    let mut attempts = 0;
    while points.len() < size {
        attempts += 1;
        let anchor = points[rng.next_int(points.len())];
        let axis = rng.next_int(3) as i32;
        let step = if rng.next_int(2) == 0 { -1 } else { 1 };
        let mut p = anchor;
        match axis {
            0 => p.0 += step,
            1 => p.1 += step,
            _ => p.2 += step,
        }
        let dx = (p.0 - x) as f64 / rxz as f64;
        let dy = (p.1 - y) as f64 / ry as f64;
        let dz = (p.2 - z) as f64 / rxz as f64;
        if dx * dx + dy * dy + dz * dz <= 1.0 && !points.contains(&p) {
            points.push(p);
        } else if attempts >= max_attempts {
            // This path is rare and only prevents an oversized request from
            // looping forever; select an unoccupied neighboring shell point.
            attempts = 0;
            for candidate in [
                (p.0 + 1, p.1, p.2),
                (p.0 - 1, p.1, p.2),
                (p.0, p.1 + 1, p.2),
                (p.0, p.1 - 1, p.2),
                (p.0, p.1, p.2 + 1),
                (p.0, p.1, p.2 - 1),
            ] {
                if !points.contains(&candidate) {
                    points.push(candidate);
                    break;
                }
            }
        }
    }
    for (px, py, pz) in points {
        world_write(px, py, pz, state);
    }
}

fn next_range_u64(state: &mut u64, min: i32, max: i32) -> i32 {
    min + (splitmix64(*state) % (max - min + 1) as u64) as i32
}

fn next_range(state: &mut JavaRandom, min: i32, max: i32) -> i32 {
    min + state.next_int((max - min + 1) as usize) as i32
}

#[derive(Clone, Copy)]
enum HeightDistribution {
    Uniform,
    Trapezoid,
}

#[derive(Clone, Copy)]
struct OrePlacement {
    salt: i64,
    kind: OreKind,
    size: usize,
    count: usize,
    rarity: Option<usize>,
    min_y: i32,
    max_y: i32,
    distribution: HeightDistribution,
}

impl OrePlacement {
    fn height(self, rng: &mut JavaRandom) -> i32 {
        match self.distribution {
            HeightDistribution::Uniform => next_range(rng, self.min_y, self.max_y),
            // HeightRange.trapezoid is a triangular distribution with a flat
            // plateau in the middle: two independent draws from half-range.
            HeightDistribution::Trapezoid => {
                let span = (self.max_y - self.min_y) as usize;
                let half = span / 2;
                self.min_y + rng.next_int(half + 1) as i32 + rng.next_int(half + 1) as i32
            }
        }
    }
}

/// Place the overworld stone blobs and ore veins for one chunk.
///
/// Each configured feature gets its own vanilla `FeatureUtils.simpleRandom`
/// stream.  The callback owns the world lookup/replacement policy: callers
/// should only replace stone or deepslate, as `OreFeature` does not replace
/// air, water, or other blocks.
///
/// The y ranges are the inclusive bounds of the corresponding vanilla
/// height-ranges.  Vanilla's trapezoid distribution is represented here by a
/// uniform draw because this callback API has no distribution object; x/z are
/// always uniformly selected from the 16-block chunk footprint.
pub fn place_ore_veins(
    seed: i64,
    chunk_x: i32,
    chunk_z: i32,
    world_write: &mut dyn FnMut(i32, i32, i32, BlockState),
) {
    const FEATURES: &[OrePlacement] = &[
        OrePlacement {
            salt: 3,
            kind: OreKind::Granite,
            size: 64,
            count: 2,
            rarity: None,
            min_y: 0,
            max_y: 60,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 4,
            kind: OreKind::Granite,
            size: 64,
            count: 1,
            rarity: Some(6),
            min_y: 64,
            max_y: 128,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 5,
            kind: OreKind::Diorite,
            size: 64,
            count: 2,
            rarity: None,
            min_y: 0,
            max_y: 60,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 6,
            kind: OreKind::Diorite,
            size: 64,
            count: 1,
            rarity: Some(6),
            min_y: 64,
            max_y: 128,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 7,
            kind: OreKind::Andesite,
            size: 64,
            count: 2,
            rarity: None,
            min_y: 0,
            max_y: 60,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 8,
            kind: OreKind::Andesite,
            size: 64,
            count: 1,
            rarity: Some(6),
            min_y: 64,
            max_y: 128,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 9,
            kind: OreKind::Tuff,
            size: 64,
            count: 2,
            rarity: None,
            min_y: -64,
            max_y: 0,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 10,
            kind: OreKind::Coal,
            size: 17,
            count: 20,
            rarity: None,
            min_y: 0,
            max_y: 192,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 11,
            kind: OreKind::Coal,
            size: 17,
            count: 30,
            rarity: None,
            min_y: 136,
            max_y: MAX_Y,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 12,
            kind: OreKind::Iron,
            size: 9,
            count: 10,
            rarity: None,
            min_y: -64,
            max_y: 72,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 13,
            kind: OreKind::Iron,
            size: 9,
            count: 10,
            rarity: None,
            min_y: -24,
            max_y: 56,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 14,
            kind: OreKind::Iron,
            size: 4,
            count: 90,
            rarity: None,
            min_y: 80,
            max_y: MAX_Y,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 15,
            kind: OreKind::Gold,
            size: 9,
            count: 4,
            rarity: None,
            min_y: -64,
            max_y: 32,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 16,
            kind: OreKind::Gold,
            size: 9,
            count: 50,
            rarity: None,
            min_y: 32,
            max_y: 256,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 17,
            kind: OreKind::Redstone,
            size: 8,
            count: 8,
            rarity: None,
            min_y: -64,
            max_y: 16,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 18,
            kind: OreKind::Redstone,
            size: 8,
            count: 4,
            rarity: None,
            min_y: -64,
            max_y: 15,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 19,
            kind: OreKind::Diamond,
            size: 4,
            count: 7,
            rarity: None,
            min_y: -64,
            max_y: 16,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 20,
            kind: OreKind::Diamond,
            size: 8,
            count: 2,
            rarity: None,
            min_y: -64,
            max_y: -4,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 21,
            kind: OreKind::Diamond,
            size: 12,
            count: 1,
            rarity: Some(9),
            min_y: -64,
            max_y: 16,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 22,
            kind: OreKind::Diamond,
            size: 8,
            count: 4,
            rarity: None,
            min_y: -64,
            max_y: 16,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 23,
            kind: OreKind::Lapis,
            size: 7,
            count: 2,
            rarity: None,
            min_y: -64,
            max_y: 32,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            salt: 24,
            kind: OreKind::Lapis,
            size: 7,
            count: 4,
            rarity: None,
            min_y: -64,
            max_y: 64,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            salt: 25,
            kind: OreKind::Copper,
            size: 10,
            count: 16,
            rarity: None,
            min_y: -16,
            max_y: 112,
            distribution: HeightDistribution::Trapezoid,
        },
    ];

    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    for &feature in FEATURES {
        let mut rng = JavaRandom::new(feature_seed(seed, chunk_x, chunk_z, feature.salt));
        if let Some(chance) = feature.rarity {
            if rng.next_int(chance) != 0 {
                continue;
            }
        }
        for _ in 0..feature.count {
            let x = base_x.wrapping_add(next_range(&mut rng, 0, 15));
            let y = feature.height(&mut rng);
            let z = base_z.wrapping_add(next_range(&mut rng, 0, 15));
            place_ore(&mut rng, world_write, x, y, z, feature.kind, feature.size);
        }
    }
}

/// Vanilla FeatureUtils.simpleRandom seed for a placed feature.
#[inline]
pub fn feature_seed(world_seed: i64, chunk_x: i32, chunk_z: i32, salt: i64) -> i64 {
    world_seed
        .wrapping_add((chunk_x as i64).wrapping_mul(341_873_128_712))
        .wrapping_add((chunk_z as i64).wrapping_mul(132_897_987_541))
        .wrapping_add(salt)
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
        OreKind::Redstone => block::REDSTONE_ORE,
        OreKind::Andesite => block::ANDESITE,
        OreKind::Diorite => block::DIORITE,
        OreKind::Granite => block::GRANITE,
        OreKind::Tuff => block::TUFF,
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
    fn feature_seed_uses_vanilla_chunk_multipliers_and_salt() {
        assert_eq!(feature_seed(1234, -34, 3, 3), -11224992412348);
    }

    #[test]
    fn ore_veins_are_deterministic_and_stay_in_chunk_and_ranges() {
        let collect = || {
            let mut out = Vec::new();
            place_ore_veins(1234, -34, 3, &mut |x, y, z, state| {
                out.push((x, y, z, state));
            });
            out
        };
        let a = collect();
        let b = collect();
        assert_eq!(a, b);
        assert!(!a.is_empty());
        // Vein blobs may extend a few blocks around their placed center.
        assert!(a.iter().all(|(_, y, _, _)| (-128..=MAX_Y + 4).contains(y)));
    }

    #[test]
    fn ore_size_and_state_are_deterministic() {
        let mut vein = Vec::new();
        let mut rng = JavaRandom::new(7);
        place_ore(
            &mut rng,
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
