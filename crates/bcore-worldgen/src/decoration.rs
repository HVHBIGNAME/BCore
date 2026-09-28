//! Biome decoration placement and bounded, region-backed placed-tree streams.
//!
//! The legacy `decorate` entry point remains chunk-local. `place_tree_feature`
//! borrows a seeded stream and dispatches complete standing/fallen children
//! through the region's capabilities, retaining explicit unsupported errors.

use crate::block;
use crate::feature_sorter::sorter;
use crate::heightmap::{placement_heights, supports_sapling};
use crate::random::WorldgenRandom;
use crate::tree::{self, TreeSelector};
use crate::{Biome, GeneratedChunk, CHUNK_SIZE, MAX_Y, MIN_Y, SEA_LEVEL};

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
    fn sample<R: tree::TreeRandom + ?Sized>(self, random: &mut R) -> i32 {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeHeightmap {
    OceanFloor,
    WorldSurface,
}

/// Reads required by the native modifiers in addition to fallen-tree placement.
/// Biome IDs refer to the bundled 26.1 registry. Heights are the first free Y.
pub trait TreeFeatureWorld: tree::fallen::FallenTreeWorld {
    fn tree_height(&self, heightmap: TreeHeightmap, x: i32, z: i32) -> i32;
    fn tree_biome(&self, pos: tree::fallen::Pos) -> u32;
    fn can_place_tree(&self, pos: tree::fallen::Pos) -> bool;

    /// A world without standing-tree capabilities must stop, not substitute a
    /// shape or skip the child. The historical 504-case adapter uses this boundary.
    fn place_standing_tree<R: tree::TreeRandom + ?Sized>(
        &mut self,
        _random: &mut R,
        configured_feature: &'static str,
        origin: tree::fallen::Pos,
    ) -> Result<bool, UnsupportedTree> {
        Err(UnsupportedTree {
            configured_feature,
            origin,
            unsupported_shape: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedTree {
    pub configured_feature: &'static str,
    pub origin: tree::fallen::Pos,
    pub unsupported_shape: Option<tree::standing::UnsupportedShape>,
}

impl std::fmt::Display for UnsupportedTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(shape) = self.unsupported_shape {
            return shape.fmt(f);
        }
        write!(
            f,
            "region-backed tree {} at {:?} is not implemented",
            self.configured_feature, self.origin
        )
    }
}

impl std::error::Error for UnsupportedTree {}

#[derive(Clone, Copy)]
enum SaplingFilter {
    BeforeBiome,
    AfterBiome,
    ChildOnly,
}

fn region_tree_placement(name: &str) -> Option<(CountProvider, SaplingFilter)> {
    let (low, weight_low, filter) = match name {
        "trees_plains" => (0, 19, SaplingFilter::BeforeBiome),
        "trees_birch" => (10, 9, SaplingFilter::AfterBiome),
        "trees_snowy" => (0, 9, SaplingFilter::AfterBiome),
        "trees_birch_and_oak_leaf_litter" | "birch_tall" | "trees_taiga" => {
            (10, 9, SaplingFilter::ChildOnly)
        }
        "trees_savanna" => (1, 9, SaplingFilter::ChildOnly),
        "trees_jungle" => (50, 9, SaplingFilter::ChildOnly),
        "trees_sparse_jungle" => (2, 9, SaplingFilter::ChildOnly),
        "trees_windswept_hills" => (0, 9, SaplingFilter::ChildOnly),
        _ => return None,
    };
    Some((
        CountProvider::Weighted {
            low,
            high: low + 1,
            weight_low,
            weight_high: 1,
        },
        filter,
    ))
}

/// Execute one supported native placed-tree stream with the caller's seeded RNG.
/// `Ok(None)` means the placed feature is unknown and consumes no draws. On an
/// unsupported standing child, `Err` retains the preceding RNG and world effects;
/// callers must stop the source replay rather than skip that attempt.
///
/// Global decoration still requires a verified source-dependency schedule and
/// persistence of the region's generated bee data and scheduled tick requests.
pub fn place_tree_feature<W: TreeFeatureWorld + ?Sized, R: tree::TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    source: crate::ChunkPos,
    name: &str,
) -> Result<Option<bool>, UnsupportedTree> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let Some((count, sapling_filter)) = region_tree_placement(name) else {
        return Ok(None);
    };
    let mut placed = false;
    for _ in 0..count.sample(random) {
        let x = source
            .x
            .wrapping_mul(16)
            .wrapping_add(random.next_i32_bounded(16));
        let z = source
            .z
            .wrapping_mul(16)
            .wrapping_add(random.next_i32_bounded(16));
        let floor = world.tree_height(TreeHeightmap::OceanFloor, x, z);
        let surface = world.tree_height(TreeHeightmap::WorldSurface, x, z);
        if surface - floor > 0 {
            continue;
        }
        let y = world.tree_height(TreeHeightmap::OceanFloor, x, z);
        if y <= MIN_Y {
            continue;
        }
        let origin = (x, y, z);
        if matches!(sapling_filter, SaplingFilter::BeforeBiome)
            && !supports_sapling(world.get_block((x, y - 1, z)))
        {
            continue;
        }
        if !sorter().feature_in_biome(crate::biome::name(world.tree_biome(origin)), name) {
            continue;
        }
        if matches!(sapling_filter, SaplingFilter::AfterBiome)
            && !supports_sapling(world.get_block((x, y - 1, z)))
        {
            continue;
        }
        if !world.can_place_tree(origin) {
            continue;
        }
        let child = tree::select_placed_tree(name, random).expect("supported placed-tree selector");
        if child.requires_sapling && !supports_sapling(world.get_block((x, y - 1, z))) {
            continue;
        }
        if !world.can_place_tree(origin) {
            continue;
        }
        placed |= if let Some(config) = tree::fallen::config_for(child.configured_feature) {
            tree::fallen::place(world, random, &config, origin)
        } else {
            world.place_standing_tree(random, child.configured_feature, origin)?
        };
    }
    Ok(Some(placed))
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

/// The per-biome tree placements in vanilla within-step index order.
fn sorted_tree_placements() -> Vec<(TreePlacement, &'static str)> {
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
    tree_placements
}

fn place_tree_attempt(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    placement: TreePlacement,
    feature_name: &str,
    x: usize,
    z: usize,
) -> bool {
    let heights = placement_heights(chunk, x, z);
    let max_water_depth = if feature_name == "trees_swamp" { 2 } else { 0 };
    if heights.world_surface - heights.ocean_floor > max_water_depth || heights.ocean_floor <= MIN_Y
    {
        return false;
    }
    let y = heights.ocean_floor;
    if !sorter().feature_in_biome(
        crate::biome::name(chunk.noise_biome_at(x, y.min(MAX_Y), z)),
        feature_name,
    ) {
        return false;
    }
    let supported = supports_sapling(chunk.get(x, y - 1, z).unwrap());
    // These placed features filter before entering their configured selector.
    if matches!(
        feature_name,
        "trees_plains" | "trees_birch" | "trees_snowy" | "trees_swamp"
    ) && !supported
    {
        return false;
    }
    let config = placement.selector.select(random).1;
    // Other selectors first choose a placed child, whose sapling predicate can fail.
    if !supported {
        return false;
    }
    let origin = (chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32);
    tree::place_tree(chunk, random, &config, origin)
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
    for placement in sorted_tree_placements() {
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
            let x = random.next_i32_bounded(16) as usize;
            let z = random.next_i32_bounded(16) as usize;
            place_tree_attempt(chunk, &mut random, placement, feature_name, x, z);
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
    use crate::ChunkPos;

    #[test]
    fn unsupported_forest_child_consumes_selector_but_outer_filter_does_not() {
        for (biome, name, consumes_selector) in [
            (Biome::Forest, "trees_birch_and_oak_leaf_litter", true),
            (Biome::Plains, "trees_plains", false),
            (Biome::BirchForest, "trees_birch", false),
        ] {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
            chunk.biomes.fill(biome);
            chunk.set(8, 64, 8, block::STONE);
            let before = chunk.states().to_vec();
            let placement = tree_placement(biome).unwrap();
            let mut actual = WorldgenRandom::from_seed(42);
            let mut expected = WorldgenRandom::from_seed(42);
            if consumes_selector {
                placement.selector.select(&mut expected);
            }
            assert!(!place_tree_attempt(
                &mut chunk,
                &mut actual,
                placement,
                name,
                8,
                8
            ));
            assert_eq!(actual.next_i64(), expected.next_i64());
            assert_eq!(chunk.states(), before);
        }
    }

    #[test]
    fn tree_can_grow_on_dirt_below_sea_level_using_live_height() {
        let mut chunk = GeneratedChunk::new(ChunkPos::new(-2, 3));
        chunk.biomes.fill(Biome::Forest);
        chunk.set(8, 40, 8, block::DIRT);
        let placement = TreePlacement {
            selector: TreeSelector::Random {
                features: &[],
                default: tree::OAK,
            },
            count: CountProvider::Constant(1),
        };
        let mut random = WorldgenRandom::from_seed(1);
        assert!(place_tree_attempt(
            &mut chunk,
            &mut random,
            placement,
            "trees_birch_and_oak_leaf_litter",
            8,
            8
        ));
        assert_eq!(chunk.get(8, 41, 8), Some(block::OAK_LOG));
        assert_eq!(chunk.height_at(8, 8), MIN_Y);
    }

    #[test]
    fn water_depth_filter_runs_before_selector_and_uses_surface_not_fluid_count() {
        for cover in [block::WATER, block::SHORT_GRASS, block::LEAF_LITTER] {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
            chunk.biomes.fill(Biome::Forest);
            chunk.set(8, 64, 8, block::GRASS_BLOCK);
            chunk.set(8, 65, 8, cover);
            let mut actual = WorldgenRandom::from_seed(42);
            let mut expected = WorldgenRandom::from_seed(42);
            assert!(!place_tree_attempt(
                &mut chunk,
                &mut actual,
                tree_placement(Biome::Forest).unwrap(),
                "trees_birch_and_oak_leaf_litter",
                8,
                8
            ));
            assert_eq!(actual.next_i64(), expected.next_i64());
        }
    }

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
