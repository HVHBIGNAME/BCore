//! Native 26.1 root systems, vegetation patches, cave vines and glow lichen.
//!
//! [`place_configured`] executes the captured lush configurations and their
//! modifier-free children. [`place_configured_with`] hands **each placed child**
//! to the scheduler through [`PlacedFeatureDelegate`], preserving lazy execution,
//! live-world reads and the caller's RNG. Rooted azalea requires that delegate to
//! implement `azalea_tree`; the built-in delegate reports an explicit error for
//! it. No tree shape is substituted. Register both new modules in the crate root.
//!
//! Pass the feature type and the JSON `config` object, not a placed-feature
//! document. [`crate::dripstone::configured_feature`] provides original JAR
//! documents for standalone callers. Retain `CaveRandomState` with the RNG.
//! Unknown providers, predicates, children and context-dependent support shapes
//! fail explicitly. Implicit write postprocessing belongs to `FeatureWorld`;
//! the explicit lichen marks and SimpleBlock tick requests are emitted here.

use crate::dripstone::{
    self, array, block_id, check_weights, data, empty_or_water, float_default, float_field, in_tag,
    int_field, integer, key, offset, opposite, read, state_info, state_with, string,
    CaveRandomState, IntProvider, DOWN, HORIZONTAL, UP,
};
use crate::feature_world::{FeatureError, FeatureWorld, Pos};
use crate::simplex::WorldgenRandom;
use crate::tick_request::{TickRequest, TickTarget};
use serde_json::Value;

/// Executes a native PlacedFeature (named holder or inline object). A scheduler
/// implementation must consume modifiers lazily and propagate unsupported errors.
pub trait PlacedFeatureDelegate<W: FeatureWorld> {
    fn place(
        &mut self,
        placed: &Value,
        world: &mut W,
        random: &mut WorldgenRandom,
        origin: Pos,
        random_state: &mut CaveRandomState,
    ) -> Result<bool, FeatureError>;
}

impl<W: FeatureWorld, F> PlacedFeatureDelegate<W> for F
where
    F: FnMut(
        &Value,
        &mut W,
        &mut WorldgenRandom,
        Pos,
        &mut CaveRandomState,
    ) -> Result<bool, FeatureError>,
{
    fn place(
        &mut self,
        placed: &Value,
        world: &mut W,
        random: &mut WorldgenRandom,
        origin: Pos,
        state: &mut CaveRandomState,
    ) -> Result<bool, FeatureError> {
        self(placed, world, random, origin, state)
    }
}

/// Modifier-free placed children, including the complete dripleaf selector.
/// Modifiers and tree features are deliberately delegated by production callers.
pub struct InlinePlacedFeatures;
impl<W: FeatureWorld> PlacedFeatureDelegate<W> for InlinePlacedFeatures {
    fn place(
        &mut self,
        placed: &Value,
        world: &mut W,
        random: &mut WorldgenRandom,
        origin: Pos,
        state: &mut CaveRandomState,
    ) -> Result<bool, FeatureError> {
        if !array(placed, "placement")?.is_empty() {
            return Err(FeatureError::Unsupported(
                "placed modifiers require the external lazy driver".into(),
            ));
        }
        let document = if let Some(name) = placed["feature"].as_str() {
            dripstone::configured_feature(name)?
        } else {
            &placed["feature"]
        };
        place_configured_with(
            string(document, "type")?,
            &document["config"],
            world,
            random,
            origin,
            state,
            self,
        )
    }
}

pub fn place_configured(
    name: &str,
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: &mut CaveRandomState,
) -> Result<bool, FeatureError> {
    place_configured_with(
        name,
        config,
        world,
        random,
        origin,
        state,
        &mut InlinePlacedFeatures,
    )
}

pub fn place_configured_with<W: FeatureWorld>(
    name: &str,
    config: &Value,
    world: &mut W,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: &mut CaveRandomState,
    delegate: &mut impl PlacedFeatureDelegate<W>,
) -> Result<bool, FeatureError> {
    match key(name) {
        "root_system" => place_root_system(config, world, random, origin, state, delegate),
        "vegetation_patch" => {
            place_vegetation_patch(config, world, random, origin, state, false, delegate)
        }
        "waterlogged_vegetation_patch" => {
            place_vegetation_patch(config, world, random, origin, state, true, delegate)
        }
        "block_column" => place_block_column(config, world, random, origin, state),
        "simple_block" => place_simple_block(config, world, random, origin, state),
        "multiface_growth" => place_glow_lichen(config, world, random, origin),
        "simple_random_selector" => {
            let choices = array(config, "features")?;
            if choices.is_empty() {
                return Err(FeatureError::InvalidConfig("empty simple selector".into()));
            }
            if !world.can_write_feature(origin) {
                return Ok(false);
            }
            let selected = random.next_int(choices.len());
            delegate.place(&choices[selected], world, random, origin, state)
        }
        "random_boolean_selector" => {
            if !world.can_write_feature(origin) {
                return Ok(false);
            }
            let field = if random.next_bool() {
                "feature_true"
            } else {
                "feature_false"
            };
            delegate.place(&config[field], world, random, origin, state)
        }
        "pointed_dripstone" | "dripstone_cluster" | "large_dripstone" => {
            dripstone::place_configured(name, config, world, random, origin, state)
        }
        other => Err(FeatureError::Unsupported(format!(
            "lush configured feature {other}"
        ))),
    }
}

struct PatchConfig<'a> {
    replaceable: &'a str,
    ground: StateProvider,
    vegetation: &'a Value,
    direction: usize,
    depth: IntProvider,
    extra_bottom: f32,
    vertical: i32,
    chance: f32,
    radius: IntProvider,
    edge: f32,
}
impl<'a> PatchConfig<'a> {
    fn parse(v: &'a Value) -> Result<Self, FeatureError> {
        let tag = string(v, "replaceable")?;
        in_tag(0, tag)?;
        Ok(Self {
            replaceable: tag,
            ground: StateProvider::parse(&v["ground_state"])?,
            vegetation: v
                .get("vegetation_feature")
                .ok_or_else(|| FeatureError::InvalidConfig("missing vegetation_feature".into()))?,
            direction: match string(v, "surface")? {
                "floor" => DOWN,
                "ceiling" => UP,
                other => return Err(FeatureError::InvalidConfig(format!("cave surface {other}"))),
            },
            depth: IntProvider::parse(&v["depth"], 1, 128)?,
            extra_bottom: float_field(v, "extra_bottom_block_chance", 0.0, 1.0)?,
            vertical: int_field(v, "vertical_range", 1, 256)?,
            chance: float_field(v, "vegetation_chance", 0.0, 1.0)?,
            radius: IntProvider::parse(&v["xz_radius"], 0, 128)?,
            edge: float_field(v, "extra_edge_column_chance", 0.0, 1.0)?,
        })
    }

    fn ground(
        &self,
        world: &mut impl FeatureWorld,
        random: &mut WorldgenRandom,
        state: &mut CaveRandomState,
        mut pos: Pos,
        depth: i32,
    ) -> Result<bool, FeatureError> {
        for i in 0..depth {
            let replacement = self.ground.sample(random, state)?;
            let old = read(world, pos)?;
            // Native continues without moving when the block already matches.
            if state_info(replacement)?.block == state_info(old)?.block {
                continue;
            }
            if !in_tag(old, self.replaceable)? {
                return Ok(i != 0);
            }
            world.set_feature_block(pos, replacement, 2);
            pos = offset(pos, self.direction);
        }
        Ok(true)
    }

    fn ground_patch(
        &self,
        world: &mut impl FeatureWorld,
        random: &mut WorldgenRandom,
        state: &mut CaveRandomState,
        origin: Pos,
        rx: i32,
        rz: i32,
    ) -> Result<Positions, FeatureError> {
        let mut positions = Positions::new();
        for x in -rx..=rx {
            let x_edge = x == -rx || x == rx;
            for z in -rz..=rz {
                let z_edge = z == -rz || z == rz;
                if x_edge && z_edge {
                    continue;
                }
                if (x_edge || z_edge) && (self.edge == 0.0 || random.next_float() > self.edge) {
                    continue;
                }
                let mut p = (origin.0 + x, origin.1, origin.2 + z);
                let mut distance = 0;
                while is_air(read(world, p)?)? && distance < self.vertical {
                    p = offset(p, self.direction);
                    distance += 1;
                }
                distance = 0;
                while !is_air(read(world, p)?)? && distance < self.vertical {
                    p = offset(p, opposite(self.direction));
                    distance += 1;
                }
                let ground = offset(p, self.direction);
                let old = read(world, ground)?;
                if !is_air(read(world, p)?)? || !face_sturdy(old, opposite(self.direction))? {
                    continue;
                }
                let depth = self.depth.sample(random, state)
                    + i32::from(self.extra_bottom > 0.0 && random.next_float() < self.extra_bottom);
                if self.ground(world, random, state, ground, depth)? {
                    positions.insert(ground)?;
                }
            }
        }
        Ok(positions)
    }
}

pub fn place_vegetation_patch<W: FeatureWorld>(
    config: &Value,
    world: &mut W,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: &mut CaveRandomState,
    waterlogged: bool,
    delegate: &mut impl PlacedFeatureDelegate<W>,
) -> Result<bool, FeatureError> {
    let config = PatchConfig::parse(config)?;
    if !world.can_write_feature(origin) {
        return Ok(false);
    }
    let rx = config.radius.sample(random, state) + 1;
    let rz = config.radius.sample(random, state) + 1;
    let ground = config.ground_patch(world, random, state, origin, rx, rz)?;
    let positions = if waterlogged {
        let mut pools = Positions::new();
        for p in ground.iter() {
            let mut exposed = false;
            for direction in HORIZONTAL.into_iter().chain([DOWN]) {
                if !face_sturdy(read(world, offset(p, direction))?, opposite(direction))? {
                    exposed = true;
                    break;
                }
            }
            if !exposed {
                pools.insert(p)?;
            }
        }
        for p in pools.iter() {
            world.set_feature_block(p, block_id("water")?, 2);
        }
        pools
    } else {
        ground
    };
    for p in positions.iter() {
        if config.chance <= 0.0 || random.next_float() >= config.chance {
            continue;
        }
        let child_origin = offset(
            if waterlogged { offset(p, DOWN) } else { p },
            opposite(config.direction),
        );
        let placed = delegate.place(config.vegetation, world, random, child_origin, state)?;
        if waterlogged && placed {
            let current = read(world, p)?;
            if property(current, "waterlogged")? == Some("false") {
                world.set_feature_block(p, change_property(current, "waterlogged", "true")?, 2);
            }
        }
    }
    Ok(!positions.is_empty())
}

struct RootConfig<'a> {
    feature: &'a Value,
    space: i32,
    radius: i32,
    replaceable: &'a str,
    root: StateProvider,
    root_attempts: i32,
    column: i32,
    hanging_radius: i32,
    hanging_span: i32,
    hanging: StateProvider,
    hanging_attempts: i32,
    water: i32,
    allowed: Predicate,
}
impl<'a> RootConfig<'a> {
    fn parse(v: &'a Value) -> Result<Self, FeatureError> {
        let replaceable = string(v, "root_replaceable")?;
        in_tag(0, replaceable)?;
        Ok(Self {
            feature: v
                .get("feature")
                .ok_or_else(|| FeatureError::InvalidConfig("missing root feature".into()))?,
            space: int_field(v, "required_vertical_space_for_tree", 1, 64)?,
            radius: int_field(v, "root_radius", 1, 64)?,
            replaceable,
            root: StateProvider::parse(&v["root_state_provider"])?,
            root_attempts: int_field(v, "root_placement_attempts", 1, 256)?,
            column: int_field(v, "root_column_max_height", 1, 4096)?,
            hanging_radius: int_field(v, "hanging_root_radius", 1, 64)?,
            hanging_span: int_field(v, "hanging_roots_vertical_span", 1, 16)?,
            hanging: StateProvider::parse(&v["hanging_root_state_provider"])?,
            hanging_attempts: int_field(v, "hanging_root_placement_attempts", 1, 256)?,
            water: int_field(v, "allowed_vertical_water_for_tree", 1, 64)?,
            allowed: Predicate::parse(&v["allowed_tree_position"])?,
        })
    }

    fn space_for_tree(
        &self,
        world: &impl FeatureWorld,
        mut pos: Pos,
    ) -> Result<bool, FeatureError> {
        for i in 1..=self.space {
            pos = offset(pos, UP);
            let flags = state_info(read(world, pos)?)?.flags;
            if flags & 1 == 0 && !(i < self.water && flags & 8 != 0) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn rooted_dirt(
        &self,
        world: &mut impl FeatureWorld,
        random: &mut WorldgenRandom,
        state: &mut CaveRandomState,
        origin: Pos,
        end_y: i32,
    ) -> Result<(), FeatureError> {
        for y in origin.1..end_y {
            for _ in 0..self.root_attempts {
                let x = random.next_int(self.radius as usize) as i32
                    - random.next_int(self.radius as usize) as i32;
                let z = random.next_int(self.radius as usize) as i32
                    - random.next_int(self.radius as usize) as i32;
                let p = (origin.0 + x, y, origin.2 + z);
                if in_tag(read(world, p)?, self.replaceable)? {
                    world.set_feature_block(p, self.root.sample(random, state)?, 2);
                }
            }
        }
        Ok(())
    }

    fn hanging_roots(
        &self,
        world: &mut impl FeatureWorld,
        random: &mut WorldgenRandom,
        state: &mut CaveRandomState,
        origin: Pos,
    ) -> Result<(), FeatureError> {
        for _ in 0..self.hanging_attempts {
            let x = random.next_int(self.hanging_radius as usize) as i32
                - random.next_int(self.hanging_radius as usize) as i32;
            let y = random.next_int(self.hanging_span as usize) as i32
                - random.next_int(self.hanging_span as usize) as i32;
            let z = random.next_int(self.hanging_radius as usize) as i32
                - random.next_int(self.hanging_radius as usize) as i32;
            let p = (origin.0 + x, origin.1 + y, origin.2 + z);
            if !is_air(read(world, p)?)? {
                continue;
            }
            let root = self.hanging.sample(random, state)?;
            if can_survive(root, world, p)? && face_sturdy(read(world, offset(p, UP))?, DOWN)? {
                world.set_feature_block(p, root, 2);
            }
        }
        Ok(())
    }
}

pub fn place_root_system<W: FeatureWorld>(
    config: &Value,
    world: &mut W,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: &mut CaveRandomState,
    delegate: &mut impl PlacedFeatureDelegate<W>,
) -> Result<bool, FeatureError> {
    let config = RootConfig::parse(config)?;
    if !world.can_write_feature(origin) || !is_air(read(world, origin)?)? {
        return Ok(false);
    }
    let mut p = origin;
    for i in 0..config.column {
        p = offset(p, UP);
        if !config.allowed.test(world, p)? || !config.space_for_tree(world, p)? {
            continue;
        }
        let below = state_info(read(world, offset(p, DOWN))?)?;
        if below.flags & 16 != 0 || below.flags & 64 == 0 {
            break;
        }
        if delegate.place(config.feature, world, random, p, state)? {
            config.rooted_dirt(world, random, state, origin, origin.1 + i)?;
            config.hanging_roots(world, random, state, origin)?;
            break;
        }
    }
    // The native feature succeeds for any air origin, even if no tree was placed.
    Ok(true)
}

struct Layer {
    height: IntProvider,
    provider: StateProvider,
}

pub fn place_block_column(
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: &mut CaveRandomState,
) -> Result<bool, FeatureError> {
    let layers = array(config, "layers")?
        .iter()
        .map(|v| {
            Ok(Layer {
                height: IntProvider::parse(&v["height"], 0, i32::MAX)?,
                provider: StateProvider::parse(&v["provider"])?,
            })
        })
        .collect::<Result<Vec<_>, FeatureError>>()?;
    let direction = direction(string(config, "direction")?)?;
    let allowed = Predicate::parse(&config["allowed_placement"])?;
    let prioritize_tip = boolean(config, "prioritize_tip", false)?;
    if !world.can_write_feature(origin) {
        return Ok(false);
    }
    let mut heights: Vec<_> = layers
        .iter()
        .map(|l| l.height.sample(random, state))
        .collect();
    let total = heights
        .iter()
        .try_fold(0_i32, |sum, h| sum.checked_add(*h))
        .ok_or_else(|| FeatureError::InvalidConfig("column height overflow".into()))?;
    if total == 0 {
        return Ok(false);
    }
    let mut probe = offset(origin, direction);
    for available in 0..total {
        if !allowed.test(world, probe)? {
            let mut remove = total - available;
            let indices: Box<dyn Iterator<Item = usize>> = if prioritize_tip {
                Box::new(0..heights.len())
            } else {
                Box::new((0..heights.len()).rev())
            };
            for index in indices {
                let removed = heights[index].min(remove);
                heights[index] -= removed;
                remove -= removed;
                if remove == 0 {
                    break;
                }
            }
            break;
        }
        probe = offset(probe, direction);
    }
    let mut p = origin;
    for (layer, height) in layers.iter().zip(heights) {
        for _ in 0..height {
            world.set_feature_block(p, layer.provider.sample(random, state)?, 2);
            p = offset(p, direction);
        }
    }
    Ok(true)
}

pub fn place_simple_block(
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: &mut CaveRandomState,
) -> Result<bool, FeatureError> {
    let provider = StateProvider::parse(&config["to_place"])?;
    let tick = boolean(config, "schedule_tick", false)?;
    if !world.can_write_feature(origin) {
        return Ok(false);
    }
    let value = provider.sample(random, state)?;
    if !can_survive(value, world, origin)? {
        return Ok(false);
    }
    let block = state_info(value)?.block;
    if block == block_id("tall_grass")? || block == block_id("small_dripleaf")? {
        let above = offset(origin, UP);
        if !is_air(read(world, above)?)? {
            return Ok(false);
        }
        for (p, half) in [(origin, "lower"), (above, "upper")] {
            let mut value = change_property(value, "half", half)?;
            if property(value, "waterlogged")?.is_some() {
                value = change_property(
                    value,
                    "waterlogged",
                    if state_info(read(world, p)?)?.flags & 8 != 0 {
                        "true"
                    } else {
                        "false"
                    },
                )?;
            }
            world.set_feature_block(p, value, 2);
        }
    } else {
        world.set_feature_block(origin, value, 2);
    }
    if tick {
        world.schedule_feature_tick(TickRequest {
            block_pos: [origin.0, origin.1, origin.2],
            target: TickTarget::Block(state_info(read(world, origin)?)?.block),
            delay: 1,
        });
    }
    Ok(true)
}

struct LichenConfig {
    directions: Vec<usize>,
    search: i32,
    chance: f32,
    supports: BlockSet,
}
impl LichenConfig {
    fn parse(v: &Value) -> Result<Self, FeatureError> {
        if key(v
            .get("block")
            .and_then(Value::as_str)
            .unwrap_or("glow_lichen"))
            != "glow_lichen"
        {
            return Err(FeatureError::Unsupported(
                "multiface growth block other than glow_lichen".into(),
            ));
        }
        let mut directions = Vec::new();
        if boolean(v, "can_place_on_ceiling", false)? {
            directions.push(UP);
        }
        if boolean(v, "can_place_on_floor", false)? {
            directions.push(DOWN);
        }
        if boolean(v, "can_place_on_wall", false)? {
            directions.extend(HORIZONTAL);
        }
        Ok(Self {
            directions,
            search: v
                .get("search_range")
                .map_or(Ok(10), |v| integer(v, 1, 64))?,
            chance: float_default(v, "chance_of_spreading", 0.5, 0.0, 1.0)?,
            supports: BlockSet::parse(&v["can_be_placed_on"])?,
        })
    }

    fn place(
        &self,
        world: &mut impl FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        old: u32,
        directions: &[usize],
    ) -> Result<bool, FeatureError> {
        for &face in directions {
            let support = read(world, offset(pos, face))?;
            if !self.supports.contains(support)? {
                continue;
            }
            let Some(value) = lichen_state(world, pos, old, face)? else {
                return Ok(false);
            };
            world.set_feature_block(pos, value, 3);
            world.mark_feature_postprocessing(pos);
            if random.next_float() < self.chance {
                spread_lichen(world, random, pos, value, face)?;
            }
            return Ok(true);
        }
        Ok(false)
    }
}

pub fn place_glow_lichen(
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
) -> Result<bool, FeatureError> {
    let config = LichenConfig::parse(config)?;
    if !world.can_write_feature(origin) || !empty_or_water(read(world, origin)?)? {
        return Ok(false);
    }
    let directions = shuffle(config.directions.clone(), random);
    if config.place(world, random, origin, read(world, origin)?, &directions)? {
        return Ok(true);
    }
    for &toward in &directions {
        let candidates = shuffle(
            config
                .directions
                .iter()
                .copied()
                .filter(|d| *d != opposite(toward))
                .collect(),
            random,
        );
        for _ in 0..config.search {
            // 26.1 resets to origin + direction on every iteration (not a walk).
            let p = offset(origin, toward);
            let old = read(world, p)?;
            if !empty_or_water(old)? && state_info(old)?.block != block_id("glow_lichen")? {
                break;
            }
            if config.place(world, random, p, old, &candidates)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn lichen_state(
    world: &impl FeatureWorld,
    p: Pos,
    old: u32,
    face: usize,
) -> Result<Option<u32>, FeatureError> {
    let is_lichen = state_info(old)?.block == block_id("glow_lichen")?;
    if is_lichen && property(old, direction_name(face))? == Some("true") {
        return Ok(None);
    }
    let support_state = read(world, offset(p, face))?;
    let support = state_info(support_state)?;
    let can_attach = if support.attach < 0 {
        // Sculk's native masks include proven uncached worldgen shapes and use
        // the direction toward the neighbour. Cached cave masks use its face.
        crate::sculk::multiface_can_attach_to(support_state, face)?
    } else {
        support.attach & (1 << opposite(face)) != 0
    };
    if !can_attach {
        return Ok(None);
    }
    let base = if is_lichen {
        old
    } else {
        state_with(
            "glow_lichen",
            &[(
                "waterlogged",
                if state_info(old)?.flags & 32 != 0 {
                    "true"
                } else {
                    "false"
                },
            )],
        )?
    };
    Ok(Some(change_property(base, direction_name(face), "true")?))
}

fn spread_lichen(
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    pos: Pos,
    source: u32,
    face: usize,
) -> Result<(), FeatureError> {
    for toward in shuffle((0..6).collect(), random) {
        if toward / 2 == face / 2
            || property(source, direction_name(face))? != Some("true")
            || property(source, direction_name(toward))? == Some("true")
        {
            continue;
        }
        let options = [
            (pos, toward),
            (offset(pos, toward), face),
            (offset(offset(pos, toward), face), opposite(toward)),
        ];
        for (p, attach_face) in options {
            let old = read(world, p)?;
            let info = state_info(old)?;
            if info.flags & 1 == 0
                && info.block != block_id("glow_lichen")?
                && !(info.flags & 2 != 0 && info.flags & 32 != 0)
            {
                continue;
            }
            let Some(value) = lichen_state(world, p, old, attach_face)? else {
                continue;
            };
            world.mark_feature_postprocessing(p);
            if world.set_feature_block(p, value, 2) {
                return Ok(());
            }
            // The first viable spread type was selected; a rejected write moves
            // to the next direction, not the next spread type.
            break;
        }
    }
    Ok(())
}

fn shuffle(mut values: Vec<usize>, random: &mut WorldgenRandom) -> Vec<usize> {
    for i in (2..=values.len()).rev() {
        values.swap(i - 1, random.next_int(i));
    }
    values
}
fn is_air(state: u32) -> Result<bool, FeatureError> {
    Ok(state_info(state)?.flags & 1 != 0)
}
fn face_sturdy(state: u32, direction: usize) -> Result<bool, FeatureError> {
    let info = state_info(state)?;
    if info.faces < 0 {
        return Err(FeatureError::Unsupported(format!(
            "contextual support shape of state {state}"
        )));
    }
    Ok(info.faces & (1 << direction) != 0)
}

fn can_survive(state: u32, world: &impl FeatureWorld, pos: Pos) -> Result<bool, FeatureError> {
    let (name, _) = state_schema(state)?;
    match name {
        "azalea" | "flowering_azalea" => in_tag(read(world, offset(pos, DOWN))?, "supports_azalea"),
        "short_grass" => in_tag(read(world, offset(pos, DOWN))?, "supports_vegetation"),
        "tall_grass" | "small_dripleaf" if property(state, "half")? == Some("upper") => {
            let below = read(world, offset(pos, DOWN))?;
            Ok(state_info(below)?.block == state_info(state)?.block
                && property(below, "half")? == Some("lower"))
        }
        "tall_grass" => in_tag(read(world, offset(pos, DOWN))?, "supports_vegetation"),
        "small_dripleaf" => {
            let below = read(world, offset(pos, DOWN))?;
            Ok(in_tag(below, "supports_small_dripleaf")?
                || (state_info(read(world, pos)?)?.flags & 32 != 0
                    && in_tag(below, "supports_vegetation")?))
        }
        "moss_carpet" => Ok(!is_air(read(world, offset(pos, DOWN))?)?),
        "hanging_roots" => face_sturdy(read(world, offset(pos, UP))?, DOWN),
        "spore_blossom" => {
            let above = read(world, offset(pos, UP))?;
            if in_tag(above, "unstable_bottom_center")? {
                return Ok(false);
            }
            let shape = state_info(above)?;
            if shape.center < 0 {
                return Err(FeatureError::Unsupported(format!(
                    "contextual center support {above}"
                )));
            }
            Ok(shape.center & 1 != 0 && state_info(read(world, pos)?)?.flags & 8 == 0)
        }
        "air" | "stone" | "deepslate" | "bedrock" | "water" | "lava" | "clay" | "moss_block"
        | "rooted_dirt" | "dripstone_block" => Ok(true),
        other => Err(FeatureError::Unsupported(format!(
            "cave canSurvive for {other}"
        ))),
    }
}

fn boolean(v: &Value, field: &str, default: bool) -> Result<bool, FeatureError> {
    v.get(field).map_or(Ok(default), |v| {
        v.as_bool()
            .ok_or_else(|| FeatureError::InvalidConfig(format!("expected boolean {field}")))
    })
}
fn direction(value: &str) -> Result<usize, FeatureError> {
    ["down", "up", "north", "south", "west", "east"]
        .iter()
        .position(|d| *d == value)
        .ok_or_else(|| FeatureError::InvalidConfig(format!("direction {value}")))
}
fn direction_name(direction: usize) -> &'static str {
    ["down", "up", "north", "south", "west", "east"][direction]
}

fn state_schema(
    state: u32,
) -> Result<(&'static str, &'static dripstone::StateSchema), FeatureError> {
    let default = state_info(state)?.block;
    for (name, block) in &data().blocks {
        if block.default == default {
            return Ok((
                name,
                block
                    .states
                    .iter()
                    .find(|s| s.id == state)
                    .expect("captured state schema"),
            ));
        }
    }
    Err(FeatureError::Unsupported(format!(
        "cave state schema {state}"
    )))
}
fn property(state: u32, name: &str) -> Result<Option<&'static str>, FeatureError> {
    let (_, schema) = state_schema(state)?;
    Ok(schema.properties.get(name).map(String::as_str))
}
fn change_property(state: u32, name: &str, value: &str) -> Result<u32, FeatureError> {
    let (block, schema) = state_schema(state)?;
    let mut properties: Vec<_> = schema
        .properties
        .iter()
        .filter(|(k, _)| k.as_str() != name)
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    properties.push((name, value));
    state_with(block, &properties)
}
fn parse_state(v: &Value) -> Result<u32, FeatureError> {
    let name = string(v, "Name")?;
    if let Some(properties) = v.get("Properties") {
        let properties = properties.as_object().ok_or_else(|| {
            FeatureError::InvalidConfig("state Properties must be an object".into())
        })?;
        let values = properties
            .iter()
            .map(|(k, v)| {
                v.as_str().map(|s| (k.as_str(), s)).ok_or_else(|| {
                    FeatureError::InvalidConfig(format!("state property {k} must be a string"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        state_with(name, &values)
    } else {
        block_id(name)
    }
}

#[derive(Clone)]
enum StateProvider {
    Simple(u32),
    Weighted(Vec<(u32, i32)>, i32),
    RandomInt(Box<StateProvider>, String, IntProvider),
}
impl StateProvider {
    fn parse(v: &Value) -> Result<Self, FeatureError> {
        match key(string(v, "type")?) {
            "simple_state_provider" => Ok(Self::Simple(parse_state(&v["state"])?)),
            "weighted_state_provider" => {
                let entries = array(v, "entries")?
                    .iter()
                    .map(|v| {
                        Ok((
                            parse_state(&v["data"])?,
                            int_field(v, "weight", 0, i32::MAX)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, FeatureError>>()?;
                let total = check_weights(entries.iter().map(|(_, w)| *w))?;
                Ok(Self::Weighted(entries, total))
            }
            "randomized_int_state_provider" => Ok(Self::RandomInt(
                Box::new(Self::parse(&v["source"])?),
                string(v, "property")?.to_owned(),
                IntProvider::parse(&v["values"], i32::MIN, i32::MAX)?,
            )),
            other => Err(FeatureError::Unsupported(format!(
                "cave state provider {other}"
            ))),
        }
    }
    fn sample(
        &self,
        random: &mut WorldgenRandom,
        state: &mut CaveRandomState,
    ) -> Result<u32, FeatureError> {
        match self {
            Self::Simple(block) => Ok(*block),
            Self::Weighted(entries, total) => {
                let mut choice = random.next_int(*total as usize) as i32;
                for &(value, weight) in entries {
                    choice -= weight;
                    if choice < 0 {
                        return Ok(value);
                    }
                }
                unreachable!("validated weights")
            }
            Self::RandomInt(source, name, values) => {
                let value = source.sample(random, state)?;
                if property(value, name)?.is_none_or(|v| v.parse::<i32>().is_err()) {
                    return Ok(value);
                }
                change_property(value, name, &values.sample(random, state).to_string())
            }
        }
    }
}

enum BlockSet {
    Blocks(Vec<u32>),
    Tag(String),
}
impl BlockSet {
    fn parse(v: &Value) -> Result<Self, FeatureError> {
        if let Some(name) = v.as_str() {
            if name.starts_with('#') {
                in_tag(0, name)?;
                return Ok(Self::Tag(name.to_owned()));
            }
            return Ok(Self::Blocks(vec![block_id(name)?]));
        }
        let entries = v
            .as_array()
            .ok_or_else(|| FeatureError::InvalidConfig("expected block holder set".into()))?;
        Ok(Self::Blocks(
            entries
                .iter()
                .map(|v| {
                    v.as_str()
                        .ok_or_else(|| {
                            FeatureError::InvalidConfig("block holder must be a name".into())
                        })
                        .and_then(block_id)
                })
                .collect::<Result<_, _>>()?,
        ))
    }
    fn contains(&self, state: u32) -> Result<bool, FeatureError> {
        match self {
            Self::Blocks(blocks) => Ok(blocks.contains(&state_info(state)?.block)),
            Self::Tag(tag) => in_tag(state, tag),
        }
    }
}

enum Predicate {
    True,
    Blocks(BlockSet, Pos),
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
    Solid(Pos),
    Replaceable(Pos),
    Inside(Pos),
    Sturdy(Pos, usize),
    Survives(u32, Pos),
}
impl Predicate {
    fn parse(v: &Value) -> Result<Self, FeatureError> {
        let p = if let Some(offset) = v.get("offset") {
            let a = offset.as_array().filter(|a| a.len() == 3).ok_or_else(|| {
                FeatureError::InvalidConfig("predicate offset must have three coordinates".into())
            })?;
            (
                integer(&a[0], -16, 16)?,
                integer(&a[1], -16, 16)?,
                integer(&a[2], -16, 16)?,
            )
        } else {
            (0, 0, 0)
        };
        match key(string(v, "type")?) {
            "true" => Ok(Self::True),
            "matching_blocks" => Ok(Self::Blocks(BlockSet::parse(&v["blocks"])?, p)),
            "matching_block_tag" => {
                let tag = string(v, "tag")?;
                in_tag(0, tag)?;
                Ok(Self::Blocks(BlockSet::Tag(tag.to_owned()), p))
            }
            "all_of" | "any_of" => {
                let children = array(v, "predicates")?
                    .iter()
                    .map(Self::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(if key(string(v, "type")?) == "all_of" {
                    Self::All(children)
                } else {
                    Self::Any(children)
                })
            }
            "not" => Ok(Self::Not(Box::new(Self::parse(&v["predicate"])?))),
            "solid" => Ok(Self::Solid(p)),
            "replaceable" => Ok(Self::Replaceable(p)),
            "inside_world_bounds" => Ok(Self::Inside(p)),
            "has_sturdy_face" => Ok(Self::Sturdy(p, direction(string(v, "direction")?)?)),
            "would_survive" => Ok(Self::Survives(parse_state(&v["state"])?, p)),
            other => Err(FeatureError::Unsupported(format!(
                "cave block predicate {other}"
            ))),
        }
    }
    fn test(&self, world: &impl FeatureWorld, pos: Pos) -> Result<bool, FeatureError> {
        let translated = |p: Pos| (pos.0 + p.0, pos.1 + p.1, pos.2 + p.2);
        match self {
            Self::True => Ok(true),
            Self::Blocks(set, offset) => set.contains(read(world, translated(*offset))?),
            Self::All(children) => {
                for child in children {
                    if !child.test(world, pos)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Any(children) => {
                for child in children {
                    if child.test(world, pos)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(child) => Ok(!child.test(world, pos)?),
            Self::Solid(p) => Ok(state_info(read(world, translated(*p))?)?.flags & 64 != 0),
            Self::Replaceable(p) => Ok(state_info(read(world, translated(*p))?)?.flags & 128 != 0),
            Self::Inside(p) => Ok((crate::MIN_Y..=crate::MAX_Y).contains(&translated(*p).1)),
            Self::Sturdy(p, direction) => face_sturdy(read(world, translated(*p))?, *direction),
            Self::Survives(state, p) => can_survive(*state, world, translated(*p)),
        }
    }
}

/// Java HashSet's bin traversal determines vegetation's RNG order. Tree bins
/// move their red-black root to the front, including after a resize split.
struct Positions {
    bins: Vec<PositionBin>,
    len: usize,
}
impl Positions {
    fn new() -> Self {
        Self {
            bins: (0..16).map(|_| PositionBin::default()).collect(),
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
    fn insert(&mut self, p: Pos) -> Result<(), FeatureError> {
        let bin = Self::hash(p) & (self.bins.len() - 1);
        if self.bins[bin].entries.contains(&p) {
            return Ok(());
        }
        let capacity = self.bins.len();
        self.bins[bin].insert(p, capacity >= 64)?;
        self.len += 1;
        if self.len > capacity * 3 / 4 || (capacity < 64 && self.bins[bin].entries.len() > 8) {
            let old = std::mem::replace(
                &mut self.bins,
                (0..capacity * 2).map(|_| PositionBin::default()).collect(),
            );
            for (index, bin) in old.into_iter().enumerate() {
                let (lo, hi) = bin.split(capacity)?;
                self.bins[index] = lo;
                self.bins[index + capacity] = hi;
            }
        }
        Ok(())
    }
    fn iter(&self) -> impl Iterator<Item = Pos> + '_ {
        self.bins.iter().flat_map(|bin| bin.entries.iter().copied())
    }
    fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Default)]
struct PositionBin {
    entries: Vec<Pos>,
    tree: Option<PositionTree>,
}
impl PositionBin {
    fn insert(&mut self, p: Pos, treeify: bool) -> Result<(), FeatureError> {
        if let Some(tree) = &mut self.tree {
            let parent = tree.insert(p)?.expect("nonempty tree bin");
            let index = self
                .entries
                .iter()
                .position(|p| *p == parent)
                .expect("tree parent in bin");
            self.entries.insert(index + 1, p);
        } else {
            self.entries.push(p);
            if treeify && self.entries.len() > 8 {
                self.tree = Some(PositionTree::build(&self.entries)?);
            }
        }
        self.move_root_first();
        Ok(())
    }

    fn move_root_first(&mut self) {
        if let Some(tree) = &self.tree {
            let root = tree.nodes[tree.root].pos;
            let index = self
                .entries
                .iter()
                .position(|p| *p == root)
                .expect("tree root in bin");
            if index != 0 {
                self.entries.remove(index);
                self.entries.insert(0, root);
            }
        }
    }

    fn split(self, bit: usize) -> Result<(Self, Self), FeatureError> {
        if self.entries.iter().all(|p| Positions::hash(*p) & bit == 0) {
            return Ok((self, Self::default()));
        }
        if self.entries.iter().all(|p| Positions::hash(*p) & bit != 0) {
            return Ok((Self::default(), self));
        }
        let was_tree = self.tree.is_some();
        let (lo, hi) = self
            .entries
            .into_iter()
            .partition::<Vec<_>, _>(|p| Positions::hash(*p) & bit == 0);
        let rebuild = |entries: Vec<Pos>| -> Result<Self, FeatureError> {
            let tree = if was_tree && entries.len() > 6 {
                Some(PositionTree::build(&entries)?)
            } else {
                None
            };
            let mut bin = Self { entries, tree };
            bin.move_root_first();
            Ok(bin)
        };
        Ok((rebuild(lo)?, rebuild(hi)?))
    }
}

struct PositionNode {
    pos: Pos,
    hash: i32,
    parent: Option<usize>,
    children: [Option<usize>; 2],
    red: bool,
}
struct PositionTree {
    nodes: Vec<PositionNode>,
    root: usize,
}
impl PositionTree {
    fn build(positions: &[Pos]) -> Result<Self, FeatureError> {
        let mut tree = Self {
            nodes: Vec::new(),
            root: 0,
        };
        for &pos in positions {
            tree.insert(pos)?;
        }
        Ok(tree)
    }

    fn insert(&mut self, pos: Pos) -> Result<Option<Pos>, FeatureError> {
        let hash = Positions::hash(pos) as i32;
        let mut parent = None;
        let mut side = 0;
        let mut cursor = (!self.nodes.is_empty()).then_some(self.root);
        while let Some(index) = cursor {
            if hash == self.nodes[index].hash {
                // BlockPos does not implement Comparable<BlockPos>. Equal full
                // hashes use JVM identityHashCode, not a portable coordinate order.
                return Err(FeatureError::Unsupported(
                    "vegetation HashSet identity-hash tie between distinct positions".into(),
                ));
            }
            parent = Some(index);
            side = usize::from(hash > self.nodes[index].hash);
            cursor = self.nodes[index].children[side];
        }
        let index = self.nodes.len();
        self.nodes.push(PositionNode {
            pos,
            hash,
            parent,
            children: [None, None],
            red: true,
        });
        if let Some(parent) = parent {
            self.nodes[parent].children[side] = Some(index);
        } else {
            self.root = index;
        }
        self.balance(index);
        Ok(parent.map(|i| self.nodes[i].pos))
    }

    /// Lift the child on `side`: 1 is a left rotation, 0 a right rotation.
    fn rotate(&mut self, index: usize, side: usize) {
        let pivot = self.nodes[index].children[side].expect("rotation child");
        let middle = self.nodes[pivot].children[side ^ 1];
        self.nodes[index].children[side] = middle;
        if let Some(middle) = middle {
            self.nodes[middle].parent = Some(index);
        }
        let parent = self.nodes[index].parent;
        self.nodes[pivot].parent = parent;
        if let Some(parent) = parent {
            let slot = usize::from(self.nodes[parent].children[1] == Some(index));
            self.nodes[parent].children[slot] = Some(pivot);
        } else {
            self.root = pivot;
        }
        self.nodes[pivot].children[side ^ 1] = Some(index);
        self.nodes[index].parent = Some(pivot);
    }

    fn balance(&mut self, mut index: usize) {
        while let Some(mut parent) = self.nodes[index].parent {
            if !self.nodes[parent].red {
                break;
            }
            let Some(mut grandparent) = self.nodes[parent].parent else {
                break;
            };
            let side = usize::from(self.nodes[grandparent].children[1] == Some(parent));
            let uncle = self.nodes[grandparent].children[side ^ 1];
            if let Some(uncle) = uncle.filter(|i| self.nodes[*i].red) {
                self.nodes[parent].red = false;
                self.nodes[uncle].red = false;
                self.nodes[grandparent].red = true;
                index = grandparent;
                continue;
            }
            if self.nodes[parent].children[side ^ 1] == Some(index) {
                index = parent;
                self.rotate(index, side ^ 1);
                parent = self.nodes[index].parent.expect("rotated parent");
                grandparent = self.nodes[parent].parent.expect("rotated grandparent");
            }
            self.nodes[parent].red = false;
            self.nodes[grandparent].red = true;
            self.rotate(grandparent, side);
            break;
        }
        self.nodes[self.root].red = false;
    }
}

#[cfg(test)]
mod hashset_tests {
    use super::Positions;
    use serde_json::Value;

    #[test]
    fn native_tree_bin_iteration_and_resize_splits() {
        let data: Value =
            serde_json::from_str(include_str!("../data/cave_feature_data_26_1.json")).unwrap();
        let samples = data["position_sets"]
            .as_array()
            .expect("native position-set fixtures");
        for sample in samples {
            let inputs: Vec<[i32; 3]> = serde_json::from_value(sample["input"].clone()).unwrap();
            let mut set = Positions::new();
            for [x, y, z] in inputs {
                set.insert((x, y, z)).unwrap();
            }
            let order: Vec<_> = set.iter().map(|(x, y, z)| [x, y, z]).collect();
            assert_eq!(
                serde_json::to_value(order).unwrap(),
                sample["order"],
                "{}",
                sample["name"]
            );
        }
        assert_eq!(samples.len(), 16);
    }
}
