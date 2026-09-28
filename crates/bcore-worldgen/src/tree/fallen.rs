//! Isolated 26.1 `FallenTreeFeature`, using world-space reads and writes.
//!
//! The native feature places its stump unconditionally, decorates it, then checks
//! the entire horizontal log before writing that log. Its return value is always
//! true, including unsupported terrain and rejected writes. Configurations and
//! state IDs are pinned in `data/fallen_tree_configs_26_1.json`.

use super::{IntProvider, TreeRandom};
use crate::{block, heightmap::is_air};

pub type Pos = (i32, i32, i32);

/// World access needed by the native feature. Coordinates are absolute, including
/// neighbours outside the originating chunk. Reads must reflect earlier writes;
/// an unavailable neighbour must not be substituted with air or a wrapped column.
pub trait FallenTreeWorld {
    fn get_block(&self, pos: Pos) -> u32;

    /// Native update flags: 3 for logs, 19 for decorations. The native feature
    /// ignores the return value; the world owns build-height/write restrictions.
    fn set_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool;

    /// `state.isFaceSturdy(world, at, UP, FULL)`, including contextual shapes.
    /// Vanilla reads the state *below* a log but evaluates it at the log position.
    /// This is a full support-face test, not a motion-blocking or non-air test.
    fn is_face_sturdy_up(&self, state: u32, at: Pos) -> bool;

    /// Queue this absolute position in its owning chunk for postprocessing.
    fn mark_for_postprocessing(&mut self, pos: Pos);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Direction {
    fn relative(self, (x, y, z): Pos, distance: i32) -> Pos {
        match self {
            Self::Down => (x, y - distance, z),
            Self::Up => (x, y + distance, z),
            Self::North => (x, y, z - distance),
            Self::South => (x, y, z + distance),
            Self::West => (x - distance, y, z),
            Self::East => (x + distance, y, z),
        }
    }

    fn axis_index(self) -> usize {
        match self {
            Self::West | Self::East => 0,
            Self::Down | Self::Up => 1,
            Self::North | Self::South => 2,
        }
    }
}

/// The decorator/provider forms used by the five pinned fallen-tree configs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FallenTreeDecorator {
    TrunkVine,
    AttachedToLogs {
        probability: f32,
        /// Ordered `(state, weight)` entries of a native weighted state provider.
        entries: &'static [(u32, u32)],
        directions: &'static [Direction],
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FallenTreeConfig {
    /// Simple trunk-provider state with axis X, Y and Z respectively. For a state
    /// without an axis, all three entries must be the same (`trySetValue`).
    pub trunk_states: [u32; 3],
    pub log_length: IntProvider,
    pub stump_decorators: &'static [FallenTreeDecorator],
    pub log_decorators: &'static [FallenTreeDecorator],
}

const MUSHROOM_DECORATORS: &[FallenTreeDecorator] = &[FallenTreeDecorator::AttachedToLogs {
    probability: 0.1,
    entries: &[(2337, 2), (2336, 1)],
    directions: &[Direction::Up],
}];

pub const OAK: FallenTreeConfig = FallenTreeConfig {
    trunk_states: [block::OAK_LOG - 1, block::OAK_LOG, block::OAK_LOG + 1],
    log_length: IntProvider::Uniform { min: 4, max: 7 },
    stump_decorators: &[FallenTreeDecorator::TrunkVine],
    log_decorators: MUSHROOM_DECORATORS,
};

pub const BIRCH: FallenTreeConfig = FallenTreeConfig {
    trunk_states: [block::BIRCH_LOG - 1, block::BIRCH_LOG, block::BIRCH_LOG + 1],
    log_length: IntProvider::Uniform { min: 5, max: 8 },
    stump_decorators: &[],
    log_decorators: MUSHROOM_DECORATORS,
};

pub const SUPER_BIRCH: FallenTreeConfig = FallenTreeConfig {
    log_length: IntProvider::Uniform { min: 5, max: 15 },
    ..BIRCH
};

pub const JUNGLE: FallenTreeConfig = FallenTreeConfig {
    trunk_states: [
        block::JUNGLE_LOG - 1,
        block::JUNGLE_LOG,
        block::JUNGLE_LOG + 1,
    ],
    log_length: IntProvider::Uniform { min: 4, max: 11 },
    ..OAK
};

pub const SPRUCE: FallenTreeConfig = FallenTreeConfig {
    trunk_states: [
        block::SPRUCE_LOG - 1,
        block::SPRUCE_LOG,
        block::SPRUCE_LOG + 1,
    ],
    log_length: IntProvider::Uniform { min: 6, max: 10 },
    ..BIRCH
};

/// Resolve a native configured-feature ID. Unknown configurations are explicit.
pub fn config_for(name: &str) -> Option<FallenTreeConfig> {
    Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
        "fallen_oak_tree" => OAK,
        "fallen_birch_tree" => BIRCH,
        "fallen_super_birch_tree" => SUPER_BIRCH,
        "fallen_jungle_tree" => JUNGLE,
        "fallen_spruce_tree" => SPRUCE,
        _ => return None,
    })
}

/// Native `TreeFeature.validTreePos`, with every state in the pinned
/// `minecraft:replaceable_by_trees` tag, rather than the chunk tree subset.
pub fn valid_tree_state(state: u32) -> bool {
    is_air(state)
        || matches!(
            state,
            86..=101
                | 252..=559
                | 2248..=2256
                | 2321..=2335
                | 8358..=8517
                | 12915..=12926
                | 14809..=14810
                | 20960..=20961
                | 21031
                | 27846..=27861
                | 27919..=27920
                | 29704..=29865
                | 29868..=29869
                | 29872
        )
}

/// Place one configured fallen tree using the caller's feature RNG stream.
/// Stump writes survive a failed horizontal-log clearance check, as in vanilla.
pub fn place<W: FallenTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    config: &FallenTreeConfig,
    origin: Pos,
) -> bool {
    validate_config(config);
    place_log(world, origin, config.trunk_states[1]);
    decorate_logs(world, random, &[origin], config.stump_decorators);

    // Direction.Plane.HORIZONTAL has N/E/S/W order (not enum declaration order).
    let direction = [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
    ][random.next_i32_bounded(4) as usize];
    let length = config.log_length.sample(random) - 2;
    let start = direction.relative(origin, 2 + random.next_i32_bounded(2));
    let start = ground_start(world, start);

    if can_place_log(world, start, direction, length) {
        let mut logs = Vec::new();
        for step in 0..length {
            let pos = direction.relative(start, step);
            place_log(world, pos, config.trunk_states[direction.axis_index()]);
            logs.push(pos);
        }
        java_log_order(&mut logs);
        decorate_logs(world, random, &logs, config.log_decorators);
    }
    true
}

fn validate_config(config: &FallenTreeConfig) {
    let (min, max) = match config.log_length {
        IntProvider::Constant(value) => (value, value),
        IntProvider::Uniform { min, max } => (min, max),
    };
    assert!(
        0 <= min && min <= max && max <= 16,
        "native log_length range is 0..=16"
    );
    for decorator in config.stump_decorators.iter().chain(config.log_decorators) {
        if let FallenTreeDecorator::AttachedToLogs {
            probability,
            entries,
            directions,
        } = decorator
        {
            assert!((0.0..=1.0).contains(probability));
            assert!(!directions.is_empty());
            weighted_total(entries);
        }
    }
}

fn place_log<W: FallenTreeWorld + ?Sized>(world: &mut W, pos: Pos, state: u32) {
    world.set_block(pos, state, 3);
    // Feature.markAboveForPostProcessing stops at the first air block.
    for height in 1..=2 {
        let above = Direction::Up.relative(pos, height);
        if is_air(world.get_block(above)) {
            break;
        }
        world.mark_for_postprocessing(above);
    }
}

fn over_solid_ground<W: FallenTreeWorld + ?Sized>(world: &W, pos: Pos) -> bool {
    world.is_face_sturdy_up(world.get_block(Direction::Down.relative(pos, 1)), pos)
}

fn ground_start<W: FallenTreeWorld + ?Sized>(world: &W, mut pos: Pos) -> Pos {
    pos = Direction::Up.relative(pos, 1);
    for _ in 0..6 {
        if valid_tree_state(world.get_block(pos)) && over_solid_ground(world, pos) {
            return pos;
        }
        pos = Direction::Down.relative(pos, 1);
    }
    pos
}

fn can_place_log<W: FallenTreeWorld + ?Sized>(
    world: &W,
    start: Pos,
    direction: Direction,
    length: i32,
) -> bool {
    let mut gap = 0;
    for step in 0..length {
        let pos = direction.relative(start, step);
        if !valid_tree_state(world.get_block(pos)) {
            return false;
        }
        if over_solid_ground(world, pos) {
            gap = 0;
        } else {
            gap += 1;
            if gap > 2 {
                return false;
            }
        }
    }
    true
}

fn java_log_order(logs: &mut [Pos]) {
    // A horizontal run has at most 14 distinct positions (the native codec cap
    // minus two). HashSet starts with 16 buckets and grows to 32 at entry 13.
    // Stable bucket sorting preserves collision-chain order through that resize.
    let mask = if logs.len() > 12 { 31 } else { 15 };
    logs.sort_by_key(|&(x, y, z)| {
        let hash = y
            .wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x) as u32;
        (hash ^ (hash >> 16)) & mask
    });
    // Context's stable Y sort leaves this horizontal run in HashSet order.
}

fn decorate_logs<W: FallenTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    logs: &[Pos],
    decorators: &[FallenTreeDecorator],
) {
    for decorator in decorators {
        match *decorator {
            FallenTreeDecorator::TrunkVine => {
                for &pos in logs {
                    for (direction, state) in [
                        (Direction::West, 8373),  // east=true
                        (Direction::East, 8388),  // west=true
                        (Direction::North, 8385), // south=true
                        (Direction::South, 8381), // north=true
                    ] {
                        if random.next_i32_bounded(3) > 0 {
                            let adjacent = direction.relative(pos, 1);
                            if is_air(world.get_block(adjacent)) {
                                world.set_block(adjacent, state, 19);
                            }
                        }
                    }
                }
            }
            FallenTreeDecorator::AttachedToLogs {
                probability,
                entries,
                directions,
            } => {
                let mut shuffled = logs.to_vec();
                // Util.shuffledCopy: descending Fisher-Yates, on a fresh copy
                // for each decorator. The context list itself is never shuffled.
                for size in (2..=shuffled.len()).rev() {
                    let index = random.next_i32_bounded(size as i32) as usize;
                    shuffled.swap(size - 1, index);
                }
                for pos in shuffled {
                    // Even a singleton direction list consumes nextInt(1).
                    let direction =
                        directions[random.next_i32_bounded(directions.len() as i32) as usize];
                    let adjacent = direction.relative(pos, 1);
                    if random.next_f32() <= probability && is_air(world.get_block(adjacent)) {
                        let state = sample_weighted(random, entries);
                        world.set_block(adjacent, state, 19);
                    }
                }
            }
        }
    }
}

fn weighted_total(entries: &[(u32, u32)]) -> i32 {
    let total: u64 = entries.iter().map(|&(_, weight)| u64::from(weight)).sum();
    assert!(
        total > 0 && total <= i32::MAX as u64,
        "invalid native weighted state provider"
    );
    total as i32
}

fn sample_weighted<R: TreeRandom + ?Sized>(random: &mut R, entries: &[(u32, u32)]) -> u32 {
    let mut choice = random.next_i32_bounded(weighted_total(entries)) as u32;
    for &(state, weight) in entries {
        if choice < weight {
            return state;
        }
        choice -= weight;
    }
    unreachable!("validated weighted state provider");
}
