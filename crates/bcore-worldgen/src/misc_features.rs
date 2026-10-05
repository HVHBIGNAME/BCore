//! Native 26.1 misc configured-feature kernels, with caller-owned world/RNG.
//!
//! `supports_configured` validates static configuration. Environment callbacks
//! remain fallible: use `requirements` before advertising full runtime support.
use serde_json::Value;
use std::sync::OnceLock;

use crate::base_features::StateProvider;
use crate::block_predicate::{
    catalog, integer, invalid, is_air, kind, number, offset, ordered_block_tag, read_block,
    shape_info, would_survive, BlockPredicate, Direction, FeatureEnvironment, FeatureResult,
};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::placement::{ConfiguredFeatureDispatcher, IntProvider, RejectUnsupported};
use crate::simplex::WorldgenRandom;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requirements {
    /// Native biome freezing (including an explicitly supplied biome identity).
    pub freezing: bool,
    pub snow: bool,
    pub raw_brightness: bool,
    /// A caller-owned cached Gaussian stream for nested integer providers.
    pub gaussian: bool,
}

enum Config {
    Magma {
        search: i32,
        radius: i32,
        chance: f32,
    },
    Bamboo(f32),
    Vines,
    Freeze,
    Kelp,
    Pickle(IntProvider),
    Pile(StateProvider),
    Mushroom(Mushroom),
    Coral(Coral),
    Blob {
        state: u32,
        soil: BlockPredicate,
    },
    Spike {
        state: u32,
        soil: BlockPredicate,
        replace: BlockPredicate,
    },
    BlueIce,
    Iceberg(u32),
}

struct Mushroom {
    red: bool,
    radius: i32,
    cap: StateProvider,
    stem: StateProvider,
    soil: BlockPredicate,
}

#[derive(Clone, Copy)]
enum Coral {
    Tree,
    Claw,
    Mushroom,
}

impl Config {
    fn parse(document: &Value) -> FeatureResult<Option<Self>> {
        let feature_type = kind(document)?;
        let config = &document["config"];
        if !config.is_object() {
            return Err(invalid("configured feature requires config object"));
        }
        Ok(Some(match feature_type {
            "underwater_magma" => Self::Magma {
                search: bounded(config, "floor_search_range", 0, 512)?,
                radius: bounded(config, "placement_radius_around_floor", 0, 64)?,
                chance: probability(config, "placement_probability_per_valid_position")?,
            },
            "bamboo" => Self::Bamboo(probability(config, "probability")?),
            "vines" => Self::Vines,
            "freeze_top_layer" => Self::Freeze,
            "kelp" => Self::Kelp,
            "sea_pickle" => {
                Self::Pickle(IntProvider::parse(&config["count"])?.require_range(0, 256)?)
            }
            "block_pile" => Self::Pile(StateProvider::parse(&config["state_provider"])?),
            "huge_brown_mushroom" | "huge_red_mushroom" => {
                let radius = config
                    .get("foliage_radius")
                    .map(|v| crate::block_predicate::int(v))
                    .transpose()?
                    .unwrap_or(2);
                if !(-64..=64).contains(&radius) {
                    return Err(FeatureError::Unsupported(format!(
                        "huge mushroom radius {radius} outside supported -64..=64 domain"
                    )));
                }
                Self::Mushroom(Mushroom {
                    red: feature_type == "huge_red_mushroom",
                    radius,
                    cap: StateProvider::parse(&config["cap_provider"])?,
                    stem: StateProvider::parse(&config["stem_provider"])?,
                    soil: BlockPredicate::parse(&config["can_place_on"])?,
                })
            }
            "coral_tree" => Self::Coral(Coral::Tree),
            "coral_claw" => Self::Coral(Coral::Claw),
            "coral_mushroom" => Self::Coral(Coral::Mushroom),
            "block_blob" => Self::Blob {
                state: catalog().state(&config["state"])?,
                soil: BlockPredicate::parse(&config["can_place_on"])?,
            },
            "spike" => Self::Spike {
                state: catalog().state(&config["state"])?,
                soil: BlockPredicate::parse(&config["can_place_on"])?,
                replace: BlockPredicate::parse(&config["can_replace"])?,
            },
            "blue_ice" => Self::BlueIce,
            "iceberg" => Self::Iceberg(catalog().state(&config["state"])?),
            _ => return Ok(None),
        }))
    }

    fn requirements(&self) -> FeatureResult<Requirements> {
        let mut needs = Requirements {
            freezing: matches!(self, Self::Freeze),
            snow: matches!(self, Self::Freeze),
            ..Requirements::default()
        };
        match self {
            Self::Pickle(count) => int_requirements(count, &mut needs),
            Self::Pile(provider) => provider_requirements(provider, &mut needs)?,
            Self::Mushroom(config) => {
                provider_requirements(&config.cap, &mut needs)?;
                provider_requirements(&config.stem, &mut needs)?;
                predicate_requirements(&config.soil, &mut needs)?;
            }
            Self::Blob { soil, .. } => predicate_requirements(soil, &mut needs)?,
            Self::Spike { soil, replace, .. } => {
                predicate_requirements(soil, &mut needs)?;
                predicate_requirements(replace, &mut needs)?;
            }
            _ => {}
        }
        Ok(needs)
    }
}

/// `None` means the document belongs to another dispatcher. Invalid configurations
/// are errors, even when a sampled origin would be rejected without doing work.
pub fn requirements(document: &Value) -> FeatureResult<Option<Requirements>> {
    Config::parse(document)?
        .map(|config| config.requirements())
        .transpose()
}

pub fn supports_configured(document: &Value) -> FeatureResult<bool> {
    Ok(requirements(document)?.is_some())
}

pub fn place_configured(
    document: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    place_configured_with(
        document,
        world,
        random,
        origin,
        environment,
        &mut RejectUnsupported,
    )
}

/// Uses the enclosing dispatcher's Gaussian cache for nested state/count providers.
/// Never creates or reseeds a replacement random stream.
pub fn place_configured_with(
    document: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let config = Config::parse(document)?.ok_or_else(|| {
        FeatureError::Unsupported(format!("misc configured {}", document["type"]))
    })?;
    config.requirements()?;
    if !world.can_write_feature(origin) {
        return Ok(false);
    }
    match config {
        Config::Magma {
            search,
            radius,
            chance,
        } => magma(world, random, origin, search, radius, chance),
        Config::Bamboo(chance) => bamboo(world, random, origin, environment, chance),
        Config::Vines => vines(world, origin),
        Config::Freeze => freeze_top_layer(world, origin, environment),
        Config::Kelp => kelp(world, random, origin, environment),
        Config::Pickle(count) => sea_pickle(world, random, origin, environment, &count, dispatcher),
        Config::Pile(provider) => pile(world, random, origin, environment, &provider, dispatcher),
        Config::Mushroom(config) => {
            mushroom(world, random, origin, environment, &config, dispatcher)
        }
        Config::Coral(config) => coral(world, random, origin, config),
        Config::Blob { state, soil } => blob(world, random, origin, environment, state, &soil),
        Config::Spike {
            state,
            soil,
            replace,
        } => spike(world, random, origin, environment, state, &soil, &replace),
        Config::BlueIce => blue_ice(world, random, origin, environment),
        Config::Iceberg(state) => iceberg(world, random, origin, environment, state),
    }
}

fn int_requirements(provider: &IntProvider, needs: &mut Requirements) {
    match provider {
        IntProvider::Normal(..) => needs.gaussian = true,
        IntProvider::Clamped(source, ..) => int_requirements(source, needs),
        IntProvider::Weighted(entries, _) => {
            for (source, _) in entries {
                int_requirements(source, needs);
            }
        }
        _ => {}
    }
}

fn provider_requirements(provider: &StateProvider, needs: &mut Requirements) -> FeatureResult<()> {
    match provider {
        StateProvider::Randomized(source, _, values) => {
            provider_requirements(source, needs)?;
            int_requirements(values, needs);
        }
        StateProvider::RuleBased(fallback, rules) => {
            if let Some(fallback) = fallback {
                provider_requirements(fallback, needs)?;
            }
            for (predicate, source) in rules {
                predicate_requirements(predicate, needs)?;
                provider_requirements(source, needs)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn predicate_requirements(
    predicate: &BlockPredicate,
    needs: &mut Requirements,
) -> FeatureResult<()> {
    match predicate {
        BlockPredicate::All(children) | BlockPredicate::Any(children) => {
            for child in children {
                predicate_requirements(child, needs)?;
            }
        }
        BlockPredicate::Not(child) => predicate_requirements(child, needs)?,
        BlockPredicate::Survives(state, _) => {
            needs.raw_brightness |= crate::block_predicate::survival_requires_brightness(*state)?;
        }
        _ => {}
    }
    Ok(())
}

fn bounded(value: &Value, key: &str, min: i32, max: i32) -> FeatureResult<i32> {
    let number = integer(value, key)?;
    if !(min..=max).contains(&number) {
        return Err(invalid(format!("{key} outside {min}..={max}")));
    }
    Ok(number)
}

fn probability(value: &Value, key: &str) -> FeatureResult<f32> {
    let number = number(value, key)? as f32;
    if !(0.0..=1.0).contains(&number) {
        return Err(invalid(format!("{key} outside 0..=1")));
    }
    Ok(number)
}

fn is_water(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
    catalog().is_block(read_block(world, pos)?, "water")
}

fn magma(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    search: i32,
    radius: i32,
    chance: f32,
) -> FeatureResult<bool> {
    if !is_water(world, origin)? {
        return Ok(false);
    }
    let mut floor = None;
    // Column.scan also scans upward first. Preserve its reads and the range-1
    // boundary even though only the floor is subsequently used by the feature.
    for direction in [Direction::Up, Direction::Down] {
        let mut pos = origin;
        let mut distance = 1;
        while distance < search && is_water(world, pos)? {
            pos = direction.step(pos);
            distance += 1;
        }
        if !is_water(world, pos)? && direction == Direction::Down {
            floor = Some(pos);
        }
    }
    let Some(floor) = floor else { return Ok(false) };
    let state = catalog().default_state("magma_block")?;
    let mut placed = false;
    // Native BlockPos.betweenClosed: X, then Y, then Z.
    for z in -radius..=radius {
        for y in -radius..=radius {
            for x in -radius..=radius {
                if random.next_float() >= chance {
                    continue;
                }
                let pos = offset(floor, (x, y, z));
                let old = read_block(world, pos)?;
                if catalog().info(old)?.is_air() || catalog().is_block(old, "water")? {
                    continue;
                }
                if !shape_info(read_block(world, Direction::Down.step(pos))?)?
                    .occludes(Direction::Up)
                {
                    continue;
                }
                let mut enclosed = true;
                for direction in Direction::HORIZONTAL {
                    if !shape_info(read_block(world, direction.step(pos))?)?
                        .occludes(direction.opposite())
                    {
                        enclosed = false;
                        break;
                    }
                }
                if enclosed {
                    world.set_feature_block(pos, state, 2);
                    // Native mapToInt counts the attempt, not setBlock's return.
                    placed = true;
                }
            }
        }
    }
    Ok(placed)
}

fn bamboo(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    chance: f32,
) -> FeatureResult<bool> {
    if !is_air(world, origin)? {
        return Ok(false);
    }
    let data = catalog();
    let block = data.definition("bamboo")?;
    if !would_survive(world, block.default_state, origin, environment)? {
        // Native counts the initially empty origin even when survival fails.
        return Ok(true);
    }
    let height = random.next_int(12) + 5;
    if random.next_float() < chance {
        let radius = random.next_int(4) as i32 + 1;
        let podzol = data.default_state("podzol")?;
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x * x + z * z > radius * radius {
                    continue;
                }
                let (x, _, z) = offset(origin, (x, 0, z));
                let pos = (
                    x,
                    world.feature_height(FeatureHeightmap::WorldSurface, x, z) - 1,
                    z,
                );
                if data
                    .in_block_tag(read_block(world, pos)?, "beneath_bamboo_podzol_replaceable")?
                {
                    world.set_feature_block(pos, podzol, 2);
                }
            }
        }
    }
    let trunk = block.with_property(block.default_state, "age", "1")?;
    let mut pos = origin;
    for _ in 0..height {
        if !is_air(world, pos)? {
            break;
        }
        world.set_feature_block(pos, trunk, 2);
        pos = Direction::Up.step(pos);
    }
    if pos.1 - origin.1 >= 3 {
        let large = block.with_property(trunk, "leaves", "large")?;
        let final_large = block.with_property(large, "stage", "1")?;
        // 26.1 writes at the first position AFTER the trunk; this may overwrite
        // the obstruction which stopped the loop. Do not move down first.
        world.set_feature_block(pos, final_large, 2);
        pos = Direction::Down.step(pos);
        world.set_feature_block(pos, large, 2);
        pos = Direction::Down.step(pos);
        world.set_feature_block(pos, block.with_property(trunk, "leaves", "small")?, 2);
    }
    Ok(true)
}

fn direction_property(direction: Direction) -> &'static str {
    match direction {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::North => "north",
        Direction::South => "south",
        Direction::West => "west",
        Direction::East => "east",
    }
}

fn vines(world: &mut dyn FeatureWorld, origin: Pos) -> FeatureResult<bool> {
    if !is_air(world, origin)? {
        return Ok(false);
    }
    for direction in Direction::ALL {
        if direction == Direction::Down {
            continue;
        }
        let neighbor = read_block(world, direction.step(origin))?;
        if catalog().info(neighbor)?.can_attach_from(direction) {
            let vine = catalog().definition("vine")?;
            let state =
                vine.with_property(vine.default_state, direction_property(direction), "true")?;
            world.set_feature_block(origin, state, 2);
            return Ok(true);
        }
    }
    Ok(false)
}

fn freeze_top_layer(
    world: &mut dyn FeatureWorld,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    let data = catalog();
    let ice = data.default_state("ice")?;
    let snow = data.default_state("snow")?;
    for x in 0..16 {
        for z in 0..16 {
            let (x, _, z) = offset(origin, (x, 0, z));
            let top = (
                x,
                world.feature_height(FeatureHeightmap::MotionBlocking, x, z),
                z,
            );
            let below = Direction::Down.step(top);
            let biome = world.feature_biome(top);
            if environment.should_freeze_in_biome(world, biome, below, false)? {
                world.set_feature_block(below, ice, 2);
            }
            if environment.should_snow(world, top)? {
                world.set_feature_block(top, snow, 2);
                let state = read_block(world, below)?;
                let block = data.block(state)?.1;
                if block.property(state, "snowy").is_some() {
                    world.set_feature_block(below, block.with_property(state, "snowy", "true")?, 2);
                }
            }
        }
    }
    Ok(true)
}

fn kelp(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    let mut pos = (
        origin.0,
        world.feature_height(FeatureHeightmap::OceanFloor, origin.0, origin.2),
        origin.2,
    );
    if !is_water(world, pos)? {
        return Ok(false);
    }
    let data = catalog();
    let head = data.definition("kelp")?;
    let body = data.default_state("kelp_plant")?;
    let height = random.next_int(10) + 1;
    let mut placed = false;
    for y in 0..=height {
        if is_water(world, pos)?
            && is_water(world, Direction::Up.step(pos))?
            && would_survive(world, body, pos, environment)?
        {
            if y == height {
                let state = head.with_property(
                    head.default_state,
                    "age",
                    &(random.next_int(4) + 20).to_string(),
                )?;
                world.set_feature_block(pos, state, 2);
                placed = true;
            } else {
                world.set_feature_block(pos, body, 2);
            }
        } else if y > 0 {
            let below = Direction::Down.step(pos);
            if would_survive(world, head.default_state, below, environment)?
                && !data.is_block(read_block(world, Direction::Down.step(below))?, "kelp")?
            {
                let state = head.with_property(
                    head.default_state,
                    "age",
                    &(random.next_int(4) + 20).to_string(),
                )?;
                world.set_feature_block(below, state, 2);
                placed = true;
            }
            break;
        }
        // A failed first iteration does not terminate the native loop.
        pos = Direction::Up.step(pos);
    }
    Ok(placed)
}

fn sea_pickle(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    count: &IntProvider,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let attempts = count.sample_with(random, dispatcher)?;
    let pickle = catalog().definition("sea_pickle")?;
    let mut placed = false;
    for _ in 0..attempts {
        let x = random.next_int(8) as i32 - random.next_int(8) as i32;
        let z = random.next_int(8) as i32 - random.next_int(8) as i32;
        let (x, _, z) = offset(origin, (x, 0, z));
        let pos = (
            x,
            world.feature_height(FeatureHeightmap::OceanFloor, x, z),
            z,
        );
        // Pickle count is drawn even for a dry or unsupported target.
        let state = pickle.with_property(
            pickle.default_state,
            "pickles",
            &(random.next_int(4) + 1).to_string(),
        )?;
        if is_water(world, pos)? && would_survive(world, state, pos, environment)? {
            world.set_feature_block(pos, state, 2);
            placed = true;
        }
    }
    Ok(placed)
}

fn pile(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    provider: &StateProvider,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    if origin.1 < environment.generation_bounds().0 + 5 {
        return Ok(false);
    }
    let rx = random.next_int(2) as i32 + 2;
    let rz = random.next_int(2) as i32 + 2;
    for z in -rz..=rz {
        for y in 0..=1 {
            for x in -rx..=rx {
                let in_core = (x * x + z * z) as f32
                    <= random.next_float() * 10.0 - random.next_float() * 6.0;
                if !in_core && random.next_float() as f64 >= 0.031 {
                    continue;
                }
                let pos = offset(origin, (x, y, z));
                if !is_air(world, pos)? {
                    continue;
                }
                let below = read_block(world, Direction::Down.step(pos))?;
                let supported = if catalog().is_block(below, "dirt_path")? {
                    random.next_bool()
                } else {
                    catalog().info(below)?.sturdy(Direction::Up)
                };
                if supported {
                    let state =
                        provider.sample_with(world, random, pos, environment, dispatcher)?;
                    world.set_feature_block(pos, state, 260);
                }
            }
        }
    }
    Ok(true)
}

fn mushroom(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    config: &Mushroom,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let mut height = random.next_int(3) as i32 + 4;
    if random.next_int(12) == 0 {
        height *= 2;
    }
    let (min, max) = environment.generation_bounds();
    if origin.1 < min + 1 || origin.1 + height + 1 > max - 1 {
        return Ok(false);
    }
    if !config
        .soil
        .test(world, Direction::Down.step(origin), environment)?
    {
        return Ok(false);
    }
    for y in 0..=height {
        // The native validity call supplies (-1, -1, radius, y). For red
        // mushrooms that means a zero clearance radius at every checked level.
        let radius = if !config.red && y > 3 {
            config.radius
        } else {
            0
        };
        for x in -radius..=radius {
            for z in -radius..=radius {
                let old = read_block(world, offset(origin, (x, y, z)))?;
                if !catalog().info(old)?.is_air() && !catalog().in_block_tag(old, "leaves")? {
                    return Ok(false);
                }
            }
        }
    }
    let first_cap_y = if config.red { height - 3 } else { height };
    for y in first_cap_y..=height {
        let radius = config.radius - i32::from(config.red && y == height);
        for x in -radius..=radius {
            for z in -radius..=radius {
                let west = x == -radius;
                let east = x == radius;
                let north = z == -radius;
                let south = z == radius;
                let x_edge = west || east;
                let z_edge = north || south;
                if if config.red {
                    y < height && x_edge == z_edge
                } else {
                    x_edge && z_edge
                } {
                    continue;
                }
                // Native providers are sampled at ORIGIN, even for cap/trunk
                // positions, and before the per-position replacement check.
                let mut state =
                    config
                        .cap
                        .sample_with(world, random, origin, environment, dispatcher)?;
                let block = catalog().block(state)?.1;
                let properties: &[&str] = if config.red {
                    &["west", "east", "north", "south", "up"]
                } else {
                    &["west", "east", "north", "south"]
                };
                let has_properties = properties.iter().all(|name| {
                    block.properties.iter().any(|(key, values)| {
                        key == name
                            && values.len() == 2
                            && values
                                .iter()
                                .all(|value| value == "true" || value == "false")
                    })
                });
                if has_properties {
                    let (w, e, n, s) = if config.red {
                        let inner = config.radius - 2;
                        state = block.with_property(state, "up", bool_name(y >= height - 1))?;
                        (x < -inner, x > inner, z < -inner, z > inner)
                    } else {
                        (
                            west || z_edge && x == 1 - radius,
                            east || z_edge && x == radius - 1,
                            north || x_edge && z == 1 - radius,
                            south || x_edge && z == radius - 1,
                        )
                    };
                    for (key, value) in [("west", w), ("east", e), ("north", n), ("south", s)] {
                        state = block.with_property(state, key, bool_name(value))?;
                    }
                }
                place_mushroom_block(world, offset(origin, (x, y, z)), state)?;
            }
        }
    }
    for y in 0..height {
        let state = config
            .stem
            .sample_with(world, random, origin, environment, dispatcher)?;
        place_mushroom_block(world, offset(origin, (0, y, 0)), state)?;
    }
    Ok(true)
}

fn bool_name(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn place_mushroom_block(world: &mut dyn FeatureWorld, pos: Pos, state: u32) -> FeatureResult<()> {
    let old = read_block(world, pos)?;
    if catalog().info(old)?.is_air() || catalog().in_block_tag(old, "replaceable_by_mushrooms")? {
        world.set_feature_block(pos, state, 3);
    }
    Ok(())
}

fn random_tag(random: &mut WorldgenRandom, tag: &str) -> FeatureResult<Option<u32>> {
    let states = ordered_block_tag(tag)?;
    Ok(if states.is_empty() {
        None
    } else {
        Some(states[random.next_int(states.len())])
    })
}

fn coral_block(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    pos: Pos,
    state: u32,
) -> FeatureResult<bool> {
    let old = read_block(world, pos)?;
    let above = Direction::Up.step(pos);
    if (!catalog().is_block(old, "water")? && !catalog().in_block_tag(old, "corals")?)
        || !is_water(world, above)?
    {
        return Ok(false);
    }
    world.set_feature_block(pos, state, 3);
    if random.next_float() < 0.25 {
        if let Some(state) = random_tag(random, "corals")? {
            world.set_feature_block(above, state, 2);
        }
    } else if random.next_float() < 0.05 {
        let pickle = catalog().definition("sea_pickle")?;
        let state = pickle.with_property(
            pickle.default_state,
            "pickles",
            &(random.next_int(4) + 1).to_string(),
        )?;
        world.set_feature_block(above, state, 2);
    }
    for direction in Direction::HORIZONTAL {
        if random.next_float() >= 0.2 {
            continue;
        }
        let target = direction.step(pos);
        if is_water(world, target)? {
            if let Some(mut state) = random_tag(random, "wall_corals")? {
                let block = catalog().block(state)?.1;
                if block.property(state, "facing").is_some() {
                    state = block.with_property(state, "facing", direction_property(direction))?;
                }
                world.set_feature_block(target, state, 2);
            }
        }
    }
    Ok(true)
}

fn shuffle<T>(values: &mut [T], random: &mut WorldgenRandom) {
    for count in (2..=values.len()).rev() {
        let other = random.next_int(count);
        values.swap(count - 1, other);
    }
}

fn coral(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    config: Coral,
) -> FeatureResult<bool> {
    let Some(state) = random_tag(random, "coral_blocks")? else {
        return Ok(false);
    };
    match config {
        Coral::Tree => coral_tree(world, random, origin, state),
        Coral::Claw => coral_claw(world, random, origin, state),
        Coral::Mushroom => coral_mushroom(world, random, origin, state),
    }
}

fn coral_tree(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: u32,
) -> FeatureResult<bool> {
    let mut pos = origin;
    let height = random.next_int(3) + 1;
    for _ in 0..height {
        if !coral_block(world, random, pos, state)? {
            return Ok(true);
        }
        pos = Direction::Up.step(pos);
    }
    let branches = random.next_int(3) + 2;
    let mut directions = Direction::HORIZONTAL;
    shuffle(&mut directions, random);
    for direction in directions.into_iter().take(branches) {
        let mut pos = direction.step(pos);
        let height = random.next_int(5) + 2;
        let mut vertical_run = 0;
        for y in 0..height {
            if !coral_block(world, random, pos, state)? {
                break;
            }
            vertical_run += 1;
            pos = Direction::Up.step(pos);
            if y == 0 || (vertical_run >= 2 && random.next_float() < 0.25) {
                pos = direction.step(pos);
                vertical_run = 0;
            }
        }
    }
    Ok(true)
}

fn coral_claw(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: u32,
) -> FeatureResult<bool> {
    if !coral_block(world, random, origin, state)? {
        return Ok(false);
    }
    let forward_index = random.next_int(4);
    let forward = Direction::HORIZONTAL[forward_index];
    let count = random.next_int(2) + 2;
    let mut directions = [
        forward,
        Direction::HORIZONTAL[(forward_index + 1) % 4],
        Direction::HORIZONTAL[(forward_index + 3) % 4],
    ];
    shuffle(&mut directions, random);
    for direction in directions.into_iter().take(count) {
        let initial_run = random.next_int(2) + 1;
        let mut pos = direction.step(origin);
        let (growth, length) = if direction == forward {
            (forward, random.next_int(3) + 2)
        } else {
            pos = Direction::Up.step(pos);
            let growth = [direction, Direction::Up][random.next_int(2)];
            (growth, random.next_int(3) + 3)
        };
        for _ in 0..initial_run {
            if !coral_block(world, random, pos, state)? {
                break;
            }
            pos = growth.step(pos);
        }
        pos = Direction::Up.step(growth.opposite().step(pos));
        for _ in 0..length {
            pos = forward.step(pos);
            if !coral_block(world, random, pos, state)? {
                break;
            }
            if random.next_float() < 0.25 {
                pos = Direction::Up.step(pos);
            }
        }
    }
    Ok(true)
}

fn coral_mushroom(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    state: u32,
) -> FeatureResult<bool> {
    let height = random.next_int(3) as i32 + 3;
    let width = random.next_int(3) as i32 + 3;
    let depth = random.next_int(3) as i32 + 3;
    let down = random.next_int(3) as i32 + 1;
    for x in 0..=width {
        for y in 0..=height {
            for z in 0..=depth {
                let edges = i32::from(x == 0 || x == width)
                    + i32::from(y == 0 || y == height)
                    + i32::from(z == 0 || z == depth);
                if edges != 1 || random.next_float() < 0.1 {
                    continue;
                }
                coral_block(world, random, offset(origin, (x, y - down, z)), state)?;
            }
        }
    }
    Ok(true)
}

fn blob(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    mut origin: Pos,
    environment: &dyn FeatureEnvironment,
    state: u32,
    soil: &BlockPredicate,
) -> FeatureResult<bool> {
    let min = environment.generation_bounds().0;
    while origin.1 > min + 3 {
        if soil.test(world, Direction::Down.step(origin), environment)? {
            break;
        }
        origin = Direction::Down.step(origin);
    }
    if origin.1 <= min + 3 {
        return Ok(false);
    }
    for _ in 0..3 {
        let rx = random.next_int(2) as i32;
        let ry = random.next_int(2) as i32;
        let rz = random.next_int(2) as i32;
        let radius = (rx + ry + rz) as f32 * 0.333_f32 + 0.5_f32;
        for z in -rz..=rz {
            for y in -ry..=ry {
                for x in -rx..=rx {
                    if (x * x + y * y + z * z) as f64 <= (radius * radius) as f64 {
                        world.set_feature_block(offset(origin, (x, y, z)), state, 3);
                    }
                }
            }
        }
        let x = -1 + random.next_int(2) as i32;
        let y = -(random.next_int(2) as i32);
        let z = -1 + random.next_int(2) as i32;
        origin = offset(origin, (x, y, z));
    }
    Ok(true)
}

fn blue_ice(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    if origin.1 > environment.sea_level() - 1 {
        return Ok(false);
    }
    if !is_water(world, origin)? && !is_water(world, Direction::Down.step(origin))? {
        return Ok(false);
    }
    let mut packed_neighbor = false;
    for direction in Direction::ALL {
        if direction == Direction::Down {
            continue;
        }
        if catalog().is_block(read_block(world, direction.step(origin))?, "packed_ice")? {
            packed_neighbor = true;
            break;
        }
    }
    if !packed_neighbor {
        return Ok(false);
    }
    let state = catalog().default_state("blue_ice")?;
    world.set_feature_block(origin, state, 2);
    for _ in 0..200 {
        let y = random.next_int(5) as i32 - random.next_int(6) as i32;
        let radius = if y < 2 { 3 + y / 2 } else { 3 };
        if radius < 1 {
            continue;
        }
        let x = random.next_int(radius as usize) as i32 - random.next_int(radius as usize) as i32;
        let z = random.next_int(radius as usize) as i32 - random.next_int(radius as usize) as i32;
        let pos = offset(origin, (x, y, z));
        let old = read_block(world, pos)?;
        if !catalog().info(old)?.is_air()
            && !catalog().is_block(old, "water")?
            && !catalog().is_block(old, "packed_ice")?
            && !catalog().is_block(old, "ice")?
        {
            continue;
        }
        for direction in Direction::ALL {
            if catalog().is_block(read_block(world, direction.step(pos))?, "blue_ice")? {
                world.set_feature_block(pos, state, 2);
                break;
            }
        }
    }
    Ok(true)
}

fn spike(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    mut origin: Pos,
    environment: &dyn FeatureEnvironment,
    state: u32,
    soil: &BlockPredicate,
    replace: &BlockPredicate,
) -> FeatureResult<bool> {
    while is_air(world, origin)? && origin.1 > environment.generation_bounds().0 + 2 {
        origin = Direction::Down.step(origin);
    }
    if !soil.test(world, origin, environment)? {
        return Ok(false);
    }
    origin = offset(origin, (0, random.next_int(4) as i32, 0));
    let height = random.next_int(4) as i32 + 7;
    let radius = height / 4 + random.next_int(2) as i32;
    if radius > 1 && random.next_int(60) == 0 {
        origin = offset(origin, (0, 10 + random.next_int(30) as i32, 0));
    }
    for y in 0..height {
        let f = (1.0_f32 - y as f32 / height as f32) * radius as f32;
        let width = f.ceil() as i32;
        for x in -width..=width {
            let dx = x.abs() as f32 - 0.25_f32;
            for z in -width..=width {
                let dz = z.abs() as f32 - 0.25_f32;
                if (x != 0 || z != 0) && dx * dx + dz * dz > f * f {
                    continue;
                }
                if (x == -width || x == width || z == -width || z == width)
                    && random.next_float() > 0.75_f32
                {
                    continue;
                }
                let pos = offset(origin, (x, y, z));
                if is_air(world, pos)? || replace.test(world, pos, environment)? {
                    world.set_feature_block(pos, state, 3);
                }
                if y != 0 && width > 1 {
                    let pos = offset(origin, (x, -y, z));
                    if is_air(world, pos)? || replace.test(world, pos, environment)? {
                        world.set_feature_block(pos, state, 3);
                    }
                }
            }
        }
    }
    let roots = (radius - 1).clamp(0, 1);
    for x in -roots..=roots {
        for z in -roots..=roots {
            let mut pos = offset(origin, (x, -1, z));
            let mut run = if x.abs() == 1 && z.abs() == 1 {
                random.next_int(5) as i32
            } else {
                50
            };
            // Native uses absolute Y=50 here, not a dimension-relative bound.
            while pos.1 > 50 {
                let old = read_block(world, pos)?;
                if !catalog().info(old)?.is_air()
                    && !replace.test(world, pos, environment)?
                    && old != state
                {
                    break;
                }
                world.set_feature_block(pos, state, 3);
                pos = Direction::Down.step(pos);
                run -= 1;
                if run <= 0 {
                    pos = offset(pos, (0, -(random.next_int(5) as i32 + 1), 0));
                    run = random.next_int(5) as i32;
                }
            }
        }
    }
    Ok(true)
}

struct Iceberg {
    origin: Pos,
    state: u32,
    snowy: bool,
    elliptical: bool,
    angle: f64,
    major: i32,
    minor: i32,
    radius: i32,
}

fn iceberg(
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    state: u32,
) -> FeatureResult<bool> {
    let origin = (origin.0, environment.generator_sea_level(), origin.2);
    let snowy = random.next_double() > 0.7;
    let angle = random.next_double() * 2.0 * std::f64::consts::PI;
    let major = 11 - random.next_int(5) as i32;
    let minor = 3 + random.next_int(3) as i32;
    let elliptical = random.next_double() > 0.7;
    let mut height = if elliptical {
        random.next_int(6) as i32 + 6
    } else {
        random.next_int(15) as i32 + 3
    };
    if !elliptical && random.next_double() > 0.9 {
        height += random.next_int(19) as i32 + 7;
    }
    let depth = (height + random.next_int(11) as i32).min(18);
    let radius = (height + random.next_int(7) as i32 - random.next_int(5) as i32).min(11);
    let extent = if elliptical { major } else { 11 };
    let shape = Iceberg {
        origin,
        state,
        snowy,
        elliptical,
        angle,
        major,
        minor,
        radius,
    };
    for x in -extent..extent {
        for z in -extent..extent {
            for y in 0..height {
                let r = if elliptical {
                    (((1.0_f32 - (y as f64).powi(2) as f32 / height as f32) * radius as f32)
                        / 2.0_f32)
                        .ceil() as i32
                } else {
                    iceberg_radius_round(random, y, height, radius)
                };
                if elliptical || x < r {
                    shape.block(world, random, (x, y, z), height, r, extent)?;
                }
            }
        }
    }
    shape.smooth(world, height)?;
    for x in -extent..extent {
        for z in -extent..extent {
            for y in ((-depth + 1)..=-1).rev() {
                let major = if elliptical {
                    (extent as f32
                        * (1.0_f32 - (y as f64).powi(2) as f32 / (depth as f32 * 8.0_f32)))
                        .ceil() as i32
                } else {
                    extent
                };
                let r = iceberg_radius_steep(random, -y, depth, radius);
                if x < r {
                    shape.block(world, random, (x, y, z), depth, r, major)?;
                }
            }
        }
    }
    let cut = random.next_double() > if elliptical { 0.1 } else { 0.7 };
    if cut {
        shape.cut(world, random, height)?;
    }
    Ok(true)
}

impl Iceberg {
    fn block(
        &self,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        delta: Pos,
        height: i32,
        radius: i32,
        major: i32,
    ) -> FeatureResult<()> {
        let (x, y, z) = delta;
        let distance = if self.elliptical {
            let minor = if y > 0 && height - y <= 3 {
                self.minor - (4 - (height - y))
            } else {
                self.minor
            };
            ellipse_distance(x, z, (0, 0, 0), major, minor, self.angle)
        } else {
            let fuzz = 10.0_f32 * random.next_float().clamp(0.2, 0.8) / radius as f32;
            fuzz as f64 + (x as f64).powi(2) + (z as f64).powi(2) - (radius as f64).powi(2)
        };
        // Zero ellipse axes can produce NaN; Java's dcmpg rejects those cells.
        if !(distance < 0.0) {
            return Ok(());
        }
        let threshold = if self.elliptical {
            -0.5
        } else {
            (-6 - random.next_int(3) as i32) as f64
        };
        if distance > threshold && random.next_double() > 0.9 {
            return Ok(());
        }
        let pos = offset(self.origin, delta);
        let old = read_block(world, pos)?;
        let data = catalog();
        if !data.info(old)?.is_air()
            && !data.is_block(old, "snow_block")?
            && !data.is_block(old, "ice")?
            && !data.is_block(old, "water")?
        {
            return Ok(());
        }
        let snow_allowed = !self.elliptical || random.next_double() > 0.05;
        let divisor = if self.elliptical { 3 } else { 2 };
        let state = if self.snowy
            && !data.is_block(old, "water")?
            && (height - y) as f64
                <= random.next_int((height / divisor).max(1) as usize) as f64 + height as f64 * 0.6
            && snow_allowed
        {
            data.default_state("snow_block")?
        } else {
            self.state
        };
        world.set_feature_block(pos, state, 3);
        Ok(())
    }

    fn smooth(&self, world: &mut dyn FeatureWorld, height: i32) -> FeatureResult<()> {
        let radius = if self.elliptical {
            self.major
        } else {
            self.radius / 2
        };
        let air = catalog().default_state("air")?;
        for x in -radius..=radius {
            for z in -radius..=radius {
                for y in 0..=height {
                    let pos = offset(self.origin, (x, y, z));
                    let state = read_block(world, pos)?;
                    let ice = iceberg_state(state)?;
                    if !ice && !catalog().is_block(state, "snow")? {
                        continue;
                    }
                    if is_air(world, Direction::Down.step(pos))? {
                        world.set_feature_block(pos, air, 3);
                        world.set_feature_block(Direction::Up.step(pos), air, 3);
                    } else if ice {
                        let mut exposed = 0;
                        for direction in [
                            Direction::West,
                            Direction::East,
                            Direction::North,
                            Direction::South,
                        ] {
                            if !iceberg_state(read_block(world, direction.step(pos))?)? {
                                exposed += 1;
                            }
                        }
                        if exposed >= 3 {
                            world.set_feature_block(pos, air, 3);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn cut(
        &self,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        height: i32,
    ) -> FeatureResult<()> {
        let sx = if random.next_bool() { -1 } else { 1 };
        let sz = if random.next_bool() { -1 } else { 1 };
        let mut x = random.next_int((self.radius / 2 - 2).max(1) as usize) as i32;
        if random.next_bool() {
            x = self.radius / 2 + 1
                - random.next_int((self.radius - self.radius / 2 - 1).max(1) as usize) as i32;
        }
        let mut z = random.next_int((self.radius / 2 - 2).max(1) as usize) as i32;
        if random.next_bool() {
            z = self.radius / 2 + 1
                - random.next_int((self.radius - self.radius / 2 - 1).max(1) as usize) as i32;
        }
        if self.elliptical {
            z = random.next_int((self.major - 5).max(1) as usize) as i32;
            x = z;
        }
        let center = (sx * x, 0, sz * z);
        let angle = if self.elliptical {
            self.angle + std::f64::consts::FRAC_PI_2
        } else {
            random.next_double() * 2.0 * std::f64::consts::PI
        };
        for y in 0..height - 3 {
            let radius = iceberg_radius_round(random, y, height, self.radius);
            self.carve(world, y, radius, false, center, angle)?;
        }
        let mut y = -1;
        // nextInt is part of the loop CONDITION, including its final failure.
        while y > -height + random.next_int(5) as i32 {
            let radius = iceberg_radius_steep(random, -y, height, self.radius);
            self.carve(world, y, radius, true, center, angle)?;
            y -= 1;
        }
        Ok(())
    }

    fn carve(
        &self,
        world: &mut dyn FeatureWorld,
        y: i32,
        radius: i32,
        water: bool,
        center: Pos,
        angle: f64,
    ) -> FeatureResult<()> {
        let major = radius + 1 + self.major / 3;
        let minor = (radius - 3).min(3) + self.minor / 2 - 1;
        let fill = catalog().default_state(if water { "water" } else { "air" })?;
        for x in -major..major {
            for z in -major..major {
                if !(ellipse_distance(x, z, center, major, minor, angle) < 0.0) {
                    continue;
                }
                let pos = offset(self.origin, (x, y, z));
                if iceberg_state(read_block(world, pos)?)? {
                    world.set_feature_block(pos, fill, 3);
                    if !water {
                        let above = Direction::Up.step(pos);
                        if catalog().is_block(read_block(world, above)?, "snow")? {
                            world.set_feature_block(above, fill, 3);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn iceberg_radius_round(random: &mut WorldgenRandom, y: i32, height: i32, radius: i32) -> i32 {
    let scale = 3.5_f32 - random.next_float();
    let mut value = (1.0_f32 - (y as f64).powi(2) as f32 / (height as f32 * scale)) * radius as f32;
    if height > 15 + random.next_int(5) as i32 {
        let adjusted_y = if y < 3 + random.next_int(6) as i32 {
            y / 2
        } else {
            y
        };
        value = (1.0_f32 - adjusted_y as f32 / (height as f32 * scale * 0.4_f32)) * radius as f32;
    }
    (value / 2.0_f32).ceil() as i32
}

fn iceberg_radius_steep(random: &mut WorldgenRandom, y: i32, height: i32, radius: i32) -> i32 {
    let scale = 1.0_f32 + random.next_float() / 2.0_f32;
    (((1.0_f32 - y as f32 / (height as f32 * scale)) * radius as f32) / 2.0_f32).ceil() as i32
}

fn iceberg_state(state: u32) -> FeatureResult<bool> {
    Ok(catalog().is_block(state, "packed_ice")?
        || catalog().is_block(state, "snow_block")?
        || catalog().is_block(state, "blue_ice")?)
}

fn ellipse_distance(x: i32, z: i32, center: Pos, major: i32, minor: i32, angle: f64) -> f64 {
    let dx = (x - center.0) as f64;
    let dz = (z - center.2) as f64;
    let (sin, cos) = iceberg_sin_cos(angle);
    ((dx * cos - dz * sin) / major as f64).powi(2) + ((dx * sin + dz * cos) / minor as f64).powi(2)
        - 1.0
}

/// Compensated pi/32 reduction for the iceberg's generated [0, 5*pi/2) angles.
/// The pinned x86-64 HotSpot Math intrinsics differ from both host libm and
/// StrictMath. Numerical constants are independently derived by
/// `probes/misc_features/build_trig_table.py`; this is not a general sin/cos API.
fn iceberg_sin_cos(angle: f64) -> (f64, f64) {
    #[derive(serde::Deserialize)]
    struct Data {
        pi32_inverse: u64,
        pi32_split: [u64; 3],
        coefficients: [[u64; 2]; 4],
        table: Vec<[u64; 4]>,
    }
    struct Numbers {
        inverse: f64,
        split: [f64; 3],
        coefficients: [[f64; 2]; 4],
        table: Vec<[f64; 4]>,
    }
    static DATA: OnceLock<Numbers> = OnceLock::new();
    let data = DATA.get_or_init(|| {
        let input: Data = serde_json::from_str(include_str!("../data/misc_feature_trig_26_1.json"))
            .expect("independently derived compensated trig constants");
        assert_eq!(input.table.len(), 64);
        Numbers {
            inverse: f64::from_bits(input.pi32_inverse),
            split: input.pi32_split.map(f64::from_bits),
            coefficients: input.coefficients.map(|row| row.map(f64::from_bits)),
            table: input
                .table
                .into_iter()
                .map(|row| row.map(f64::from_bits))
                .collect(),
        }
    });
    debug_assert!((0.0..=2.5 * std::f64::consts::PI).contains(&angle));
    let n = (angle * data.inverse + 0.5) as i32;
    let r1 = angle - n as f64 * data.split[0];
    let middle = n as f64 * data.split[1];
    let r = r1 - middle;
    let negative_tail = n as f64 * data.split[2] - ((r1 - r) - middle);
    let r2 = r * r;
    let r4 = r2 * r2;
    let mut output = [0.0; 2];
    for (slot, shift) in output.iter_mut().zip([0, 16]) {
        let [c, hi, lo, sigma] = data.table[((n + shift) & 63) as usize];
        let cosine = c + sigma;
        let [p1, p2, p3, p4] = data.coefficients;
        let sin_poly =
            ((p2[0] * r2 + p1[0]) + ((p4[0] * r1) * r + p3[0]) * r4) * ((cosine * r) * r2);
        let cos_poly = ((p2[1] * r2 + p1[1]) + ((p4[1] * r1) * r + p3[1]) * r4) * (hi * r2);
        let sigma_r = sigma * r;
        let mid = c * r;
        let high = sigma_r + hi;
        let total = mid + high;
        let mut correction = negative_tail * (hi * r - cosine) + lo;
        correction += (hi - high) + sigma_r;
        correction += (high - total) + mid;
        correction += sin_poly;
        correction += cos_poly;
        *slot = total + correction;
    }
    (output[0], output[1])
}

#[cfg(test)]
mod native_math_tests {
    use super::*;

    #[test]
    fn native_iceberg_ellipse_float_bits() {
        let fixture: Value =
            serde_json::from_str(include_str!("../data/misc_features_shapes_26_1.json")).unwrap();
        for sample in fixture["ellipse_math"].as_array().unwrap() {
            let [x, z, cx, cz, a, b]: [i32; 6] =
                serde_json::from_value(sample["parameters"].clone()).unwrap();
            let angle = f64::from_bits(sample["angle_bits"].as_str().unwrap().parse().unwrap());
            let expected: u64 = sample["result_bits"].as_str().unwrap().parse().unwrap();
            let actual = ellipse_distance(x, z, (cx, 0, cz), a, b, angle);
            if f64::from_bits(expected).is_nan() {
                assert!(actual.is_nan(), "native degenerate ellipse {sample}");
            } else {
                assert_eq!(
                    actual.to_bits(),
                    expected,
                    "native ellipse arithmetic {sample}"
                );
            }
        }
    }

    #[test]
    fn native_hotspot_iceberg_trigonometry() {
        for fixture in [
            include_str!("../data/misc_feature_math_26_1.json"),
            include_str!("../data/misc_feature_math_boundaries_26_1.json"),
        ] {
            let fixture: Value = serde_json::from_str(fixture).unwrap();
            for sample in fixture["samples"].as_array().unwrap() {
                let angle = f64::from_bits(sample["angle"].as_str().unwrap().parse().unwrap());
                let (sin, cos) = iceberg_sin_cos(angle);
                assert_eq!(
                    sin.to_bits().to_string(),
                    sample["sin"].as_str().unwrap(),
                    "native Math.sin({angle})"
                );
                assert_eq!(
                    cos.to_bits().to_string(),
                    sample["cos"].as_str().unwrap(),
                    "native Math.cos({angle})"
                );
                let distance = ellipse_distance(-7, 3, (1, 0, -2), 11, -1, angle);
                assert_eq!(
                    distance.to_bits().to_string(),
                    sample["distance"].as_str().unwrap(),
                    "native ellipse({angle})"
                );
            }
        }
    }
}
