//! Vanilla biome decoration placement for chunk-local vegetation.
//!
//! Placement is deliberately kept separate from feature shapes: this module
//! owns the decoration and feature seeds, while `features` owns the tree.

use crate::block;
use crate::feature_sorter::sorter;
use crate::random::WorldgenRandom;
use crate::tree::{self, TreeSelector};
use crate::{Biome, GeneratedChunk, CHUNK_SIZE, SEA_LEVEL};

/// Vanilla tree selector for a biome's tree feature.
fn tree_selector(name: &str) -> TreeSelector {
    tree::selector_for(name).expect("tree feature missing selector")
}

/// Vanilla's vegetal-decoration generation step.
const VEGETAL_DECORATION_STEP: i32 = 9;

/// A configured/placed tree feature in the compact overworld registry used by
/// this crate. Indices are the feature order within the vanilla step.
#[derive(Clone, Copy)]
struct TreePlacement {
    selector: TreeSelector,
    count: CountProvider,
}

/// The vanilla `count` placement modifier for a tree placed feature.
#[derive(Clone, Copy)]
enum CountProvider {
    /// `"count": N` — a plain int, consumes no randomness.
    Constant(i32),
    /// `weighted_list`: consumes exactly one bounded draw, then returns the
    /// matching entry. Vanilla `WeightedListInt.sample` draws `nextInt(total)`.
    Weighted {
        low: i32,
        high: i32,
        weight_low: i32,
        weight_high: i32,
    },
}

impl CountProvider {
    /// Vanilla `IntProvider.sample`.
    fn sample(self, random: &mut WorldgenRandom) -> i32 {
        match self {
            Self::Constant(value) => value,
            Self::Weighted {
                low,
                high,
                weight_low,
                weight_high,
            } => {
                if random.next_i32_bounded(weight_low + weight_high) < weight_low {
                    low
                } else {
                    high
                }
            }
        }
    }
}

fn tree_placement(biome: Biome) -> Option<TreePlacement> {
    // `count` values and weights are read verbatim from vanilla's
    // `data/minecraft/worldgen/placed_feature/trees_*.json` (26.1). `count` is
    // the first placement modifier, so it is the first consumer of the
    // feature's random stream — getting the provider kind right (constant vs
    // weighted list) matters as much as the value.
    Some(match biome {
        // trees_plains: weighted 0 (w19) / 1 (w1).
        Biome::Plains => TreePlacement {
            selector: tree_selector("trees_plains"),
            count: CountProvider::Weighted {
                low: 0,
                high: 1,
                weight_low: 19,
                weight_high: 1,
            },
        },
        // trees_birch_and_oak_leaf_litter: weighted 10 (w9) / 11 (w1).
        Biome::Forest => TreePlacement {
            selector: tree_selector("trees_birch_and_oak_leaf_litter"),
            count: CountProvider::Weighted {
                low: 10,
                high: 11,
                weight_low: 9,
                weight_high: 1,
            },
        },
        // trees_birch: weighted 10 (w9) / 11 (w1).
        Biome::BirchForest => TreePlacement {
            selector: tree_selector("trees_birch"),
            count: CountProvider::Weighted {
                low: 10,
                high: 11,
                weight_low: 9,
                weight_high: 1,
            },
        },
        // dark_forest_vegetation: plain 16.
        Biome::DarkForest => TreePlacement {
            selector: TreeSelector::Random {
                features: &[],
                default: tree::DARK_OAK,
            },
            count: CountProvider::Constant(16),
        },
        // trees_taiga: weighted 10 (w9) / 11 (w1).
        Biome::Taiga => TreePlacement {
            selector: tree_selector("trees_taiga"),
            count: CountProvider::Weighted {
                low: 10,
                high: 11,
                weight_low: 9,
                weight_high: 1,
            },
        },
        // trees_snowy: weighted 0 (w9) / 1 (w1).
        Biome::SnowyPlains => TreePlacement {
            selector: tree_selector("trees_snowy"),
            count: CountProvider::Weighted {
                low: 0,
                high: 1,
                weight_low: 9,
                weight_high: 1,
            },
        },
        // trees_savanna: weighted 1 (w9) / 2 (w1).
        Biome::Savanna => TreePlacement {
            selector: tree_selector("trees_savanna"),
            count: CountProvider::Weighted {
                low: 1,
                high: 2,
                weight_low: 9,
                weight_high: 1,
            },
        },
        // trees_jungle: weighted 50 (w9) / 51 (w1).
        Biome::Jungle => TreePlacement {
            selector: tree_selector("trees_jungle"),
            count: CountProvider::Weighted {
                low: 50,
                high: 51,
                weight_low: 9,
                weight_high: 1,
            },
        },
        // trees_swamp: weighted 2 (w9) / 3 (w1).
        Biome::Swamp => TreePlacement {
            selector: TreeSelector::Random {
                features: &[],
                default: tree::OAK,
            },
            count: CountProvider::Weighted {
                low: 2,
                high: 3,
                weight_low: 9,
                weight_high: 1,
            },
        },
        // trees_windswept_hills: weighted 0 (w9) / 1 (w1).
        Biome::Mountains | Biome::SnowyMountains => TreePlacement {
            selector: tree_selector("trees_windswept_hills"),
            count: CountProvider::Weighted {
                low: 0,
                high: 1,
                weight_low: 9,
                weight_high: 1,
            },
        },
        _ => return None,
    })
}

/// Apply vanilla-style placed tree and ground-cover features to one chunk.
///
/// Every feature is reseeded exactly as `ChunkGenerator.applyBiomeDecoration`:
/// one decoration seed per chunk, then one feature seed per feature and step.
/// Positions are sampled with `in_square` (two bounded draws), and the target
/// column's biome is checked before the feature is admitted.
pub fn decorate(world_seed: i64, chunk: &mut GeneratedChunk) {
    let base_x = chunk.pos.x * CHUNK_SIZE as i32;
    let base_z = chunk.pos.z * CHUNK_SIZE as i32;
    let mut random = WorldgenRandom::from_seed(0);
    let decoration_seed = random.set_decoration_seed(world_seed, base_x, base_z);

    // Vanilla iterates decoration steps in order. Structures are intentionally
    // absent from this crate's registry; vegetal decoration is step 9.
    // The compact implementation currently has one entry per supported biome.
    // The sorter supplies vanilla's within-step index for the placed feature.
    let feature_sorter = sorter();
    let mut tree_placements: Vec<_> = [
        (Biome::Plains, "trees_plains"),
        (Biome::Forest, "trees_birch_and_oak_leaf_litter"),
        (Biome::BirchForest, "trees_birch"),
        (Biome::DarkForest, "dark_forest_vegetation"),
        (Biome::Taiga, "trees_taiga"),
        (Biome::SnowyPlains, "trees_snowy"),
        (Biome::Savanna, "trees_savanna"),
        (Biome::Jungle, "trees_jungle"),
        (Biome::Swamp, "trees_swamp"),
        (Biome::Mountains, "trees_windswept_hills"),
    ]
    .into_iter()
    .filter_map(|(biome, name)| tree_placement(biome).map(|p| (p, name)))
    .collect();
    // Vanilla places the union of the region's biome feature indices in index order.
    tree_placements.sort_by_key(|(_, name)| feature_sorter.within_step_index(name));
    for placement in tree_placements {
        let (placement, feature_name) = placement;
        let (feature_step, feature_index) = feature_sorter
            .within_step_index(feature_name)
            .expect("tree feature missing from vanilla sorter");
        debug_assert_eq!(feature_step as i32, VEGETAL_DECORATION_STEP);
        random.set_feature_seed(decoration_seed, feature_index as i32, feature_step as i32);
        // `count` is the first placement modifier, so it draws before
        // `in_square` does.
        let count = placement.count.sample(&mut random);
        for _ in 0..count {
            let x = base_x + random.next_i32_bounded(16);
            let z = base_z + random.next_i32_bounded(16);
            let lx = (x - base_x) as usize;
            let lz = (z - base_z) as usize;
            let y = chunk.height_at(lx, lz);
            // in_square + heightmap(world_surface), followed by the feature's
            // biome predicate. The column's biome must admit *this* feature
            // (same index), not merely some tree feature: otherwise the jungle
            // entry would plant jungle trees inside forest columns.
            if !feature_sorter.feature_in_biome(
                crate::biome::name(chunk.noise_biome_at(lx, y, lz)),
                feature_name,
            ) {
                continue;
            }
            if y < SEA_LEVEL || chunk.get(lx, y, lz) != Some(block::GRASS_BLOCK) {
                continue;
            }
            let config = placement.selector.select(&mut random).1;
            if !tree::place_tree(chunk, &mut random, &config, (x, y + 1, z)) {
                continue;
            }
            tree::place_tree_decorators(&mut random, config.beehive_probability);
            if config.leaf_litter {
                tree::place_on_ground(chunk, &mut random, (x, y + 1, z), 4, 2, 96, 3);
                tree::place_on_ground(chunk, &mut random, (x, y + 1, z), 2, 2, 150, 4);
            }
        }
    }

    // A small vanilla-style grass patch pass. It uses the same placement
    // modifiers and seed mechanism, but deliberately does not invent flowers.
    let (_, grass_index) = sorter()
        .within_step_index("patch_grass_normal")
        .expect("grass feature missing from vanilla sorter");
    random.set_feature_seed(decoration_seed, grass_index as i32, VEGETAL_DECORATION_STEP);
    for _ in 0..32 {
        let x = base_x + random.next_i32_bounded(16);
        let z = base_z + random.next_i32_bounded(16);
        let lx = (x - base_x) as usize;
        let lz = (z - base_z) as usize;
        let y = chunk.height_at(lx, lz);
        if chunk.biome_at(lx, lz) != Biome::Ocean
            && y >= SEA_LEVEL
            && chunk.get(lx, y, lz) == Some(block::GRASS_BLOCK)
            && chunk.get(lx, y + 1, lz) == Some(block::AIR)
        {
            chunk.set(lx, y + 1, lz, block::SHORT_GRASS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_seed_is_chunk_stable_and_feature_seed_changes_positions() {
        let mut a = WorldgenRandom::from_seed(0);
        let seed_a = a.set_decoration_seed(846_692_123_413_862_008, 0, 0);
        a.set_feature_seed(seed_a, 1, VEGETAL_DECORATION_STEP);
        let first_a = (a.next_i32_bounded(16), a.next_i32_bounded(16));
        let mut b = WorldgenRandom::from_seed(0);
        let seed_b = b.set_decoration_seed(846_692_123_413_862_008, 0, 0);
        b.set_feature_seed(seed_b, 1, VEGETAL_DECORATION_STEP);
        let first_b = (b.next_i32_bounded(16), b.next_i32_bounded(16));
        assert_eq!(seed_a, seed_b);
        assert_eq!(first_a, first_b);
        let mut c = WorldgenRandom::from_seed(0);
        c.set_decoration_seed(846_692_123_413_862_008, 0, 0);
        c.set_feature_seed(seed_a, 2, VEGETAL_DECORATION_STEP);
        assert_ne!(first_a, (c.next_i32_bounded(16), c.next_i32_bounded(16)));
    }
}
