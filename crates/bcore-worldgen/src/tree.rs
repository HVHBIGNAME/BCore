//! Vanilla `TreeFeature` shape placement.
//!
//! Ported from the vanilla tree feature (`StraightTrunkPlacer`,
//! `BlobFoliagePlacer`, `SpruceFoliagePlacer`, ...) so that, given the same
//! [`WorldgenRandom`] stream, the produced blocks match vanilla exactly.
//!
//! The single most important property of this module is the **random
//! consumption order**: vanilla samples the tree height, then the foliage
//! height, then the leaf radius, then the root origin, and only then starts
//! touching blocks. [`IntProvider::Constant`] consumes no randomness at all,
//! which is why the oak/birch (whose radius/offset/height are constants) draw
//! exactly two values for their height and nothing else.

use crate::block;
use crate::random::WorldgenRandom;
use crate::GeneratedChunk;

const CHUNK_SIZE_I32: i32 = 16;

/// Vanilla `IntProvider` — the subset used by tree placers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntProvider {
    /// Always the same value; **consumes no randomness** when sampled.
    Constant(i32),
    /// Uniform inclusive over `[min, max]`.
    Uniform { min: i32, max: i32 },
}

impl IntProvider {
    /// Vanilla `IntProvider.sample`.
    pub fn sample(self, random: &mut WorldgenRandom) -> i32 {
        match self {
            Self::Constant(value) => value,
            Self::Uniform { min, max } => min + random.next_i32_bounded(max - min + 1),
        }
    }
}

/// Vanilla trunk placers, carrying the vanilla `base_height`/`height_rand_a`/`height_rand_b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrunkPlacer {
    /// `minecraft:straight_trunk_placer`.
    Straight {
        base_height: i32,
        height_rand_a: i32,
        height_rand_b: i32,
    },
    /// `minecraft:forking_trunk_placer`.
    Forking {
        base_height: i32,
        height_rand_a: i32,
        height_rand_b: i32,
    },
    /// `minecraft:dark_oak_trunk_placer`.
    DarkOak {
        base_height: i32,
        height_rand_a: i32,
        height_rand_b: i32,
    },
    /// `minecraft:giant_trunk_placer` (2x2 trunk, e.g. mega jungle).
    Giant {
        base_height: i32,
        height_rand_a: i32,
        height_rand_b: i32,
    },
}

impl TrunkPlacer {
    const fn random_range(self) -> (i32, i32) {
        match self {
            Self::Straight {
                height_rand_a,
                height_rand_b,
                ..
            }
            | Self::Forking {
                height_rand_a,
                height_rand_b,
                ..
            }
            | Self::DarkOak {
                height_rand_a,
                height_rand_b,
                ..
            }
            | Self::Giant {
                height_rand_a,
                height_rand_b,
                ..
            } => (height_rand_a, height_rand_b),
        }
    }

    const fn base_height(self) -> i32 {
        match self {
            Self::Straight { base_height, .. }
            | Self::Forking { base_height, .. }
            | Self::DarkOak { base_height, .. }
            | Self::Giant { base_height, .. } => base_height,
        }
    }
}

/// Vanilla foliage placers — the subset needed by the common overworld trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoliagePlacer {
    /// `minecraft:blob_foliage_placer` — oak, birch, jungle.
    Blob {
        radius: IntProvider,
        offset: IntProvider,
        height: IntProvider,
    },
    /// `minecraft:spruce_foliage_placer` — spruce, mega spruce.
    Spruce {
        radius: IntProvider,
        offset: IntProvider,
        trunk_height: IntProvider,
    },
    /// `minecraft:pine_foliage_placer` — pine, mega pine.
    Pine {
        radius: IntProvider,
        offset: IntProvider,
        height: IntProvider,
    },
    /// `minecraft:acacia_foliage_placer` — acacia.
    Acacia {
        radius: IntProvider,
        offset: IntProvider,
    },
    /// `minecraft:dark_oak_foliage_placer` — dark oak.
    DarkOak {
        radius: IntProvider,
        offset: IntProvider,
    },
    /// `minecraft:fancy_foliage_placer` — fancy oak.
    Fancy {
        radius: IntProvider,
        offset: IntProvider,
        height: IntProvider,
    },
}

/// A vanilla `TreeConfiguration` (only the fields that affect the blocks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeConfig {
    pub trunk: TrunkPlacer,
    pub foliage: FoliagePlacer,
    pub log: u32,
    pub leaves: u32,
    /// Vanilla tree decorators, retained in the compact configuration.
    pub beehive_probability: Option<f32>,
    pub leaf_litter: bool,
}

const fn blob(radius: i32, offset: i32, height: i32) -> FoliagePlacer {
    FoliagePlacer::Blob {
        radius: IntProvider::Constant(radius),
        offset: IntProvider::Constant(offset),
        height: IntProvider::Constant(height),
    }
}

/// `data/minecraft/worldgen/configured_feature/oak.json`.
pub const OAK: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Straight {
        base_height: 4,
        height_rand_a: 2,
        height_rand_b: 0,
    },
    foliage: blob(2, 0, 3),
    log: block::OAK_LOG,
    leaves: block::OAK_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

/// `birch.json`.
pub const BIRCH: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Straight {
        base_height: 5,
        height_rand_a: 2,
        height_rand_b: 0,
    },
    foliage: blob(2, 0, 3),
    log: block::BIRCH_LOG,
    leaves: block::BIRCH_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

/// `spruce.json`.
pub const SPRUCE: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Straight {
        base_height: 5,
        height_rand_a: 2,
        height_rand_b: 1,
    },
    foliage: FoliagePlacer::Spruce {
        radius: IntProvider::Uniform { min: 2, max: 3 },
        offset: IntProvider::Uniform { min: 0, max: 2 },
        trunk_height: IntProvider::Uniform { min: 1, max: 2 },
    },
    log: block::SPRUCE_LOG,
    leaves: block::SPRUCE_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

/// `pine.json`.
pub const PINE: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Straight {
        base_height: 6,
        height_rand_a: 4,
        height_rand_b: 0,
    },
    foliage: FoliagePlacer::Pine {
        radius: IntProvider::Constant(1),
        offset: IntProvider::Constant(1),
        height: IntProvider::Uniform { min: 3, max: 4 },
    },
    log: block::SPRUCE_LOG,
    leaves: block::SPRUCE_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

/// `jungle_tree.json`.
pub const JUNGLE: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Straight {
        base_height: 4,
        height_rand_a: 8,
        height_rand_b: 0,
    },
    foliage: blob(2, 0, 3),
    log: block::JUNGLE_LOG,
    leaves: block::JUNGLE_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

/// `acacia.json`.
pub const ACACIA: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Forking {
        base_height: 5,
        height_rand_a: 2,
        height_rand_b: 2,
    },
    foliage: FoliagePlacer::Acacia {
        radius: IntProvider::Constant(2),
        offset: IntProvider::Constant(0),
    },
    log: block::ACACIA_LOG,
    leaves: block::ACACIA_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

/// `dark_oak.json`.
pub const DARK_OAK: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::DarkOak {
        base_height: 6,
        height_rand_a: 2,
        height_rand_b: 1,
    },
    foliage: FoliagePlacer::DarkOak {
        radius: IntProvider::Constant(0),
        offset: IntProvider::Constant(0),
    },
    log: block::DARK_OAK_LOG,
    leaves: block::DARK_OAK_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

pub const FANCY_OAK: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Straight {
        base_height: 3,
        height_rand_a: 11,
        height_rand_b: 0,
    },
    foliage: FoliagePlacer::Fancy {
        radius: IntProvider::Constant(2),
        offset: IntProvider::Constant(4),
        height: IntProvider::Constant(4),
    },
    log: block::OAK_LOG,
    leaves: block::OAK_LEAVES,
    beehive_probability: None,
    leaf_litter: false,
};

pub const OAK_BEES_005: TreeConfig = TreeConfig {
    beehive_probability: Some(0.005),
    ..OAK
};
pub const OAK_BEES_0002_LEAF_LITTER: TreeConfig = TreeConfig {
    beehive_probability: Some(0.002),
    leaf_litter: true,
    ..OAK
};
pub const BIRCH_BEES_0002: TreeConfig = TreeConfig {
    beehive_probability: Some(0.002),
    ..BIRCH
};
pub const BIRCH_BEES_0002_LEAF_LITTER: TreeConfig = TreeConfig {
    beehive_probability: Some(0.002),
    leaf_litter: true,
    ..BIRCH
};
pub const FANCY_OAK_BEES_005: TreeConfig = TreeConfig {
    beehive_probability: Some(0.005),
    ..FANCY_OAK
};
pub const FANCY_OAK_BEES_0002_LEAF_LITTER: TreeConfig = TreeConfig {
    beehive_probability: Some(0.002),
    leaf_litter: true,
    ..FANCY_OAK
};

/// A configured tree variant selected by a vanilla random selector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightedTree {
    pub chance: f32,
    pub tree: TreeConfig,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TreeSelector {
    Random {
        features: &'static [WeightedTree],
        default: TreeConfig,
    },
    Simple {
        features: &'static [TreeConfig],
    },
}

impl TreeSelector {
    pub fn select(self, random: &mut WorldgenRandom) -> (usize, TreeConfig) {
        match self {
            Self::Random { features, default } => {
                for (index, weighted) in features.iter().enumerate() {
                    if random.next_f32() < weighted.chance {
                        return (index, weighted.tree);
                    }
                }
                (features.len(), default)
            }
            Self::Simple { features } => {
                assert!(!features.is_empty());
                let index = random.next_i32_bounded(features.len() as i32) as usize;
                (index, features[index])
            }
        }
    }
}

const PLAINS_SELECTOR_FEATURES: [WeightedTree; 2] = [
    WeightedTree {
        chance: 0.33333334,
        tree: FANCY_OAK_BEES_005,
    },
    WeightedTree {
        chance: 0.0125,
        tree: OAK,
    },
];
const BIRCH_SELECTOR_FEATURES: [WeightedTree; 1] = [WeightedTree {
    chance: 0.0125,
    tree: BIRCH,
}];
const FOREST_SELECTOR_FEATURES: [WeightedTree; 4] = [
    WeightedTree {
        chance: 0.0025,
        tree: BIRCH,
    },
    WeightedTree {
        chance: 0.2,
        tree: BIRCH_BEES_0002_LEAF_LITTER,
    },
    WeightedTree {
        chance: 0.1,
        tree: FANCY_OAK_BEES_0002_LEAF_LITTER,
    },
    WeightedTree {
        chance: 0.0125,
        tree: OAK,
    },
];
const TAIGA_SELECTOR_FEATURES: [WeightedTree; 2] = [
    WeightedTree {
        chance: 0.33333334,
        tree: PINE,
    },
    WeightedTree {
        chance: 0.0125,
        tree: SPRUCE,
    },
];
const SAVANNA_SELECTOR_FEATURES: [WeightedTree; 2] = [
    WeightedTree {
        chance: 0.8,
        tree: ACACIA,
    },
    WeightedTree {
        chance: 0.0125,
        tree: OAK,
    },
];
const JUNGLE_SELECTOR_FEATURES: [WeightedTree; 4] = [
    WeightedTree {
        chance: 0.1,
        tree: FANCY_OAK_BEES_0002_LEAF_LITTER,
    },
    WeightedTree {
        chance: 0.5,
        tree: JUNGLE,
    },
    WeightedTree {
        chance: 0.33333334,
        tree: JUNGLE,
    },
    WeightedTree {
        chance: 0.0125,
        tree: JUNGLE,
    },
];
const SNOWY_SELECTOR_FEATURES: [WeightedTree; 1] = [WeightedTree {
    chance: 0.0125,
    tree: SPRUCE,
}];
const WINDSWEPT_SELECTOR_FEATURES: [WeightedTree; 4] = [
    WeightedTree {
        chance: 0.008325,
        tree: SPRUCE,
    },
    WeightedTree {
        chance: 0.666,
        tree: SPRUCE,
    },
    WeightedTree {
        chance: 0.1,
        tree: FANCY_OAK_BEES_0002_LEAF_LITTER,
    },
    WeightedTree {
        chance: 0.0125,
        tree: OAK,
    },
];

pub fn selector_for(name: &str) -> Option<TreeSelector> {
    Some(match name {
        "trees_plains" => TreeSelector::Random {
            features: &PLAINS_SELECTOR_FEATURES,
            default: OAK_BEES_005,
        },
        "trees_birch" => TreeSelector::Random {
            features: &BIRCH_SELECTOR_FEATURES,
            default: BIRCH_BEES_0002,
        },
        "trees_birch_and_oak_leaf_litter" => TreeSelector::Random {
            features: &FOREST_SELECTOR_FEATURES,
            default: OAK_BEES_0002_LEAF_LITTER,
        },
        "trees_taiga" => TreeSelector::Random {
            features: &TAIGA_SELECTOR_FEATURES,
            default: SPRUCE,
        },
        "trees_savanna" => TreeSelector::Random {
            features: &SAVANNA_SELECTOR_FEATURES,
            default: OAK,
        },
        "trees_jungle" => TreeSelector::Random {
            features: &JUNGLE_SELECTOR_FEATURES,
            default: JUNGLE,
        },
        "trees_snowy" => TreeSelector::Random {
            features: &SNOWY_SELECTOR_FEATURES,
            default: SPRUCE,
        },
        "trees_windswept_hills" => TreeSelector::Random {
            features: &WINDSWEPT_SELECTOR_FEATURES,
            default: OAK,
        },
        _ => return None,
    })
}

/// Consume the vanilla tree decorators that affect the feature random stream.
/// Block placement is intentionally limited to blocks represented by this crate.
pub fn place_tree_decorators(random: &mut WorldgenRandom, beehive_probability: Option<f32>) {
    if let Some(probability) = beehive_probability {
        if random.next_f32() < probability {
            let _log_position = random.next_i32_bounded(2);
            let bees = 2 + random.next_i32_bounded(2);
            for _ in 0..bees {
                let _ = random.next_i32_bounded(599);
            }
        }
    }
}

/// PlaceOnGroundDecorator for single-base trees. Provider sampling happens only
/// after all placement predicates pass, so failed attempts consume three draws.
pub fn place_on_ground(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    origin: (i32, i32, i32),
    radius: i32,
    height: i32,
    tries: i32,
    segment_count: i32,
) {
    let (ox, oy, oz) = origin;
    for _ in 0..tries {
        let wx = random.next_i32_between(ox - radius, ox + radius);
        let y = random.next_i32_between(oy - height, oy + height);
        let wz = random.next_i32_between(oz - radius, oz + radius);
        let Some((x, z)) = local_coords(chunk, wx, wz) else {
            continue;
        };
        if chunk.get(x, y + 1, z) != Some(block::AIR) {
            continue;
        }
        let Some(ground) = chunk.get(x, y, z) else {
            continue;
        };
        if !is_solid_ground(ground) {
            continue;
        }
        if (y + 2..=crate::MAX_Y).any(|above| chunk.get(x, above, z).is_some_and(is_solid_ground)) {
            continue;
        }
        let sample = random.next_i32_bounded(segment_count * 4);
        // Provider order: N/E/S/W for each amount. State registry order: N/S/W/E.
        let facing = [0, 3, 1, 2][(sample % 4) as usize];
        let state = block::LEAF_LITTER + (facing * 4 + sample / 4) as u32;
        chunk.set(x, y + 1, z, state);
    }
}

fn is_solid_ground(state: u32) -> bool {
    !matches!(
        state,
        block::AIR
            | block::WATER
            | block::LAVA
            | block::SHORT_GRASS
            | block::DEAD_BUSH
            | block::OAK_LEAVES
            | block::BIRCH_LEAVES
            | block::SPRUCE_LEAVES
            | block::JUNGLE_LEAVES
            | block::ACACIA_LEAVES
            | block::DARK_OAK_LEAVES
    ) && !(block::LEAF_LITTER..block::LEAF_LITTER + 16).contains(&state)
}

/// A foliage attachment point produced by a trunk placer.
#[derive(Debug, Clone, Copy)]
struct FoliageAttachment {
    x: i32,
    y: i32,
    z: i32,
    /// Vanilla `FoliageAttachment.radiusOffset`.
    radius_offset: i32,
    double_trunk: bool,
}

/// Places one vanilla tree. `origin` is the world-space trunk base.
///
/// Returns `true` if the tree was placed. The caller owns the random stream;
/// this function draws from it in exactly the vanilla order.
pub fn place_tree(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    config: &TreeConfig,
    origin: (i32, i32, i32),
) -> bool {
    let tree_height = sample_tree_height(random, config.trunk);
    let foliage_height = sample_foliage_height(random, config.foliage, tree_height);
    let trunk_height = tree_height - foliage_height;
    let leaf_radius = sample_foliage_radius(random, config.foliage, trunk_height);

    let (ox, oy, oz) = origin;
    if oy + tree_height + 1 > 319 || oy < -63 {
        return false;
    }

    let mut attachments = Vec::with_capacity(4);
    place_trunk(
        chunk,
        random,
        config,
        (ox, oy, oz),
        tree_height,
        &mut attachments,
    );
    if attachments.is_empty() {
        return false;
    }
    for attachment in &attachments {
        create_foliage(
            chunk,
            random,
            config,
            attachment,
            foliage_height,
            leaf_radius,
        );
    }
    true
}

/// Vanilla `TrunkPlacer.getTreeHeight` — identical across the straight-family
/// placers: `base + next(a+1) + next(b+1)`.
fn sample_tree_height(random: &mut WorldgenRandom, trunk: TrunkPlacer) -> i32 {
    let (a, b) = trunk.random_range();
    trunk.base_height() + random.next_i32_bounded(a + 1) + random.next_i32_bounded(b + 1)
}

/// Vanilla `FoliagePlacer.foliageHeight`.
fn sample_foliage_height(
    random: &mut WorldgenRandom,
    foliage: FoliagePlacer,
    tree_height: i32,
) -> i32 {
    match foliage {
        FoliagePlacer::Blob { height, .. } => height.sample(random),
        FoliagePlacer::Spruce { trunk_height, .. } => {
            (tree_height - trunk_height.sample(random)).max(4)
        }
        FoliagePlacer::Pine { height, .. } => height.sample(random),
        FoliagePlacer::Acacia { .. } => 0,
        FoliagePlacer::DarkOak { .. } => 4,
        FoliagePlacer::Fancy { height, .. } => height.sample(random),
    }
}

/// Vanilla `FoliagePlacer.foliageRadius`.
fn sample_foliage_radius(
    random: &mut WorldgenRandom,
    foliage: FoliagePlacer,
    trunk_height: i32,
) -> i32 {
    match foliage {
        FoliagePlacer::Blob { radius, .. }
        | FoliagePlacer::Spruce { radius, .. }
        | FoliagePlacer::Acacia { radius, .. }
        | FoliagePlacer::DarkOak { radius, .. }
        | FoliagePlacer::Fancy { radius, .. } => radius.sample(random),
        FoliagePlacer::Pine { radius, .. } => {
            // Vanilla PineFoliagePlacer.foliageRadius samples both the
            // configured radius and a second bounded value, even when the
            // configured radius is Constant.  The latter is based on the
            // trunk height and is part of the feature's RNG contract.
            radius.sample(random) + random.next_i32_bounded((trunk_height + 1).max(1))
        }
    }
}

/// Vanilla `FoliagePlacer.foliageOffset`.
fn sample_foliage_offset(random: &mut WorldgenRandom, foliage: FoliagePlacer) -> i32 {
    match foliage {
        FoliagePlacer::Blob { offset, .. }
        | FoliagePlacer::Spruce { offset, .. }
        | FoliagePlacer::Pine { offset, .. }
        | FoliagePlacer::Acacia { offset, .. }
        | FoliagePlacer::DarkOak { offset, .. }
        | FoliagePlacer::Fancy { offset, .. } => offset.sample(random),
    }
}

/// Places the trunk logs and returns the foliage attachment points.
fn place_trunk(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    config: &TreeConfig,
    origin: (i32, i32, i32),
    height: i32,
    attachments: &mut Vec<FoliageAttachment>,
) {
    let (ox, oy, oz) = origin;
    match config.trunk {
        TrunkPlacer::Straight { .. } => {
            for y in 0..height {
                place_log(chunk, config, (ox, oy + y, oz));
            }
            place_below_trunk_block(chunk, config, (ox, oy - 1, oz));
            attachments.push(FoliageAttachment {
                x: ox,
                y: oy + height,
                z: oz,
                radius_offset: 0,
                double_trunk: false,
            });
        }
        TrunkPlacer::Giant { .. } | TrunkPlacer::DarkOak { .. } => {
            for y in 0..height {
                for dx in 0..2 {
                    for dz in 0..2 {
                        place_log(chunk, config, (ox + dx, oy + y, oz + dz));
                    }
                }
            }
            for dx in 0..2 {
                for dz in 0..2 {
                    place_below_trunk_block(chunk, config, (ox + dx, oy - 1, oz + dz));
                }
            }
            attachments.push(FoliageAttachment {
                x: ox,
                y: oy + height - 1,
                z: oz,
                radius_offset: 0,
                double_trunk: true,
            });
        }
        TrunkPlacer::Forking { .. } => {
            place_forking_trunk(chunk, random, config, origin, height, attachments);
        }
    }
}

/// Vanilla `ForkingTrunkPlacer.placeTrunk` (acacia).
fn place_forking_trunk(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    config: &TreeConfig,
    origin: (i32, i32, i32),
    height: i32,
    attachments: &mut Vec<FoliageAttachment>,
) {
    let (ox, oy, oz) = origin;
    let mut x = ox;
    let mut z = oz;
    let direction = random.next_i32_bounded(4);
    let lean_height = height - random.next_i32_bounded(4) - 1;
    let mut branch_steps = 3 - random.next_i32_bounded(3);

    for y in 0..height {
        place_log(chunk, config, (x, oy + y, z));
        if y >= lean_height && branch_steps > 0 {
            match direction {
                0 => x += 1,
                1 => x -= 1,
                2 => z += 1,
                _ => z -= 1,
            }
            branch_steps -= 1;
        }
    }
    place_below_trunk_block(chunk, config, (ox, oy - 1, oz));
    attachments.push(FoliageAttachment {
        x,
        y: oy + height,
        z,
        radius_offset: 1,
        double_trunk: false,
    });
}

/// Vanilla `createFoliage` dispatch.
fn create_foliage(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    config: &TreeConfig,
    attachment: &FoliageAttachment,
    foliage_height: i32,
    leaf_radius: i32,
) {
    match config.foliage {
        FoliagePlacer::Blob { .. } => {
            let offset = sample_foliage_offset(random, config.foliage);
            for y in (offset - foliage_height..=offset).rev() {
                let current_radius = (leaf_radius + attachment.radius_offset - 1 - y / 2).max(0);
                place_leaves_row(chunk, random, config, attachment, current_radius, y);
            }
        }
        FoliagePlacer::Fancy { .. } => {
            let offset = sample_foliage_offset(random, config.foliage);
            for y in (offset - foliage_height..=offset).rev() {
                let current_radius = if y != offset && y != offset - foliage_height {
                    leaf_radius + 1
                } else {
                    leaf_radius
                };
                place_leaves_row(chunk, random, config, attachment, current_radius, y);
            }
        }
        FoliagePlacer::Spruce { .. } => {
            let offset = sample_foliage_offset(random, config.foliage);
            // Vanilla SpruceFoliagePlacer consumes one draw to choose the
            // initial layer radius before placing the first row.
            let mut current_radius = random.next_i32_bounded(2);
            let mut max_radius = 1;
            let mut min_radius = 0;
            for y in (-foliage_height..=offset).rev() {
                place_leaves_row(chunk, random, config, attachment, current_radius, y);
                if current_radius >= max_radius {
                    current_radius = min_radius;
                    min_radius = 1;
                    max_radius = (max_radius + 1).min(leaf_radius + attachment.radius_offset);
                } else {
                    current_radius += 1;
                }
            }
        }
        FoliagePlacer::Pine { .. } => {
            let offset = sample_foliage_offset(random, config.foliage);
            let mut current_radius = 0;
            for y in (offset - foliage_height..=offset).rev() {
                place_leaves_row(chunk, random, config, attachment, current_radius, y);
                if current_radius >= 1 && y == offset - foliage_height + 1 {
                    current_radius -= 1;
                } else if current_radius < leaf_radius + attachment.radius_offset {
                    current_radius += 1;
                }
            }
        }
        FoliagePlacer::Acacia { .. } => {
            let offset = sample_foliage_offset(random, config.foliage);
            let leaf_height = 1 + random.next_i32_bounded(2);
            for y in (offset - leaf_height..=offset).rev() {
                let current_radius = leaf_radius + attachment.radius_offset + 1 - y.abs();
                place_leaves_row(chunk, random, config, attachment, current_radius, y);
            }
        }
        FoliagePlacer::DarkOak { .. } => {
            let offset = sample_foliage_offset(random, config.foliage);
            let base = FoliageAttachment {
                y: attachment.y + offset,
                ..*attachment
            };
            let (inner, outer, top) = if attachment.double_trunk {
                (leaf_radius + 2, leaf_radius + 3, leaf_radius)
            } else {
                (leaf_radius + 2, leaf_radius + 2, leaf_radius)
            };
            place_leaves_row(chunk, random, config, &base, inner, -1);
            place_leaves_row(chunk, random, config, &base, outer, 0);
            place_leaves_row(chunk, random, config, &base, inner, 1);
            if random.next_bool() {
                place_leaves_row(chunk, random, config, &base, top, 2);
            }
        }
    }
}

/// Vanilla `FoliagePlacer.placeLeavesRow`.
fn place_leaves_row(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    config: &TreeConfig,
    attachment: &FoliageAttachment,
    current_radius: i32,
    y: i32,
) {
    let offset = i32::from(attachment.double_trunk);
    for dx in -current_radius..=current_radius + offset {
        for dz in -current_radius..=current_radius + offset {
            if should_skip_location(
                random,
                config.foliage,
                dx,
                y,
                dz,
                current_radius,
                attachment,
            ) {
                continue;
            }
            try_place_leaf(
                chunk,
                config,
                (attachment.x + dx, attachment.y + y, attachment.z + dz),
            );
        }
    }
}

/// Vanilla `FoliagePlacer.shouldSkipLocationSigned` + per-placer skip rule.
fn should_skip_location(
    random: &mut WorldgenRandom,
    foliage: FoliagePlacer,
    dx: i32,
    y: i32,
    dz: i32,
    current_radius: i32,
    attachment: &FoliageAttachment,
) -> bool {
    if let FoliagePlacer::DarkOak { .. } = foliage {
        return dark_oak_should_skip_location(dx, y, dz, current_radius, attachment.double_trunk);
    }
    let (dx, dz) = signed_distances(dx, dz, attachment.double_trunk);
    match foliage {
        // NOTE: the short-circuit matters — vanilla only draws the coin flip
        // when the (dx, dz) corner test passes.
        FoliagePlacer::Blob { .. } => {
            dx == current_radius
                && dz == current_radius
                && (random.next_i32_bounded(2) == 0 || y == 0)
        }
        FoliagePlacer::Spruce { .. } | FoliagePlacer::Pine { .. } => {
            dx == current_radius && dz == current_radius && current_radius > 0
        }
        FoliagePlacer::Acacia { .. } => {
            dx == current_radius
                && dz == current_radius
                && (y == 0 || random.next_i32_bounded(2) == 0)
        }
        FoliagePlacer::Fancy { .. } => false,
        FoliagePlacer::DarkOak { .. } => unreachable!(),
    }
}

/// Vanilla `FoliagePlacer.foliageSignedDistances`.
fn signed_distances(dx: i32, dz: i32, double_trunk: bool) -> (i32, i32) {
    if double_trunk {
        (dx.abs().min((dx - 1).abs()), dz.abs().min((dz - 1).abs()))
    } else {
        (dx.abs(), dz.abs())
    }
}

/// Vanilla `DarkOakFoliagePlacer.shouldSkipLocation`.
fn dark_oak_should_skip_location(
    dx: i32,
    y: i32,
    dz: i32,
    current_radius: i32,
    double_trunk: bool,
) -> bool {
    if y == 0
        && double_trunk
        && (dx == -current_radius || dx >= current_radius)
        && (dz == -current_radius || dz >= current_radius)
    {
        return true;
    }
    let (dx, dz) = signed_distances(dx, dz, double_trunk);
    if y == -1 && !double_trunk {
        dx == current_radius && dz == current_radius
    } else if y == 1 {
        dx + dz > current_radius * 2 - 2
    } else {
        false
    }
}

/// Vanilla `TreeFeature.tryPlaceLeaf`.
fn try_place_leaf(chunk: &mut GeneratedChunk, config: &TreeConfig, pos: (i32, i32, i32)) -> bool {
    let (x, y, z) = pos;
    let Some((lx, lz)) = local_coords(chunk, x, z) else {
        return false;
    };
    let Some(state) = chunk.get(lx, y, lz) else {
        return false;
    };
    if !is_valid_tree_pos(state) {
        return false;
    }
    chunk.set(lx, y, lz, config.leaves)
}

/// Vanilla `TreeFeature.placeLog`.
fn place_log(chunk: &mut GeneratedChunk, config: &TreeConfig, pos: (i32, i32, i32)) -> bool {
    let (x, y, z) = pos;
    let Some((lx, lz)) = local_coords(chunk, x, z) else {
        return false;
    };
    let Some(state) = chunk.get(lx, y, lz) else {
        return false;
    };
    if !is_valid_tree_pos(state) {
        return false;
    }
    chunk.set(lx, y, lz, config.log)
}

/// Vanilla `TrunkPlacer.placeBelowTrunkBlock` (the supportive dirt).
fn place_below_trunk_block(chunk: &mut GeneratedChunk, _config: &TreeConfig, pos: (i32, i32, i32)) {
    let (x, y, z) = pos;
    let Some((lx, lz)) = local_coords(chunk, x, z) else {
        return;
    };
    let Some(state) = chunk.get(lx, y, lz) else {
        return;
    };
    // Only replace soil-like blocks, matching vanilla's
    // `isStateAtPosition(state -> state.is(BlockTags.DIRT))`-style guard.
    if matches!(state, block::GRASS_BLOCK | block::DIRT) {
        chunk.set(lx, y, lz, block::DIRT);
    }
}

fn local_coords(chunk: &GeneratedChunk, x: i32, z: i32) -> Option<(usize, usize)> {
    let x = x - chunk.pos.x * CHUNK_SIZE_I32;
    let z = z - chunk.pos.z * CHUNK_SIZE_I32;
    ((0..CHUNK_SIZE_I32).contains(&x) && (0..CHUNK_SIZE_I32).contains(&z))
        .then_some((x as usize, z as usize))
}

/// Vanilla `TreeFeature.validTreePos`: air, or a block in
/// `minecraft:replaceable_by_trees`.
fn is_valid_tree_pos(state: u32) -> bool {
    state == block::AIR || is_replaceable_by_trees(state)
}

/// Blocks tagged `minecraft:replaceable_by_trees` that can occur at tree
/// height in the overworld. Trees whose leaves would land on any other block
/// are skipped, exactly as in vanilla.
fn is_replaceable_by_trees(state: u32) -> bool {
    (block::LEAF_LITTER..block::LEAF_LITTER + 16).contains(&state)
        || matches!(
            state,
            block::WATER
                | block::SHORT_GRASS
                | block::LEAF_LITTER
                | block::DEAD_BUSH
                | block::OAK_LEAVES
                | block::BIRCH_LEAVES
                | block::SPRUCE_LEAVES
                | block::JUNGLE_LEAVES
                | block::ACACIA_LEAVES
                | block::DARK_OAK_LEAVES
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChunkPos;

    #[test]
    fn foliage_does_not_wrap_into_the_opposite_chunk_edge() {
        for pos in [ChunkPos::new(0, 0), ChunkPos::new(-2, 3)] {
            let mut chunk = GeneratedChunk::new(pos);
            let origin = (pos.x * 16, 65, pos.z * 16 + 8);
            let mut random = WorldgenRandom::from_seed(1);
            assert!(place_tree(&mut chunk, &mut random, &OAK, origin));
            assert!((65..80).all(|y| chunk.get(15, y, 8) == Some(block::AIR)));
            assert!((65..80).any(|y| chunk.get(1, y, 8) == Some(block::OAK_LEAVES)));
        }
    }

    #[test]
    fn leaf_litter_is_placed_on_exposed_ground_with_valid_state_properties() {
        let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
        for z in 0..16 {
            for x in 0..16 {
                chunk.set(x, 64, z, block::GRASS_BLOCK);
            }
        }
        let mut random = WorldgenRandom::from_seed(17);
        place_on_ground(&mut chunk, &mut random, (8, 65, 8), 4, 2, 96, 3);
        let litter: Vec<_> = chunk
            .states()
            .iter()
            .copied()
            .filter(|s| (block::LEAF_LITTER..block::LEAF_LITTER + 16).contains(s))
            .collect();
        assert!(!litter.is_empty());
        assert!(litter.iter().all(|s| (s - block::LEAF_LITTER) % 4 < 3));
        assert!((0..16).all(|z| (0..16).all(|x| chunk.get(x, 64, z) == Some(block::GRASS_BLOCK))));
        assert!((66..80)
            .all(|y| (0..16).all(|z| (0..16).all(|x| chunk.get(x, y, z) == Some(block::AIR)))));
    }

    fn chunk_at(seed: i64) -> GeneratedChunk {
        let generator = crate::WorldGenerator::new(seed);
        generator.generate_chunk_vanilla(ChunkPos::new(0, 0))
    }

    /// The oak draws exactly two bounded values and nothing else; this pins the
    /// vanilla random consumption order that bit-exact parity depends on.
    #[test]
    fn oak_height_matches_vanilla_formula() {
        for seed in [0_u64, 1, 0x1234, 846_692_123_413_862_008] {
            let mut a = WorldgenRandom::from_seed(seed);
            let mut b = WorldgenRandom::from_seed(seed);
            let expected = 4 + b.next_i32_bounded(3) + b.next_i32_bounded(1);
            assert_eq!(sample_tree_height(&mut a, OAK.trunk), expected);
        }
    }

    /// `IntProvider::Constant` must not consume randomness — this is the bug
    /// that silently shifted every leaf of every oak.
    #[test]
    fn constant_int_provider_consumes_no_randomness() {
        let mut a = WorldgenRandom::from_seed(7);
        let mut b = WorldgenRandom::from_seed(7);
        assert_eq!(IntProvider::Constant(2).sample(&mut a), 2);
        assert_eq!(a.next_i32(), b.next_i32());
    }

    #[test]
    fn oak_radius_and_height_constants_do_not_advance_the_stream() {
        let mut a = WorldgenRandom::from_seed(99);
        let mut b = WorldgenRandom::from_seed(99);
        sample_tree_height(&mut a, OAK.trunk);
        let _ = sample_foliage_height(&mut a, OAK.foliage, 6);
        let _ = sample_foliage_radius(&mut a, OAK.foliage, 3);
        sample_tree_height(&mut b, OAK.trunk);
        assert_eq!(a.next_i32(), b.next_i32());
    }

    #[test]
    fn birch_uses_vanilla_config_values() {
        assert_eq!(
            BIRCH.trunk,
            TrunkPlacer::Straight {
                base_height: 5,
                height_rand_a: 2,
                height_rand_b: 0
            }
        );
        assert_eq!(BIRCH.log, block::BIRCH_LOG);
    }

    #[test]
    fn spruce_trunk_and_foliage_ranges_match_vanilla_json() {
        assert_eq!(
            SPRUCE.trunk,
            TrunkPlacer::Straight {
                base_height: 5,
                height_rand_a: 2,
                height_rand_b: 1
            }
        );
        match SPRUCE.foliage {
            FoliagePlacer::Spruce {
                radius,
                offset,
                trunk_height,
            } => {
                assert_eq!(radius, IntProvider::Uniform { min: 2, max: 3 });
                assert_eq!(offset, IntProvider::Uniform { min: 0, max: 2 });
                assert_eq!(trunk_height, IntProvider::Uniform { min: 1, max: 2 });
            }
            other => panic!("expected spruce foliage, got {other:?}"),
        }
    }

    #[test]
    fn placing_an_oak_on_flat_ground_produces_logs_under_leaves() {
        let mut chunk = chunk_at(846_692_123_413_862_008);
        let mut random = WorldgenRandom::from_seed(1);
        let ground = chunk.height_at(8, 8);
        assert!(place_tree(
            &mut chunk,
            &mut random,
            &OAK,
            (8, ground + 1, 8)
        ));

        let mut logs = 0;
        let mut leaves = 0;
        for y in ground..ground + 12 {
            match chunk.get(8, y, 8) {
                Some(block::OAK_LOG) => logs += 1,
                Some(_) => {}
                None => {}
            }
        }
        for dx in -3..=3_i32 {
            for dz in -3..=3_i32 {
                for y in ground..ground + 12 {
                    let (lx, lz) = (
                        (8 + dx).rem_euclid(16) as usize,
                        (8 + dz).rem_euclid(16) as usize,
                    );
                    if chunk.get(lx, y, lz) == Some(block::OAK_LEAVES) {
                        leaves += 1;
                    }
                }
            }
        }
        assert!(logs >= 4, "expected a trunk of at least 4 logs, got {logs}");
        assert!(leaves > 0, "expected some leaves, got {leaves}");
    }

    #[test]
    fn pine_radius_consumes_vanilla_trunk_height_draw() {
        let mut actual = WorldgenRandom::from_seed(0x51_9e37);
        let mut expected = WorldgenRandom::from_seed(0x51_9e37);
        let tree_height = sample_tree_height(&mut actual, PINE.trunk);
        let foliage_height = sample_foliage_height(&mut actual, PINE.foliage, tree_height);
        let trunk_height = tree_height - foliage_height;
        let radius = sample_foliage_radius(&mut actual, PINE.foliage, trunk_height);

        sample_tree_height(&mut expected, PINE.trunk);
        sample_foliage_height(&mut expected, PINE.foliage, tree_height);
        let expected_radius = 1 + expected.next_i32_bounded((trunk_height + 1).max(1));

        assert_eq!(radius, expected_radius);
        assert_eq!(actual.next_i32(), expected.next_i32());
    }

    #[test]
    fn spruce_foliage_consumes_initial_layer_radius_draw() {
        let mut actual = WorldgenRandom::from_seed(0x5a_7ce);
        let mut expected = WorldgenRandom::from_seed(0x5a_7ce);
        let tree_height = sample_tree_height(&mut actual, SPRUCE.trunk);
        let foliage_height = sample_foliage_height(&mut actual, SPRUCE.foliage, tree_height);
        let trunk_height = tree_height - foliage_height;
        let leaf_radius = sample_foliage_radius(&mut actual, SPRUCE.foliage, trunk_height);
        let offset = sample_foliage_offset(&mut actual, SPRUCE.foliage);
        let initial_radius = actual.next_i32_bounded(2);

        sample_tree_height(&mut expected, SPRUCE.trunk);
        sample_foliage_height(&mut expected, SPRUCE.foliage, tree_height);
        let _ = sample_foliage_radius(&mut expected, SPRUCE.foliage, trunk_height);
        let expected_offset = expected.next_i32_bounded(3);
        let expected_initial_radius = expected.next_i32_bounded(2);

        assert_eq!(offset, expected_offset);
        assert_eq!(initial_radius, expected_initial_radius);
        assert!((2..=3).contains(&leaf_radius));
        assert_eq!(actual.next_i32(), expected.next_i32());
    }
    #[test]
    fn tree_placement_is_deterministic_for_a_fixed_seed() {
        let mut a = chunk_at(846_692_123_413_862_008);
        let mut b = chunk_at(846_692_123_413_862_008);
        let mut ra = WorldgenRandom::from_seed(42);
        let mut rb = WorldgenRandom::from_seed(42);
        let ga = a.height_at(4, 4);
        let gb = b.height_at(4, 4);
        place_tree(&mut a, &mut ra, &OAK, (4, ga + 1, 4));
        place_tree(&mut b, &mut rb, &OAK, (4, gb + 1, 4));
        assert_eq!(a.states(), b.states());
    }
}
