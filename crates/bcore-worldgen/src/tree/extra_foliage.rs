use super::{
    extra_data::{chance, int_provider, number},
    fallen::Pos,
    place_leaves_row, try_place_leaf, FoliageAttachment, IntProvider, ShapeSink, TreeConfig,
    TreeRandom,
};
use serde_json::Value;

#[derive(Debug, PartialEq)]
pub enum ExtraFoliage {
    Bush(i32),
    Jungle(i32),
    MegaPine(IntProvider),
    RandomSpread {
        height: IntProvider,
        attempts: i32,
    },
    Cherry {
        height: IntProvider,
        wide_hole: f32,
        corner_hole: f32,
        hanging: f32,
        extension: f32,
    },
}

impl ExtraFoliage {
    pub(super) fn parse(value: &Value) -> Self {
        match value["type"].as_str().expect("foliage type") {
            "minecraft:bush_foliage_placer" => Self::Bush(number(&value["height"])),
            "minecraft:jungle_foliage_placer" => Self::Jungle(number(&value["height"])),
            "minecraft:mega_pine_foliage_placer" => {
                Self::MegaPine(int_provider(&value["crown_height"]))
            }
            "minecraft:random_spread_foliage_placer" => Self::RandomSpread {
                height: int_provider(&value["foliage_height"]),
                attempts: number(&value["leaf_placement_attempts"]),
            },
            "minecraft:cherry_foliage_placer" => Self::Cherry {
                height: int_provider(&value["height"]),
                wide_hole: chance(&value["wide_bottom_layer_hole_chance"]),
                corner_hole: chance(&value["corner_hole_chance"]),
                hanging: chance(&value["hanging_leaves_chance"]),
                extension: chance(&value["hanging_leaves_extension_chance"]),
            },
            kind => panic!("unsupported native foliage {kind}"),
        }
    }
    pub(super) fn height<R: TreeRandom + ?Sized>(&self, random: &mut R) -> i32 {
        match self {
            Self::Bush(h) | Self::Jungle(h) => *h,
            Self::MegaPine(p)
            | Self::RandomSpread { height: p, .. }
            | Self::Cherry { height: p, .. } => p.sample(random),
        }
    }
    pub(super) fn skip<R: TreeRandom + ?Sized>(
        &self,
        r: &mut R,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
    ) -> bool {
        match self {
            Self::Bush(_) => x == radius && z == radius && r.next_i32_bounded(2) == 0,
            Self::Jungle(_) | Self::MegaPine(_) => x + z >= 7 || x * x + z * z > radius * radius,
            Self::RandomSpread { .. } => false,
            Self::Cherry {
                wide_hole,
                corner_hole,
                ..
            } => {
                if y == -1 && (x == radius || z == radius) && r.next_f32() < *wide_hole {
                    return true;
                }
                let corner = x == radius && z == radius;
                if radius > 2 {
                    corner || (x + z > radius * 2 - 2 && r.next_f32() < *corner_hole)
                } else {
                    corner && r.next_f32() < *corner_hole
                }
            }
        }
    }
}

pub(super) fn place<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    kind: &ExtraFoliage,
    world: &mut P,
    r: &mut R,
    config: &TreeConfig,
    a: &FoliageAttachment,
    height: i32,
    radius: i32,
    offset: i32,
) {
    match kind {
        ExtraFoliage::Bush(_) => {
            for y in (offset - height..=offset).rev() {
                place_leaves_row(world, r, config, a, radius + a.radius_offset - 1 - y, y);
            }
        }
        ExtraFoliage::Jungle(_) => {
            let h = if a.double_trunk {
                height
            } else {
                1 + r.next_i32_bounded(2)
            };
            for y in (offset - h..=offset).rev() {
                place_leaves_row(world, r, config, a, radius + a.radius_offset + 1 - y, y);
            }
        }
        ExtraFoliage::MegaPine(_) => {
            let mut previous = 0;
            for y in a.y - height + offset..=a.y + offset {
                let below = a.y - y;
                let smooth = radius
                    + a.radius_offset
                    + (below as f32 / height as f32 * 3.5_f32).floor() as i32;
                let jagged = smooth + i32::from(below > 0 && smooth == previous && y & 1 == 0);
                place_leaves_row(world, r, config, &FoliageAttachment { y, ..*a }, jagged, 0);
                previous = smooth;
            }
        }
        ExtraFoliage::RandomSpread { attempts, .. } => {
            for _ in 0..*attempts {
                let x = a.x + r.next_i32_bounded(radius) - r.next_i32_bounded(radius);
                let y = a.y + r.next_i32_bounded(height) - r.next_i32_bounded(height);
                let z = a.z + r.next_i32_bounded(radius) - r.next_i32_bounded(radius);
                try_place_leaf(world, r, config, (x, y, z));
            }
        }
        ExtraFoliage::Cherry {
            hanging, extension, ..
        } => {
            let a = FoliageAttachment {
                y: a.y + offset,
                ..*a
            };
            let size = radius + a.radius_offset - 1;
            place_leaves_row(world, r, config, &a, size - 2, height - 3);
            place_leaves_row(world, r, config, &a, size - 1, height - 4);
            for y in (0..=height - 5).rev() {
                place_leaves_row(world, r, config, &a, size, y);
            }
            hanging_row(world, r, config, &a, size, -1, *hanging, *extension);
            hanging_row(world, r, config, &a, size - 1, -2, *hanging, *extension);
        }
    }
}

fn hanging_row<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut P,
    r: &mut R,
    config: &TreeConfig,
    a: &FoliageAttachment,
    radius: i32,
    y: i32,
    chance: f32,
    extension: f32,
) {
    place_leaves_row(world, r, config, a, radius, y);
    let offset = i32::from(a.double_trunk);
    let log = (a.x, a.y - 1, a.z);
    for ((dx, dz), (ex, ez)) in [
        ((0, -1), (1, 0)),
        ((1, 0), (0, 1)),
        ((0, 1), (-1, 0)),
        ((-1, 0), (0, -1)),
    ] {
        let edge = radius + if ex > 0 || ez > 0 { offset } else { 0 };
        let mut p = (
            a.x + ex * edge - dx * radius,
            a.y + y - 1,
            a.z + ez * edge - dz * radius,
        );
        for _ in -radius..radius + offset {
            if world.leaf_is_set((p.0, p.1 + 1, p.2))
                && try_extension(world, r, config, chance, log, p)
            {
                try_extension(world, r, config, extension, log, (p.0, p.1 - 1, p.2));
            }
            p.0 += dx;
            p.2 += dz;
        }
    }
}

fn try_extension<P: ShapeSink + ?Sized, R: TreeRandom + ?Sized>(
    world: &mut P,
    r: &mut R,
    config: &TreeConfig,
    chance: f32,
    log: Pos,
    pos: Pos,
) -> bool {
    if (pos.0 - log.0).abs() + (pos.1 - log.1).abs() + (pos.2 - log.2).abs() >= 7
        || r.next_f32() > chance
    {
        return false;
    }
    try_place_leaf(world, r, config, pos)
}
