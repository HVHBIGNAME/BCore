//! Vanilla MonsterRoomFeature validation, masonry, chests and spawner.
use crate::block_entity::{BlockEntity, SpawnerMob};
use crate::ore::OreWorld;
use crate::simplex::WorldgenRandom;
use crate::{block, MIN_Y};
use std::sync::OnceLock;

pub const CAVE_AIR: u32 = 15293;
pub const CHEST: u32 = 3988;
pub const SPAWNER: u32 = 3888;
const MOSSY_COBBLESTONE: u32 = 3368;
const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
const CHEST_FACING: [u32; 4] = [3988, 4006, 3994, 4000];

pub trait DungeonWorld: OreWorld {
    fn set_block_entity(&mut self, pos: (i32, i32, i32), data: BlockEntity);
}

pub(crate) fn state_flags(state: u32) -> u8 {
    static FLAGS: OnceLock<Vec<u8>> = OnceLock::new();
    let flags = FLAGS.get_or_init(|| {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../data/monster_rooms_26_1.json"))
                .expect("dungeon block predicates");
        let mut result = Vec::new();
        for range in data["ranges"].as_array().unwrap() {
            let start = range[0].as_u64().unwrap() as usize;
            let end = range[1].as_u64().unwrap() as usize;
            assert_eq!(start, result.len());
            assert!(end > start);
            result.resize(end, range[2].as_u64().unwrap() as u8);
        }
        assert_eq!(result.len(), data["state_count"].as_u64().unwrap() as usize);
        result
    });
    flags[state as usize]
}

fn read(world: &impl OreWorld, pos: (i32, i32, i32)) -> u32 {
    world
        .get_block(pos)
        .expect("dungeon region missing a required neighbour")
}
fn safe_set(world: &mut impl OreWorld, pos: (i32, i32, i32), state: u32) {
    if state_flags(read(world, pos)) & 8 == 0 {
        world.set_block(pos, state);
    }
}

fn chest_state(world: &impl OreWorld, (x, y, z): (i32, i32, i32)) -> u32 {
    let mut wall = None;
    for (i, (dx, dz)) in HORIZONTAL.iter().enumerate() {
        let flags = state_flags(read(world, (x + dx, y, z + dz)));
        if flags & 16 != 0 {
            return CHEST;
        }
        if flags & 4 != 0 {
            if wall.is_some() {
                wall = None;
                break;
            }
            wall = Some(i);
        }
    }
    if let Some(wall) = wall {
        return CHEST_FACING[(wall + 2) % 4];
    }
    let solid = |dir: usize| {
        let (dx, dz) = HORIZONTAL[dir];
        state_flags(read(world, (x + dx, y, z + dz))) & 4 != 0
    };
    let mut facing = 0;
    if solid(facing) {
        facing = (facing + 2) % 4;
    }
    if solid(facing) {
        facing = (facing + 1) % 4;
    }
    if solid(facing) {
        facing = (facing + 2) % 4;
    }
    CHEST_FACING[facing]
}

pub fn place(
    world: &mut impl DungeonWorld,
    random: &mut WorldgenRandom,
    (x, y, z): (i32, i32, i32),
) -> bool {
    let rx = 2 + random.next_int(2) as i32;
    let rz = 2 + random.next_int(2) as i32;
    let mut openings = 0;
    for dx in -rx - 1..=rx + 1 {
        for dy in -1..=4 {
            for dz in -rz - 1..=rz + 1 {
                let p = (x + dx, y + dy, z + dz);
                let flags = state_flags(read(world, p));
                if (dy == -1 || dy == 4) && flags & 2 == 0 {
                    return false;
                }
                if (dx.abs() == rx + 1 || dz.abs() == rz + 1)
                    && dy == 0
                    && flags & 1 != 0
                    && state_flags(read(world, (p.0, p.1 + 1, p.2))) & 1 != 0
                {
                    openings += 1;
                }
            }
        }
    }
    if !(1..=5).contains(&openings) {
        return false;
    }
    for dx in -rx - 1..=rx + 1 {
        for dy in (-1..=3).rev() {
            for dz in -rz - 1..=rz + 1 {
                let p = (x + dx, y + dy, z + dz);
                let flags = state_flags(read(world, p));
                if dx.abs() == rx + 1 || dz.abs() == rz + 1 || dy == -1 {
                    if p.1 >= MIN_Y && state_flags(read(world, (p.0, p.1 - 1, p.2))) & 2 == 0 {
                        world.set_block(p, CAVE_AIR);
                    } else if flags & 2 != 0 && flags & 16 == 0 {
                        let state = if dy == -1 && random.next_int(4) != 0 {
                            MOSSY_COBBLESTONE
                        } else {
                            block::COBBLESTONE
                        };
                        safe_set(world, p, state);
                    }
                } else if flags & (16 | 32) == 0 {
                    safe_set(world, p, CAVE_AIR);
                }
            }
        }
    }
    for _ in 0..2 {
        for _ in 0..3 {
            let p = (
                x + random.next_int((rx * 2 + 1) as usize) as i32 - rx,
                y,
                z + random.next_int((rz * 2 + 1) as usize) as i32 - rz,
            );
            if state_flags(read(world, p)) & 1 == 0 {
                continue;
            }
            let walls = HORIZONTAL
                .iter()
                .filter(|&&(dx, dz)| state_flags(read(world, (p.0 + dx, p.1, p.2 + dz))) & 2 != 0)
                .count();
            if walls != 1 {
                continue;
            }
            let state = chest_state(world, p);
            safe_set(world, p, state);
            if state_flags(read(world, p)) & 16 != 0 {
                world.set_block_entity(
                    p,
                    BlockEntity::DungeonChest {
                        loot_seed: random.next_long(),
                    },
                );
            }
            break;
        }
    }
    safe_set(world, (x, y, z), SPAWNER);
    if state_flags(read(world, (x, y, z))) & 32 != 0 {
        let mob = [
            SpawnerMob::Skeleton,
            SpawnerMob::Zombie,
            SpawnerMob::Zombie,
            SpawnerMob::Spider,
        ][random.next_int(4)];
        world.set_block_entity((x, y, z), BlockEntity::Spawner { mob });
    }
    true
}
