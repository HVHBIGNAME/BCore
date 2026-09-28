//! Absolute-coordinate TreeFeature, sharing the legacy trunk/foliage algorithms.
//! Leaf propagation, decorators and shape-edge updates use the live region.
use std::collections::BTreeMap;

use super::{
    fallen::{self, FallenTreeWorld, Pos},
    grow_shape, ShapeSink, TreeConfig, TreeRandom, TrunkPlacer,
};
use crate::{block, heightmap::is_air, MAX_Y, MIN_Y};

#[path = "standing_blocks.rs"]
mod blocks;

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

/// Requests remain owned by their absolute chunk coordinates until the pipeline
/// transfers them to generated-chunk block entities and scheduled-tick storage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeEffects {
    pub beehives: BTreeMap<Pos, Vec<i32>>,
    /// x, y, z, default block-state / fluid-registry ID, delay, fluid flag.
    pub tick_requests: Vec<[i32; 6]>,
}

impl TreeEffects {
    /// Native generated-nest metadata, before occupants tick or gather nectar.
    pub fn beehive_data(&self, (x, y, z): Pos) -> Option<serde_json::Value> {
        use serde_json::json;
        let bees: Vec<_> = self
            .beehives
            .get(&(x, y, z))?
            .iter()
            .map(|&ticks| {
                json!({
                    "ticks_in_hive": ticks,
                    "entity_data": {"id": "minecraft:bee"},
                    "min_ticks_in_hive": 600,
                })
            })
            .collect();
        Some(json!({
            "components": {}, "x": x, "y": y, "z": z,
            "id": "minecraft:beehive", "bees": bees,
        }))
    }
}

pub trait StandingTreeWorld: FallenTreeWorld {
    /// Resolve the actual block entity after the nest write, including rejection.
    fn has_beehive(&mut self, pos: Pos) -> bool;
    fn store_bee(&mut self, pos: Pos, ticks_in_hive: i32);
    fn schedule_tree_tick(&mut self, request: [i32; 6]);

    fn motion_no_leaves_height(&self, x: i32, z: i32) -> i32 {
        (MIN_Y..=MAX_Y)
            .rev()
            .find(|&y| blocks::get(self.get_block((x, y, z))).flags & 2 != 0)
            .map_or(MIN_Y, |y| y + 1)
    }
}

/// Only native configurations whose complete body is implemented here are exposed.
pub fn config_for(name: &str) -> Option<TreeConfig> {
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

#[derive(Clone)]
struct Positions {
    buckets: Vec<Vec<Pos>>,
    len: usize,
}
impl Positions {
    fn new() -> Self {
        Self {
            buckets: vec![Vec::new(); 16],
            len: 0,
        }
    }
    fn hash((x, y, z): Pos) -> usize {
        let h = y
            .wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x) as u32;
        (h ^ (h >> 16)) as usize
    }
    fn insert(&mut self, pos: Pos) {
        let index = Self::hash(pos) & (self.buckets.len() - 1);
        if self.buckets[index].contains(&pos) {
            return;
        }
        self.buckets[index].push(pos);
        self.len += 1;
        // HashMap.treeifyBin grows small tables on the ninth colliding entry,
        // even if removals kept the overall set below its load threshold.
        if self.len > self.buckets.len() * 3 / 4
            || (self.buckets[index].len() > 8 && self.buckets.len() < 64)
        {
            let size = self.buckets.len() * 2;
            let old = std::mem::replace(&mut self.buckets, vec![Vec::new(); size]);
            for p in old.into_iter().flatten() {
                self.buckets[Self::hash(p) & (size - 1)].push(p);
            }
        }
    }
    fn iter(&self) -> impl Iterator<Item = Pos> + '_ {
        self.buckets.iter().flatten().copied()
    }
    fn pop(&mut self) -> Option<Pos> {
        let bucket = self.buckets.iter_mut().find(|b| !b.is_empty())?;
        self.len -= 1;
        Some(bucket.remove(0))
    }
    fn sorted_y(&self) -> Vec<Pos> {
        let mut positions: Vec<_> = self.iter().collect();
        positions.sort_by_key(|p| p.1);
        positions
    }
}

struct Placement<'a, W: ?Sized> {
    world: &'a mut W,
    logs: Positions,
    leaves: Positions,
    decorations: Positions,
}

impl<W: StandingTreeWorld + ?Sized> ShapeSink for Placement<'_, W> {
    fn state(&self, pos: Pos) -> Option<u32> {
        Some(self.world.get_block(pos))
    }
    fn valid_state(&self, state: u32) -> bool {
        fallen::valid_tree_state(state)
    }
    fn free_state(&self, state: u32) -> bool {
        self.valid_state(state) || blocks::get(state).distance == 0
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
    fn below_trunk(&mut self, pos: Pos) {
        if !crate::heightmap::protected_below_trunk(self.world.get_block(pos)) {
            self.log(pos, block::DIRT);
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
            (logs[0].1 + 1 + random.next_i32_bounded(3)).min(logs.last().unwrap().1)
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
            .chain(self.decorations.iter());
        let first = points.next().expect("nonempty tree");
        let (mut min, mut max) = (first, first);
        for (x, y, z) in points {
            min = (min.0.min(x), min.1.min(y), min.2.min(z));
            max = (max.0.max(x), max.1.max(y), max.2.max(z));
        }
        let mut voxel = Voxel::new(min, max);
        for pos in self.decorations.iter() {
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
            let pos = pending[distance].pop().unwrap();
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
                                world.set_block(pos, updated, 2);
                            }
                            let updated_other =
                                blocks::update(world, neighbour, other, dir ^ 1, updated)?;
                            if updated_other != other {
                                world.set_block(neighbour, updated_other, 2);
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

/// Unknown configurations return `Ok(None)` before drawing. Unsupported world
/// shape callbacks return an explicit error after any already-performed writes.
pub fn place<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut W,
    random: &mut R,
    name: &str,
    origin: Pos,
) -> Result<Option<bool>, UnsupportedShape> {
    let Some(config) = config_for(name) else {
        return Ok(None);
    };
    let mut placement = Placement {
        world,
        logs: Positions::new(),
        leaves: Positions::new(),
        decorations: Positions::new(),
    };
    if !grow_shape(&mut placement, random, &config, origin)
        || (placement.logs.len == 0 && placement.leaves.len == 0)
    {
        return Ok(Some(false));
    }
    if let Some(chance) = config.beehive_probability {
        placement.beehive(random, chance);
    }
    if config.leaf_litter {
        placement.litter(random, 4, 96, 3);
        placement.litter(random, 2, 150, 4);
    }
    let voxel = placement.update_leaves();
    voxel.update_edges(placement.world)?;
    Ok(Some(true))
}
