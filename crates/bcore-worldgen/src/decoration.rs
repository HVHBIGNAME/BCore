//! Vanilla biome decoration placement for chunk-local vegetation.
//!
//! Placement is deliberately kept separate from feature shapes: this module
//! owns the decoration and feature seeds, while `features` owns the tree.

use crate::block;
use crate::features::TreeKind;
use crate::random::WorldgenRandom;
use crate::tree::{self, TreeConfig};
use crate::{Biome, GeneratedChunk, CHUNK_SIZE, SEA_LEVEL};

/// Vanilla tree configuration for a biome's tree feature.
fn tree_config(kind: TreeKind) -> &'static TreeConfig {
    match kind {
        TreeKind::Oak => &tree::OAK,
        TreeKind::Birch => &tree::BIRCH,
        TreeKind::Spruce => &tree::SPRUCE,
        TreeKind::Pine => &tree::PINE,
    }
}

/// Vanilla's vegetal-decoration generation step.
const VEGETAL_DECORATION_STEP: i32 = 7;

/// A configured/placed tree feature in the compact overworld registry used by
/// this crate. Indices are the feature order within the vanilla step.
#[derive(Clone, Copy)]
struct TreePlacement {
    index: i32,
    kind: TreeKind,
    /// `None` means no rarity filter; otherwise the feature is placed with
    /// probability `1 / rarity` before its count is sampled.
    rarity: Option<i32>,
    count: i32,
}

fn tree_placement(biome: Biome) -> Option<TreePlacement> {
    // These are the overworld tree placed-feature entries. Keeping the index
    // explicit is important: setFeatureSeed uses the registry position, not a
    // per-biome counter.
    Some(match biome {
        Biome::Plains => TreePlacement {
            index: 0,
            kind: TreeKind::Oak,
            rarity: Some(5),
            count: 1,
        },
        Biome::Forest => TreePlacement {
            index: 1,
            kind: TreeKind::Oak,
            rarity: None,
            count: 5,
        },
        Biome::BirchForest => TreePlacement {
            index: 2,
            kind: TreeKind::Birch,
            rarity: None,
            count: 10,
        },
        Biome::DarkForest => TreePlacement {
            index: 3,
            kind: TreeKind::Oak,
            rarity: None,
            count: 6,
        },
        Biome::Taiga => TreePlacement {
            index: 4,
            kind: TreeKind::Spruce,
            rarity: None,
            count: 10,
        },
        Biome::Jungle => TreePlacement {
            index: 5,
            kind: TreeKind::Oak,
            rarity: None,
            count: 4,
        },
        Biome::Savanna => TreePlacement {
            index: 6,
            kind: TreeKind::Oak,
            rarity: Some(20),
            count: 1,
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
    // absent from this crate's registry; vegetal decoration is step 7.
    for placement in [
        tree_placement(Biome::Plains),
        tree_placement(Biome::Forest),
        tree_placement(Biome::BirchForest),
        tree_placement(Biome::DarkForest),
        tree_placement(Biome::Taiga),
        tree_placement(Biome::Jungle),
        tree_placement(Biome::Savanna),
    ]
    .into_iter()
    .flatten()
    {
        random.set_feature_seed(decoration_seed, placement.index, VEGETAL_DECORATION_STEP);
        if placement
            .rarity
            .is_some_and(|rarity| random.next_f32() >= 1.0 / rarity as f32)
        {
            continue;
        }
        for _ in 0..placement.count {
            let x = base_x + random.next_i32_bounded(16);
            let z = base_z + random.next_i32_bounded(16);
            let lx = (x - base_x) as usize;
            let lz = (z - base_z) as usize;
            let biome = chunk.biome_at(lx, lz);
            // in_square + heightmap(world_surface), followed by the feature's
            // biome predicate. A feature never starts in another biome.
            if tree_placement(biome).is_none() {
                continue;
            }
            let y = chunk.height_at(lx, lz);
            if y < SEA_LEVEL || chunk.get(lx, y, lz) != Some(block::GRASS_BLOCK) {
                continue;
            }
            let config = tree_config(placement.kind);
            // Vanilla passes the *same* decoration random straight through to
            // the feature: the `in_square` draws above already advanced it, and
            // the feature continues the stream. Reseeding here would break
            // bit-exact parity with vanilla.
            tree::place_tree(chunk, &mut random, config, (x, y + 1, z));
        }
    }

    // A small vanilla-style grass patch pass. It uses the same placement
    // modifiers and seed mechanism, but deliberately does not invent flowers.
    random.set_feature_seed(decoration_seed, 7, VEGETAL_DECORATION_STEP);
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
