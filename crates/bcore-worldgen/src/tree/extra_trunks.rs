//! Additional trunk geometries, using the common world-space shape sink.
use super::{
    extra_data::{chance, int_provider, number, with_property},
    fallen::Pos,
    place_below_trunk_block, place_log, FoliageAttachment, IntProvider, ShapeSink, TreeConfig,
    TreeRandom,
};
use serde_json::Value;

const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

#[derive(Debug, PartialEq)]
pub enum ExtraTrunk {
    MegaJungle,
    Bending {
        min_leaves: i32,
        length: IntProvider,
    },
    Cherry {
        counts: Vec<(i32, i32)>,
        horizontal: IntProvider,
        start: (i32, i32),
        end: IntProvider,
    },
    Upwards {
        steps: IntProvider,
        length: IntProvider,
        probability: f32,
    },
}

impl ExtraTrunk {
    pub(super) fn parse(value: &Value) -> Self {
        match value["type"].as_str().expect("trunk type") {
            "minecraft:mega_jungle_trunk_placer" => Self::MegaJungle,
            "minecraft:bending_trunk_placer" => Self::Bending {
                min_leaves: number(&value["min_height_for_leaves"]),
                length: int_provider(&value["bend_length"]),
            },
            "minecraft:cherry_trunk_placer" => Self::Cherry {
                counts: value["branch_count"]["distribution"]
                    .as_array()
                    .expect("branch distribution")
                    .iter()
                    .map(|e| (number(&e["data"]), number(&e["weight"])))
                    .collect(),
                horizontal: int_provider(&value["branch_horizontal_length"]),
                start: (
                    number(&value["branch_start_offset_from_top"]["min_inclusive"]),
                    number(&value["branch_start_offset_from_top"]["max_inclusive"]),
                ),
                end: int_provider(&value["branch_end_offset_from_top"]),
            },
            "minecraft:upwards_branching_trunk_placer" => Self::Upwards {
                steps: int_provider(&value["extra_branch_steps"]),
                length: int_provider(&value["extra_branch_length"]),
                probability: chance(&value["place_branch_per_log_probability"]),
            },
            kind => panic!("unsupported native trunk {kind}"),
        }
    }
}

fn attachment((x, y, z): Pos, radius_offset: i32, double_trunk: bool) -> FoliageAttachment {
    FoliageAttachment {
        x,
        y,
        z,
        radius_offset,
        double_trunk,
    }
}

fn soil<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut P,
    random: &mut R,
    config: &TreeConfig,
    (x, y, z): Pos,
) {
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        place_below_trunk_block(world, random, config, (x + dx, y - 1, z + dz));
    }
}

pub(super) fn giant<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut P,
    random: &mut R,
    config: &TreeConfig,
    origin: Pos,
    height: i32,
    out: &mut Vec<FoliageAttachment>,
) {
    soil(world, random, config, origin);
    let (x, y, z) = origin;
    for dy in 0..height {
        for (dx, dz) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
            if dy == height - 1 && (dx != 0 || dz != 0) {
                continue;
            }
            let pos = (x + dx, y + dy, z + dz);
            if world
                .state(pos)
                .is_some_and(|state| world.free_state(state))
            {
                place_log(world, config, pos);
            }
        }
    }
    out.push(attachment((x, y + height, z), 0, true));
}

pub(super) fn dark_oak<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut P,
    random: &mut R,
    config: &TreeConfig,
    origin: Pos,
    height: i32,
    out: &mut Vec<FoliageAttachment>,
) {
    soil(world, random, config, origin);
    let (dx, dz) = HORIZONTAL[random.next_i32_bounded(4) as usize];
    let bend_y = height - random.next_i32_bounded(4);
    let mut bend_steps = 2 - random.next_i32_bounded(3);
    let (x, y, z) = origin;
    let (mut tx, mut tz) = (x, z);
    let top = y + height - 1;
    for dy in 0..height {
        if dy >= bend_y && bend_steps > 0 {
            tx += dx;
            tz += dz;
            bend_steps -= 1;
        }
        if !world
            .state((tx, y + dy, tz))
            .is_some_and(|state| world.air_or_leaves(state))
        {
            continue;
        }
        for (ox, oz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            place_log(world, config, (tx + ox, y + dy, tz + oz));
        }
    }
    out.push(attachment((tx, top, tz), 0, true));
    for ox in -1..=2 {
        for oz in -1..=2 {
            if ((0..=1).contains(&ox) && (0..=1).contains(&oz)) || random.next_i32_bounded(3) > 0 {
                continue;
            }
            let length = random.next_i32_bounded(3) + 2;
            for dy in 0..length {
                place_log(world, config, (x + ox, top - dy - 1, z + oz));
            }
            out.push(attachment((x + ox, top, z + oz), 0, false));
        }
    }
}

pub(super) fn place<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    kind: &ExtraTrunk,
    world: &mut P,
    random: &mut R,
    config: &TreeConfig,
    origin: Pos,
    height: i32,
    out: &mut Vec<FoliageAttachment>,
) {
    let (x, y, z) = origin;
    match kind {
        ExtraTrunk::MegaJungle => {
            giant(world, random, config, origin, height, out);
            let mut branch_y = height - 2 - random.next_i32_bounded(4);
            while branch_y > height / 2 {
                let angle = random.next_f32() * std::f32::consts::TAU;
                let (mut bx, mut bz) = (0, 0);
                for step in 0..5 {
                    bx = (1.5_f32 + crate::mth::cos_f32(angle) * step as f32) as i32;
                    bz = (1.5_f32 + crate::mth::sin_f32(angle) * step as f32) as i32;
                    place_log(world, config, (x + bx, y + branch_y - 3 + step / 2, z + bz));
                }
                out.push(attachment((x + bx, y + branch_y, z + bz), -2, false));
                branch_y -= 2 + random.next_i32_bounded(4);
            }
        }
        ExtraTrunk::Bending { min_leaves, length } => {
            let (dx, dz) = HORIZONTAL[random.next_i32_bounded(4) as usize];
            place_below_trunk_block(world, random, config, (x, y - 1, z));
            let mut p = origin;
            for i in 0..height {
                if i + 1 >= height - 1 + random.next_i32_bounded(2) {
                    p.0 += dx;
                    p.2 += dz;
                }
                place_log(world, config, p);
                if i >= *min_leaves {
                    out.push(attachment(p, 0, false));
                }
                p.1 += 1;
            }
            for _ in 0..=length.sample(random) {
                place_log(world, config, p);
                out.push(attachment(p, 0, false));
                p.0 += dx;
                p.2 += dz;
            }
        }
        ExtraTrunk::Cherry {
            counts,
            horizontal,
            start: (min, max),
            end,
        } => {
            place_below_trunk_block(world, random, config, (x, y - 1, z));
            let first = (height - 1 + min + random.next_i32_bounded(max - min + 1)).max(0);
            let mut second = (height - 1 + min + random.next_i32_bounded(max - min)).max(0);
            if second >= first {
                second += 1;
            }
            let mut pick = random.next_i32_bounded(counts.iter().map(|c| c.1).sum());
            let count = counts
                .iter()
                .find_map(|&(value, weight)| {
                    pick -= weight;
                    (pick < 0).then_some(value)
                })
                .expect("branch count");
            let trunk_height = if count == 3 {
                height
            } else if count >= 2 {
                first.max(second) + 1
            } else {
                first + 1
            };
            for dy in 0..trunk_height {
                place_log(world, config, (x, y + dy, z));
            }
            if count == 3 {
                out.push(attachment((x, y + trunk_height, z), 0, false));
            }
            let direction = random.next_i32_bounded(4) as usize;
            cherry_branch(
                world,
                random,
                config,
                origin,
                height,
                HORIZONTAL[direction],
                first,
                first < trunk_height - 1,
                *horizontal,
                *end,
                out,
            );
            if count >= 2 {
                cherry_branch(
                    world,
                    random,
                    config,
                    origin,
                    height,
                    HORIZONTAL[(direction + 2) % 4],
                    second,
                    second < trunk_height - 1,
                    *horizontal,
                    *end,
                    out,
                );
            }
        }
        ExtraTrunk::Upwards {
            steps,
            length,
            probability,
        } => {
            for dy in 0..height {
                let p = (x, y + dy, z);
                if place_log(world, config, p)
                    && dy < height - 1
                    && random.next_f32() < *probability
                {
                    let (dx, dz) = HORIZONTAL[random.next_i32_bounded(4) as usize];
                    let length_a = length.sample(random);
                    let start = (length_a - length.sample(random) - 1).max(0);
                    let count = steps.sample(random);
                    let (mut bx, mut bz, mut top) = (x, z, p.1 + start);
                    for branch in start..height.min(start + count) {
                        if branch < 1 {
                            continue;
                        }
                        bx += dx;
                        bz += dz;
                        top = p.1 + branch;
                        if place_log(world, config, (bx, top, bz)) {
                            top += 1;
                        }
                        out.push(attachment((bx, p.1 + branch, bz), 0, false));
                    }
                    if top - p.1 > 1 {
                        out.push(attachment((bx, top, bz), 0, false));
                        out.push(attachment((bx, top - 2, bz), 0, false));
                    }
                }
                if dy == height - 1 {
                    out.push(attachment((x, p.1 + 1, z), 0, false));
                }
            }
        }
    }
}

fn cherry_branch<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut P,
    random: &mut R,
    config: &TreeConfig,
    (x, y, z): Pos,
    height: i32,
    (dx, dz): (i32, i32),
    start: i32,
    middle: bool,
    horizontal: IntProvider,
    end_offset: IntProvider,
    out: &mut Vec<FoliageAttachment>,
) {
    let mut p = (x, y + start, z);
    let end_y = height - 1 + end_offset.sample(random);
    let extended = middle || end_y < start;
    let distance = horizontal.sample(random) + i32::from(extended);
    let end = (x + dx * distance, y + end_y, z + dz * distance);
    let side = TreeConfig {
        log: with_property(config.log, "axis", if dx == 0 { "z" } else { "x" }),
        ..*config
    };
    for _ in 0..if extended { 2 } else { 1 } {
        p.0 += dx;
        p.2 += dz;
        place_log(world, &side, p);
    }
    let vertical = if end.1 > p.1 { 1 } else { -1 };
    while p != end {
        let distance = (end.0 - p.0).abs() + (end.1 - p.1).abs() + (end.2 - p.2).abs();
        let upward = random.next_f32() < (end.1 - p.1).abs() as f32 / distance as f32;
        if upward {
            p.1 += vertical;
        } else {
            p.0 += dx;
            p.2 += dz;
        }
        place_log(world, if upward { config } else { &side }, p);
    }
    out.push(attachment((end.0, end.1 + 1, end.2), 0, false));
}
