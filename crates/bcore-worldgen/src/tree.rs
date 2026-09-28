//! Vanilla `TreeFeature` shape placement.
//!
//! Implements the supported trunk and foliage placers with a caller-owned
//! [`TreeRandom`] stream. [`standing`] and [`fallen`] use absolute-coordinate
//! worlds; the legacy [`place_tree`] entry point clips to its owning chunk.
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

pub mod fallen;
mod fancy;
pub mod standing;

/// The draws used by tree providers, shape placers and decorators. Implementations
/// borrow the caller's current feature stream; adapting never forks or reseeds it.
pub trait TreeRandom {
    fn next_i32_bounded(&mut self, bound: i32) -> i32;
    fn next_f32(&mut self) -> f32;

    fn next_bool(&mut self) -> bool {
        self.next_i32_bounded(2) != 0
    }
}

impl TreeRandom for WorldgenRandom {
    fn next_i32_bounded(&mut self, bound: i32) -> i32 {
        WorldgenRandom::next_i32_bounded(self, bound)
    }

    fn next_f32(&mut self) -> f32 {
        WorldgenRandom::next_f32(self)
    }
}

impl TreeRandom for crate::simplex::WorldgenRandom {
    fn next_i32_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        self.next_int(bound as usize) as i32
    }

    fn next_f32(&mut self) -> f32 {
        self.next_float()
    }
}

const CHUNK_SIZE_I32: i32 = 16;

type LocalPos = (usize, i32, usize);

// HashSet<BlockPos> bucket iteration affects vanilla's leaf-update order.
// Spatial sets use Java's spread hash, 0.75 load factor and collision chains.
struct PositionSet {
    buckets: Vec<Vec<LocalPos>>,
    len: usize,
    base: (i32, i32),
}

impl PositionSet {
    fn new(base: (i32, i32)) -> Self {
        Self {
            buckets: vec![Vec::new(); 16],
            len: 0,
            base,
        }
    }
    fn hash(&self, (x, y, z): LocalPos) -> usize {
        let x = self.base.0.wrapping_add(x as i32);
        let z = self.base.1.wrapping_add(z as i32);
        let hash = y
            .wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x) as u32;
        (hash ^ (hash >> 16)) as usize
    }
    fn insert(&mut self, pos: LocalPos) {
        let index = self.hash(pos) & (self.buckets.len() - 1);
        if self.buckets[index].contains(&pos) {
            return;
        }
        self.buckets[index].push(pos);
        self.len += 1;
        if self.len > self.buckets.len() * 3 / 4 {
            let capacity = self.buckets.len() * 2;
            let old = std::mem::replace(&mut self.buckets, vec![Vec::new(); capacity]);
            for pos in old.into_iter().flatten() {
                let index = self.hash(pos) & (capacity - 1);
                self.buckets[index].push(pos);
            }
        }
    }
    fn take_first(&mut self) -> Option<LocalPos> {
        let bucket = self.buckets.iter_mut().find(|bucket| !bucket.is_empty())?;
        self.len -= 1;
        Some(bucket.remove(0))
    }
}

struct TreePlacement<'a> {
    chunk: &'a mut GeneratedChunk,
    logs: Vec<LocalPos>,
    bounds: Option<(LocalPos, LocalPos)>,
    foliage_count: usize,
}

impl<'a> TreePlacement<'a> {
    fn new(chunk: &'a mut GeneratedChunk) -> Self {
        Self {
            chunk,
            logs: Vec::new(),
            bounds: None,
            foliage_count: 0,
        }
    }

    fn set(&mut self, x: usize, y: i32, z: usize, state: u32) -> bool {
        if !self.chunk.set(x, y, z, state) {
            return false;
        }
        if is_log(state) {
            self.logs.push((x, y, z));
        } else if is_leaves(state) {
            self.foliage_count += 1;
        }
        let (min, max) = self.bounds.get_or_insert(((x, y, z), (x, y, z)));
        *min = (min.0.min(x), min.1.min(y), min.2.min(z));
        *max = (max.0.max(x), max.1.max(y), max.2.max(z));
        true
    }

    fn update_leaves(&mut self) {
        let Some((min, max)) = self.bounds else {
            return;
        };
        let width = max.0 - min.0 + 1;
        let depth = max.2 - min.2 + 1;
        let mut visited = vec![false; width * depth * (max.1 - min.1 + 1) as usize];
        let base = (self.chunk.pos.x * 16, self.chunk.pos.z * 16);
        let mut pending: [PositionSet; 7] = std::array::from_fn(|_| PositionSet::new(base));
        let mut logs = PositionSet::new(base);
        for &pos in &self.logs {
            logs.insert(pos);
        }
        while let Some(pos) = logs.take_first() {
            pending[0].insert(pos);
        }
        let mut distance = 0;
        loop {
            while distance < 7 && pending[distance].len == 0 {
                distance += 1;
            }
            if distance == 7 {
                break;
            }
            let (x, y, z) = pending[distance].take_first().unwrap();
            let index = (y - min.1) as usize * width * depth + (z - min.2) * width + x - min.0;
            // Entries already queued in another distance bucket are not discarded.
            visited[index] = true;
            if distance > 0 {
                let state = self.chunk.get(x, y, z).unwrap();
                let base = leaf_state_base(state).unwrap();
                self.chunk.set(
                    x,
                    y,
                    z,
                    base + (distance as u32 - 1) * 4 + (state - base) % 4,
                );
            }
            for (nx, ny, nz) in [
                (x as i32, y - 1, z as i32),
                (x as i32, y + 1, z as i32),
                (x as i32, y, z as i32 - 1),
                (x as i32, y, z as i32 + 1),
                (x as i32 - 1, y, z as i32),
                (x as i32 + 1, y, z as i32),
            ] {
                if nx < min.0 as i32
                    || nx > max.0 as i32
                    || ny < min.1
                    || ny > max.1
                    || nz < min.2 as i32
                    || nz > max.2 as i32
                {
                    continue;
                }
                let index = (ny - min.1) as usize * width * depth
                    + (nz as usize - min.2) * width
                    + nx as usize
                    - min.0;
                if visited[index] {
                    continue;
                }
                let state = self.chunk.get(nx as usize, ny, nz as usize).unwrap();
                let current_distance = if is_log(state) {
                    0
                } else if let Some(base) = leaf_state_base(state) {
                    ((state - base) / 4 + 1) as usize
                } else {
                    continue;
                };
                let next_distance = current_distance.min(distance + 1);
                if next_distance < 7 {
                    pending[next_distance].insert((nx as usize, ny, nz as usize));
                    distance = distance.min(next_distance);
                }
            }
        }
    }
}

impl std::ops::Deref for TreePlacement<'_> {
    type Target = GeneratedChunk;
    fn deref(&self) -> &GeneratedChunk {
        self.chunk
    }
}

/// Shape algorithms use world positions; the legacy sink alone clips to a chunk.
trait ShapeSink {
    fn state(&self, pos: fallen::Pos) -> Option<u32>;
    fn valid_state(&self, state: u32) -> bool;
    fn free_state(&self, state: u32) -> bool;
    fn log(&mut self, pos: fallen::Pos, state: u32) -> bool;
    fn leaf(&mut self, pos: fallen::Pos, state: u32) -> bool;
    fn below_trunk(&mut self, pos: fallen::Pos);
}

impl ShapeSink for TreePlacement<'_> {
    fn state(&self, (x, y, z): fallen::Pos) -> Option<u32> {
        let (x, z) = local_coords(self, x, z)?;
        self.get(x, y, z)
    }

    fn valid_state(&self, state: u32) -> bool {
        is_valid_tree_pos(state)
    }

    fn free_state(&self, state: u32) -> bool {
        is_valid_tree_pos(state) || is_log(state)
    }

    fn log(&mut self, (x, y, z): fallen::Pos, state: u32) -> bool {
        let Some((x, z)) = local_coords(self, x, z) else {
            return false;
        };
        self.set(x, y, z, state)
    }

    fn leaf(&mut self, pos: fallen::Pos, state: u32) -> bool {
        self.log(pos, state)
    }

    fn below_trunk(&mut self, (x, y, z): fallen::Pos) {
        let Some((x, z)) = local_coords(self, x, z) else {
            return;
        };
        let Some(state) = self.get(x, y, z) else {
            return;
        };
        if !crate::heightmap::protected_below_trunk(state) {
            self.set(x, y, z, block::DIRT);
            self.logs.push((x, y, z));
        }
    }
}

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
    pub fn sample<R: TreeRandom + ?Sized>(self, random: &mut R) -> i32 {
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
    /// `minecraft:fancy_trunk_placer` (branching oak).
    Fancy {
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
            | Self::Fancy {
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
            | Self::Fancy { base_height, .. }
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

/// Clearance radius at each height, from a tree's `minimum_size` configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureSize {
    TwoLayers {
        limit: i32,
        lower_size: i32,
        upper_size: i32,
    },
    ThreeLayers {
        limit: i32,
        upper_limit: i32,
        lower_size: i32,
        middle_size: i32,
        upper_size: i32,
    },
}

impl FeatureSize {
    fn radius_at(self, height: i32, y: i32) -> i32 {
        match self {
            Self::TwoLayers {
                limit,
                lower_size,
                upper_size,
            } => {
                if y < limit {
                    lower_size
                } else {
                    upper_size
                }
            }
            Self::ThreeLayers {
                limit,
                upper_limit,
                lower_size,
                middle_size,
                upper_size,
            } => {
                if y < limit {
                    lower_size
                } else if y >= height - upper_limit {
                    upper_size
                } else {
                    middle_size
                }
            }
        }
    }
}

/// The supported fields of vanilla `TreeConfiguration`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeConfig {
    pub trunk: TrunkPlacer,
    pub foliage: FoliagePlacer,
    pub log: u32,
    pub leaves: u32,
    pub minimum_size: FeatureSize,
    /// `minimum_size.min_clipped_height`; absent means the full height is required.
    pub min_clipped_height: Option<i32>,
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
    minimum_size: FeatureSize::TwoLayers {
        limit: 1,
        lower_size: 0,
        upper_size: 1,
    },
    min_clipped_height: None,
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
    minimum_size: OAK.minimum_size,
    min_clipped_height: None,
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
    minimum_size: FeatureSize::TwoLayers {
        limit: 2,
        lower_size: 0,
        upper_size: 2,
    },
    min_clipped_height: None,
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
    minimum_size: SPRUCE.minimum_size,
    min_clipped_height: None,
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
    minimum_size: OAK.minimum_size,
    min_clipped_height: None,
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
    minimum_size: FeatureSize::TwoLayers {
        limit: 1,
        lower_size: 0,
        upper_size: 2,
    },
    min_clipped_height: None,
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
    minimum_size: FeatureSize::ThreeLayers {
        limit: 1,
        upper_limit: 1,
        lower_size: 0,
        middle_size: 1,
        upper_size: 2,
    },
    min_clipped_height: None,
    beehive_probability: None,
    leaf_litter: false,
};

pub const FANCY_OAK: TreeConfig = TreeConfig {
    trunk: TrunkPlacer::Fancy {
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
    minimum_size: FeatureSize::TwoLayers {
        limit: 0,
        lower_size: 0,
        upper_size: 0,
    },
    min_clipped_height: Some(4),
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

/// A selected native child, retaining its configured-feature identity rather
/// than coercing fallen trees or unsupported standing variants to `TreeConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlacedTreeSelection {
    pub configured_feature: &'static str,
    pub requires_sapling: bool,
}

pub(crate) fn select_placed_tree<R: TreeRandom + ?Sized>(
    name: &str,
    random: &mut R,
) -> Option<PlacedTreeSelection> {
    let (choices, default): (&[(f32, &str)], &str) = match name {
        "trees_plains" => (
            &[
                (0.33333334, "fancy_oak_bees_005"),
                (0.0125, "fallen_oak_tree"),
            ],
            "oak_bees_005",
        ),
        "trees_birch" => (&[(0.0125, "fallen_birch_tree")], "birch_bees_0002"),
        "trees_birch_and_oak_leaf_litter" => (
            &[
                (0.0025, "fallen_birch_tree"),
                (0.2, "birch_bees_0002_leaf_litter"),
                (0.1, "fancy_oak_bees_0002_leaf_litter"),
                (0.0125, "fallen_oak_tree"),
            ],
            "oak_bees_0002_leaf_litter",
        ),
        "birch_tall" => (
            &[
                (0.00625, "fallen_super_birch_tree"),
                (0.5, "super_birch_bees_0002"),
                (0.0125, "fallen_birch_tree"),
            ],
            "birch_bees_0002",
        ),
        "trees_taiga" => (
            &[(0.33333334, "pine"), (0.0125, "fallen_spruce_tree")],
            "spruce",
        ),
        "trees_snowy" => (&[(0.0125, "fallen_spruce_tree")], "spruce"),
        "trees_savanna" => (&[(0.8, "acacia"), (0.0125, "fallen_oak_tree")], "oak"),
        "trees_jungle" => (
            &[
                (0.1, "fancy_oak"),
                (0.5, "jungle_bush"),
                (0.33333334, "mega_jungle_tree"),
                (0.0125, "fallen_jungle_tree"),
            ],
            "jungle_tree",
        ),
        "trees_sparse_jungle" => (
            &[
                (0.1, "fancy_oak"),
                (0.5, "jungle_bush"),
                (0.0125, "fallen_jungle_tree"),
            ],
            "jungle_tree",
        ),
        "trees_windswept_hills" => (
            &[
                (0.008325, "fallen_spruce_tree"),
                (0.666, "spruce"),
                (0.1, "fancy_oak"),
                (0.0125, "fallen_oak_tree"),
            ],
            "oak",
        ),
        _ => return None,
    };
    let configured_feature = choices
        .iter()
        .find(|&&(chance, _)| random.next_f32() < chance)
        .map_or(default, |&(_, child)| child);
    Some(PlacedTreeSelection {
        configured_feature,
        // Plains embeds two unfiltered standing children. Its fallen child is
        // still a checked placed feature, in addition to the outer predicate.
        requires_sapling: name != "trees_plains" || configured_feature == "fallen_oak_tree",
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

#[derive(Clone, Copy)]
struct GroundBounds {
    x: (i32, i32),
    y: i32,
    z: (i32, i32),
}

impl GroundBounds {
    fn from_trunks(placement: &TreePlacement<'_>) -> Option<Self> {
        let y = placement.logs.iter().map(|pos| pos.1).min()?;
        let mut bases = placement.logs.iter().filter(|pos| pos.1 == y);
        let &(x, _, z) = bases.next()?;
        let (mut min_x, mut max_x, mut min_z, mut max_z) = (x, x, z, z);
        for &(x, _, z) in bases {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_z = min_z.min(z);
            max_z = max_z.max(z);
        }
        let bx = placement.pos.x * 16;
        let bz = placement.pos.z * 16;
        Some(Self {
            x: (bx + min_x as i32, bx + max_x as i32),
            y,
            z: (bz + min_z as i32, bz + max_z as i32),
        })
    }
}

/// PlaceOnGroundDecorator. Provider sampling happens only
/// after all placement predicates pass, so failed attempts consume three draws.
fn place_on_ground(
    chunk: &mut TreePlacement<'_>,
    random: &mut WorldgenRandom,
    bounds: GroundBounds,
    radius: i32,
    height: i32,
    tries: i32,
    segment_count: i32,
) {
    for _ in 0..tries {
        let wx = random.next_i32_between(bounds.x.0 - radius, bounds.x.1 + radius);
        let y = random.next_i32_between(bounds.y - height, bounds.y + height);
        let wz = random.next_i32_between(bounds.z.0 - radius, bounds.z.1 + radius);
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
    !is_leaves(state)
        && !matches!(
            state,
            block::AIR | block::WATER | block::LAVA | block::SHORT_GRASS | block::DEAD_BUSH
        )
        && !(block::LEAF_LITTER..block::LEAF_LITTER + 16).contains(&state)
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
/// Samples dimensions before checking clearance. A rejected attempt must not
/// place any blocks or consume foliage/decorator randomness.
pub fn place_tree(
    chunk: &mut GeneratedChunk,
    random: &mut WorldgenRandom,
    config: &TreeConfig,
    origin: (i32, i32, i32),
) -> bool {
    let mut placement = TreePlacement::new(chunk);
    if !grow_shape(&mut placement, random, config, origin) {
        return false;
    }
    if placement.logs.is_empty() && placement.foliage_count == 0 {
        return false;
    }
    place_tree_decorators(random, config.beehive_probability);
    if config.leaf_litter {
        if let Some(bounds) = GroundBounds::from_trunks(&placement) {
            place_on_ground(&mut placement, random, bounds, 4, 2, 96, 3);
            place_on_ground(&mut placement, random, bounds, 2, 2, 150, 4);
        }
    }
    placement.update_leaves();
    true
}

fn grow_shape<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    placement: &mut P,
    random: &mut R,
    config: &TreeConfig,
    origin: fallen::Pos,
) -> bool {
    let tree_height = sample_tree_height(random, config.trunk);
    let foliage_height = sample_foliage_height(random, config.foliage, tree_height);
    let trunk_height = tree_height - foliage_height;
    let leaf_radius = sample_foliage_radius(random, config.foliage, trunk_height);

    let (ox, oy, oz) = origin;
    if oy + tree_height + 1 > crate::MAX_Y + 1 || oy < crate::MIN_Y + 1 {
        return false;
    }

    let free_height = free_tree_height(config.minimum_size, origin, tree_height, |pos| {
        placement
            .state(pos)
            .is_none_or(|state| placement.free_state(state))
    });
    if free_height < tree_height
        && config
            .min_clipped_height
            .is_none_or(|minimum| free_height < minimum)
    {
        return false;
    }

    let mut attachments = Vec::with_capacity(4);
    place_trunk(
        placement,
        random,
        config,
        (ox, oy, oz),
        free_height,
        &mut attachments,
    );
    for attachment in &attachments {
        create_foliage(
            placement,
            random,
            config,
            attachment,
            foliage_height,
            leaf_radius,
        );
    }
    true
}

#[cfg(test)]
fn max_free_tree_height(
    chunk: &GeneratedChunk,
    size: FeatureSize,
    origin: (i32, i32, i32),
    height: i32,
) -> i32 {
    free_tree_height(size, origin, height, |(x, y, z)| {
        let Some((x, z)) = local_coords(chunk, x, z) else {
            return true;
        };
        let state = chunk.get(x, y, z).unwrap_or(block::AIR);
        is_valid_tree_pos(state) || is_log(state)
    })
}

fn free_tree_height(
    size: FeatureSize,
    origin: fallen::Pos,
    height: i32,
    mut is_free: impl FnMut(fallen::Pos) -> bool,
) -> i32 {
    let (ox, oy, oz) = origin;
    for dy in 0..=height + 1 {
        let radius = size.radius_at(height, dy);
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if !is_free((ox + dx, oy + dy, oz + dz)) {
                    return dy - 2;
                }
            }
        }
    }
    height
}

fn is_log(state: u32) -> bool {
    [
        block::OAK_LOG,
        block::BIRCH_LOG,
        block::SPRUCE_LOG,
        block::JUNGLE_LOG,
        block::ACACIA_LOG,
        block::DARK_OAK_LOG,
    ]
    .iter()
    .any(|&vertical| (vertical - 1..=vertical + 1).contains(&state))
}

fn leaf_state_base(state: u32) -> Option<u32> {
    [
        block::OAK_LEAVES,
        block::BIRCH_LEAVES,
        block::SPRUCE_LEAVES,
        block::JUNGLE_LEAVES,
        block::ACACIA_LEAVES,
        block::DARK_OAK_LEAVES,
    ]
    .into_iter()
    // Seven distances, two persistence values and two waterlogging values.
    .find(|&default| (default - 27..=default).contains(&state))
    .map(|default| default - 27)
}

fn is_leaves(state: u32) -> bool {
    leaf_state_base(state).is_some()
}

/// Vanilla `TrunkPlacer.getTreeHeight` — identical across the straight-family
/// placers: `base + next(a+1) + next(b+1)`.
fn sample_tree_height<R: TreeRandom + ?Sized>(random: &mut R, trunk: TrunkPlacer) -> i32 {
    let (a, b) = trunk.random_range();
    trunk.base_height() + random.next_i32_bounded(a + 1) + random.next_i32_bounded(b + 1)
}

/// Vanilla `FoliagePlacer.foliageHeight`.
fn sample_foliage_height<R: TreeRandom + ?Sized>(
    random: &mut R,
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
fn sample_foliage_radius<R: TreeRandom + ?Sized>(
    random: &mut R,
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
fn sample_foliage_offset<R: TreeRandom + ?Sized>(random: &mut R, foliage: FoliagePlacer) -> i32 {
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
fn place_trunk<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    chunk: &mut P,
    random: &mut R,
    config: &TreeConfig,
    origin: (i32, i32, i32),
    height: i32,
    attachments: &mut Vec<FoliageAttachment>,
) {
    let (ox, oy, oz) = origin;
    match config.trunk {
        TrunkPlacer::Straight { .. } => {
            place_below_trunk_block(chunk, config, (ox, oy - 1, oz));
            for y in 0..height {
                place_log(chunk, config, (ox, oy + y, oz));
            }
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
        TrunkPlacer::Fancy { .. } => {
            fancy::place_trunk(chunk, random, config, origin, height, attachments);
        }
    }
}

/// Vanilla `ForkingTrunkPlacer.placeTrunk` (acacia).
fn place_forking_trunk<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    chunk: &mut P,
    random: &mut R,
    config: &TreeConfig,
    origin: (i32, i32, i32),
    height: i32,
    attachments: &mut Vec<FoliageAttachment>,
) {
    let (ox, oy, oz) = origin;
    place_below_trunk_block(chunk, config, (ox, oy - 1, oz));
    let mut x = ox;
    let mut z = oz;
    let direction = random.next_i32_bounded(4);
    let lean_height = height - random.next_i32_bounded(4) - 1;
    let mut branch_steps = 3 - random.next_i32_bounded(3);
    let directions = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    let (dx, dz) = directions[direction as usize];
    let mut top = None;
    for y in 0..height {
        if y >= lean_height && branch_steps > 0 {
            x += dx;
            z += dz;
            branch_steps -= 1;
        }
        if place_log(chunk, config, (x, oy + y, z)) {
            top = Some(oy + y + 1);
        }
    }
    if let Some(y) = top {
        attachments.push(FoliageAttachment {
            x,
            y,
            z,
            radius_offset: 1,
            double_trunk: false,
        });
    }
    x = ox;
    z = oz;
    let other = random.next_i32_bounded(4);
    if other != direction {
        let start = lean_height - random.next_i32_bounded(2) - 1;
        let length = 1 + random.next_i32_bounded(3);
        let (dx, dz) = directions[other as usize];
        top = None;
        for y in start..height.min(start + length) {
            if y >= 1 {
                x += dx;
                z += dz;
                if place_log(chunk, config, (x, oy + y, z)) {
                    top = Some(oy + y + 1);
                }
            }
        }
        if let Some(y) = top {
            attachments.push(FoliageAttachment {
                x,
                y,
                z,
                radius_offset: 0,
                double_trunk: false,
            });
        }
    }
}

/// Vanilla `createFoliage` dispatch.
fn create_foliage<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    chunk: &mut P,
    random: &mut R,
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
            let base = FoliageAttachment {
                y: attachment.y + offset,
                ..*attachment
            };
            place_leaves_row(
                chunk,
                random,
                config,
                &base,
                leaf_radius + attachment.radius_offset,
                -1 - foliage_height,
            );
            place_leaves_row(
                chunk,
                random,
                config,
                &base,
                leaf_radius - 1,
                -foliage_height,
            );
            place_leaves_row(
                chunk,
                random,
                config,
                &base,
                leaf_radius + attachment.radius_offset - 1,
                0,
            );
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
fn place_leaves_row<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    chunk: &mut P,
    random: &mut R,
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
fn should_skip_location<R: TreeRandom + ?Sized>(
    random: &mut R,
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
            if y == 0 {
                (dx > 1 || dz > 1) && dx != 0 && dz != 0
            } else {
                dx == current_radius && dz == current_radius && current_radius > 0
            }
        }
        FoliagePlacer::Fancy { .. } => {
            let x = dx as f32 + 0.5;
            let z = dz as f32 + 0.5;
            x * x + z * z > (current_radius * current_radius) as f32
        }
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
fn try_place_leaf<P: ShapeSink + ?Sized>(
    chunk: &mut P,
    config: &TreeConfig,
    pos: (i32, i32, i32),
) -> bool {
    let Some(state) = chunk.state(pos) else {
        return false;
    };
    if !chunk.valid_state(state) {
        return false;
    }
    chunk.leaf(pos, config.leaves)
}

/// Vanilla `TreeFeature.placeLog`.
fn place_log<P: ShapeSink + ?Sized>(chunk: &mut P, config: &TreeConfig, pos: fallen::Pos) -> bool {
    let Some(state) = chunk.state(pos) else {
        return false;
    };
    if !chunk.valid_state(state) {
        return false;
    }
    chunk.log(pos, config.log)
}

/// Vanilla `TrunkPlacer.placeBelowTrunkBlock` (the supportive dirt).
fn place_below_trunk_block<P: ShapeSink + ?Sized>(
    chunk: &mut P,
    _config: &TreeConfig,
    pos: (i32, i32, i32),
) {
    chunk.below_trunk(pos);
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
        || is_leaves(state)
        || matches!(
            state,
            block::WATER | block::SHORT_GRASS | block::LEAF_LITTER | block::DEAD_BUSH
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChunkPos;

    fn flat_chunk() -> GeneratedChunk {
        let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
        for z in 0..16 {
            for x in 0..16 {
                chunk.set(x, 64, z, block::GRASS_BLOCK);
            }
        }
        chunk
    }

    #[test]
    fn isolated_trees_match_vanilla_26_1() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../data/trees_26_1.json")).unwrap();
        let mut failures = Vec::new();
        for sample in reference["samples"].as_array().unwrap() {
            let kind = sample["kind"].as_str().unwrap();
            let config = match kind {
                "oak" => OAK,
                "birch" => BIRCH,
                "spruce" => SPRUCE,
                "pine" => PINE,
                "fancy_oak" => FANCY_OAK,
                "oak_bees_0002_leaf_litter" => OAK_BEES_0002_LEAF_LITTER,
                "fancy_oak_bees_0002_leaf_litter" => FANCY_OAK_BEES_0002_LEAF_LITTER,
                _ => panic!("unknown fixture tree {kind}"),
            };
            let seed = sample["seed"].as_u64().unwrap();
            let mut chunk = flat_chunk();
            let soil = sample["soil"].as_u64().unwrap_or(block::GRASS_BLOCK as u64) as u32;
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, 64, z, soil);
                }
            }
            let mut random = WorldgenRandom::from_seed(seed);
            assert_eq!(
                place_tree(&mut chunk, &mut random, &config, (8, 65, 8)),
                sample["placed"].as_bool().unwrap()
            );
            let bytes: Vec<_> = chunk
                .states()
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect();
            let hash = format!("{:x}", md5::compute(bytes));
            let next = random.next_i64();
            if hash != sample["states_md5"].as_str().unwrap()
                || next != sample["next_i64"].as_i64().unwrap()
            {
                failures.push(format!(
                    "{kind} seed={seed}: states={hash}, next_i64={next}"
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn leaf_update_preserves_flags_and_stops_at_placement_bounds() {
        for flags in 0..4 {
            let mut chunk = flat_chunk();
            let base = block::OAK_LEAVES - 27;
            let original = base + 24 + flags;
            chunk.set(11, 70, 8, original);
            chunk.set(3, 71, 8, original);
            let mut placement = TreePlacement::new(&mut chunk);
            placement.set(2, 70, 8, block::OAK_LOG);
            for x in 3..=10 {
                placement.set(x, 70, 8, original);
            }
            placement.update_leaves();
            for x in 3..=10 {
                let distance = (x as u32 - 2).min(7);
                assert_eq!(
                    placement.get(x, 70, 8),
                    Some(base + (distance - 1) * 4 + flags)
                );
            }
            assert_eq!(placement.get(11, 70, 8), Some(original));
            assert_eq!(placement.get(3, 71, 8), Some(original));
        }
    }

    #[test]
    fn obstructed_tree_is_atomic_and_stops_before_foliage_randomness() {
        let mut chunk = flat_chunk();
        // Adjacent stone intersects oak's upper clearance, not its trunk.
        chunk.set(9, 67, 8, block::STONE);
        let before = chunk.states().to_vec();
        let mut actual = WorldgenRandom::from_seed(17);
        let mut expected = WorldgenRandom::from_seed(17);
        expected.next_i32_bounded(3);
        expected.next_i32_bounded(1);
        assert!(!place_tree(&mut chunk, &mut actual, &OAK, (8, 65, 8)));
        assert_eq!(chunk.states(), before);
        assert_eq!(actual.next_i64(), expected.next_i64());
    }

    #[test]
    fn clearance_respects_layer_boundaries_and_top_margin() {
        // Expected clearances from the 26.1 configured features at height 6.
        // Each row is (config, dx, dy, expected free height).
        for (config, dx, dy, expected) in [
            (OAK, 1, 0, 6),
            (OAK, 1, 1, -1),
            (OAK, 1, 7, 5),
            (OAK, 1, 8, 6),
            (SPRUCE, 2, 1, 6),
            (SPRUCE, 2, 2, 0),
            (PINE, 2, 2, 0),
            (ACACIA, 2, 1, -1),
            (DARK_OAK, 2, 4, 6),
            (DARK_OAK, 2, 5, 3),
            (FANCY_OAK, 1, 3, 6),
            (FANCY_OAK, 0, 6, 4),
        ] {
            let mut chunk = flat_chunk();
            chunk.set((8 + dx) as usize, 65 + dy, 8, block::STONE);
            assert_eq!(
                max_free_tree_height(&chunk, config.minimum_size, (8, 65, 8), 6),
                expected,
                "{config:?}, obstacle at dx={dx}, dy={dy}"
            );
        }
    }

    #[test]
    fn clipped_height_requires_opt_in_and_controls_trunk_length() {
        // Use a straight shape to isolate minimum_size from branch geometry.
        let config = TreeConfig {
            trunk: TrunkPlacer::Straight {
                base_height: 8,
                height_rand_a: 0,
                height_rand_b: 0,
            },
            min_clipped_height: Some(4),
            ..OAK
        };
        for (obstacle_dy, placed) in [(5, false), (6, true)] {
            let mut chunk = flat_chunk();
            chunk.set(8, 65 + obstacle_dy, 8, block::STONE);
            let before = chunk.states().to_vec();
            let mut random = WorldgenRandom::from_seed(1);
            assert_eq!(
                place_tree(&mut chunk, &mut random, &config, (8, 65, 8)),
                placed
            );
            if placed {
                assert_eq!(
                    (65..80)
                        .filter(|&y| chunk.get(8, y, 8) == Some(block::OAK_LOG))
                        .count(),
                    4
                );
            } else {
                assert_eq!(chunk.states(), before);
            }
            assert_eq!(chunk.get(8, 65 + obstacle_dy, 8), Some(block::STONE));
        }
        let mut chunk = flat_chunk();
        chunk.set(8, 71, 8, block::STONE);
        let before = chunk.states().to_vec();
        let mut random = WorldgenRandom::from_seed(1);
        assert!(!place_tree(
            &mut chunk,
            &mut random,
            &TreeConfig {
                min_clipped_height: None,
                ..config
            },
            (8, 65, 8)
        ));
        assert_eq!(chunk.states(), before);
    }

    #[test]
    fn tree_height_accepts_the_last_legal_layer() {
        let config = TreeConfig {
            trunk: TrunkPlacer::Straight {
                base_height: 4,
                height_rand_a: 0,
                height_rand_b: 0,
            },
            ..OAK
        };
        for (y, placed) in [(-64, false), (-63, true), (315, true), (316, false)] {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(0, 0));
            let mut random = WorldgenRandom::from_seed(1);
            assert_eq!(
                place_tree(&mut chunk, &mut random, &config, (8, y, 8)),
                placed
            );
        }
    }

    #[test]
    fn clearance_accepts_existing_logs_and_all_supported_leaf_states() {
        for state in [block::BIRCH_LOG - 1, block::BIRCH_LOG, block::BIRCH_LOG + 1] {
            let mut chunk = flat_chunk();
            chunk.set(8, 67, 8, state);
            let mut random = WorldgenRandom::from_seed(17);
            assert!(place_tree(&mut chunk, &mut random, &OAK, (8, 65, 8)));
            assert_eq!(chunk.get(8, 67, 8), Some(state));
        }
        for default in [
            block::OAK_LEAVES,
            block::BIRCH_LEAVES,
            block::SPRUCE_LEAVES,
            block::JUNGLE_LEAVES,
            block::ACACIA_LEAVES,
            block::DARK_OAK_LEAVES,
        ] {
            for state in default - 27..=default {
                let mut chunk = flat_chunk();
                chunk.set(8, 67, 8, state);
                let mut random = WorldgenRandom::from_seed(17);
                assert!(place_tree(&mut chunk, &mut random, &OAK, (8, 65, 8)));
                assert_eq!(chunk.get(8, 67, 8), Some(block::OAK_LOG));
            }
        }
    }

    #[test]
    fn foliage_does_not_wrap_into_the_opposite_chunk_edge() {
        for pos in [ChunkPos::new(0, 0), ChunkPos::new(-2, 3)] {
            let mut chunk = GeneratedChunk::new(pos);
            let origin = (pos.x * 16, 65, pos.z * 16 + 8);
            let mut random = WorldgenRandom::from_seed(1);
            assert!(place_tree(&mut chunk, &mut random, &OAK, origin));
            assert!((65..80).all(|y| chunk.get(15, y, 8) == Some(block::AIR)));
            assert!((65..80).any(|y| chunk.get(1, y, 8).is_some_and(is_leaves)));
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
        place_on_ground(
            &mut TreePlacement::new(&mut chunk),
            &mut random,
            GroundBounds {
                x: (8, 8),
                y: 65,
                z: (8, 8),
            },
            4,
            2,
            96,
            3,
        );
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
        let mut chunk = flat_chunk();
        let mut random = WorldgenRandom::from_seed(1);
        let ground = 64;
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
                    if chunk.get(lx, y, lz).is_some_and(is_leaves) {
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
