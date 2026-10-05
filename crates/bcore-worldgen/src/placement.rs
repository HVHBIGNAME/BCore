//! Lazy Java 26.1 placed-feature traversal.
//!
//! A terminal feature runs before the next sibling origin is generated. The
//! sole eager modifier is vanilla's own `count_on_every_layer` stream builder.
use std::sync::OnceLock;

use serde_json::Value;

use crate::block_predicate::{
    array, catalog, int, integer, invalid, kind, number, offset, position, read_block, string,
    BlockPredicate, Direction, FeatureEnvironment, FeatureResult,
};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::simplex::{SimplexNoise, WorldgenRandom};
use crate::{MAX_Y, MIN_Y};

/// The scheduler supplies configured implementations through this boundary.
/// Named references have been resolved against the captured 26.1 catalog.
pub trait ConfiguredFeatureDispatcher {
    /// Use the cache belonging to this exact WorldgenRandom, including across
    /// feature reseeding. Cave kernels and placement must share the same cache.
    fn next_gaussian(&mut self, _random: &mut WorldgenRandom) -> FeatureResult<f64> {
        Err(FeatureError::MissingData(
            "WorldgenRandom Gaussian cache (share the cave random state with placement)".into(),
        ))
    }

    /// Moss carpet uses WorldGenLevel.getRandom(), a separate random source.
    fn next_world_bool(&mut self, _world: &dyn FeatureWorld) -> FeatureResult<bool> {
        Err(FeatureError::MissingData(
            "independent WorldGenLevel random for moss carpet".into(),
        ))
    }

    fn place_configured(
        &mut self,
        config: &Value,
        name: Option<&str>,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        origin: Pos,
        environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool>;
}

impl<F> ConfiguredFeatureDispatcher for F
where
    F: FnMut(
        &Value,
        Option<&str>,
        &mut dyn FeatureWorld,
        &mut WorldgenRandom,
        Pos,
        &dyn FeatureEnvironment,
    ) -> FeatureResult<bool>,
{
    fn place_configured(
        &mut self,
        config: &Value,
        name: Option<&str>,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        origin: Pos,
        environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        self(config, name, world, random, origin, environment)
    }
}

#[derive(Default)]
pub struct RejectUnsupported;
impl ConfiguredFeatureDispatcher for RejectUnsupported {
    fn place_configured(
        &mut self,
        config: &Value,
        name: Option<&str>,
        _world: &mut dyn FeatureWorld,
        _random: &mut WorldgenRandom,
        _origin: Pos,
        _environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        Err(FeatureError::Unsupported(format!(
            "configured feature {} ({})",
            name.unwrap_or("<inline>"),
            kind(config)?
        )))
    }
}

#[derive(Clone, Debug)]
pub enum IntProvider {
    Constant(i32),
    Uniform(i32, i32),
    Biased(i32, i32),
    Clamped(Box<Self>, i32, i32),
    Weighted(Vec<(Self, i32)>, i32),
    Trapezoid(i32, i32, i32),
    Normal(f32, f32, i32, i32),
}

impl IntProvider {
    pub fn parse(value: &Value) -> FeatureResult<Self> {
        if value.is_number() {
            return Ok(Self::Constant(int(value)?));
        }
        let provider = match kind(value)? {
            "constant" => Self::Constant(integer(value, "value")?),
            "uniform" => Self::Uniform(
                integer(value, "min_inclusive")?,
                integer(value, "max_inclusive")?,
            ),
            "biased_to_bottom" => Self::Biased(
                integer(value, "min_inclusive")?,
                integer(value, "max_inclusive")?,
            ),
            "clamped" => Self::Clamped(
                Box::new(Self::parse(&value["source"])?),
                integer(value, "min_inclusive")?,
                integer(value, "max_inclusive")?,
            ),
            "weighted_list" => {
                let mut total = 0_i32;
                let mut entries = Vec::new();
                for entry in array(&value["distribution"])? {
                    let weight = integer(entry, "weight")?;
                    if weight < 0 {
                        return Err(invalid("negative provider weight"));
                    }
                    total = total
                        .checked_add(weight)
                        .ok_or_else(|| invalid("provider weight overflow"))?;
                    entries.push((Self::parse(&entry["data"])?, weight));
                }
                if total == 0 {
                    return Err(invalid("weighted provider has no positive weight"));
                }
                Self::Weighted(entries, total)
            }
            "trapezoid" => Self::Trapezoid(
                integer(value, "min")?,
                integer(value, "max")?,
                integer(value, "plateau")?,
            ),
            "clamped_normal" => Self::Normal(
                number(value, "mean")? as f32,
                number(value, "deviation")? as f32,
                integer(value, "min_inclusive")?,
                integer(value, "max_inclusive")?,
            ),
            other => {
                return Err(FeatureError::Unsupported(format!(
                    "integer provider {other}"
                )))
            }
        };
        let (min, max) = provider.bounds();
        if min > max || i64::from(max) - i64::from(min) >= i64::from(i32::MAX) {
            return Err(invalid("invalid integer provider bounds"));
        }
        if let Self::Trapezoid(min, max, plateau) = provider {
            if plateau < 0 || plateau > max - min {
                return Err(invalid("trapezoid plateau outside range"));
            }
        }
        Ok(provider)
    }

    pub fn bounds(&self) -> (i32, i32) {
        match self {
            Self::Constant(v) => (*v, *v),
            Self::Uniform(a, b)
            | Self::Biased(a, b)
            | Self::Clamped(_, a, b)
            | Self::Trapezoid(a, b, _)
            | Self::Normal(_, _, a, b) => (*a, *b),
            Self::Weighted(entries, _) => entries
                .iter()
                .map(|(p, _)| p.bounds())
                .fold((i32::MAX, i32::MIN), |(a, b), (c, d)| (a.min(c), b.max(d))),
        }
    }

    pub fn require_range(self, min: i32, max: i32) -> FeatureResult<Self> {
        let (a, b) = self.bounds();
        if a < min || b > max {
            return Err(invalid(format!(
                "provider range {a}..={b} outside {min}..={max}"
            )));
        }
        Ok(self)
    }

    pub fn sample(&self, random: &mut WorldgenRandom) -> FeatureResult<i32> {
        self.sample_with(random, &mut RejectUnsupported)
    }

    pub fn sample_with(
        &self,
        random: &mut WorldgenRandom,
        dispatcher: &mut dyn ConfiguredFeatureDispatcher,
    ) -> FeatureResult<i32> {
        Ok(match self {
            Self::Constant(v) => *v,
            Self::Uniform(min, max) => between(random, *min, *max),
            Self::Biased(min, max) => {
                let inner = random.next_int((max - min + 1) as usize) + 1;
                min + random.next_int(inner) as i32
            }
            Self::Clamped(source, min, max) => {
                source.sample_with(random, dispatcher)?.clamp(*min, *max)
            }
            Self::Weighted(entries, total) => {
                let mut choice = random.next_int(*total as usize) as i32;
                for (provider, weight) in entries {
                    choice -= weight;
                    if choice < 0 {
                        return provider.sample_with(random, dispatcher);
                    }
                }
                unreachable!("validated positive provider weights")
            }
            Self::Trapezoid(min, max, plateau) => {
                // Native 26.1 special-cases a symmetric triangle as subtraction,
                // including two nextInt(1) calls for a zero-width triangle.
                if *plateau == 0 && *max == min.wrapping_neg() {
                    return Ok(random.next_int((max + 1) as usize) as i32
                        - random.next_int((max + 1) as usize) as i32);
                }
                let width = max - min;
                if *plateau == width {
                    return Ok(between(random, *min, *max));
                }
                let slope = (width - plateau) / 2;
                min + between(random, 0, width - slope) + between(random, 0, slope)
            }
            Self::Normal(mean, deviation, min, max) => {
                let value = mean + dispatcher.next_gaussian(random)? as f32 * deviation;
                value.clamp(*min as f32, *max as f32) as i32
            }
        })
    }
}

pub fn sample_int(value: &Value, random: &mut WorldgenRandom) -> FeatureResult<i32> {
    IntProvider::parse(value)?.sample(random)
}

fn between(random: &mut WorldgenRandom, min: i32, max: i32) -> i32 {
    min + random.next_int((i64::from(max) - i64::from(min) + 1) as usize) as i32
}

#[derive(Clone, Debug)]
enum Anchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
}
impl Anchor {
    fn parse(value: &Value) -> FeatureResult<Self> {
        if let Some(v) = value.get("absolute") {
            return Ok(Self::Absolute(int(v)?));
        }
        if let Some(v) = value.get("above_bottom") {
            return Ok(Self::AboveBottom(int(v)?));
        }
        if let Some(v) = value.get("below_top") {
            return Ok(Self::BelowTop(int(v)?));
        }
        Err(invalid(format!("vertical anchor {value}")))
    }

    fn resolve(&self, bounds: (i32, i32)) -> i32 {
        match self {
            Self::Absolute(v) => *v,
            Self::AboveBottom(v) => bounds.0.wrapping_add(*v),
            Self::BelowTop(v) => bounds.1.wrapping_sub(1).wrapping_sub(*v),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HeightProvider(HeightDistribution);
#[derive(Clone, Debug)]
enum HeightDistribution {
    Constant(Anchor),
    Uniform(Anchor, Anchor),
    Trapezoid(Anchor, Anchor, i32),
    Biased(Anchor, Anchor, i32, bool),
    Weighted(Vec<(HeightProvider, i32)>, i32),
}

impl HeightProvider {
    pub fn parse(value: &Value) -> FeatureResult<Self> {
        if value.get("type").is_none() {
            return Ok(Self(HeightDistribution::Constant(Anchor::parse(value)?)));
        }
        let distribution = match kind(value)? {
            "constant" => HeightDistribution::Constant(Anchor::parse(&value["value"])?),
            "uniform" => HeightDistribution::Uniform(
                Anchor::parse(&value["min_inclusive"])?,
                Anchor::parse(&value["max_inclusive"])?,
            ),
            "trapezoid" => HeightDistribution::Trapezoid(
                Anchor::parse(&value["min_inclusive"])?,
                Anchor::parse(&value["max_inclusive"])?,
                value.get("plateau").map(int).transpose()?.unwrap_or(0),
            ),
            "biased_to_bottom" | "very_biased_to_bottom" => {
                let inner = value.get("inner").map(int).transpose()?.unwrap_or(1);
                if inner < 1 {
                    return Err(invalid("height inner must be positive"));
                }
                HeightDistribution::Biased(
                    Anchor::parse(&value["min_inclusive"])?,
                    Anchor::parse(&value["max_inclusive"])?,
                    inner,
                    kind(value)? == "very_biased_to_bottom",
                )
            }
            "weighted_list" => {
                let mut total = 0_i32;
                let mut entries = Vec::new();
                for entry in array(&value["distribution"])? {
                    let weight = integer(entry, "weight")?;
                    if weight < 0 {
                        return Err(invalid("negative height weight"));
                    }
                    total = total
                        .checked_add(weight)
                        .ok_or_else(|| invalid("height weight overflow"))?;
                    entries.push((Self::parse(&entry["data"])?, weight));
                }
                if total == 0 {
                    return Err(invalid("height provider has no positive weight"));
                }
                HeightDistribution::Weighted(entries, total)
            }
            other => {
                return Err(FeatureError::Unsupported(format!(
                    "height provider {other}"
                )))
            }
        };
        Ok(Self(distribution))
    }

    pub fn sample(&self, random: &mut WorldgenRandom, bounds: (i32, i32)) -> FeatureResult<i32> {
        let (a, b) = match &self.0 {
            HeightDistribution::Constant(anchor) => return Ok(anchor.resolve(bounds)),
            HeightDistribution::Weighted(entries, total) => {
                let mut choice = random.next_int(*total as usize) as i32;
                for (provider, weight) in entries {
                    choice -= weight;
                    if choice < 0 {
                        return provider.sample(random, bounds);
                    }
                }
                unreachable!("validated height weights")
            }
            HeightDistribution::Uniform(a, b)
            | HeightDistribution::Trapezoid(a, b, _)
            | HeightDistribution::Biased(a, b, _, _) => (a, b),
        };
        let min = a.resolve(bounds);
        let max = b.resolve(bounds);
        if min > max {
            return Ok(min);
        }
        let width = i64::from(max) - i64::from(min);
        if width >= i64::from(i32::MAX) {
            return Err(invalid("height sample bound overflow"));
        }
        let width = width as i32;
        Ok(match &self.0 {
            HeightDistribution::Uniform(_, _) => between(random, min, max),
            HeightDistribution::Trapezoid(_, _, plateau) => {
                if *plateau >= width {
                    between(random, min, max)
                } else {
                    let slope = (width - plateau) / 2;
                    min + between(random, 0, width - slope) + between(random, 0, slope)
                }
            }
            HeightDistribution::Biased(_, _, inner, very) => {
                if i64::from(width) - i64::from(*inner) + 1 <= 0 {
                    return Ok(min);
                }
                if *very {
                    // Mth.nextInt returns min without a draw when max <= min.
                    let mut draw = |lo, hi| {
                        if lo >= hi {
                            lo
                        } else {
                            between(random, lo, hi)
                        }
                    };
                    let a = draw(min + inner, max);
                    let b = draw(min, a - 1);
                    draw(min, b - 1 + inner)
                } else {
                    let a = random.next_int((width - inner + 1) as usize) as i32;
                    min + random.next_int((a + inner) as usize) as i32
                }
            }
            _ => unreachable!("constant and weighted handled above"),
        })
    }
}

#[derive(Clone, Debug)]
enum Modifier {
    Count(IntProvider),
    Rarity(i32),
    InSquare,
    Height(HeightProvider),
    Heightmap(FeatureHeightmap),
    Biome,
    Predicate(BlockPredicate),
    Offset(IntProvider, IntProvider),
    Scan(Direction, BlockPredicate, BlockPredicate, i32),
    Surface(FeatureHeightmap, i32, i32),
    WaterDepth(i32),
    NoiseCount(i32, f64, f64),
    NoiseThreshold(f64, i32, i32),
    EveryLayer(IntProvider),
    Fixed(Vec<Pos>),
}

impl Modifier {
    fn parse(value: &Value) -> FeatureResult<Self> {
        Ok(match kind(value)? {
            "count" => Self::Count(IntProvider::parse(&value["count"])?.require_range(0, 256)?),
            "rarity_filter" => {
                let chance = integer(value, "chance")?;
                if chance < 1 {
                    return Err(invalid("rarity chance must be positive"));
                }
                Self::Rarity(chance)
            }
            "in_square" => Self::InSquare,
            "height_range" => Self::Height(HeightProvider::parse(&value["height"])?),
            "heightmap" => Self::Heightmap(heightmap(&value["heightmap"])?),
            "biome" => Self::Biome,
            "block_predicate_filter" => {
                Self::Predicate(BlockPredicate::parse(&value["predicate"])?)
            }
            "random_offset" => Self::Offset(
                IntProvider::parse(&value["xz_spread"])?.require_range(-16, 16)?,
                IntProvider::parse(&value["y_spread"])?.require_range(-16, 16)?,
            ),
            "environment_scan" => {
                let direction = Direction::parse(&value["direction_of_search"])?;
                let steps = integer(value, "max_steps")?;
                if !matches!(direction, Direction::Up | Direction::Down)
                    || !(1..=32).contains(&steps)
                {
                    return Err(invalid(
                        "environment scan requires vertical direction and 1..=32 steps",
                    ));
                }
                Self::Scan(
                    direction,
                    BlockPredicate::parse(&value["target_condition"])?,
                    value
                        .get("allowed_search_condition")
                        .map(BlockPredicate::parse)
                        .transpose()?
                        .unwrap_or(BlockPredicate::True),
                    steps,
                )
            }
            "surface_relative_threshold_filter" => Self::Surface(
                heightmap(&value["heightmap"])?,
                value
                    .get("min_inclusive")
                    .map(int)
                    .transpose()?
                    .unwrap_or(i32::MIN),
                value
                    .get("max_inclusive")
                    .map(int)
                    .transpose()?
                    .unwrap_or(i32::MAX),
            ),
            "surface_water_depth_filter" => Self::WaterDepth(integer(value, "max_water_depth")?),
            "noise_based_count" => Self::NoiseCount(
                integer(value, "noise_to_count_ratio")?,
                number(value, "noise_factor")?,
                value
                    .get("noise_offset")
                    .map(|_| number(value, "noise_offset"))
                    .transpose()?
                    .unwrap_or(0.0),
            ),
            "noise_threshold_count" => Self::NoiseThreshold(
                number(value, "noise_level")?,
                integer(value, "below_noise")?,
                integer(value, "above_noise")?,
            ),
            "count_on_every_layer" => {
                Self::EveryLayer(IntProvider::parse(&value["count"])?.require_range(0, 256)?)
            }
            "fixed_placement" => Self::Fixed(
                array(&value["positions"])?
                    .iter()
                    .map(position)
                    .collect::<FeatureResult<_>>()?,
            ),
            other => {
                return Err(FeatureError::Unsupported(format!(
                    "placement modifier {other} (not a 26.1 registered modifier)"
                )))
            }
        })
    }
}

/// Compile once when reusing a placed feature across many chunks.
#[derive(Clone, Debug)]
pub struct PlacementProgram {
    pub configured_name: Option<String>,
    pub configured: Value,
    modifiers: Vec<Modifier>,
}

impl PlacementProgram {
    pub fn parse(config: &Value) -> FeatureResult<Self> {
        let (configured_name, configured) = if let Some(name) = config["feature"].as_str() {
            (Some(name.to_owned()), catalog().configured(name)?.clone())
        } else {
            kind(&config["feature"])?;
            (None, config["feature"].clone())
        };
        let modifiers = array(&config["placement"])?
            .iter()
            .map(Modifier::parse)
            .collect::<FeatureResult<_>>()?;
        Ok(Self {
            configured_name,
            configured,
            modifiers,
        })
    }

    /// `top_feature=None` has native `PlacedFeature.place` semantics: encountering
    /// a biome filter is an error. Nested selectors deliberately use this mode.
    pub fn place(
        &self,
        top_feature: Option<&str>,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        origin: Pos,
        environment: &dyn FeatureEnvironment,
        dispatcher: &mut dyn ConfiguredFeatureDispatcher,
    ) -> FeatureResult<bool> {
        Execution {
            program: self,
            top_feature,
            environment,
            dispatcher,
        }
        .walk(world, random, origin, 0)
    }
}

pub fn place(
    config: &Value,
    name: Option<&str>,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    PlacementProgram::parse(config)?.place(name, world, random, origin, environment, dispatcher)
}

pub fn place_named(
    name: &str,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    place(
        catalog().placed(name)?,
        Some(name),
        world,
        random,
        origin,
        environment,
        dispatcher,
    )
}

/// Named or inline nested placed reference, without a top-level biome context.
pub fn place_nested(
    reference: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let config = if let Some(name) = reference.as_str() {
        catalog().placed(name)?
    } else {
        reference
    };
    place(config, None, world, random, origin, environment, dispatcher)
}

struct Execution<'a> {
    program: &'a PlacementProgram,
    top_feature: Option<&'a str>,
    environment: &'a dyn FeatureEnvironment,
    dispatcher: &'a mut dyn ConfiguredFeatureDispatcher,
}

impl Execution<'_> {
    fn repeat(
        &mut self,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        index: usize,
        count: i32,
    ) -> FeatureResult<bool> {
        let mut placed = false;
        for _ in 0..count {
            placed |= self.walk(world, random, pos, index + 1)?;
        }
        Ok(placed)
    }

    fn walk(
        &mut self,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        index: usize,
    ) -> FeatureResult<bool> {
        let Some(modifier) = self.program.modifiers.get(index) else {
            if !world.can_write_feature(pos) {
                return Ok(false);
            }
            return self.dispatcher.place_configured(
                &self.program.configured,
                self.program.configured_name.as_deref(),
                world,
                random,
                pos,
                self.environment,
            );
        };
        let next = match modifier {
            Modifier::Count(count) => {
                let count = count.sample_with(random, self.dispatcher)?;
                return self.repeat(world, random, pos, index, count);
            }
            Modifier::Rarity(chance) => {
                (random.next_float() < 1.0_f32 / *chance as f32).then_some(pos)
            }
            Modifier::InSquare => Some(offset(
                pos,
                (random.next_int(16) as i32, 0, random.next_int(16) as i32),
            )),
            Modifier::Height(height) => Some((
                pos.0,
                height.sample(random, self.environment.generation_bounds())?,
                pos.2,
            )),
            Modifier::Heightmap(kind) => {
                let y = world.feature_height(*kind, pos.0, pos.2);
                (y > MIN_Y).then_some((pos.0, y, pos.2))
            }
            Modifier::Biome => {
                let name = self
                    .top_feature
                    .ok_or_else(|| invalid("biome filter has no top-level placed feature"))?;
                self.environment
                    .biome_has_feature(world.feature_biome(pos), name)?
                    .then_some(pos)
            }
            Modifier::Predicate(predicate) => {
                predicate.test(world, pos, self.environment)?.then_some(pos)
            }
            Modifier::Offset(xz, y) => Some(offset(
                pos,
                (
                    xz.sample_with(random, self.dispatcher)?,
                    y.sample_with(random, self.dispatcher)?,
                    xz.sample_with(random, self.dispatcher)?,
                ),
            )),
            Modifier::Scan(direction, target, allowed, steps) => scan(
                world,
                pos,
                *direction,
                target,
                allowed,
                *steps,
                self.environment,
            )?,
            Modifier::Surface(kind, min, max) => {
                let surface = i64::from(world.feature_height(*kind, pos.0, pos.2));
                (surface + i64::from(*min) <= i64::from(pos.1)
                    && i64::from(pos.1) <= surface + i64::from(*max))
                .then_some(pos)
            }
            Modifier::WaterDepth(max) => {
                let floor = world.feature_height(FeatureHeightmap::OceanFloor, pos.0, pos.2);
                let surface = world.feature_height(FeatureHeightmap::WorldSurface, pos.0, pos.2);
                (surface.wrapping_sub(floor) <= *max).then_some(pos)
            }
            Modifier::NoiseCount(ratio, factor, add) => {
                let count = ((biome_info_noise(pos.0 as f64 / factor, pos.2 as f64 / factor) + add)
                    * *ratio as f64)
                    .ceil() as i32;
                return self.repeat(world, random, pos, index, count);
            }
            Modifier::NoiseThreshold(level, below, above) => {
                let count = if biome_info_noise(pos.0 as f64 / 200.0, pos.2 as f64 / 200.0) < *level
                {
                    *below
                } else {
                    *above
                };
                return self.repeat(world, random, pos, index, count);
            }
            Modifier::EveryLayer(count) => {
                let positions = every_layer(world, random, pos, count, self.dispatcher)?;
                let mut placed = false;
                for next in positions {
                    placed |= self.walk(world, random, next, index + 1)?;
                }
                return Ok(placed);
            }
            Modifier::Fixed(positions) => {
                let mut placed = false;
                for &next in positions {
                    if next.0 >> 4 == pos.0 >> 4 && next.2 >> 4 == pos.2 >> 4 {
                        placed |= self.walk(world, random, next, index + 1)?;
                    }
                }
                return Ok(placed);
            }
        };
        match next {
            Some(next) => self.walk(world, random, next, index + 1),
            None => Ok(false),
        }
    }
}

fn scan(
    world: &dyn FeatureWorld,
    mut pos: Pos,
    direction: Direction,
    target: &BlockPredicate,
    allowed: &BlockPredicate,
    steps: i32,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<Option<Pos>> {
    if !allowed.test(world, pos, environment)? {
        return Ok(None);
    }
    for _ in 0..steps {
        if target.test(world, pos, environment)? {
            return Ok(Some(pos));
        }
        pos = direction.step(pos);
        if !(MIN_Y..=MAX_Y).contains(&pos.1) {
            return Ok(None);
        }
        if !allowed.test(world, pos, environment)? {
            break;
        }
    }
    Ok(target.test(world, pos, environment)?.then_some(pos))
}

fn every_layer(
    world: &dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    count: &IntProvider,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<Vec<Pos>> {
    let mut positions = Vec::new();
    let mut layer = 0;
    loop {
        let mut found = false;
        let mut attempt = 0;
        // The native for-loop samples its condition on EVERY iteration,
        // including the final failed comparison; it does not cache count.
        while attempt < count.sample_with(random, dispatcher)? {
            let x = origin.0.wrapping_add(random.next_int(16) as i32);
            let z = origin.2.wrapping_add(random.next_int(16) as i32);
            let height = world.feature_height(FeatureHeightmap::MotionBlocking, x, z);
            if let Some(y) = ground_layer(world, x, height, z, layer)? {
                positions.push((x, y, z));
                found = true;
            }
            attempt += 1;
        }
        if !found {
            return Ok(positions);
        }
        layer += 1;
    }
}

fn ground_layer(
    world: &dyn FeatureWorld,
    x: i32,
    height: i32,
    z: i32,
    target: i32,
) -> FeatureResult<Option<i32>> {
    let empty = |state| -> FeatureResult<bool> {
        Ok(catalog().info(state)?.is_air()
            || catalog().is_block(state, "water")?
            || catalog().is_block(state, "lava")?)
    };
    let mut above = read_block(world, (x, height, z))?;
    let mut layer = 0;
    for y in ((MIN_Y + 1)..=height).rev() {
        let below = read_block(world, (x, y - 1, z))?;
        if !empty(below)? && empty(above)? && !catalog().is_block(below, "bedrock")? {
            if layer == target {
                return Ok(Some(y));
            }
            layer += 1;
        }
        above = below;
    }
    Ok(None)
}

pub fn heightmap(value: &Value) -> FeatureResult<FeatureHeightmap> {
    match string(value)? {
        "WORLD_SURFACE_WG" => Ok(FeatureHeightmap::WorldSurfaceWg),
        "WORLD_SURFACE" => Ok(FeatureHeightmap::WorldSurface),
        "OCEAN_FLOOR_WG" => Ok(FeatureHeightmap::OceanFloorWg),
        "OCEAN_FLOOR" => Ok(FeatureHeightmap::OceanFloor),
        "MOTION_BLOCKING" => Ok(FeatureHeightmap::MotionBlocking),
        "MOTION_BLOCKING_NO_LEAVES" => Ok(FeatureHeightmap::MotionBlockingNoLeaves),
        other => Err(invalid(format!("heightmap {other}"))),
    }
}

/// Biome.BIOME_INFO_NOISE: the unshifted 2-D simplex octave seeded with 2345.
pub fn biome_info_noise(x: f64, z: f64) -> f64 {
    static NOISE: OnceLock<SimplexNoise> = OnceLock::new();
    let p = &NOISE.get_or_init(|| SimplexNoise::new(2345)).p;
    let sqrt3 = 3.0_f64.sqrt();
    let f = 0.5 * (sqrt3 - 1.0);
    let g = (3.0 - sqrt3) / 6.0;
    let skew = (x + z) * f;
    let i = (x + skew).floor() as i32;
    let j = (z + skew).floor() as i32;
    let unskew = i.wrapping_add(j) as f64 * g;
    let dx = x - (i as f64 - unskew);
    let dz = z - (j as f64 - unskew);
    let (ix, iz) = if dx > dz { (1, 0) } else { (0, 1) };
    let gradients = [
        [1., 1.],
        [-1., 1.],
        [1., -1.],
        [-1., -1.],
        [1., 0.],
        [-1., 0.],
        [1., 0.],
        [-1., 0.],
        [0., 1.],
        [0., -1.],
        [0., 1.],
        [0., -1.],
    ];
    let corner = |a: f64, b: f64, di: i32, dj: i32| {
        let t = 0.5 - a * a - b * b;
        if t < 0.0 {
            return 0.0;
        }
        let hash = p[((i.wrapping_add(di) & 255) as usize
            + p[(j.wrapping_add(dj) & 255) as usize] as usize)
            & 255] as usize
            % 12;
        let t2 = t * t;
        t2 * t2 * (gradients[hash][0] * a + gradients[hash][1] * b)
    };
    70.0 * (corner(dx, dz, 0, 0)
        + corner(dx - ix as f64 + g, dz - iz as f64 + g, ix, iz)
        + corner(dx - 1.0 + 2.0 * g, dz - 1.0 + 2.0 * g, 1, 1))
}
