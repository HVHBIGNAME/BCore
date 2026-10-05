use super::{
    blocks,
    extra_data::{self, chance, default_state, number, with_property, Provider},
    relative, Placement, Pos, StandingTreeWorld, TreeRandom, UnsupportedShape,
};
use crate::heightmap::is_air;
use serde_json::Value;
use std::collections::BTreeSet;

fn shuffled<R: TreeRandom + ?Sized>(mut positions: Vec<Pos>, r: &mut R) -> Vec<Pos> {
    for size in (2..=positions.len()).rev() {
        positions.swap(size - 1, r.next_i32_bounded(size as i32) as usize);
    }
    positions
}
fn lowest<W: StandingTreeWorld + ?Sized>(p: &Placement<'_, W>) -> Vec<Pos> {
    let logs = p.logs.sorted_y();
    let roots = p.roots.sorted_y();
    if roots.is_empty() {
        logs
    } else if !logs.is_empty() && roots[0].1 == logs[0].1 {
        logs.into_iter().chain(roots).collect()
    } else {
        roots
    }
}

pub(super) fn place<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    p: &mut Placement<'_, W>,
    r: &mut R,
    decorators: &[Value],
) -> Result<(), UnsupportedShape> {
    for d in decorators {
        match d["type"].as_str().expect("tree decorator") {
            "minecraft:beehive" => p.beehive(r, chance(&d["probability"])),
            "minecraft:trunk_vine" | "minecraft:leave_vine" => {
                let trunk = d["type"] == "minecraft:trunk_vine";
                let positions = if trunk {
                    p.logs.sorted_y()
                } else {
                    p.leaves.sorted_y()
                };
                for pos in positions {
                    for (direction, property) in
                        [(4, "east"), (5, "west"), (2, "south"), (3, "north")]
                    {
                        let selected = if trunk {
                            r.next_i32_bounded(3) > 0
                        } else {
                            r.next_f32() < chance(&d["probability"])
                        };
                        let mut at = relative(pos, direction);
                        if selected && is_air(p.world.get_block(at)) {
                            let state =
                                with_property(default_state("minecraft:vine"), property, "true");
                            p.decorate(at, state);
                            if !trunk {
                                for _ in 0..4 {
                                    at.1 -= 1;
                                    if !is_air(p.world.get_block(at)) {
                                        break;
                                    }
                                    p.decorate(at, state);
                                }
                            }
                        }
                    }
                }
            }
            "minecraft:cocoa" => {
                if r.next_f32() >= chance(&d["probability"]) {
                    continue;
                }
                let logs = p.logs.sorted_y();
                let Some(first) = logs.first() else {
                    continue;
                };
                for &pos in logs.iter().filter(|pos| pos.1 - first.1 <= 2) {
                    for (direction, facing) in
                        [(3, "north"), (4, "east"), (2, "south"), (5, "west")]
                    {
                        if r.next_f32() > 0.25 {
                            continue;
                        }
                        let at = relative(pos, direction);
                        if is_air(p.world.get_block(at)) {
                            let age = r.next_i32_bounded(3).to_string();
                            let state = with_property(
                                with_property(default_state("minecraft:cocoa"), "age", &age),
                                "facing",
                                facing,
                            );
                            p.decorate(at, state);
                        }
                    }
                }
            }
            "minecraft:alter_ground" => {
                let positions = lowest(p);
                let Some(first) = positions.first() else {
                    continue;
                };
                let provider = Provider::parse(&d["provider"]);
                for &pos in positions.iter().filter(|pos| pos.1 == first.1) {
                    for (x, z) in [(-1, -1), (2, -1), (-1, 2), (2, 2)] {
                        ground_circle(p, r, &provider, (pos.0 + x, pos.1, pos.2 + z));
                    }
                    for _ in 0..5 {
                        let index = r.next_i32_bounded(64);
                        let (x, z) = (index % 8, index / 8);
                        if x == 0 || x == 7 || z == 0 || z == 7 {
                            ground_circle(p, r, &provider, (pos.0 - 3 + x, pos.1, pos.2 - 3 + z));
                        }
                    }
                }
            }
            "minecraft:place_on_ground" => {
                let positions = lowest(p);
                let Some(&first) = positions.first() else {
                    continue;
                };
                let (mut x0, mut x1, mut z0, mut z1) = (first.0, first.0, first.2, first.2);
                for &(x, _, z) in positions.iter().filter(|pos| pos.1 == first.1) {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    z0 = z0.min(z);
                    z1 = z1.max(z);
                }
                let radius = number(&d["radius"]);
                let height = number(&d["height"]);
                let provider = Provider::parse(&d["block_state_provider"]);
                for _ in 0..number(&d["tries"]) {
                    let x = x0 - radius + r.next_i32_bounded(x1 - x0 + 2 * radius + 1);
                    let y = first.1 - height + r.next_i32_bounded(2 * height + 1);
                    let z = z0 - radius + r.next_i32_bounded(z1 - z0 + 2 * radius + 1);
                    let above = p.world.get_block((x, y + 1, z));
                    if (is_air(above) || (8358..=8389).contains(&above))
                        && blocks::get(p.world.get_block((x, y, z))).flags & 1 != 0
                        && p.world.motion_no_leaves_height(x, z) <= y + 1
                    {
                        let at = (x, y + 1, z);
                        let state = provider.sample(p.world, r, at).expect("ground provider");
                        p.decorate(at, state);
                    }
                }
            }
            "minecraft:attached_to_leaves" => {
                let mut excluded = BTreeSet::new();
                let directions: Vec<_> = d["directions"]
                    .as_array()
                    .expect("attachment directions")
                    .iter()
                    .map(|v| match v.as_str().expect("direction") {
                        "down" => 0,
                        "up" => 1,
                        "north" => 2,
                        "south" => 3,
                        "west" => 4,
                        "east" => 5,
                        _ => unreachable!(),
                    })
                    .collect();
                let provider = Provider::parse(&d["block_provider"]);
                for leaf in shuffled(p.leaves.sorted_y(), r) {
                    let direction =
                        directions[r.next_i32_bounded(directions.len() as i32) as usize];
                    let at = relative(leaf, direction);
                    if excluded.contains(&at) || r.next_f32() >= chance(&d["probability"]) {
                        continue;
                    }
                    let mut cursor = leaf;
                    let mut clear = true;
                    for _ in 0..number(&d["required_empty_blocks"]) {
                        cursor = relative(cursor, direction);
                        if !is_air(p.world.get_block(cursor)) {
                            clear = false;
                            break;
                        }
                    }
                    if !clear {
                        continue;
                    }
                    let xz = number(&d["exclusion_radius_xz"]);
                    let y = number(&d["exclusion_radius_y"]);
                    for x in at.0 - xz..=at.0 + xz {
                        for yy in at.1 - y..=at.1 + y {
                            for z in at.2 - xz..=at.2 + xz {
                                excluded.insert((x, yy, z));
                            }
                        }
                    }
                    let state = provider.sample(p.world, r, at).expect("attached provider");
                    p.decorate(at, state);
                }
            }
            "minecraft:pale_moss" => {
                let logs = shuffled(p.logs.sorted_y(), r);
                let Some(&origin) = logs.iter().min_by_key(|pos| pos.1) else {
                    continue;
                };
                if r.next_f32() < chance(&d["ground_probability"]) {
                    p.world.place_tree_subfeature(
                        r,
                        "pale_moss_patch",
                        (origin.0, origin.1 + 1, origin.2),
                    )?;
                }
                for (positions, probability) in [
                    (p.logs.sorted_y(), chance(&d["trunk_probability"])),
                    (p.leaves.sorted_y(), chance(&d["leaves_probability"])),
                ] {
                    for pos in positions {
                        if r.next_f32() < probability {
                            let mut at = relative(pos, 0);
                            if !is_air(p.world.get_block(at)) {
                                continue;
                            }
                            while is_air(p.world.get_block(relative(at, 0))) && r.next_f32() >= 0.5
                            {
                                p.decorate(
                                    at,
                                    with_property(
                                        default_state("minecraft:pale_hanging_moss"),
                                        "tip",
                                        "false",
                                    ),
                                );
                                at.1 -= 1;
                            }
                            p.decorate(
                                at,
                                with_property(
                                    default_state("minecraft:pale_hanging_moss"),
                                    "tip",
                                    "true",
                                ),
                            );
                        }
                    }
                }
            }
            "minecraft:creaking_heart" => {
                let logs = p.logs.sorted_y();
                if logs.is_empty() || r.next_f32() >= chance(&d["probability"]) {
                    continue;
                }
                if let Some(pos) = shuffled(logs, r).into_iter().find(|&pos| {
                    (0..6).all(|dir| {
                        extra_data::tagged(p.world.get_block(relative(pos, dir)), "minecraft:logs")
                    })
                }) {
                    let state = with_property(
                        with_property(
                            default_state("minecraft:creaking_heart"),
                            "creaking_heart_state",
                            "dormant",
                        ),
                        "natural",
                        "true",
                    );
                    p.decorations.insert(pos);
                    p.world.set_creaking_heart(pos, state)?;
                }
            }
            kind => panic!("unsupported native tree decorator {kind}"),
        }
    }
    Ok(())
}

fn ground_circle<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    p: &mut Placement<'_, W>,
    r: &mut R,
    provider: &Provider,
    origin: Pos,
) {
    for x in -2_i32..=2 {
        for z in -2_i32..=2 {
            if x.abs() == 2 && z.abs() == 2 {
                continue;
            }
            for dy in (-3..=2).rev() {
                let pos = (origin.0 + x, origin.1 + dy, origin.2 + z);
                if let Some(state) = provider.sample(p.world, r, pos) {
                    p.decorate(pos, state);
                    break;
                }
                if dy < 0 && !is_air(p.world.get_block(pos)) {
                    break;
                }
            }
        }
    }
}
