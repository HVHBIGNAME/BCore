use super::{
    blocks,
    extra_data::{self, chance, number, Provider},
    Placement, Pos, StandingTreeWorld, TreeRandom,
};
use crate::block_predicate::{array, invalid, kind, string, FeatureResult};
use crate::tree::IntProvider;
use serde_json::Value;

pub(crate) struct Roots {
    pub offset: IntProvider,
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
    pub(crate) fn parse(value: &Value) -> FeatureResult<Self> {
        if kind(value)? != "mangrove_root_placer" {
            return Err(crate::feature_world::FeatureError::Unsupported(
                "tree root placer".into(),
            ));
        }
        let p = &value["mangrove_root_placement"];
        let width = number(&p["max_root_width"])?;
        let length = number(&p["max_root_length"])?;
        if width < 0 || length < 1 {
            return Err(invalid("mangrove root bounds"));
        }
        Ok(Self {
            offset: extra_data::int_provider(&value["trunk_offset_y"])?,
            through: extra_data::tag_name(&p["can_grow_through"])?,
            muddy: array(&p["muddy_roots_in"])?
                .iter()
                .map(|v| {
                    let name = string(v)?;
                    let b = extra_data::catalog()
                        .blocks
                        .get(crate::block_predicate::qualified(name).as_ref())
                        .ok_or_else(|| invalid(format!("muddy root block {name}")))?;
                    Ok((b.first, b.end))
                })
                .collect::<FeatureResult<_>>()?,
            width,
            length: length as usize,
            skew: chance(&p["random_skew_chance"])?,
            provider: Provider::parse(&value["root_provider"])?,
            muddy_provider: Provider::parse(&p["muddy_roots_provider"])?,
            above: value
                .get("above_root_placement")
                .map(|p| -> FeatureResult<_> {
                    Ok((
                        chance(&p["above_root_placement_chance"])?,
                        Provider::parse(&p["above_root_provider"])?,
                    ))
                })
                .transpose()?,
        })
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
    roots: &Roots,
) -> FeatureResult<bool> {
    for y in origin.1..trunk.1 {
        if !roots.can_grow(p.world, (origin.0, y, origin.2)) {
            return Ok(false);
        }
    }
    let mut positions = vec![(trunk.0, trunk.1 - 1, trunk.2)];
    for direction @ (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
        let pos = (trunk.0 + dx, trunk.1, trunk.2 + dz);
        let mut branch = Vec::new();
        if !roots.simulate(p.world, r, pos, direction, trunk, &mut branch, 0) {
            return Ok(false);
        }
        positions.extend(branch);
        positions.push(pos);
    }
    for pos in positions {
        let state = p.world.get_block(pos);
        if roots.muddy.iter().any(|&(a, b)| (a..b).contains(&state)) {
            let state = roots.muddy_provider.sample_required(p.world, r, pos)?;
            set_root(p, pos, state);
        } else if roots.can_grow(p.world, pos) {
            let state = roots.provider.sample_required(p.world, r, pos)?;
            set_root(p, pos, state);
            if let Some((chance, provider)) = &roots.above {
                let above = (pos.0, pos.1 + 1, pos.2);
                if r.next_f32() < *chance && crate::heightmap::is_air(p.world.get_block(above)) {
                    let state = provider.sample_required(p.world, r, above)?;
                    set_root(p, above, state);
                }
            }
        }
    }
    Ok(true)
}
