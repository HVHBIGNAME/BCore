//! Data-driven base configured features from Java 26.1.
//!
//! Patches in this version are placed-feature programs around `simple_block`.
//! `BaseFeatures` composes with an explicit dispatcher for trees, ores and other
//! configured types. It never substitutes an approximate feature.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use crate::block_predicate::{
    array, catalog, integer, invalid, is_air, kind, number, offset, read_block, string,
    would_survive, BlockPredicate, Direction, FeatureEnvironment, FeatureResult,
};
use crate::feature_world::{FeatureHeightmap, FeatureWorld, Pos};
use crate::noise_perlin::{java_string_hash, lerp, smoothstep, wrap};
use crate::placement::{self, ConfiguredFeatureDispatcher, IntProvider, RejectUnsupported};
use crate::simplex::{JavaRandom, SimplexNoise, WorldgenRandom};
use crate::tick_request::{TickRequest, TickTarget};
use crate::MIN_Y;

pub struct BaseFeatures<D = RejectUnsupported> {
    pub additional: D,
}

impl Default for BaseFeatures {
    fn default() -> Self {
        Self {
            additional: RejectUnsupported,
        }
    }
}

impl<D> BaseFeatures<D> {
    pub fn with_dispatcher(additional: D) -> Self {
        Self { additional }
    }
}

impl<D: ConfiguredFeatureDispatcher> ConfiguredFeatureDispatcher for BaseFeatures<D> {
    fn next_gaussian(&mut self, random: &mut WorldgenRandom) -> FeatureResult<f64> {
        self.additional.next_gaussian(random)
    }

    fn next_world_bool(&mut self, world: &dyn FeatureWorld) -> FeatureResult<bool> {
        self.additional.next_world_bool(world)
    }

    fn place_configured(
        &mut self,
        document: &Value,
        name: Option<&str>,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        origin: Pos,
        environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        let feature_type = kind(document)?;
        let config = &document["config"];
        if !config.is_object() {
            return Err(invalid("configured feature requires config object"));
        }
        if crate::misc_features::supports_configured(document)? {
            return crate::misc_features::place_configured_with(
                document,
                world,
                random,
                origin,
                environment,
                self,
            );
        }
        if !world.can_write_feature(origin) {
            return Ok(false);
        }
        match feature_type {
            "simple_block" => simple_block(config, world, random, origin, environment, self),
            "block_column" => block_column(config, world, random, origin, environment, self),
            "disk" => disk(config, world, random, origin, environment, self),
            "spring_feature" => spring(config, world, origin),
            "lake" => lake(config, world, random, origin, environment, self),
            "seagrass" => seagrass(config, world, random, origin, environment),
            "random_boolean_selector" => {
                let selected = if random.next_bool() {
                    "feature_true"
                } else {
                    "feature_false"
                };
                placement::place_nested(&config[selected], world, random, origin, environment, self)
            }
            "simple_random_selector" => {
                let features = array(&config["features"])?;
                if features.is_empty() {
                    return Err(invalid("empty simple selector"));
                }
                let selected = random.next_int(features.len());
                placement::place_nested(
                    &features[selected],
                    world,
                    random,
                    origin,
                    environment,
                    self,
                )
            }
            "random_selector" => {
                for feature in array(&config["features"])? {
                    if random.next_float() < probability(feature, "chance")? {
                        return placement::place_nested(
                            &feature["feature"],
                            world,
                            random,
                            origin,
                            environment,
                            self,
                        );
                    }
                }
                placement::place_nested(
                    &config["default"],
                    world,
                    random,
                    origin,
                    environment,
                    self,
                )
            }
            _ => {
                self.additional
                    .place_configured(document, name, world, random, origin, environment)
            }
        }
    }
}

pub fn place(
    config: &Value,
    name: Option<&str>,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    BaseFeatures::default().place_configured(config, name, world, random, origin, environment)
}

pub fn place_named(
    name: &str,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    place(
        catalog().configured(name)?,
        Some(name),
        world,
        random,
        origin,
        environment,
    )
}

#[derive(Clone)]
pub enum StateProvider {
    Simple(u32),
    Weighted(Vec<(u32, i32)>, i32),
    Rotated(u32),
    Randomized(Box<Self>, String, IntProvider),
    RuleBased(Option<Box<Self>>, Vec<(BlockPredicate, Self)>),
    Noise(Arc<ProviderNoise>, f32, Vec<u32>),
    Threshold(Arc<ProviderNoise>, f32, f32, f32, u32, Vec<u32>, Vec<u32>),
    Dual(
        Arc<ProviderNoise>,
        f32,
        Arc<ProviderNoise>,
        f32,
        (i32, i32),
        Vec<u32>,
    ),
}

impl StateProvider {
    pub fn parse(value: &Value) -> FeatureResult<Self> {
        Ok(match kind(value)? {
            "simple_state_provider" => Self::Simple(catalog().state(&value["state"])?),
            "weighted_state_provider" => {
                let mut total = 0_i32;
                let mut entries = Vec::new();
                for entry in array(&value["entries"])? {
                    let weight = integer(entry, "weight")?;
                    if weight < 0 {
                        return Err(invalid("negative state weight"));
                    }
                    total = total
                        .checked_add(weight)
                        .ok_or_else(|| invalid("state weights overflow"))?;
                    entries.push((catalog().state(&entry["data"])?, weight));
                }
                if total == 0 {
                    return Err(invalid("state provider has no positive weight"));
                }
                Self::Weighted(entries, total)
            }
            "rotated_block_provider" => {
                // Native keeps the block, not properties from the supplied state.
                let state = catalog().state(&value["state"])?;
                let block = catalog().block(state)?.1;
                block.with_property(block.default_state, "axis", "x")?;
                Self::Rotated(block.default_state)
            }
            "randomized_int_state_provider" => Self::Randomized(
                Box::new(Self::parse(&value["source"])?),
                string(&value["property"])?.to_owned(),
                IntProvider::parse(&value["values"])?,
            ),
            "rule_based_state_provider" => Self::RuleBased(
                value
                    .get("fallback")
                    .map(Self::parse)
                    .transpose()?
                    .map(Box::new),
                array(&value["rules"])?
                    .iter()
                    .map(|rule| {
                        Ok((
                            BlockPredicate::parse(&rule["if_true"])?,
                            Self::parse(&rule["then"])?,
                        ))
                    })
                    .collect::<FeatureResult<_>>()?,
            ),
            "noise_provider" => Self::Noise(
                ProviderNoise::parse(value, "noise")?,
                positive_scale(value, "scale")?,
                states(&value["states"])?,
            ),
            "noise_threshold_provider" => Self::Threshold(
                ProviderNoise::parse(value, "noise")?,
                positive_scale(value, "scale")?,
                number(value, "threshold")? as f32,
                probability(value, "high_chance")?,
                catalog().state(&value["default_state"])?,
                states(&value["low_states"])?,
                states(&value["high_states"])?,
            ),
            "dual_noise_provider" => {
                let range = array(&value["variety"])?;
                if range.len() != 2 {
                    return Err(invalid("dual noise variety"));
                }
                let min = crate::block_predicate::int(&range[0])?;
                let max = crate::block_predicate::int(&range[1])?;
                if min < 1 || max < min || max > 64 {
                    return Err(invalid("dual noise variety outside 1..=64"));
                }
                Self::Dual(
                    ProviderNoise::parse(value, "noise")?,
                    positive_scale(value, "scale")?,
                    ProviderNoise::parse(value, "slow_noise")?,
                    positive_scale(value, "slow_scale")?,
                    (min, max),
                    states(&value["states"])?,
                )
            }
            other => {
                return Err(crate::feature_world::FeatureError::Unsupported(format!(
                    "state provider {other}"
                )))
            }
        })
    }

    pub fn sample(
        &self,
        world: &dyn FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<u32> {
        self.sample_with(world, random, pos, environment, &mut RejectUnsupported)
    }

    pub fn sample_with(
        &self,
        world: &dyn FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        environment: &dyn FeatureEnvironment,
        dispatcher: &mut dyn ConfiguredFeatureDispatcher,
    ) -> FeatureResult<u32> {
        match self.sample_optional_with(world, random, pos, environment, dispatcher)? {
            Some(state) => Ok(state),
            None => read_block(world, pos),
        }
    }

    pub fn sample_optional(
        &self,
        world: &dyn FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<Option<u32>> {
        self.sample_optional_with(world, random, pos, environment, &mut RejectUnsupported)
    }

    pub fn sample_optional_with(
        &self,
        world: &dyn FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        environment: &dyn FeatureEnvironment,
        dispatcher: &mut dyn ConfiguredFeatureDispatcher,
    ) -> FeatureResult<Option<u32>> {
        Ok(Some(match self {
            Self::Simple(state) => *state,
            Self::Weighted(entries, total) => {
                let mut choice = random.next_int(*total as usize) as i32;
                let mut selected = None;
                for (state, weight) in entries {
                    choice -= weight;
                    if choice < 0 {
                        selected = Some(*state);
                        break;
                    }
                }
                selected.expect("validated positive weights")
            }
            Self::Rotated(state) => catalog().block(*state)?.1.with_property(
                *state,
                "axis",
                ["x", "y", "z"][random.next_int(3)],
            )?,
            Self::Randomized(source, property, values) => {
                let state = source.sample_with(world, random, pos, environment, dispatcher)?;
                let block = catalog().block(state)?.1;
                if block.property(state, property).is_none() {
                    return Err(invalid(format!("randomized property {property}")));
                }
                block.with_property(
                    state,
                    property,
                    &values.sample_with(random, dispatcher)?.to_string(),
                )?
            }
            Self::RuleBased(fallback, rules) => {
                for (predicate, provider) in rules {
                    if predicate.test(world, pos, environment)? {
                        return Ok(Some(provider.sample_with(
                            world,
                            random,
                            pos,
                            environment,
                            dispatcher,
                        )?));
                    }
                }
                return fallback
                    .as_ref()
                    .map(|provider| {
                        provider.sample_with(world, random, pos, environment, dispatcher)
                    })
                    .transpose();
            }
            Self::Noise(noise, scale, states) => pick_noise(states, noise.at(pos, *scale)),
            Self::Threshold(noise, scale, threshold, chance, default, low, high) => {
                if noise.at(pos, *scale) < *threshold as f64 {
                    low[random.next_int(low.len())]
                } else if random.next_float() < *chance {
                    high[random.next_int(high.len())]
                } else {
                    *default
                }
            }
            Self::Dual(noise, scale, slow, slow_scale, (min, max), states) => {
                let slow_value = |p: Pos| {
                    slow.value(
                        (p.0 as f32 * slow_scale) as f64,
                        (p.1 as f32 * slow_scale) as f64,
                        (p.2 as f32 * slow_scale) as f64,
                    )
                };
                let factor = ((slow_value(pos) + 1.0) / 2.0).clamp(0.0, 1.0);
                let count = lerp(factor, *min as f64, (max + 1) as f64) as i32;
                let local: Vec<_> = (0..count)
                    .map(|i| {
                        pick_noise(
                            states,
                            slow_value(offset(
                                pos,
                                (i.wrapping_mul(54_545), 0, i.wrapping_mul(34_234)),
                            )),
                        )
                    })
                    .collect();
                pick_noise(&local, noise.at(pos, *scale))
            }
        }))
    }
}

fn states(value: &Value) -> FeatureResult<Vec<u32>> {
    let states = array(value)?
        .iter()
        .map(|state| catalog().state(state))
        .collect::<FeatureResult<Vec<_>>>()?;
    if states.is_empty() {
        return Err(invalid("empty provider state list"));
    }
    Ok(states)
}

fn probability(value: &Value, key: &str) -> FeatureResult<f32> {
    let n = number(value, key)? as f32;
    if !(0.0..=1.0).contains(&n) {
        return Err(invalid(format!("{key} outside 0..=1")));
    }
    Ok(n)
}

fn positive_scale(value: &Value, key: &str) -> FeatureResult<f32> {
    let n = number(value, key)? as f32;
    if !n.is_finite() || n <= 0.0 {
        return Err(invalid(format!("{key} must be positive")));
    }
    Ok(n)
}

fn pick_noise(states: &[u32], noise: f64) -> u32 {
    states[(((1.0 + noise) / 2.0).clamp(0.0, 0.9999) * states.len() as f64) as usize]
}

/// Native NormalNoise with LegacyPositionalRandomFactory, used by plant providers.
/// This independent seed path never consumes the decoration random stream.
pub struct ProviderNoise {
    first: i32,
    amplitudes: Vec<f64>,
    levels: [Vec<Option<SimplexNoise>>; 2],
    value_factor: f64,
}

impl ProviderNoise {
    fn parse(value: &Value, parameters: &str) -> FeatureResult<Arc<Self>> {
        type NoiseCache = Mutex<BTreeMap<String, Arc<ProviderNoise>>>;
        static CACHE: OnceLock<NoiseCache> = OnceLock::new();
        let seed = value["seed"]
            .as_i64()
            .ok_or_else(|| invalid("noise provider seed"))?;
        let parameters = &value[parameters];
        let key = format!("{seed}:{parameters}");
        let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
        if let Some(noise) = cache.lock().expect("provider cache").get(&key).cloned() {
            return Ok(noise);
        }
        let first = integer(parameters, "firstOctave")?;
        let amplitudes: Vec<_> = array(&parameters["amplitudes"])?
            .iter()
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| invalid("noise amplitude"))
            })
            .collect::<FeatureResult<_>>()?;
        if amplitudes.is_empty() || amplitudes.len() > 1024 || !(-1022..=1023).contains(&first) {
            return Err(invalid("noise octave bounds"));
        }
        let mut random = JavaRandom::new(seed);
        let mut levels = [Vec::new(), Vec::new()];
        for part in &mut levels {
            let positional_seed = random.next_long();
            for (i, amplitude) in amplitudes.iter().enumerate() {
                // ImprovedNoise and SimplexNoise have identical legacy seed
                // construction (three offsets followed by the permutation).
                part.push(if *amplitude == 0.0 {
                    None
                } else {
                    let hash = java_string_hash(&format!("octave_{}", first + i as i32));
                    Some(SimplexNoise::new(positional_seed ^ hash as i64))
                });
            }
        }
        let nonzero: Vec<_> = amplitudes
            .iter()
            .enumerate()
            .filter(|(_, a)| **a != 0.0)
            .map(|(i, _)| i)
            .collect();
        let span = nonzero.last().copied().unwrap_or(0) - nonzero.first().copied().unwrap_or(0);
        let value_factor = (1.0 / 6.0) / (0.1 * (1.0 + 1.0 / (span as f64 + 1.0)));
        let noise = Arc::new(Self {
            first,
            amplitudes,
            levels,
            value_factor,
        });
        cache
            .lock()
            .expect("provider cache")
            .insert(key, noise.clone());
        Ok(noise)
    }

    fn at(&self, pos: Pos, scale: f32) -> f64 {
        self.value(
            pos.0 as f64 * scale as f64,
            pos.1 as f64 * scale as f64,
            pos.2 as f64 * scale as f64,
        )
    }

    pub fn value(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut parts = [0.0; 2];
        for (part, output) in parts.iter_mut().enumerate() {
            let scale = if part == 0 { 1.0 } else { 1.0181268882175227 };
            let (x, y, z) = (x * scale, y * scale, z * scale);
            let mut frequency = 2.0_f64.powi(self.first);
            let n = self.amplitudes.len() as i32;
            let mut amplitude = 2.0_f64.powi(n - 1) / (2.0_f64.powi(n) - 1.0);
            for (index, noise) in self.levels[part].iter().enumerate() {
                if let Some(noise) = noise {
                    *output += self.amplitudes[index]
                        * improved(
                            noise,
                            wrap(x * frequency),
                            wrap(y * frequency),
                            wrap(z * frequency),
                        )
                        * amplitude;
                }
                frequency *= 2.0;
                amplitude /= 2.0;
            }
        }
        (parts[0] + parts[1]) * self.value_factor
    }
}

fn improved(noise: &SimplexNoise, x: f64, y: f64, z: f64) -> f64 {
    let (x, y, z) = (x + noise.xo, y + noise.yo, z + noise.zo);
    let (ix, iy, iz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let (x, y, z) = (x - ix as f64, y - iy as f64, z - iz as f64);
    let p = |n: i32| noise.p[(n & 255) as usize] as i32;
    let gradients = [
        [1., 1., 0.],
        [-1., 1., 0.],
        [1., -1., 0.],
        [-1., -1., 0.],
        [1., 0., 1.],
        [-1., 0., 1.],
        [1., 0., -1.],
        [-1., 0., -1.],
        [0., 1., 1.],
        [0., -1., 1.],
        [0., 1., -1.],
        [0., -1., -1.],
        [1., 1., 0.],
        [0., -1., 1.],
        [-1., 1., 0.],
        [0., -1., -1.],
    ];
    let corner = |dx, dy, dz| {
        let hash = p(p(p(ix + dx) + iy + dy) + iz + dz) as usize & 15;
        let g = gradients[hash];
        g[0] * (x - dx as f64) + g[1] * (y - dy as f64) + g[2] * (z - dz as f64)
    };
    let (fx, fy, fz) = (smoothstep(x), smoothstep(y), smoothstep(z));
    lerp(
        fz,
        lerp(
            fy,
            lerp(fx, corner(0, 0, 0), corner(1, 0, 0)),
            lerp(fx, corner(0, 1, 0), corner(1, 1, 0)),
        ),
        lerp(
            fy,
            lerp(fx, corner(0, 0, 1), corner(1, 0, 1)),
            lerp(fx, corner(0, 1, 1), corner(1, 1, 1)),
        ),
    )
}

fn simple_block(
    config: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let provider = StateProvider::parse(&config["to_place"])?;
    let Some(state) =
        provider.sample_optional_with(world, random, origin, environment, dispatcher)?
    else {
        return Ok(false);
    };
    if !would_survive(world, state, origin, environment)? {
        return Ok(false);
    }
    let block = catalog().block(state)?.1;
    if block.double_plant {
        let above = Direction::Up.step(origin);
        if !is_air(world, above)? {
            return Ok(false);
        }
        let lower = copy_waterlogged(world, origin, block.with_property(state, "half", "lower")?)?;
        world.set_feature_block(origin, lower, 2);
        let upper = copy_waterlogged(world, above, block.with_property(state, "half", "upper")?)?;
        world.set_feature_block(above, upper, 2);
    } else if block.class == "MossyCarpetBlock" {
        moss_carpet(world, origin, dispatcher)?;
    } else {
        world.set_feature_block(origin, state, 2);
    }
    if config
        .get("schedule_tick")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let current = read_block(world, origin)?;
        request_tick(
            world,
            origin,
            TickTarget::Block(catalog().block(current)?.1.default_state),
            1,
        );
    }
    Ok(true)
}

fn moss_carpet(
    world: &mut dyn FeatureWorld,
    origin: Pos,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<()> {
    let block = catalog().definition("pale_moss_carpet")?;
    let base = updated_moss(world, origin, block.default_state)?;
    world.set_feature_block(origin, base, 2);
    let above = Direction::Up.step(origin);
    let old = read_block(world, above)?;
    let same_block = catalog().is_block(old, "pale_moss_carpet")?;
    if (same_block && block.property(old, "bottom") == Some("true"))
        || (!same_block && !catalog().info(old)?.replaceable())
    {
        return Ok(());
    }
    let mut topper = updated_moss(
        world,
        above,
        block.with_property(block.default_state, "bottom", "false")?,
    )?;
    let mut has_face = false;
    for (_, property) in moss_sides() {
        if block.property(topper, property) == Some("none") {
            continue;
        }
        if !dispatcher.next_world_bool(world)? {
            topper = block.with_property(topper, property, "none")?;
        } else {
            has_face = true;
        }
    }
    if has_face && topper != old {
        world.set_feature_block(above, topper, 2);
        let state = updated_moss(world, origin, base)?;
        world.set_feature_block(origin, state, 2);
    }
    Ok(())
}

fn moss_sides() -> [(Direction, &'static str); 4] {
    [
        (Direction::North, "north"),
        (Direction::East, "east"),
        (Direction::South, "south"),
        (Direction::West, "west"),
    ]
}

fn updated_moss(world: &dyn FeatureWorld, pos: Pos, mut state: u32) -> FeatureResult<u32> {
    let data = catalog();
    let block = data.definition("pale_moss_carpet")?;
    let bottom = block.property(state, "bottom") == Some("true");
    let mut above = None;
    let mut below = None;
    for (direction, property) in moss_sides() {
        let neighbor = read_block(world, direction.step(pos))?;
        let mut side = if data.info(neighbor)?.can_attach_from(direction) {
            "low"
        } else {
            "none"
        };
        if side == "low" {
            let above = match above {
                Some(value) => value,
                None => {
                    let v = read_block(world, Direction::Up.step(pos))?;
                    above = Some(v);
                    v
                }
            };
            if data.is_block(above, "pale_moss_carpet")?
                && block.property(above, property) != Some("none")
                && block.property(above, "bottom") == Some("false")
            {
                side = "tall";
            }
            if !bottom {
                let below = match below {
                    Some(value) => value,
                    None => {
                        let v = read_block(world, Direction::Down.step(pos))?;
                        below = Some(v);
                        v
                    }
                };
                if data.is_block(below, "pale_moss_carpet")?
                    && block.property(below, property) == Some("none")
                {
                    side = "none";
                }
            }
        }
        state = block.with_property(state, property, side)?;
    }
    Ok(state)
}

fn copy_waterlogged(world: &dyn FeatureWorld, pos: Pos, state: u32) -> FeatureResult<u32> {
    let block = catalog().block(state)?.1;
    if block.property(state, "waterlogged").is_none() {
        return Ok(state);
    }
    let wet = catalog().in_fluid_tag(read_block(world, pos)?, "water")?;
    block.with_property(state, "waterlogged", if wet { "true" } else { "false" })
}

fn block_column(
    config: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let layers = array(&config["layers"])?
        .iter()
        .map(|layer| {
            Ok((
                IntProvider::parse(&layer["height"])?.require_range(0, i32::MAX)?,
                StateProvider::parse(&layer["provider"])?,
            ))
        })
        .collect::<FeatureResult<Vec<_>>>()?;
    let allowed = BlockPredicate::parse(&config["allowed_placement"])?;
    let direction = Direction::parse(&config["direction"])?;
    let prioritize = config["prioritize_tip"]
        .as_bool()
        .ok_or_else(|| invalid("prioritize_tip"))?;
    let mut heights: Vec<_> = layers
        .iter()
        .map(|(height, _)| height.sample_with(random, dispatcher))
        .collect::<FeatureResult<_>>()?;
    let total = heights
        .iter()
        .try_fold(0_i32, |sum, height| sum.checked_add(*height))
        .ok_or_else(|| invalid("column height overflow"))?;
    if total == 0 {
        return Ok(false);
    }
    let mut pos = direction.step(origin);
    for accepted in 0..total {
        if !allowed.test(world, pos, environment)? {
            let mut remove = total - accepted;
            for j in 0..heights.len() {
                let i = if prioritize { j } else { heights.len() - 1 - j };
                let cut = heights[i].min(remove);
                heights[i] -= cut;
                remove -= cut;
                if remove == 0 {
                    break;
                }
            }
            break;
        }
        pos = direction.step(pos);
    }
    pos = origin;
    for ((_, provider), height) in layers.iter().zip(heights) {
        for _ in 0..height {
            let state = provider.sample_with(world, random, pos, environment, dispatcher)?;
            world.set_feature_block(pos, state, 2);
            pos = direction.step(pos);
        }
    }
    Ok(true)
}

fn disk(
    config: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let radius = IntProvider::parse(&config["radius"])?.require_range(0, 8)?;
    let half = integer(config, "half_height")?;
    if !(0..=4).contains(&half) {
        return Err(invalid("disk half_height outside 0..=4"));
    }
    let target = BlockPredicate::parse(&config["target"])?;
    let provider = StateProvider::parse(&config["state_provider"])?;
    let radius = radius.sample_with(random, dispatcher)?;
    let mut placed = false;
    // BlockPos.betweenClosed increments X before Z for a single Y slice.
    for z in -radius..=radius {
        for x in -radius..=radius {
            if x * x + z * z > radius * radius {
                continue;
            }
            let mut placed_above = false;
            for dy in (-half..=half).rev() {
                let pos = offset(origin, (x, dy, z));
                if target.test(world, pos, environment)? {
                    if let Some(state) = provider.sample_optional_with(
                        world,
                        random,
                        pos,
                        environment,
                        dispatcher,
                    )? {
                        world.set_feature_block(pos, state, 2);
                        if !placed_above {
                            mark_above(world, pos)?;
                        }
                        placed = true;
                        placed_above = true;
                    }
                } else {
                    placed_above = false;
                }
            }
        }
    }
    Ok(placed)
}

fn mark_above(world: &mut dyn FeatureWorld, mut pos: Pos) -> FeatureResult<()> {
    for _ in 0..2 {
        pos = Direction::Up.step(pos);
        if is_air(world, pos)? {
            break;
        }
        world.mark_feature_postprocessing(pos);
    }
    Ok(())
}

fn request_tick(world: &mut dyn FeatureWorld, pos: Pos, target: TickTarget, delay: i32) {
    world.schedule_feature_tick(TickRequest {
        block_pos: [pos.0, pos.1, pos.2],
        target,
        delay,
    });
}

fn spring(config: &Value, world: &mut dyn FeatureWorld, origin: Pos) -> FeatureResult<bool> {
    let valid = catalog().block_holder_set(&config["valid_blocks"])?;
    let (block, fluid) = catalog().fluid_state(&config["state"])?;
    let rock_count = config
        .get("rock_count")
        .map(crate::block_predicate::int)
        .transpose()?
        .unwrap_or(4);
    let hole_count = config
        .get("hole_count")
        .map(crate::block_predicate::int)
        .transpose()?
        .unwrap_or(1);
    let below = config
        .get("requires_block_below")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let matches =
        |state| -> FeatureResult<bool> { Ok(valid.contains(&catalog().block(state)?.1.first)) };
    if !matches(read_block(world, Direction::Up.step(origin))?)? {
        return Ok(false);
    }
    if below && !matches(read_block(world, Direction::Down.step(origin))?)? {
        return Ok(false);
    }
    let current = read_block(world, origin)?;
    if !catalog().info(current)?.is_air() && !matches(current)? {
        return Ok(false);
    }
    let directions = [
        Direction::West,
        Direction::East,
        Direction::North,
        Direction::South,
        Direction::Down,
    ];
    let mut rocks = 0;
    for direction in directions {
        if matches(read_block(world, direction.step(origin))?)? {
            rocks += 1;
        }
    }
    let mut holes = 0;
    for direction in directions {
        if is_air(world, direction.step(origin))? {
            holes += 1;
        }
    }
    if rocks != rock_count || holes != hole_count {
        return Ok(false);
    }
    world.set_feature_block(origin, block, 2);
    request_tick(world, origin, TickTarget::Fluid(fluid), 0);
    Ok(true)
}

fn seagrass(
    config: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    let chance = probability(config, "probability")?;
    let dx = random.next_int(8) as i32 - random.next_int(8) as i32;
    let dz = random.next_int(8) as i32 - random.next_int(8) as i32;
    let (x, _, z) = offset(origin, (dx, 0, dz));
    let pos = (
        x,
        world.feature_height(FeatureHeightmap::OceanFloor, x, z),
        z,
    );
    if !catalog().is_block(read_block(world, pos)?, "water")? {
        return Ok(false);
    }
    let tall = random.next_double() < chance as f64;
    let state = catalog().default_state(if tall { "tall_seagrass" } else { "seagrass" })?;
    if !would_survive(world, state, pos, environment)? {
        return Ok(false);
    }
    if tall {
        let above = Direction::Up.step(pos);
        if catalog().is_block(read_block(world, above)?, "water")? {
            let upper = catalog()
                .block(state)?
                .1
                .with_property(state, "half", "upper")?;
            world.set_feature_block(pos, state, 2);
            world.set_feature_block(above, upper, 2);
        }
    } else {
        world.set_feature_block(pos, state, 2);
    }
    // Native returns true even if a tall plant could not place its upper half.
    Ok(true)
}

fn lake(
    config: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    let fluid_provider = StateProvider::parse(&config["fluid"])?;
    let barrier_provider = StateProvider::parse(&config["barrier"])?;
    if origin.1 <= MIN_Y + 4 {
        return Ok(false);
    }
    let origin = offset(origin, (-8, -4, -8));
    let mut shape = [false; 2048];
    let ellipsoids = random.next_int(4) + 4;
    for _ in 0..ellipsoids {
        lake_ellipsoid(&mut shape, random);
    }
    let fluid = fluid_provider.sample_with(world, random, origin, environment, dispatcher)?;
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..8 {
                if !lake_boundary(&shape, x, y, z) {
                    continue;
                }
                let state = read_block(world, offset(origin, (x, y, z)))?;
                let info = catalog().info(state)?;
                if (y >= 4 && info.liquid()) || (y < 4 && !info.is_solid() && state != fluid) {
                    return Ok(false);
                }
            }
        }
    }
    let air = catalog().default_state("cave_air")?;
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..8 {
                if !shape[lake_index(x, y, z)] {
                    continue;
                }
                let pos = offset(origin, (x, y, z));
                if catalog().in_block_tag(read_block(world, pos)?, "features_cannot_replace")? {
                    continue;
                }
                world.set_feature_block(pos, if y >= 4 { air } else { fluid }, 2);
                if y >= 4 {
                    request_tick(world, pos, TickTarget::Block(air), 0);
                    mark_above(world, pos)?;
                }
            }
        }
    }
    let barrier = barrier_provider.sample_with(world, random, origin, environment, dispatcher)?;
    if !catalog().info(barrier)?.is_air() {
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..8 {
                    if !lake_boundary(&shape, x, y, z) || (y >= 4 && random.next_int(2) == 0) {
                        continue;
                    }
                    let pos = offset(origin, (x, y, z));
                    let state = read_block(world, pos)?;
                    if catalog().info(state)?.is_solid()
                        && !catalog().in_block_tag(state, "lava_pool_stone_cannot_replace")?
                    {
                        world.set_feature_block(pos, barrier, 2);
                        mark_above(world, pos)?;
                    }
                }
            }
        }
    }
    if catalog().in_fluid_tag(fluid, "water")? {
        for x in 0..16 {
            for z in 0..16 {
                let pos = offset(origin, (x, 4, z));
                if environment.should_freeze(world, pos)?
                    && !catalog()
                        .in_block_tag(read_block(world, pos)?, "features_cannot_replace")?
                {
                    world.set_feature_block(pos, catalog().default_state("ice")?, 2);
                }
            }
        }
    }
    Ok(true)
}

fn lake_ellipsoid(shape: &mut [bool; 2048], random: &mut WorldgenRandom) {
    let a = random.next_double() * 6.0 + 3.0;
    let b = random.next_double() * 4.0 + 2.0;
    let c = random.next_double() * 6.0 + 3.0;
    let x0 = random.next_double() * (16.0 - a - 2.0) + 1.0 + a / 2.0;
    let y0 = random.next_double() * (8.0 - b - 4.0) + 2.0 + b / 2.0;
    let z0 = random.next_double() * (16.0 - c - 2.0) + 1.0 + c / 2.0;
    for x in 1..15 {
        for z in 1..15 {
            for y in 1..7 {
                let dx = (x as f64 - x0) / (a / 2.0);
                let dy = (y as f64 - y0) / (b / 2.0);
                let dz = (z as f64 - z0) / (c / 2.0);
                if dx * dx + dy * dy + dz * dz < 1.0 {
                    shape[lake_index(x, y, z)] = true;
                }
            }
        }
    }
}

fn lake_index(x: i32, y: i32, z: i32) -> usize {
    ((x * 16 + z) * 8 + y) as usize
}

fn lake_boundary(shape: &[bool; 2048], x: i32, y: i32, z: i32) -> bool {
    !shape[lake_index(x, y, z)]
        && ((x < 15 && shape[lake_index(x + 1, y, z)])
            || (x > 0 && shape[lake_index(x - 1, y, z)])
            || (z < 15 && shape[lake_index(x, y, z + 1)])
            || (z > 0 && shape[lake_index(x, y, z - 1)])
            || (y < 7 && shape[lake_index(x, y + 1, z)])
            || (y > 0 && shape[lake_index(x, y - 1, z)]))
}
