//! Clipped native mineshaft post-processing. Coordinates outside the clip are
//! air only for StructurePiece-local reads; environmental reads use the region.
use std::sync::OnceLock;

use super::{Bounds, Direction, MineType, Piece, PieceKind};
use crate::block_entity::{BlockEntity, SpawnerMob};
use crate::dungeon::{DungeonWorld, CAVE_AIR, SPAWNER};
use crate::generated_entity::GeneratedEntity;
use crate::simplex::WorldgenRandom;
use crate::{block, is_air, MAX_Y, MIN_Y};

type Pos = [i32; 3];
const LIQUID: u32 = 1;
const REPLACEABLE: u32 = 2;
const SOLID_RENDER: u32 = 4;
const HANG_CHAIN: u32 = 8;
const FACE_DOWN: u32 = 16;
const FACE_UP: u32 = 32;
const PROTECT_OAK: u32 = 1024;
const PROTECT_DARK_OAK: u32 = 2048;
const LAVA: u32 = 4096;
const BLOCKS_MOTION: u32 = 8192;
const FENCE_CONNECT: u32 = 16384;
const IRON_CHAIN: u32 = 8249;
const COBWEB: u32 = 2247;
const RAIL_NS: u32 = 5728;
const RAIL_EW: u32 = 5730;
const DIRECTIONS: [Pos; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];

pub trait MineshaftWorld: DungeonWorld {
    fn mineshaft_blocked_biome(&self, pos: Pos) -> bool;
    fn add_entity(&mut self, entity: GeneratedEntity);
    fn mark_for_postprocessing(&mut self, pos: Pos);
}

fn flags(state: u32) -> u32 {
    static FLAGS: OnceLock<Vec<u32>> = OnceLock::new();
    FLAGS.get_or_init(|| {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../../data/mineshaft_blocks_26_1.json"))
                .expect("native mineshaft block predicates");
        let count = data["state_count"].as_u64().unwrap() as usize;
        let mut flags = Vec::with_capacity(count);
        for row in data["ranges"].as_array().unwrap() {
            let start = row[0].as_u64().unwrap() as usize;
            let end = row[1].as_u64().unwrap() as usize;
            let value = row[2].as_u64().unwrap();
            assert!(start == flags.len() && end > start && end <= count && value < (1 << 18));
            flags.resize(end, value as u32);
        }
        assert_eq!(flags.len(), count);
        flags
    })[state as usize]
}

pub fn blocks_motion(state: u32) -> bool {
    flags(state) & BLOCKS_MOTION != 0
}

/// Native `BlockState.isFaceSturdy(..., UP, FULL)` from the 26.1 predicate table.
pub(crate) fn is_face_sturdy_up(state: u32) -> bool {
    flags(state) & FACE_UP != 0
}

/// Neighbour-shape update for the fences and wall torches marked by mineshafts.
pub fn updated_shape(world: &impl crate::ore::OreWorld, [x, y, z]: Pos) -> u32 {
    let read = |x, y, z| {
        world
            .get_block((x, y, z))
            .expect("shape update missing neighbour")
    };
    let state = read(x, y, z);
    let fence = if (6965..=6996).contains(&state) {
        Some(6996)
    } else if (13932..=13963).contains(&state) {
        Some(13963)
    } else {
        None
    };
    if let Some(base) = fence {
        let mut result = base - ((base - state) & 2); // preserve waterlogged
        for (d, delta) in DIRECTIONS[2..].iter().enumerate() {
            if flags(read(x + delta[0], y, z + delta[2])) & (FENCE_CONNECT << (d ^ 1)) != 0 {
                result -= [8, 4, 1, 16][d];
            }
        }
        result
    } else if (3371..=3374).contains(&state) {
        let direction = (state - 3371) as usize + 2;
        let delta = DIRECTIONS[direction];
        if flags(read(x - delta[0], y, z - delta[2])) & (FACE_DOWN << direction) != 0 {
            state
        } else {
            block::AIR
        }
    } else {
        state
    }
}

#[derive(Clone, Copy)]
struct Material {
    planks: u32,
    wood: u32,
    fence: u32,
    protected: u32,
}

impl Material {
    fn new(mine_type: MineType) -> Self {
        match mine_type {
            MineType::Normal => Self {
                planks: block::OAK_PLANKS,
                wood: block::OAK_LOG,
                fence: 6996,
                protected: PROTECT_OAK,
            },
            MineType::Mesa => Self {
                planks: 21,
                wood: block::DARK_OAK_LOG,
                fence: 13963,
                protected: PROTECT_DARK_OAK,
            },
        }
    }
}

struct Placer<'a, W> {
    world: &'a mut W,
    bounds: Bounds,
    clip: Bounds,
    orientation: Option<Direction>,
    material: Material,
}

impl Piece {
    /// One vanilla `postProcess` call. Keep the same piece across adjacent
    /// chunks: its spider-spawner flag is mutated only by successful placement.
    pub fn post_process(
        &mut self,
        mine_type: MineType,
        world: &mut impl MineshaftWorld,
        random: &mut WorldgenRandom,
        clip: Bounds,
    ) {
        let mut placer = Placer {
            world,
            bounds: self.bounds,
            clip,
            orientation: self.orientation,
            material: Material::new(mine_type),
        };
        if placer.invalid_location() {
            return;
        }
        match &mut self.kind {
            PieceKind::Room { entrances } => placer.room(entrances),
            PieceKind::Stairs => placer.stairs(),
            PieceKind::Crossing { two_floors, .. } => placer.crossing(*two_floors),
            PieceKind::Corridor {
                rails,
                spider,
                placed_spider,
                sections,
            } => placer.corridor(random, *rails, *spider, placed_spider, *sections),
        }
    }
}

impl<W: MineshaftWorld> Placer<'_, W> {
    fn world_pos(&self, [x, y, z]: Pos) -> Pos {
        let b = self.bounds;
        match self.orientation {
            None => [x, y, z],
            Some(Direction::North) => [b.min[0] + x, b.min[1] + y, b.max[2] - z],
            Some(Direction::South) => [b.min[0] + x, b.min[1] + y, b.min[2] + z],
            Some(Direction::West) => [b.max[0] - z, b.min[1] + y, b.min[2] + x],
            Some(Direction::East) => [b.min[0] + z, b.min[1] + y, b.min[2] + x],
        }
    }

    fn read_world(&self, [x, y, z]: Pos) -> u32 {
        self.world
            .get_block((x, y, z))
            .expect("mineshaft region missing required neighbour")
    }

    fn write_world(&mut self, [x, y, z]: Pos, state: u32) {
        self.world.set_block((x, y, z), state);
    }

    fn get(&self, pos: Pos) -> u32 {
        let pos = self.world_pos(pos);
        if self.clip.contains(pos) {
            self.read_world(pos)
        } else {
            block::AIR
        }
    }

    // Directional states supplied here have already undergone the piece's
    // mirror/rotation; material columns and ordinary blocks are invariant.
    fn put(&mut self, pos: Pos, state: u32) {
        let pos = self.world_pos(pos);
        if !self.clip.contains(pos) || flags(self.read_world(pos)) & self.material.protected != 0 {
            return;
        }
        self.write_world(pos, state);
        if (self.material.fence - 31..=self.material.fence).contains(&state)
            || (3371..=3374).contains(&state)
        {
            self.world.mark_for_postprocessing(pos);
        }
    }

    fn interior(&self, [x, y, z]: Pos) -> bool {
        let pos = self.world_pos([x, y + 1, z]);
        self.clip.contains(pos) && pos[1] < self.world.ocean_floor_wg(pos[0], pos[2])
    }

    fn invalid_location(&self) -> bool {
        let lo = std::array::from_fn::<_, 3, _>(|i| (self.bounds.min[i] - 1).max(self.clip.min[i]));
        let hi = std::array::from_fn::<_, 3, _>(|i| (self.bounds.max[i] + 1).min(self.clip.max[i]));
        let center = std::array::from_fn(|i| (lo[i] + hi[i]) / 2);
        if self.world.mineshaft_blocked_biome(center) {
            return true;
        }
        let liquid = |p| flags(self.read_world(p)) & LIQUID != 0;
        for x in lo[0]..=hi[0] {
            for z in lo[2]..=hi[2] {
                if liquid([x, lo[1], z]) || liquid([x, hi[1], z]) {
                    return true;
                }
            }
        }
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                if liquid([x, y, lo[2]]) || liquid([x, y, hi[2]]) {
                    return true;
                }
            }
        }
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                if liquid([lo[0], y, z]) || liquid([hi[0], y, z]) {
                    return true;
                }
            }
        }
        false
    }

    fn fill(&mut self, lo: Pos, hi: Pos, state: u32) {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                for z in lo[2]..=hi[2] {
                    self.put([x, y, z], state);
                }
            }
        }
    }

    fn room(&mut self, entrances: &[Bounds]) {
        let [x, y, z] = self.bounds.min;
        let [xx, yy, zz] = self.bounds.max;
        self.fill([x, y + 1, z], [xx, (y + 3).min(yy), zz], CAVE_AIR);
        for b in entrances {
            self.fill([b.min[0], b.max[1] - 2, b.min[2]], b.max, CAVE_AIR);
        }
        let lo = [x, y + 4, z];
        let hi = [xx, yy, zz];
        let span = std::array::from_fn::<_, 3, _>(|i| (hi[i] - lo[i] + 1) as f32);
        let center_x = x as f32 + span[0] / 2.0;
        let center_z = z as f32 + span[2] / 2.0;
        for y in lo[1]..=hi[1] {
            let dy = (y - lo[1]) as f32 / span[1];
            for x in lo[0]..=hi[0] {
                let dx = (x as f32 - center_x) / (span[0] * 0.5);
                for z in lo[2]..=hi[2] {
                    let dz = (z as f32 - center_z) / (span[2] * 0.5);
                    if dx * dx + dy * dy + dz * dz <= 1.05 {
                        self.put([x, y, z], CAVE_AIR);
                    }
                }
            }
        }
    }

    fn stairs(&mut self) {
        self.fill([0, 5, 0], [2, 7, 1], CAVE_AIR);
        self.fill([0, 0, 7], [2, 2, 8], CAVE_AIR);
        for step in 0..5 {
            self.fill(
                [0, 5 - step - i32::from(step < 4), 2 + step],
                [2, 7 - step, 2 + step],
                CAVE_AIR,
            );
        }
    }

    fn crossing(&mut self, two_floors: bool) {
        let [x, y, z] = self.bounds.min;
        let [xx, yy, zz] = self.bounds.max;
        let mut cross = |lo, hi| {
            self.fill([x + 1, lo, z], [xx - 1, hi, zz], CAVE_AIR);
            self.fill([x, lo, z + 1], [xx, hi, zz - 1], CAVE_AIR);
        };
        if two_floors {
            cross(y, y + 2);
            cross(yy - 2, yy);
            self.fill([x + 1, y + 3, z + 1], [xx - 1, y + 3, zz - 1], CAVE_AIR);
        } else {
            cross(y, yy);
        }
        for x in [x + 1, xx - 1] {
            for z in [z + 1, zz - 1] {
                if !is_air(self.get([x, yy + 1, z])) {
                    self.fill([x, y, z], [x, yy, z], self.material.planks);
                }
            }
        }
        for x in x..=xx {
            for z in z..=zz {
                self.set_planks([x, y - 1, z]);
            }
        }
    }

    fn set_planks(&mut self, pos: Pos) {
        if self.interior(pos) {
            let pos = self.world_pos(pos);
            if flags(self.read_world(pos)) & FACE_UP == 0 {
                self.write_world(pos, self.material.planks);
            }
        }
    }

    fn corridor(
        &mut self,
        random: &mut WorldgenRandom,
        rails: bool,
        spider: bool,
        placed_spider: &mut bool,
        sections: i32,
    ) {
        let end = sections * 5 - 1;
        self.fill([0, 0, 0], [2, 1, end], CAVE_AIR);
        self.chance_box(random, end, false);
        if spider {
            self.chance_box(random, end, true);
        }
        for section in 0..sections {
            let z = 2 + section * 5;
            self.support(random, z);
            for (offset, chance) in [(-1, 0.1), (1, 0.1), (-2, 0.05), (2, 0.05)] {
                for x in [0, 2] {
                    let pos = [x, 2, z + offset];
                    if self.interior(pos)
                        && random.next_float() < chance
                        && self.sturdy_neighbours(pos)
                    {
                        self.put(pos, COBWEB);
                    }
                }
            }
            if random.next_int(100) == 0 {
                self.chest(random, [2, 0, z - 1]);
            }
            if random.next_int(100) == 0 {
                self.chest(random, [0, 0, z + 1]);
            }
            if spider && !*placed_spider {
                let pos = [1, 0, z - 1 + random.next_int(3) as i32];
                let [x, y, z] = self.world_pos(pos);
                if self.clip.contains([x, y, z]) && self.interior(pos) {
                    *placed_spider = true;
                    self.world.set_block((x, y, z), SPAWNER);
                    self.world.set_block_entity(
                        (x, y, z),
                        BlockEntity::Spawner {
                            mob: SpawnerMob::CaveSpider,
                        },
                    );
                }
            }
        }
        for x in 0..=2 {
            for z in 0..=end {
                self.set_planks([x, -1, z]);
            }
        }
        for z in [Some(2), (sections > 1).then_some(end - 2)]
            .into_iter()
            .flatten()
        {
            for x in [0, 2] {
                if self.get([x, -1, z]) == self.material.planks {
                    self.pillar_or_chain([x, -1, z]);
                }
            }
        }
        if rails {
            for z in 0..=end {
                let below = self.get([1, -1, z]);
                if !is_air(below) && flags(below) & SOLID_RENDER != 0 {
                    let chance = if self.interior([1, 0, z]) { 0.7 } else { 0.9 };
                    if random.next_float() < chance {
                        self.put([1, 0, z], self.rail(true));
                    }
                }
            }
        }
    }

    fn chance_box(&mut self, random: &mut WorldgenRandom, end: i32, spider: bool) {
        let (bottom, top, chance, state) = if spider {
            (0, 1, 0.6, COBWEB)
        } else {
            (2, 2, 0.8, CAVE_AIR)
        };
        for y in bottom..=top {
            for x in 0..=2 {
                for z in 0..=end {
                    // Vanilla draws before clipping and before isInterior.
                    if random.next_float() <= chance && (!spider || self.interior([x, y, z])) {
                        self.put([x, y, z], state);
                    }
                }
            }
        }
    }

    fn support(&mut self, random: &mut WorldgenRandom, z: i32) {
        if (0..=2).any(|x| is_air(self.get([x, 3, z]))) {
            return;
        }
        let along_z = self.orientation.unwrap().along_z();
        let left = self.material.fence - if along_z { 1 } else { 8 };
        let right = self.material.fence - if along_z { 16 } else { 4 };
        self.fill([0, 0, z], [0, 1, z], left);
        self.fill([2, 0, z], [2, 1, z], right);
        if random.next_int(4) == 0 {
            self.put([0, 2, z], self.material.planks);
            self.put([2, 2, z], self.material.planks);
        } else {
            self.fill([0, 2, z], [2, 2, z], self.material.planks);
            let orientation = self.orientation.unwrap().id() as usize;
            if random.next_float() < 0.05 {
                self.put([1, 2, z - 1], [3371, 3374, 3372, 3373][orientation]);
            }
            if random.next_float() < 0.05 {
                self.put([1, 2, z + 1], [3372, 3373, 3371, 3374][orientation]);
            }
        }
    }

    fn sturdy_neighbours(&self, pos: Pos) -> bool {
        let pos = self.world_pos(pos);
        let mut sturdy = 0;
        for (i, delta) in DIRECTIONS.iter().enumerate() {
            let p = std::array::from_fn(|axis| pos[axis] + delta[axis]);
            if self.clip.contains(p) && flags(self.read_world(p)) & (FACE_DOWN << (i ^ 1)) != 0 {
                sturdy += 1;
                if sturdy == 2 {
                    return true;
                }
            }
        }
        false
    }

    fn rail(&self, north_south: bool) -> u32 {
        if self.orientation.unwrap().along_z() == north_south {
            RAIL_NS
        } else {
            RAIL_EW
        }
    }

    fn chest(&mut self, random: &mut WorldgenRandom, pos: Pos) {
        let [x, y, z] = self.world_pos(pos);
        if self.clip.contains([x, y, z])
            && is_air(self.read_world([x, y, z]))
            && !is_air(self.read_world([x, y - 1, z]))
        {
            let rail = self.rail(random.next_bool());
            self.put(pos, rail);
            self.world.add_entity(GeneratedEntity::ChestMinecart {
                block_pos: [x, y, z],
                loot_seed: random.next_long(),
            });
        }
    }

    fn pillar_or_chain(&mut self, pos: Pos) {
        let [x, y, z] = self.world_pos(pos);
        if !self.clip.contains([x, y, z]) {
            return;
        }
        let (mut down, mut up) = (true, true);
        let mut distance = 1;
        while down || up {
            if down {
                let py = y - distance;
                let f = flags(self.read_world([x, py, z]));
                let traversable = f & REPLACEABLE != 0 && f & LAVA == 0;
                if !traversable && f & FACE_UP != 0 {
                    for py in py + 1..y {
                        self.write_world([x, py, z], self.material.wood);
                    }
                    return;
                }
                down = distance <= 20 && traversable && py > MIN_Y + 1;
            }
            if up {
                let py = y + distance;
                let f = flags(self.read_world([x, py, z]));
                let traversable = f & REPLACEABLE != 0;
                if !traversable && f & HANG_CHAIN != 0 {
                    self.write_world([x, y + 1, z], self.material.fence);
                    for py in y + 2..py {
                        self.write_world([x, py, z], IRON_CHAIN);
                    }
                    return;
                }
                up = distance <= 50 && traversable && py < MAX_Y;
            }
            distance += 1;
        }
    }
}
