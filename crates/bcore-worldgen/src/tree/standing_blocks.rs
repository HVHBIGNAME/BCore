use super::{relative, Pos, StandingTreeWorld, UnsupportedShape};
use std::sync::OnceLock;

#[path = "extra_blocks.rs"]
mod extra;

#[cfg(test)]
#[path = "falling_shape_tests.rs"]
mod falling_shape_tests;

#[cfg(test)]
#[path = "stair_shape_tests.rs"]
mod stair_shape_tests;

#[cfg(test)]
#[path = "bamboo_shape_tests.rs"]
mod bamboo_shape_tests;

#[derive(Clone, Copy)]
pub(super) struct State {
    pub flags: i32,
    pub distance: i32,
    shape: i32,
    default: u32,
    attach: i32,
}

fn data() -> &'static (Vec<State>, u32, [i32; 2]) {
    static DATA: OnceLock<(Vec<State>, u32, [i32; 2])> = OnceLock::new();
    DATA.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Data {
            state_count: usize,
            ranges: Vec<[i32; 7]>,
            bee_nest_south: u32,
            water_fluid_ids: [i32; 2],
        }
        let data: Data = serde_json::from_str(include_str!("../../data/standing_blocks_26_1.json"))
            .expect("native tree block predicates");
        let mut states = Vec::with_capacity(data.state_count);
        for [start, end, flags, distance, shape, default, attach] in data.ranges {
            assert_eq!(start as usize, states.len());
            assert!(end > start && end as usize <= data.state_count);
            states.resize(
                end as usize,
                State {
                    flags,
                    distance,
                    shape,
                    default: default as u32,
                    attach,
                },
            );
        }
        assert_eq!(states.len(), data.state_count);
        (states, data.bee_nest_south, data.water_fluid_ids)
    })
}

pub(super) fn get(state: u32) -> State {
    data().0[state as usize]
}
pub(super) fn bee_nest_state() -> u32 {
    data().1
}

fn vine_faces(state: u32) -> u32 {
    if (8358..=8389).contains(&state) {
        31 - (state - 8358)
    } else {
        0
    }
}
pub(super) fn attaches<W: StandingTreeWorld + ?Sized>(
    world: &W,
    pos: Pos,
    dir: usize,
) -> Result<bool, UnsupportedShape> {
    let state = world.get_block(pos);
    let mask = get(state).attach;
    if mask < 0 {
        return Err(UnsupportedShape { pos, state });
    }
    Ok(mask & (1 << dir) != 0)
}

pub(super) fn update<W: StandingTreeWorld + ?Sized>(
    world: &mut W,
    pos: Pos,
    state: u32,
    dir: usize,
    neighbour: u32,
) -> Result<u32, UnsupportedShape> {
    let info = get(state);
    match info.shape {
        0 => Ok(state),
        1 => {
            if info.flags & 4 != 0 {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, data().2[1], 5, 1]);
            }
            let next = get(neighbour).distance;
            if next != 0 || info.distance != 1 {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, info.default as i32, 1, 0]);
            }
            Ok(state)
        }
        2 => {
            let below = world.get_block(relative(pos, 0));
            Ok(if get(below).flags & 8 != 0 {
                state
            } else {
                crate::block::AIR
            })
        }
        3 => {
            if dir == 0 {
                return Ok(state);
            }
            let mut faces = vine_faces(state);
            let above = relative(pos, 1);
            if faces & 2 != 0 && !attaches(world, above, 0)? {
                faces &= !2;
            }
            for (direction, bit) in [(2, 8), (5, 16), (3, 4), (4, 1)] {
                if faces & bit != 0
                    && !attaches(world, relative(pos, direction), direction)?
                    && vine_faces(world.get_block(above)) & bit == 0
                {
                    faces &= !bit;
                }
            }
            Ok(if faces == 0 {
                crate::block::AIR
            } else {
                8358 + 31 - faces
            })
        }
        4 => {
            if dir != 1 {
                return Ok(state);
            }
            // Snowy dirt states have true/false in that registry order.
            let base = info.default - 1;
            Ok(base + u32::from(get(neighbour).flags & 16 == 0))
        }
        5 => {
            let below = relative(pos, 0);
            let supported = world.is_face_sturdy_up(world.get_block(below), below);
            Ok(if supported { state } else { crate::block::AIR })
        }
        6 => {
            let neighbour = get(neighbour);
            if info.flags & 64 != 0 || neighbour.flags & 64 != 0 {
                let fluid = data().2[usize::from(info.flags & 64 != 0)];
                world.schedule_tree_tick([pos.0, pos.1, pos.2, fluid, 5, 1]);
            }
            if dir == 0 && info.flags & 128 != 0 && neighbour.flags & 256 != 0 {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, info.default as i32, 20, 0]);
            }
            Ok(state)
        }
        7 => {
            // Native hives re-read the live neighbour before releasing bees near
            // fire. Entity release needs a server level, outside this adapter.
            let neighbour_pos = relative(pos, dir);
            let live = world.get_block(neighbour_pos);
            if get(live).flags & 512 != 0 {
                Err(UnsupportedShape {
                    pos: neighbour_pos,
                    state: live,
                })
            } else {
                Ok(state)
            }
        }
        _ => extra::update(world, pos, state, dir, neighbour),
    }
}
