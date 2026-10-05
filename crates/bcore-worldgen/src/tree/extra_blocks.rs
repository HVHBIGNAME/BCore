use super::super::{extra_data as data, relative};
use super::{attaches, get, Pos, StandingTreeWorld, UnsupportedShape};
use crate::{block, heightmap::is_air};
use std::sync::OnceLock;

fn falling_delay(state: u32) -> Option<i32> {
    #[derive(serde::Deserialize)]
    struct FallingBlock {
        first: u32,
        count: u32,
        delay: i32,
    }
    #[derive(serde::Deserialize)]
    struct Fixture {
        blocks: Vec<FallingBlock>,
    }
    static BLOCKS: OnceLock<Vec<FallingBlock>> = OnceLock::new();
    BLOCKS
        .get_or_init(|| {
            serde_json::from_str::<Fixture>(include_str!(
                "../../data/falling_block_shapes_26_1.json"
            ))
            .expect("native falling-block delays")
            .blocks
        })
        .iter()
        .find(|block| (block.first..block.first + block.count).contains(&state))
        .map(|block| block.delay)
}

fn same(a: u32, b: u32) -> bool {
    data::block(a).first == data::block(b).first
}
fn direction(name: &str) -> usize {
    match name {
        "down" => 0,
        "up" => 1,
        "north" => 2,
        "south" => 3,
        "west" => 4,
        "east" => 5,
        _ => unreachable!("native direction"),
    }
}

fn stair_shape<W: StandingTreeWorld + ?Sized>(world: &W, pos: Pos, state: u32) -> &'static str {
    let facing = direction(data::property(state, "facing").expect("stair facing"));
    let half = data::property(state, "half");
    let is_stair = |state| data::block(state).update_shape == "StairBlock";
    // Native tests the outer corner before the inner corner, short-circuiting
    // each live world read. Different stair materials participate equally.
    for (toward, inner) in [(facing, false), (facing ^ 1, true)] {
        let neighbor = world.get_block(relative(pos, toward));
        if !is_stair(neighbor) || data::property(neighbor, "half") != half {
            continue;
        }
        let corner = direction(data::property(neighbor, "facing").expect("neighbor stair facing"));
        if corner / 2 == facing / 2 {
            continue;
        }
        let side = world.get_block(relative(pos, if inner { corner } else { corner ^ 1 }));
        if is_stair(side)
            && data::property(side, "facing") == data::property(state, "facing")
            && data::property(side, "half") == half
        {
            continue;
        }
        let counter_clockwise = match facing {
            2 => 4,
            3 => 5,
            4 => 3,
            5 => 2,
            _ => unreachable!(),
        };
        return match (inner, corner == counter_clockwise) {
            (false, true) => "outer_left",
            (false, false) => "outer_right",
            (true, true) => "inner_left",
            (true, false) => "inner_right",
        };
    }
    "straight"
}

fn support(state: u32, below: u32, pos: Pos) -> Result<bool, UnsupportedShape> {
    let ranges = data::block(state)
        .may_place_on
        .as_ref()
        .ok_or(UnsupportedShape { pos, state })?;
    Ok(ranges.iter().any(|&[a, b]| (a..b).contains(&below)))
}

pub(super) fn survives<W: StandingTreeWorld + ?Sized>(
    world: &W,
    pos: Pos,
    state: u32,
) -> Result<bool, UnsupportedShape> {
    let definition = data::block(state);
    let below = world.get_block(relative(pos, 0));
    match definition.can_survive.as_str() {
        "BlockBehaviour" => Ok(true),
        "VegetationBlock" => support(state, below, pos),
        "DoublePlantBlock" => {
            if data::property(state, "half") == Some("upper") {
                Ok(same(state, below) && data::property(below, "half") == Some("lower"))
            } else {
                support(state, below, pos)
            }
        }
        "MushroomBlock" => {
            if data::tagged(below, "minecraft:overrides_mushroom_light_requirement") {
                return Ok(true);
            }
            let light = world
                .tree_raw_brightness(pos)
                .ok_or(UnsupportedShape { pos, state })?;
            Ok(light < 13 && get(below).flags & 1 != 0)
        }
        "MangrovePropaguleBlock" => {
            if data::property(state, "hanging") == Some("true") {
                Ok(data::tagged(
                    world.get_block(relative(pos, 1)),
                    "minecraft:supports_hanging_mangrove_propagule",
                ))
            } else {
                support(state, below, pos)
            }
        }
        "CocoaBlock" => Ok(data::tagged(
            world.get_block(relative(
                pos,
                direction(data::property(state, "facing").expect("cocoa facing")),
            )),
            "minecraft:supports_cocoa",
        )),
        "CarpetBlock" => Ok(!is_air(below)),
        "BambooStalkBlock" | "BambooSaplingBlock" => crate::block_predicate::catalog()
            .in_block_tag(below, "supports_bamboo")
            .map_err(|_| UnsupportedShape { pos, state }),
        _ => Err(UnsupportedShape { pos, state }),
    }
}

pub(super) fn update<W: StandingTreeWorld + ?Sized>(
    world: &mut W,
    pos: Pos,
    state: u32,
    dir: usize,
    neighbour: u32,
) -> Result<u32, UnsupportedShape> {
    let definition = data::block(state);
    match definition.update_shape.as_str() {
        "BambooStalkBlock" => {
            if !survives(world, pos, state)? {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, definition.default as i32, 1, 0]);
            }
            // Support is read live, but the UP age propagation uses the supplied
            // neighbor state. The only larger native AGE is 1 following AGE 0.
            let bamboo = data::default_state("minecraft:bamboo");
            Ok(
                if dir == 1
                    && same(neighbour, bamboo)
                    && data::property(neighbour, "age") == Some("1")
                    && data::property(state, "age") == Some("0")
                {
                    data::with_property(state, "age", "1")
                } else {
                    state
                },
            )
        }
        "BambooSaplingBlock" => {
            let bamboo = data::default_state("minecraft:bamboo");
            Ok(if !survives(world, pos, state)? {
                block::AIR
            } else if dir == 1 && same(neighbour, bamboo) {
                bamboo
            } else {
                state
            })
        }
        "StairBlock" => {
            if data::property(state, "waterlogged") == Some("true") {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, super::data().2[1], 5, 1]);
            }
            Ok(if dir < 2 {
                state
            } else {
                data::with_property(state, "shape", stair_shape(world, pos, state))
            })
        }
        "FallingBlock" => {
            // FallingBlock schedules even with solid support and in every
            // direction. The virtual delay is 5 for dragon eggs, normally 2.
            let delay = falling_delay(state).ok_or(UnsupportedShape { pos, state })?;
            world.schedule_tree_tick([pos.0, pos.1, pos.2, definition.default as i32, delay, 0]);
            Ok(state)
        }
        "VegetationBlock" => Ok(if survives(world, pos, state)? {
            state
        } else {
            block::AIR
        }),
        "DoublePlantBlock" => {
            let lower = data::property(state, "half") == Some("lower");
            if dir < 2
                && lower == (dir == 1)
                && !(same(state, neighbour)
                    && data::property(state, "half") != data::property(neighbour, "half"))
            {
                return Ok(block::AIR);
            }
            Ok(if survives(world, pos, state)? {
                state
            } else {
                block::AIR
            })
        }
        "MangrovePropaguleBlock" => {
            if data::property(state, "waterlogged") == Some("true") {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, super::data().2[1], 5, 1]);
            }
            Ok(if survives(world, pos, state)? {
                state
            } else {
                block::AIR
            })
        }
        "MangroveRootsBlock" => {
            if data::property(state, "waterlogged") == Some("true") {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, super::data().2[1], 5, 1]);
            }
            Ok(state)
        }
        "CocoaBlock" => {
            if dir == direction(data::property(state, "facing").expect("cocoa facing"))
                && !survives(world, pos, state)?
            {
                Ok(block::AIR)
            } else {
                Ok(state)
            }
        }
        "CarpetBlock" => Ok(if survives(world, pos, state)? {
            state
        } else {
            block::AIR
        }),
        "HangingMossBlock" => {
            let above = relative(pos, 1);
            if !attaches(world, above, 1)? && !same(state, world.get_block(above)) {
                world.schedule_tree_tick([pos.0, pos.1, pos.2, definition.default as i32, 1, 0]);
            }
            Ok(data::with_property(
                state,
                "tip",
                if same(state, world.get_block(relative(pos, 0))) {
                    "false"
                } else {
                    "true"
                },
            ))
        }
        "HugeMushroomBlock" => {
            if same(state, neighbour) {
                Ok(data::with_property(
                    state,
                    ["down", "up", "north", "south", "west", "east"][dir],
                    "false",
                ))
            } else {
                Ok(state)
            }
        }
        _ => Err(UnsupportedShape { pos, state }),
    }
}
