//! Absolute-coordinate TreeFeature, sharing the legacy trunk/foliage algorithms.
//! Leaf propagation, decorators and shape-edge updates use the live region.
use std::collections::BTreeMap;

use super::{
    extra_data,
    fallen::{self, FallenTreeWorld, Pos},
    grow_shape, ShapeSink, TreeConfig, TreeRandom, TrunkPlacer,
};
use crate::{block, heightmap::is_air, MAX_Y, MIN_Y};

#[path = "standing_blocks.rs"]
mod blocks;
#[path = "extra_decorators.rs"]
mod extra_decorators;
#[path = "extra_roots.rs"]
mod extra_roots;
#[path = "positions.rs"]
mod positions;
use positions::Positions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedShape {
    pub pos: Pos,
    pub state: u32,
}

impl std::fmt::Display for UnsupportedShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsupported standing-tree edge shape {} at {:?}",
            self.state, self.pos
        )
    }
}
impl std::error::Error for UnsupportedShape {}

/// Staged effects, transferred by FeatureRegion into their owning chunks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeEffects {
    pub beehives: BTreeMap<Pos, Vec<i32>>,
    /// x, y, z, default block-state / fluid-registry ID, delay, fluid flag.
    pub tick_requests: Vec<[i32; 6]>,
}

impl TreeEffects {
    /// Native generated-nest metadata, before occupants tick or gather nectar.
    pub fn beehive_data(&self, (x, y, z): Pos) -> Option<serde_json::Value> {
        Some(
            crate::block_entity::BlockEntity::Beehive {
                ticks_in_hive: self.beehives.get(&(x, y, z))?.clone(),
            }
            .full_data((x, y, z)),
        )
    }
}

pub trait StandingTreeWorld: FallenTreeWorld {
    /// Resolve the actual block entity after the nest write, including rejection.
    fn has_beehive(&mut self, pos: Pos) -> bool;
    fn store_bee(&mut self, pos: Pos, ticks_in_hive: i32);
    fn schedule_tree_tick(&mut self, request: [i32; 6]);

    /// Native getRawBrightness(pos, 0), needed by mushroom survival checks.
    fn tree_raw_brightness(&self, _pos: Pos) -> Option<i32> {
        None
    }

    /// Nested configured features (not reseeded), e.g. pale_moss_patch.
    fn place_tree_subfeature<R: TreeRandom + ?Sized>(
        &mut self,
        _random: &mut R,
        _name: &'static str,
        pos: Pos,
    ) -> Result<bool, UnsupportedShape> {
        Err(UnsupportedShape {
            pos,
            state: self.get_block(pos),
        })
    }

    /// Write with flags 19 and create/preserve the native heart block entity.
    fn set_creaking_heart(&mut self, pos: Pos, state: u32) -> Result<bool, UnsupportedShape> {
        Err(UnsupportedShape { pos, state })
    }

    fn motion_no_leaves_height(&self, x: i32, z: i32) -> i32 {
        (MIN_Y..=MAX_Y)
            .rev()
            .find(|&y| blocks::get(self.get_block((x, y, z))).flags & 2 != 0)
            .map_or(MIN_Y, |y| y + 1)
    }
}

/// Resolve pinned tree geometry. Placement can require additional world callbacks
/// for contextual survival, nested features or block entities.
pub fn config_for(name: &str) -> Option<TreeConfig> {
    legacy_config_for(name).or_else(|| {
        extra_data::tree(name.strip_prefix("minecraft:").unwrap_or(name)).map(|d| d.shape)
    })
}

fn legacy_config_for(name: &str) -> Option<TreeConfig> {
    Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
        "oak" => super::OAK,
        "birch" => super::BIRCH,
        "spruce" => super::SPRUCE,
        "pine" => super::PINE,
        "fancy_oak" => super::FANCY_OAK,
        "acacia" => super::ACACIA,
        "oak_bees_005" => TreeConfig {
            beehive_probability: Some(0.05),
            ..super::OAK
        },
        "fancy_oak_bees_005" => TreeConfig {
            beehive_probability: Some(0.05),
            ..super::FANCY_OAK
        },
        "birch_bees_0002" => super::BIRCH_BEES_0002,
        "super_birch_bees_0002" => TreeConfig {
            trunk: TrunkPlacer::Straight {
                base_height: 5,
                height_rand_a: 2,
                height_rand_b: 6,
            },
            ..super::BIRCH_BEES_0002
        },
        "oak_bees_0002_leaf_litter" => super::OAK_BEES_0002_LEAF_LITTER,
        "birch_bees_0002_leaf_litter" => super::BIRCH_BEES_0002_LEAF_LITTER,
        "fancy_oak_bees_0002_leaf_litter" => super::FANCY_OAK_BEES_0002_LEAF_LITTER,
        _ => return None,
    })
}

pub(crate) fn bee_nest_state() -> u32 {
    blocks::bee_nest_state()
}

struct Placement<'a, W: ?Sized> {
    world: &'a mut W,
    logs: Positions,
    leaves: Positions,
    decorations: Positions,
    roots: Positions,
    definition: Option<&'static extra_data::TreeDefinition>,
}

impl<W: StandingTreeWorld + ?Sized> ShapeSink for Placement<'_, W> {
    fn state(&self, pos: Pos) -> Option<u32> {
        Some(self.world.get_block(pos))
    }
    fn valid_state(&self, state: u32) -> bool {
        fallen::valid_tree_state(state)
    }
    fn free_state(&self, state: u32) -> bool {
        self.loggable_state(state) || extra_data::tagged(state, "minecraft:logs")
    }
    fn loggable_state(&self, state: u32) -> bool {
        self.valid_state(state)
            || self
                .definition
                .and_then(|d| d.grow_through.as_deref())
                .is_some_and(|tag| extra_data::tagged(state, tag))
    }
    fn air_or_leaves(&self, state: u32) -> bool {
        is_air(state) || extra_data::tagged(state, "minecraft:leaves")
    }
    fn ignore_vines(&self) -> bool {
        self.definition.is_none_or(|d| d.ignore_vines)
    }
    fn log(&mut self, pos: Pos, state: u32) -> bool {
        self.logs.insert(pos);
        self.world.set_block(pos, state, 19);
        true // Native setters record positions even when the world rejects a write.
    }
    fn leaf(&mut self, pos: Pos, mut state: u32) -> bool {
        let previous = blocks::get(self.world.get_block(pos));
        if previous.flags & 32 != 0 {
            return false;
        }
        if previous.flags & 4 != 0 {
            state -= 1;
        } // WATERLOGGED true precedes false.
        self.leaves.insert(pos);
        self.world.set_block(pos, state, 19);
        true
    }
    fn below_trunk<R: TreeRandom + ?Sized>(&mut self, random: &mut R, pos: Pos) {
        if let Some(definition) = self.definition {
            if let Some(state) = definition.below.sample(self.world, random, pos) {
                self.log(pos, state);
            }
        } else if !crate::heightmap::protected_below_trunk(self.world.get_block(pos)) {
            self.log(pos, block::DIRT);
        }
    }

    fn leaf_is_set(&self, pos: Pos) -> bool {
        self.leaves.contains(pos)
    }
    fn provided_leaf<R: TreeRandom + ?Sized>(
        &mut self,
        random: &mut R,
        pos: Pos,
        default: u32,
    ) -> bool {
        if blocks::get(self.world.get_block(pos)).flags & 32 != 0 {
            return false;
        }
        let state = self.definition.map_or(default, |d| {
            d.foliage
                .sample(self.world, random, pos)
                .expect("foliage provider state")
        });
        self.leaf(pos, state)
    }
    fn trunk_origin<R: TreeRandom + ?Sized>(&self, random: &mut R, origin: Pos) -> Pos {
        let offset = self.definition.and_then(|d| d.roots).map_or(0, |root| {
            extra_data::int_provider(&root["trunk_offset_y"]).sample(random)
        });
        (origin.0, origin.1 + offset, origin.2)
    }
    fn roots<R: TreeRandom + ?Sized>(&mut self, random: &mut R, origin: Pos, trunk: Pos) -> bool {
        self.definition
            .and_then(|d| d.roots)
            .is_none_or(|root| extra_roots::place(self, random, origin, trunk, root))
    }
}

impl<W: StandingTreeWorld + ?Sized> Placement<'_, W> {
    fn decorate(&mut self, pos: Pos, state: u32) {
        self.decorations.insert(pos);
        self.world.set_block(pos, state, 19);
    }

    fn beehive<R: TreeRandom + ?Sized>(&mut self, random: &mut R, probability: f32) {
        let logs = self.logs.sorted_y();
        if logs.is_empty() || random.next_f32() >= probability {
            return;
        }
        let leaves = self.leaves.sorted_y();
        let y = if let Some(lowest) = leaves.first() {
            (lowest.1 - 1).max(logs[0].1 + 1)
        } else {
            (logs[0].1 + 1 + random.next_i32_bounded(3))
                .min(logs.last().expect("nonempty trunk positions").1)
        };
        let mut candidates = Vec::new();
        for (x, ly, z) in logs.into_iter().filter(|p| p.1 == y) {
            candidates.extend([(x + 1, ly, z), (x, ly, z + 1), (x - 1, ly, z)]);
        }
        for size in (2..=candidates.len()).rev() {
            candidates.swap(size - 1, random.next_i32_bounded(size as i32) as usize);
        }
        if let Some(pos) = candidates.into_iter().find(|&(x, y, z)| {
            is_air(self.world.get_block((x, y, z))) && is_air(self.world.get_block((x, y, z + 1)))
        }) {
            self.decorate(pos, bee_nest_state());
            if self.world.has_beehive(pos) {
                let count = 2 + random.next_i32_bounded(2);
                for _ in 0..count {
                    self.world.store_bee(pos, random.next_i32_bounded(599));
                }
            }
        }
    }

    fn litter<R: TreeRandom + ?Sized>(
        &mut self,
        random: &mut R,
        radius: i32,
        tries: i32,
        segments: i32,
    ) {
        let logs = self.logs.sorted_y();
        let Some(&(x, y, z)) = logs.first() else {
            return;
        };
        let (mut xmin, mut xmax, mut zmin, mut zmax) = (x, x, z, z);
        for &(x, _, z) in logs.iter().filter(|p| p.1 == y) {
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            zmin = zmin.min(z);
            zmax = zmax.max(z);
        }
        for _ in 0..tries {
            let x = xmin - radius + random.next_i32_bounded(xmax - xmin + 2 * radius + 1);
            let y = y - 2 + random.next_i32_bounded(5);
            let z = zmin - radius + random.next_i32_bounded(zmax - zmin + 2 * radius + 1);
            let above = self.world.get_block((x, y + 1, z));
            if !is_air(above) && !(8358..=8389).contains(&above) {
                continue;
            }
            if blocks::get(self.world.get_block((x, y, z))).flags & 1 == 0 {
                continue;
            }
            if self.world.motion_no_leaves_height(x, z) > y + 1 {
                continue;
            }
            let index = random.next_i32_bounded(segments * 4);
            let facing = [0, 3, 1, 2][(index % 4) as usize];
            self.decorate(
                (x, y + 1, z),
                block::LEAF_LITTER + (facing * 4 + index / 4) as u32,
            );
        }
    }

    fn update_leaves(&mut self) -> Voxel {
        let mut points = self
            .logs
            .iter()
            .chain(self.leaves.iter())
            .chain(self.decorations.iter())
            .chain(self.roots.iter());
        let first = points.next().expect("nonempty tree");
        let (mut min, mut max) = (first, first);
        for (x, y, z) in points {
            min = (min.0.min(x), min.1.min(y), min.2.min(z));
            max = (max.0.max(x), max.1.max(y), max.2.max(z));
        }
        let mut voxel = Voxel::new(min, max);
        for pos in self.decorations.iter().chain(self.roots.iter()) {
            voxel.fill(pos);
        }
        let mut pending: [Positions; 7] = std::array::from_fn(|_| Positions::new());
        for pos in self.logs.iter() {
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
            let pos = pending[distance]
                .pop()
                .expect("nonempty leaf-distance bucket");
            if distance > 0 {
                let state = self.world.get_block(pos);
                let previous = blocks::get(state).distance;
                assert!(previous > 0, "leaf propagation target has no distance");
                self.world.set_block(
                    pos,
                    (state as i32 + (distance as i32 - previous) * 4) as u32,
                    19,
                );
            }
            voxel.fill(pos);
            for dir in 0..6 {
                let adjacent = relative(pos, dir);
                if !voxel.inside(adjacent) || voxel.full(adjacent) {
                    continue;
                }
                let current = blocks::get(self.world.get_block(adjacent)).distance;
                if current < 0 {
                    continue;
                }
                let next = (current as usize).min(distance + 1);
                if next < 7 {
                    pending[next].insert(adjacent);
                    distance = distance.min(next);
                }
            }
        }
        voxel
    }
}

pub(crate) fn relative((x, y, z): Pos, direction: usize) -> Pos {
    let (dx, dy, dz) = [
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
        (-1, 0, 0),
        (1, 0, 0),
    ][direction];
    (x + dx, y + dy, z + dz)
}

struct Voxel {
    min: Pos,
    max: Pos,
    cells: Vec<bool>,
    width: usize,
    depth: usize,
}
impl Voxel {
    fn new(min: Pos, max: Pos) -> Self {
        let width = (max.0 - min.0 + 1) as usize;
        let depth = (max.2 - min.2 + 1) as usize;
        Self {
            min,
            max,
            width,
            depth,
            cells: vec![false; width * depth * (max.1 - min.1 + 1) as usize],
        }
    }
    fn inside(&self, (x, y, z): Pos) -> bool {
        (self.min.0..=self.max.0).contains(&x)
            && (self.min.1..=self.max.1).contains(&y)
            && (self.min.2..=self.max.2).contains(&z)
    }
    fn index(&self, (x, y, z): Pos) -> usize {
        (y - self.min.1) as usize * self.width * self.depth
            + (z - self.min.2) as usize * self.width
            + (x - self.min.0) as usize
    }
    fn fill(&mut self, pos: Pos) {
        let i = self.index(pos);
        self.cells[i] = true;
    }
    fn full(&self, pos: Pos) -> bool {
        self.inside(pos) && self.cells[self.index(pos)]
    }

    fn update_edges<W: StandingTreeWorld + ?Sized>(
        &self,
        world: &mut W,
        flags: i32,
    ) -> Result<(), UnsupportedShape> {
        let dimensions = [
            self.max.0 - self.min.0 + 1,
            self.max.1 - self.min.1 + 1,
            self.max.2 - self.min.2 + 1,
        ];
        let base = [self.min.0, self.min.1, self.min.2];
        // Native forAllFaces cycles: (x,y,z), (z,x,y), (y,z,x).
        for (outer, middle, scan, negative) in [(0, 1, 2, 2), (2, 0, 1, 0), (1, 2, 0, 4)] {
            for a in 0..dimensions[outer] {
                for b in 0..dimensions[middle] {
                    let mut previous = false;
                    for c in 0..=dimensions[scan] {
                        let mut p = base;
                        p[outer] += a;
                        p[middle] += b;
                        p[scan] += c;
                        let current = c < dimensions[scan] && self.full((p[0], p[1], p[2]));
                        if current != previous {
                            let dir = if current {
                                negative
                            } else {
                                p[scan] -= 1;
                                negative + 1
                            };
                            let pos = (p[0], p[1], p[2]);
                            let neighbour = relative(pos, dir);
                            let old = world.get_block(pos);
                            let other = world.get_block(neighbour);
                            let updated = blocks::update(world, pos, old, dir, other)?;
                            if updated != old {
                                world.set_block(pos, updated, flags & !1);
                            }
                            let updated_other =
                                blocks::update(world, neighbour, other, dir ^ 1, updated)?;
                            if updated_other != other {
                                world.set_block(neighbour, updated_other, flags & !1);
                            }
                        }
                        previous = current;
                    }
                }
            }
        }
        Ok(())
    }
}

/// StructureTemplate's unknown-shape completion, used by overworld fossils.
/// Only accepted writes fill the voxel; callbacks run before the overlay pass.
pub(crate) fn update_template_shapes<W: StandingTreeWorld + ?Sized>(
    world: &mut W,
    positions: &[Pos],
    flags: i32,
) -> Result<(), UnsupportedShape> {
    let Some(&first) = positions.first() else {
        return Ok(());
    };
    let (mut min, mut max) = (first, first);
    for &(x, y, z) in positions {
        min = (min.0.min(x), min.1.min(y), min.2.min(z));
        max = (max.0.max(x), max.1.max(y), max.2.max(z));
    }
    let mut voxel = Voxel::new(min, max);
    for &pos in positions {
        voxel.fill(pos);
    }
    voxel.update_edges(world, flags)?;
    for &pos in positions {
        let old = world.get_block(pos);
        let mut updated = old;
        // BlockBehaviour.UPDATE_SHAPE_ORDER, not Direction enum order.
        for dir in [4, 5, 2, 3, 0, 1] {
            let neighbour = world.get_block(relative(pos, dir));
            updated = blocks::update(world, pos, updated, dir, neighbour)?;
        }
        if updated != old {
            world.set_block(pos, updated, (flags & !1) | 16);
        }
        // WorldGenRegion inherits LevelAccessor's empty updateNeighborsAt.
    }
    Ok(())
}

/// Unknown configurations return `Ok(None)` before drawing. Unsupported world
/// shape callbacks return an explicit error after any already-performed writes.
pub fn place<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    name: &str,
    origin: Pos,
) -> Result<Option<bool>, UnsupportedShape> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let legacy = legacy_config_for(name);
    let definition = if legacy.is_none() {
        extra_data::tree(name)
    } else {
        None
    };
    let Some(config) = legacy.or_else(|| definition.map(|d| d.shape)) else {
        return Ok(None);
    };
    let mut placement = Placement {
        world,
        logs: Positions::new(),
        leaves: Positions::new(),
        decorations: Positions::new(),
        roots: Positions::new(),
        definition,
    };
    if !grow_shape(&mut placement, random, &config, origin)
        || (placement.logs.len == 0 && placement.leaves.len == 0)
    {
        return Ok(Some(false));
    }
    if let Some(definition) = definition {
        extra_decorators::place(&mut placement, random, definition.decorators)?;
    } else {
        if let Some(chance) = config.beehive_probability {
            placement.beehive(random, chance);
        }
        if config.leaf_litter {
            placement.litter(random, 4, 96, 3);
            placement.litter(random, 2, 150, 4);
        }
    }
    let voxel = placement.update_leaves();
    voxel.update_edges(placement.world, 3)?;
    Ok(Some(true))
}
