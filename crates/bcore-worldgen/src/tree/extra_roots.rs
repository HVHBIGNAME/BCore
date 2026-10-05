use super::{
    blocks,
    extra_data::{self, chance, number, Provider},
    Placement, Pos, StandingTreeWorld, TreeRandom,
};
use serde_json::Value;

struct Roots {
    through: String,
    muddy: Vec<(u32, u32)>,
    width: i32,
    length: usize,
    skew: f32,
    provider: Provider,
    muddy_provider: Provider,
    above: Option<(f32, Provider)>,
}
impl Roots {
    fn parse(value: &Value) -> Self {
        assert_eq!(value["type"], "minecraft:mangrove_root_placer");
        let p = &value["mangrove_root_placement"];
        Self {
            through: p["can_grow_through"]
                .as_str()
                .expect("root tag")
                .trim_start_matches('#')
                .to_owned(),
            muddy: p["muddy_roots_in"]
                .as_array()
                .expect("muddy root blocks")
                .iter()
                .map(|v| {
                    let b = &extra_data::catalog().blocks[v.as_str().expect("muddy block")];
                    (b.first, b.end)
                })
                .collect(),
            width: number(&p["max_root_width"]),
            length: number(&p["max_root_length"]) as usize,
            skew: chance(&p["random_skew_chance"]),
            provider: Provider::parse(&value["root_provider"]),
            muddy_provider: Provider::parse(&p["muddy_roots_provider"]),
            above: value.get("above_root_placement").map(|p| {
                (
                    chance(&p["above_root_placement_chance"]),
                    Provider::parse(&p["above_root_provider"]),
                )
            }),
        }
    }
    fn can_grow<W: StandingTreeWorld + ?Sized>(&self, world: &W, pos: Pos) -> bool {
        let state = world.get_block(pos);
        super::fallen::valid_tree_state(state) || extra_data::tagged(state, &self.through)
    }
    fn simulate<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
        &self,
        world: &W,
        random: &mut R,
        p: Pos,
        (dx, dz): (i32, i32),
        origin: Pos,
        points: &mut Vec<Pos>,
        depth: usize,
    ) -> bool {
        if depth == self.length || points.len() > self.length {
            return false;
        }
        let below = (p.0, p.1 - 1, p.2);
        let side = (p.0 + dx, p.1, p.2 + dz);
        let width = (p.0 - origin.0).abs() + (p.1 - origin.1).abs() + (p.2 - origin.2).abs();
        let candidates = if width > self.width - 3 && width <= self.width {
            if random.next_f32() < self.skew {
                vec![below, (side.0, side.1 - 1, side.2)]
            } else {
                vec![below]
            }
        } else if width > self.width || random.next_f32() < self.skew {
            vec![below]
        } else if random.next_bool() {
            vec![side]
        } else {
            vec![below]
        };
        for next in candidates {
            if self.can_grow(world, next) {
                points.push(next);
                if !self.simulate(world, random, next, (dx, dz), origin, points, depth + 1) {
                    return false;
                }
            }
        }
        true
    }
}

fn waterlogged<W: StandingTreeWorld + ?Sized>(world: &W, pos: Pos, state: u32) -> u32 {
    if extra_data::property(state, "waterlogged").is_none() {
        return state;
    }
    let old = world.get_block(pos);
    let water = extra_data::block(old).first == extra_data::default_state("minecraft:water")
        || blocks::get(old).flags & 4 != 0;
    extra_data::with_property(state, "waterlogged", if water { "true" } else { "false" })
}

fn set_root<W: StandingTreeWorld + ?Sized>(p: &mut Placement<'_, W>, pos: Pos, state: u32) {
    let state = waterlogged(p.world, pos, state);
    p.roots.insert(pos);
    p.world.set_block(pos, state, 19);
}

pub(super) fn place<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
    p: &mut Placement<'_, W>,
    r: &mut R,
    origin: Pos,
    trunk: Pos,
    config: &Value,
) -> bool {
    let roots = Roots::parse(config);
    for y in origin.1..trunk.1 {
        if !roots.can_grow(p.world, (origin.0, y, origin.2)) {
            return false;
        }
    }
    let mut positions = vec![(trunk.0, trunk.1 - 1, trunk.2)];
    for direction @ (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
        let pos = (trunk.0 + dx, trunk.1, trunk.2 + dz);
        let mut branch = Vec::new();
        if !roots.simulate(p.world, r, pos, direction, trunk, &mut branch, 0) {
            return false;
        }
        positions.extend(branch);
        positions.push(pos);
    }
    for pos in positions {
        let state = p.world.get_block(pos);
        if roots.muddy.iter().any(|&(a, b)| (a..b).contains(&state)) {
            set_root(
                p,
                pos,
                roots
                    .muddy_provider
                    .sample(p.world, r, pos)
                    .expect("muddy root provider"),
            );
        } else if roots.can_grow(p.world, pos) {
            set_root(
                p,
                pos,
                roots
                    .provider
                    .sample(p.world, r, pos)
                    .expect("root provider"),
            );
            if let Some((chance, provider)) = &roots.above {
                let above = (pos.0, pos.1 + 1, pos.2);
                if r.next_f32() < *chance && crate::heightmap::is_air(p.world.get_block(above)) {
                    set_root(
                        p,
                        above,
                        provider
                            .sample(p.world, r, above)
                            .expect("above root provider"),
                    );
                }
            }
        }
    }
    true
}
