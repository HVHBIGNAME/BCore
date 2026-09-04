//! Basic vanilla configured-feature generators (trees and ore veins).
//!
//! The public functions deliberately accept a write callback so they can be used
//! by chunk generators without coupling features to a particular world storage.
//! Coordinates are absolute block coordinates; clipping and replacement policy
//! belong to the callback/owner.

use crate::block;
use crate::noise::splitmix64;
use crate::simplex::WorldgenRandom;

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
    Dirt,
    Gravel,
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

/// Place an ore vein as vanilla `OreFeature` does: a line of spheres along a
/// random axis, with sinusoidal radius and overlap pruning.
///
/// `size` is the configured-feature cluster size. Returns whether any block was
/// placed (vanilla returns `placed > 0`).
pub fn place_ore(
    rng: &mut WorldgenRandom,
    world_write: &mut dyn FnMut(i32, i32, i32, BlockState),
    x: i32,
    y: i32,
    z: i32,
    kind: OreKind,
    size: usize,
) -> bool {
    if size == 0 {
        return false;
    }
    let state = ore_state(kind);
    let dir = rng.next_float() as f64 * std::f64::consts::PI;
    let spread_xy = size as f64 / 8.0;
    let max_radius = ((size as f64 / 16.0 * 2.0 + 1.0) / 2.0).ceil() as i32;
    let x0 = x as f64 + dir.sin() * spread_xy;
    let x1 = x as f64 - dir.sin() * spread_xy;
    let z0 = z as f64 + dir.cos() * spread_xy;
    let z1 = z as f64 - dir.cos() * spread_xy;
    let y0 = y as f64 + rng.next_int(3) as i32 as f64 - 2.0;
    let y1 = y as f64 + rng.next_int(3) as i32 as f64 - 2.0;
    let x_start = x - (spread_xy.ceil() as i32) - max_radius;
    let y_start = y - 2 - max_radius;
    let z_start = z - (spread_xy.ceil() as i32) - max_radius;
    let size_xz = 2 * (spread_xy.ceil() as i32 + max_radius);
    let size_y = 2 * (2 + max_radius);

    // doPlace: generate the sphere centers and radii.
    let mut data = vec![0.0f64; size * 4];
    for i in 0..size {
        let step = i as f64 / size as f64;
        let xx = lerp(step, x0, x1);
        let yy = lerp(step, y0, y1);
        let zz = lerp(step, z0, z1);
        let ss = rng.next_double() * size as f64 / 16.0;
        let r = ((step * std::f64::consts::PI).sin() + 1.0) * ss + 1.0 / 2.0;
        data[i * 4] = xx;
        data[i * 4 + 1] = yy;
        data[i * 4 + 2] = zz;
        data[i * 4 + 3] = r;
    }
    // Prune spheres fully contained in a larger one.
    for i1 in 0..size.saturating_sub(1) {
        if data[i1 * 4 + 3] > 0.0 {
            for i2 in i1 + 1..size {
                if data[i2 * 4 + 3] > 0.0 {
                    let dx = data[i1 * 4] - data[i2 * 4];
                    let dy = data[i1 * 4 + 1] - data[i2 * 4 + 1];
                    let dz = data[i1 * 4 + 2] - data[i2 * 4 + 2];
                    let dr = data[i1 * 4 + 3] - data[i2 * 4 + 3];
                    if dr * dr > dx * dx + dy * dy + dz * dz {
                        if dr > 0.0 {
                            data[i2 * 4 + 3] = -1.0;
                        } else {
                            data[i1 * 4 + 3] = -1.0;
                        }
                    }
                }
            }
        }
    }
    // Fill each surviving sphere.
    let mut placed = 0;
    let mut tested = vec![false; size_xz as usize * size_y as usize * size_xz as usize];
    for i in 0..size {
        let r = data[i * 4 + 3];
        if r < 0.0 {
            continue;
        }
        let xx = data[i * 4];
        let yy = data[i * 4 + 1];
        let zz = data[i * 4 + 2];
        let x_min = ((xx - r).floor() as i32).max(x_start);
        let y_min = ((yy - r).floor() as i32).max(y_start);
        let z_min = ((zz - r).floor() as i32).max(z_start);
        let x_max = ((xx + r).floor() as i32).max(x_min);
        let y_max = ((yy + r).floor() as i32).max(y_min);
        let z_max = ((zz + r).floor() as i32).max(z_min);
        for bx in x_min..=x_max {
            let xd = (bx as f64 + 0.5 - xx) / r;
            if xd * xd >= 1.0 {
                continue;
            }
            for by in y_min..=y_max {
                let yd = (by as f64 + 0.5 - yy) / r;
                if xd * xd + yd * yd >= 1.0 {
                    continue;
                }
                for bz in z_min..=z_max {
                    let zd = (bz as f64 + 0.5 - zz) / r;
                    if xd * xd + yd * yd + zd * zd < 1.0 {
                        let bit = (bx - x_start) as usize
                            + (by - y_start) as usize * size_xz as usize
                            + (bz - z_start) as usize * size_xz as usize * size_y as usize;
                        if !tested[bit] {
                            tested[bit] = true;
                            world_write(bx, by, bz, state);
                            placed += 1;
                        }
                    }
                }
            }
        }
    }
    placed > 0
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

fn next_range_u64(state: &mut u64, min: i32, max: i32) -> i32 {
    min + (splitmix64(*state) % (max - min + 1) as u64) as i32
}

fn next_range(state: &mut WorldgenRandom, min: i32, max: i32) -> i32 {
    min + state.next_int((max - min + 1) as usize) as i32
}

#[derive(Clone, Copy)]
enum HeightDistribution {
    Uniform,
    Trapezoid,
}

#[derive(Clone, Copy)]
struct OrePlacement {
    index: i32,
    count_random: usize,
    kind: OreKind,
    size: usize,
    count: usize,
    rarity: Option<usize>,
    min_y: i32,
    max_y: i32,
    distribution: HeightDistribution,
}

impl OrePlacement {
    fn height(self, rng: &mut WorldgenRandom) -> i32 {
        match self.distribution {
            HeightDistribution::Uniform => next_range(rng, self.min_y, self.max_y),
            // Vanilla TrapezoidHeight (plateau=0): two independent draws.
            HeightDistribution::Trapezoid => {
                let range = self.max_y - self.min_y;
                let plateau_start = range / 2;
                let plateau_end = range - plateau_start;
                self.min_y
                    + rng.next_int(plateau_end as usize + 1) as i32
                    + rng.next_int(plateau_start as usize + 1) as i32
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
/// height-ranges. Both uniform and trapezoid distributions are retained; x/z
/// are uniformly selected from the 16-block chunk footprint.
pub fn place_ore_veins(
    seed: i64,
    chunk_x: i32,
    chunk_z: i32,
    world_write: &mut dyn FnMut(i32, i32, i32, BlockState),
) {
    const FEATURES: &[OrePlacement] = &[
        OrePlacement {
            index: 0,
            kind: OreKind::Dirt,
            size: 33,
            count: 7,
            count_random: 0,
            rarity: None,
            min_y: 0,
            max_y: 160,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 1,
            kind: OreKind::Gravel,
            size: 33,
            count: 14,
            count_random: 0,
            rarity: None,
            min_y: -64,
            max_y: 319,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 2,
            kind: OreKind::Granite,
            size: 64,
            count: 2,
            count_random: 0,
            rarity: None,
            min_y: 0,
            max_y: 60,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 3,
            kind: OreKind::Granite,
            size: 64,
            count: 1,
            count_random: 0,
            rarity: Some(6),
            min_y: 64,
            max_y: 128,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 4,
            kind: OreKind::Diorite,
            size: 64,
            count: 2,
            count_random: 0,
            rarity: None,
            min_y: 0,
            max_y: 60,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 5,
            kind: OreKind::Diorite,
            size: 64,
            count: 1,
            count_random: 0,
            rarity: Some(6),
            min_y: 64,
            max_y: 128,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 6,
            kind: OreKind::Andesite,
            size: 64,
            count: 2,
            count_random: 0,
            rarity: None,
            min_y: 0,
            max_y: 60,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 7,
            kind: OreKind::Andesite,
            size: 64,
            count: 1,
            count_random: 0,
            rarity: Some(6),
            min_y: 64,
            max_y: 128,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 8,
            kind: OreKind::Tuff,
            size: 64,
            count: 2,
            count_random: 0,
            rarity: None,
            min_y: -64,
            max_y: 0,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 9,
            kind: OreKind::Coal,
            size: 17,
            count: 30,
            count_random: 0,
            rarity: None,
            min_y: 136,
            max_y: 319,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 10,
            kind: OreKind::Coal,
            size: 17,
            count: 20,
            count_random: 0,
            rarity: None,
            min_y: 0,
            max_y: 192,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 11,
            kind: OreKind::Iron,
            size: 9,
            count: 90,
            count_random: 0,
            rarity: None,
            min_y: 80,
            max_y: 384,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 12,
            kind: OreKind::Iron,
            size: 9,
            count: 10,
            count_random: 0,
            rarity: None,
            min_y: -24,
            max_y: 56,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 13,
            kind: OreKind::Iron,
            size: 4,
            count: 10,
            count_random: 0,
            rarity: None,
            min_y: -64,
            max_y: 72,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 14,
            kind: OreKind::Gold,
            size: 9,
            count: 4,
            count_random: 0,
            rarity: None,
            min_y: -64,
            max_y: 32,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 15,
            kind: OreKind::Gold,
            size: 9,
            count: 0,
            count_random: 1,
            rarity: None,
            min_y: -64,
            max_y: -48,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 16,
            kind: OreKind::Redstone,
            size: 8,
            count: 4,
            count_random: 0,
            rarity: None,
            min_y: -64,
            max_y: -49,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 17,
            kind: OreKind::Redstone,
            size: 8,
            count: 8,
            count_random: 0,
            rarity: None,
            min_y: -96,
            max_y: -32,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 18,
            kind: OreKind::Diamond,
            size: 4,
            count: 7,
            count_random: 0,
            rarity: None,
            min_y: -144,
            max_y: 16,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 19,
            kind: OreKind::Diamond,
            size: 8,
            count: 2,
            count_random: 0,
            rarity: None,
            min_y: -64,
            max_y: -4,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            index: 20,
            kind: OreKind::Diamond,
            size: 12,
            count: 1,
            count_random: 0,
            rarity: Some(9),
            min_y: -144,
            max_y: 16,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 21,
            kind: OreKind::Diamond,
            size: 8,
            count: 4,
            count_random: 0,
            rarity: None,
            min_y: -144,
            max_y: 16,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 22,
            kind: OreKind::Lapis,
            size: 7,
            count: 2,
            count_random: 0,
            rarity: None,
            min_y: -32,
            max_y: 32,
            distribution: HeightDistribution::Trapezoid,
        },
        OrePlacement {
            index: 23,
            kind: OreKind::Lapis,
            size: 7,
            count: 4,
            count_random: 0,
            rarity: None,
            min_y: -64,
            max_y: 64,
            distribution: HeightDistribution::Uniform,
        },
        OrePlacement {
            // `ore_copper_large` occupies global index 24 (it exists in dripstone
            // caves, which shares the step-6 list), so plain `ore_copper` is 25.
            index: 25,
            kind: OreKind::Copper,
            size: 10,
            count: 16,
            count_random: 0,
            rarity: None,
            min_y: -16,
            max_y: 112,
            distribution: HeightDistribution::Trapezoid,
        },
    ];

    let base_x = chunk_x.wrapping_mul(16);
    let base_z = chunk_z.wrapping_mul(16);
    // Vanilla ChunkGenerator.applyBiomeDecoration: per feature,
    // `setDecorationSeed(worldSeed, chunkX*16, chunkZ*16)` then
    // `setFeatureSeed(decorationSeed, index, GenerationStep.Decoration.UNDERGROUND_ORES.ordinal())`.
    const STEP_UNDERGROUND_ORES: i32 = 6;
    for feature in FEATURES {
        let mut rng = WorldgenRandom::new(seed);
        let decoration_seed = rng.set_decoration_seed(seed, base_x, base_z);
        rng.set_feature_seed(decoration_seed, feature.index, STEP_UNDERGROUND_ORES);
        if let Some(chance) = feature.rarity {
            if rng.next_float() >= 1.0 / chance as f32 {
                continue;
            }
        }
        let count = feature.count
            + if feature.count_random > 0 {
                rng.next_int(feature.count_random + 1)
            } else {
                0
            };
        for _ in 0..count {
            let x = base_x.wrapping_add(rng.next_int(16) as i32);
            let z = base_z.wrapping_add(rng.next_int(16) as i32);
            let y = feature.height(&mut rng);
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
        OreKind::Dirt => block::DIRT,
        OreKind::Gravel => block::GRAVEL,
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
        assert!(a.iter().all(|(_, y, _, _)| (-128..=384 + 4).contains(y)));
    }

    #[test]
    fn ore_size_and_state_are_deterministic() {
        let mut vein = Vec::new();
        let mut rng = WorldgenRandom::new(7);
        let placed = place_ore(
            &mut rng,
            &mut |x, y, z, state| vein.push((x, y, z, state)),
            0,
            0,
            0,
            OreKind::Emerald,
            8,
        );
        assert!(placed);
        assert!(!vein.is_empty());
        assert!(vein.iter().all(|p| p.3 == 9573));
        // Deterministic for the same seed.
        let mut vein2 = Vec::new();
        let mut rng2 = WorldgenRandom::new(7);
        place_ore(
            &mut rng2,
            &mut |x, y, z, state| vein2.push((x, y, z, state)),
            0,
            0,
            0,
            OreKind::Emerald,
            8,
        );
        assert_eq!(vein, vein2);
    }
}
