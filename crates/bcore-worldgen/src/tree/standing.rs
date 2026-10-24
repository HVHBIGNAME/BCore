//! Absolute-coordinate TreeFeature, sharing the legacy trunk/foliage algorithms.
//! Leaf propagation, decorators and shape-edge updates use the live region.
use std::collections::BTreeMap;

use super::{
    extra_data,
    fallen::{self, FallenTreeWorld, Pos},
    grow_shape, ShapeSink, TreeConfig, TreeRandom,
};
use crate::block_predicate::FeatureResult;
use crate::feature_world::FeatureError;
use crate::{block, heightmap::is_air, MAX_Y, MIN_Y};

#[path = "standing_blocks.rs"]
mod blocks;
#[path = "extra_decorators.rs"]
pub(super) mod extra_decorators;
#[path = "extra_roots.rs"]
pub(super) mod extra_roots;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    Configuration(FeatureError),
    Shape(UnsupportedShape),
}

impl TreeError {
    pub fn shape(&self) -> Option<UnsupportedShape> {
        match self {
            Self::Shape(shape) => Some(*shape),
            Self::Configuration(_) => None,
        }
    }

    pub(crate) fn into_feature_error(self) -> FeatureError {
        match self {
            Self::Configuration(error) => error,
            Self::Shape(shape) => FeatureError::Unsupported(shape.to_string()),
        }
    }
}

impl From<FeatureError> for TreeError {
    fn from(error: FeatureError) -> Self {
        Self::Configuration(error)
    }
}

impl From<UnsupportedShape> for TreeError {
    fn from(error: UnsupportedShape) -> Self {
        Self::Shape(error)
    }
}

impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(error) => error.fmt(f),
            Self::Shape(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for TreeError {}

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

    fn has_scheduled_tree_tick(&self, _pos: Pos, _block: u32) -> Option<bool> {
        None
    }

    fn reset_potent_sulfur_countdown(&mut self, pos: Pos) -> Result<(), UnsupportedShape> {
        Err(UnsupportedShape {
            pos,
            state: self.get_block(pos),
        })
    }

    /// LevelAccessor's default notification is empty in native WorldGenRegion.
    fn update_template_neighbours(
        &mut self,
        _pos: Pos,
        _state: u32,
    ) -> Result<(), UnsupportedShape> {
        Ok(())
    }

    fn standing_biome(&self, _pos: Pos) -> FeatureResult<u32> {
        Err(FeatureError::MissingData(
            "standing-tree biome lookup".into(),
        ))
    }

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

/// Resolve target catalog geometry. Placement can require additional world callbacks
/// for contextual survival, nested features or block entities.
pub fn config_for(name: &str) -> FeatureResult<Option<TreeConfig>> {
    Ok(extra_data::tree(name)?.map(|d| d.shape))
}

pub(crate) fn check_configured(name: &str) -> crate::block_predicate::FeatureResult<()> {
    if extra_data::check_tree(name)? {
        Ok(())
    } else {
        Err(crate::feature_world::FeatureError::Unsupported(format!(
            "configured tree {name}"
        )))
    }
}

#[cfg(test)]
pub(crate) fn historical_config_for(name: &str) -> Option<TreeConfig> {
    use super::TrunkPlacer;
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
    definition: Option<&'a extra_data::TreeDefinition>,
    failure: Option<FeatureError>,
}

impl<W: StandingTreeWorld + ?Sized> ShapeSink for Placement<'_, W> {
    fn failed(&self) -> bool {
        self.failure.is_some()
    }
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
        if self.failed() {
            return false;
        }
        self.logs.insert(pos);
        self.world.set_block(pos, state, 19);
        true // Native setters record positions even when the world rejects a write.
    }
    fn leaf(&mut self, pos: Pos, mut state: u32) -> bool {
        if self.failed() {
            return false;
        }
        let previous = blocks::get(self.world.get_block(pos));
        if previous.flags & 32 != 0 {
            return false;
        }
        if previous.flags & 4 != 0 && extra_data::property(state, "waterlogged").is_some() {
            state = extra_data::with_property(state, "waterlogged", "true");
        }
        self.leaves.insert(pos);
        self.world.set_block(pos, state, 19);
        true
    }
    fn below_trunk<R: TreeRandom + ?Sized>(&mut self, random: &mut R, pos: Pos) {
        if self.failed() {
            return;
        }
        if let Some(definition) = self.definition {
            match definition.below.sample(self.world, random, pos) {
                Ok(Some(state)) => {
                    self.log(pos, state);
                }
                Ok(None) => {}
                Err(error) => self.failure = Some(error),
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
        if self.failed() {
            return false;
        }
        if blocks::get(self.world.get_block(pos)).flags & 32 != 0 {
            return false;
        }
        let state = if let Some(d) = self.definition {
            match d.foliage.sample_required(self.world, random, pos) {
                Ok(state) => state,
                Err(error) => {
                    self.failure = Some(error);
                    return false;
                }
            }
        } else {
            default
        };
        self.leaf(pos, state)
    }
    fn trunk_origin<R: TreeRandom + ?Sized>(&self, random: &mut R, origin: Pos) -> Pos {
        let offset = self
            .definition
            .and_then(|d| d.roots.as_ref())
            .map_or(0, |root| root.offset.sample(random));
        (origin.0, origin.1 + offset, origin.2)
    }
    fn roots<R: TreeRandom + ?Sized>(&mut self, random: &mut R, origin: Pos, trunk: Pos) -> bool {
        let Some(root) = self.definition.and_then(|d| d.roots.as_ref()) else {
            return true;
        };
        match extra_roots::place(self, random, origin, trunk, root) {
            Ok(placed) => placed,
            Err(error) => {
                self.failure = Some(error);
                false
            }
        }
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
        let catalog = crate::block_predicate::catalog();
        let vine = catalog.definition("vine").expect("native vine definition");
        let litter = catalog
            .definition("leaf_litter")
            .expect("native leaf litter definition");
        for _ in 0..tries {
            let x = xmin - radius + random.next_i32_bounded(xmax - xmin + 2 * radius + 1);
            let y = y - 2 + random.next_i32_bounded(5);
            let z = zmin - radius + random.next_i32_bounded(zmax - zmin + 2 * radius + 1);
            let above = self.world.get_block((x, y + 1, z));
            if !is_air(above) && !(vine.first..vine.first + vine.count).contains(&above) {
                continue;
            }
            if blocks::get(self.world.get_block((x, y, z))).flags & 1 == 0 {
                continue;
            }
            if self.world.motion_no_leaves_height(x, z) > y + 1 {
                continue;
            }
            let index = random.next_i32_bounded(segments * 4);
            let facing = ["north", "east", "south", "west"][(index % 4) as usize];
            let state = litter
                .with_property(litter.default_state, "facing", facing)
                .expect("native leaf litter facing");
            let state = litter
                .with_property(state, "segment_amount", &(index / 4 + 1).to_string())
                .expect("native leaf litter segments");
            self.decorate((x, y + 1, z), state);
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
        world.update_template_neighbours(pos, updated)?;
    }
    Ok(())
}

/// Resolve the target definition before drawing. Unsupported schemas are errors;
/// unknown names return `Ok(None)`. Shape errors retain preceding writes.
pub fn place<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    name: &str,
    origin: Pos,
) -> Result<Option<bool>, TreeError> {
    let Some(definition) = extra_data::tree(name)? else {
        return Ok(None);
    };
    Ok(Some(place_definition(
        world,
        random,
        &definition.shape,
        Some(definition),
        origin,
    )?))
}

/// Caller-supplied geometry/configuration, including explicit historical replay.
/// This boundary does not resolve a target configured-feature name.
pub fn place_geometry<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    config: &TreeConfig,
    origin: Pos,
) -> Result<bool, TreeError> {
    place_definition(world, random, config, None, origin)
}

pub(super) fn place_definition<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    config: &TreeConfig,
    definition: Option<&extra_data::TreeDefinition>,
    origin: Pos,
) -> Result<bool, TreeError> {
    let mut placement = Placement {
        world,
        logs: Positions::new(),
        leaves: Positions::new(),
        decorations: Positions::new(),
        roots: Positions::new(),
        definition,
        failure: None,
    };
    if !grow_shape(&mut placement, random, config, origin)
        || (placement.logs.len == 0 && placement.leaves.len == 0)
    {
        if let Some(error) = placement.failure {
            return Err(error.into());
        }
        return Ok(false);
    }
    if let Some(definition) = definition {
        extra_decorators::place(&mut placement, random, &definition.decorators)?;
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
    Ok(true)
}
