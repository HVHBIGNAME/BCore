//! Minecraft 26.1 `GeodeFeature`, including its independent world-seeded noise.
//!
//! Geometry and probabilities use doubles; random floats are widened before
//! comparison, as in the pinned JAR. The caller retains the decoration random
//! (and, with [`place_configured_with`], its Gaussian cache). Reads, writes and
//! deferred fluid ticks go through the caller's guarded [`FeatureWorld`].
//! Implicit write postprocessing belongs to that world, not to this kernel.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::base_features::{ProviderNoise, StateProvider};
use crate::block_predicate::{
    array, catalog, integer, invalid, offset, string, Direction, FeatureEnvironment, FeatureResult,
};
use crate::feature_world::{FeatureError, FeatureWorld, Pos};
use crate::placement::{ConfiguredFeatureDispatcher, IntProvider, RejectUnsupported};
use crate::simplex::WorldgenRandom;
use crate::tick_request::{TickRequest, TickTarget};

/// Kernel capability; use [`check_configured`] to validate the actual document.
pub fn supports(feature_type: &str) -> bool {
    matches!(feature_type, "geode" | "minecraft:geode")
}

/// Validate configuration, tags and providers without consuming randomness.
pub fn check_configured(feature_type: &str, config: &Value) -> FeatureResult<()> {
    require_type(feature_type)?;
    Config::parse(config).map(|_| ())
}

/// Place a configured geode with the caller's current RNG state. `world_seed`
/// must be the level seed, not the decoration/feature seed.
pub fn place_configured(
    feature_type: &str,
    config: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    world_seed: i64,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    place_configured_with(
        feature_type,
        config,
        world,
        random,
        origin,
        world_seed,
        environment,
        &mut RejectUnsupported,
    )
}

/// As [`place_configured`], with the existing provider sampling context. This
/// allows custom normal IntProviders to share the caller's exact Gaussian cache.
pub fn place_configured_with(
    feature_type: &str,
    config: &Value,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    world_seed: i64,
    environment: &dyn FeatureEnvironment,
    dispatcher: &mut dyn ConfiguredFeatureDispatcher,
) -> FeatureResult<bool> {
    require_type(feature_type)?;
    let config = Config::parse(config)?;
    // Feature.place(config, ...) guards the origin once. safeSetBlock does not
    // pre-check each target: rejected writes still consume provider/placement RNG.
    if !world.can_write_feature(origin) {
        return Ok(false);
    }
    let count = config.distribution_points.sample_with(random, dispatcher)?;
    let noise = geode_noise(world_seed)?;
    let scale = count as f64 / config.outer_wall_distance.bounds().1 as f64;
    let filling = 1.0 / config.layers[0].sqrt();
    let inner = 1.0 / (config.layers[1] + scale).sqrt();
    let middle = 1.0 / (config.layers[2] + scale).sqrt();
    let outer = 1.0 / (config.layers[3] + scale).sqrt();
    let crack_threshold = 1.0
        / (config.base_crack_size
            + random.next_double() / 2.0
            + if count > 3 { scale } else { 0.0 })
        .sqrt();
    let generate_crack = f64::from(random.next_float()) < config.generate_crack_chance;

    let mut points = Vec::with_capacity(count as usize);
    let mut invalid_blocks = 0;
    for _ in 0..count {
        let x = config.outer_wall_distance.sample_with(random, dispatcher)?;
        let y = config.outer_wall_distance.sample_with(random, dispatcher)?;
        let z = config.outer_wall_distance.sample_with(random, dispatcher)?;
        let pos = offset(origin, (x, y, z));
        let state = read(world, pos)?;
        if catalog().info(state)?.is_air() || in_tag(state, &config.invalid_blocks)? {
            invalid_blocks += 1;
            if invalid_blocks > config.invalid_blocks_threshold {
                // In particular, the failing point does not sample point_offset.
                return Ok(false);
            }
        }
        points.push((pos, config.point_offset.sample_with(random, dispatcher)?));
    }

    let mut crack_points = Vec::new();
    if generate_crack {
        let direction = random.next_int(4);
        let distance = count * 2 + 1;
        let x = if direction == 0 || direction == 2 {
            distance
        } else {
            0
        };
        let z = if direction == 1 || direction == 2 {
            distance
        } else {
            0
        };
        for y in [7, 5, 1] {
            crack_points.push(offset(origin, (x, y, z)));
        }
    }

    let air = catalog().default_state("air")?;
    let empty_fluid = catalog().fluids["minecraft:empty"];
    let corner0 = offset(
        origin,
        (config.min_offset, config.min_offset, config.min_offset),
    );
    let corner1 = offset(
        origin,
        (config.max_offset, config.max_offset, config.max_offset),
    );
    let mut placements = Vec::new();
    // BlockPos.betweenClosed normalizes the two corners and advances X, Y, Z.
    for z in corner0.2.min(corner1.2)..=corner0.2.max(corner1.2) {
        for y in corner0.1.min(corner1.1)..=corner0.1.max(corner1.1) {
            for x in corner0.0.min(corner1.0)..=corner0.0.max(corner1.0) {
                let pos = (x, y, z);
                let offset_noise =
                    noise.value(x as f64, y as f64, z as f64) * config.noise_multiplier;
                let mut density = 0.0;
                let mut crack_density = 0.0;
                for &(point, point_offset) in &points {
                    density += 1.0 / (distance_squared(pos, point) + point_offset as f64).sqrt()
                        + offset_noise;
                }
                for &point in &crack_points {
                    crack_density += 1.0
                        / (distance_squared(pos, point) + config.crack_point_offset as f64).sqrt()
                        + offset_noise;
                }
                if density < outer {
                    continue;
                }
                if generate_crack && crack_density >= crack_threshold && density < filling {
                    safe_set(world, pos, air, &config.cannot_replace)?;
                    // Requests are made after the attempted carve, even when it
                    // was rejected by a tag/write guard, and retain duplicates.
                    for direction in Direction::ALL {
                        let neighbor = direction.step(pos);
                        let fluid = catalog().info(read(world, neighbor)?)?.fluid;
                        if fluid != empty_fluid {
                            world.schedule_feature_tick(TickRequest {
                                block_pos: [neighbor.0, neighbor.1, neighbor.2],
                                target: TickTarget::Fluid(fluid),
                                delay: 0,
                            });
                        }
                    }
                } else if density >= filling {
                    let state =
                        config
                            .filling
                            .sample_with(world, random, pos, environment, dispatcher)?;
                    safe_set(world, pos, state, &config.cannot_replace)?;
                } else if density >= inner {
                    let alternate = f64::from(random.next_float()) < config.alternate_chance;
                    let provider = if alternate {
                        &config.alternate
                    } else {
                        &config.inner
                    };
                    let state =
                        provider.sample_with(world, random, pos, environment, dispatcher)?;
                    safe_set(world, pos, state, &config.cannot_replace)?;
                    if (!config.require_alternate || alternate)
                        && f64::from(random.next_float()) < config.placement_chance
                    {
                        placements.push(pos);
                    }
                } else if density >= middle {
                    let state =
                        config
                            .middle
                            .sample_with(world, random, pos, environment, dispatcher)?;
                    safe_set(world, pos, state, &config.cannot_replace)?;
                } else if density >= outer {
                    let state =
                        config
                            .outer
                            .sample_with(world, random, pos, environment, dispatcher)?;
                    safe_set(world, pos, state, &config.cannot_replace)?;
                }
            }
        }
    }

    let water = catalog().definition("water")?;
    let source_water = catalog().fluids["minecraft:water"];
    let source_lava = catalog().fluids["minecraft:lava"];
    for pos in placements {
        // Util.getRandom draws even for a singleton list or a fully blocked anchor.
        let mut state = config.placements[random.next_int(config.placements.len())];
        let block = catalog().block(state)?.1;
        // Native tests the six-direction FACING property, not HORIZONTAL_FACING.
        let facing = block.properties.iter().any(|(name, values)| {
            name == "facing" && values.len() == 6 && values.iter().any(|v| v == "up")
        });
        let waterlogged = block.property(state, "waterlogged").is_some();
        for (index, direction) in Direction::ALL.into_iter().enumerate() {
            if facing {
                state = block.with_property(
                    state,
                    "facing",
                    ["down", "up", "north", "south", "west", "east"][index],
                )?;
            }
            let neighbor = direction.step(pos);
            let existing = read(world, neighbor)?;
            let info = catalog().info(existing)?;
            if waterlogged {
                let is_source = info.fluid == source_water || info.fluid == source_lava;
                state = block.with_property(
                    state,
                    "waterlogged",
                    if is_source { "true" } else { "false" },
                )?;
            }
            // BuddingAmethystBlock checks WATER + FluidState.isFull(), not
            // isSource(): falling full water permits a *dry* crystal in 26.1.
            if info.is_air()
                || ((water.first..water.first + water.count).contains(&existing)
                    && info.fluid_amount == 8)
            {
                safe_set(world, neighbor, state, &config.cannot_replace)?;
                // Native stops at the first growth candidate even if safeSetBlock
                // did not write anything. It does not test crystal survival.
                break;
            }
        }
    }
    Ok(true)
}

fn require_type(feature_type: &str) -> FeatureResult<()> {
    if supports(feature_type) {
        Ok(())
    } else {
        Err(FeatureError::Unsupported(format!(
            "geode kernel for {feature_type}"
        )))
    }
}

fn read(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<u32> {
    // The adapter owns dimensional bounds and read availability. Do not hide
    // missing chunks behind air, or clip to hard-coded overworld bounds here.
    world
        .get_block(pos)
        .ok_or_else(|| FeatureError::MissingData(format!("geode block at {pos:?}")))
}

fn in_tag(state: u32, members: &BTreeSet<u32>) -> FeatureResult<bool> {
    Ok(members.contains(&catalog().block(state)?.1.first))
}

fn safe_set(
    world: &mut dyn FeatureWorld,
    pos: Pos,
    state: u32,
    cannot_replace: &BTreeSet<u32>,
) -> FeatureResult<()> {
    if !in_tag(read(world, pos)?, cannot_replace)? {
        world.set_feature_block(pos, state, 2);
    }
    Ok(())
}

fn distance_squared(a: Pos, b: Pos) -> f64 {
    let x = a.0 as f64 - b.0 as f64;
    let y = a.1 as f64 - b.1 as f64;
    let z = a.2 as f64 - b.2 as f64;
    x * x + y * y + z * z
}

fn geode_noise(seed: i64) -> FeatureResult<Arc<ProviderNoise>> {
    // StateProvider exposes the already native-verified LegacyPositional noise
    // and its cache. Use that constructor instead of duplicating Perlin/LCG code.
    let StateProvider::Noise(noise, _, _) = StateProvider::parse(&json!({
        "type": "minecraft:noise_provider", "seed": seed, "scale": 1.0,
        "noise": {"firstOctave": -4, "amplitudes": [1.0]},
        "states": [{"Name": "minecraft:air"}],
    }))?
    else {
        unreachable!("noise_provider constructor")
    };
    Ok(noise)
}

struct Config {
    filling: StateProvider,
    inner: StateProvider,
    alternate: StateProvider,
    middle: StateProvider,
    outer: StateProvider,
    placements: Vec<u32>,
    cannot_replace: BTreeSet<u32>,
    invalid_blocks: BTreeSet<u32>,
    layers: [f64; 4],
    generate_crack_chance: f64,
    base_crack_size: f64,
    crack_point_offset: i32,
    placement_chance: f64,
    alternate_chance: f64,
    require_alternate: bool,
    outer_wall_distance: IntProvider,
    distribution_points: IntProvider,
    point_offset: IntProvider,
    min_offset: i32,
    max_offset: i32,
    noise_multiplier: f64,
    invalid_blocks_threshold: i32,
}

impl Config {
    fn parse(value: &Value) -> FeatureResult<Self> {
        let blocks = &value["blocks"];
        let layers = &value["layers"];
        let crack = &value["crack"];
        if !value.is_object() || !blocks.is_object() || !layers.is_object() || !crack.is_object() {
            return Err(invalid(
                "geode requires config, blocks, layers and crack objects",
            ));
        }
        let placements = array(&blocks["inner_placements"])?
            .iter()
            .map(|state| catalog().state(state))
            .collect::<FeatureResult<Vec<_>>>()?;
        if placements.is_empty() {
            return Err(invalid("geode inner_placements must be nonempty"));
        }
        let tag = |key| {
            let name = string(&blocks[key])?;
            if !name.starts_with('#') {
                return Err(invalid(format!("geode {key} requires a #block_tag")));
            }
            catalog().block_holder_set(&blocks[key])
        };
        Ok(Self {
            filling: StateProvider::parse(&blocks["filling_provider"])?,
            inner: StateProvider::parse(&blocks["inner_layer_provider"])?,
            alternate: StateProvider::parse(&blocks["alternate_inner_layer_provider"])?,
            middle: StateProvider::parse(&blocks["middle_layer_provider"])?,
            outer: StateProvider::parse(&blocks["outer_layer_provider"])?,
            placements,
            cannot_replace: tag("cannot_replace")?,
            invalid_blocks: tag("invalid_blocks")?,
            layers: [
                ranged_or(layers, "filling", 1.7, 0.01, 50.0),
                ranged_or(layers, "inner_layer", 2.2, 0.01, 50.0),
                ranged_or(layers, "middle_layer", 3.2, 0.01, 50.0),
                ranged_or(layers, "outer_layer", 4.2, 0.01, 50.0),
            ],
            generate_crack_chance: ranged_or(crack, "generate_crack_chance", 1.0, 0.0, 1.0),
            base_crack_size: ranged_or(crack, "base_crack_size", 2.0, 0.0, 5.0),
            crack_point_offset: integer(crack, "crack_point_offset")
                .ok()
                .filter(|n| (0..=10).contains(n))
                .unwrap_or(2),
            placement_chance: ranged_or(value, "use_potential_placements_chance", 0.35, 0.0, 1.0),
            alternate_chance: ranged_or(value, "use_alternate_layer0_chance", 0.0, 0.0, 1.0),
            require_alternate: value["placements_require_layer0_alternate"]
                .as_bool()
                .unwrap_or(true),
            outer_wall_distance: int_provider_or(value, "outer_wall_distance", (4, 5), (1, 20))?,
            distribution_points: int_provider_or(value, "distribution_points", (3, 4), (1, 20))?,
            point_offset: int_provider_or(value, "point_offset", (1, 2), (0, 10))?,
            min_offset: integer(value, "min_gen_offset").unwrap_or(-16),
            max_offset: integer(value, "max_gen_offset").unwrap_or(16),
            noise_multiplier: ranged_or(value, "noise_multiplier", 0.05, 0.0, 1.0),
            invalid_blocks_threshold: integer(value, "invalid_blocks_threshold")?,
        })
    }
}

// These native codec fields use fieldOf(...).orElse(...), so missing or
// out-of-range numbers fall back to their defaults, rather than being clamped.
fn ranged_or(value: &Value, key: &str, default: f64, min: f64, max: f64) -> f64 {
    value[key]
        .as_f64()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
        .unwrap_or(default)
}

fn int_provider_or(
    value: &Value,
    key: &str,
    default: (i32, i32),
    range: (i32, i32),
) -> FeatureResult<IntProvider> {
    let fallback = || IntProvider::Uniform(default.0, default.1);
    let Some(value) = value.get(key) else {
        return Ok(fallback());
    };
    match IntProvider::parse(value).and_then(|provider| provider.require_range(range.0, range.1)) {
        Ok(provider) => Ok(provider),
        Err(FeatureError::InvalidConfig(_)) => Ok(fallback()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_world_seeded_noise_is_bit_exact() {
        let fixture: Value = serde_json::from_str(include_str!("../data/geode_26_1.json")).unwrap();
        let mut count = 0;
        for sample in fixture["numerics"].as_array().unwrap() {
            let seed = sample["seed"].as_i64().unwrap();
            let noise = geode_noise(seed).unwrap();
            // WorldgenRandom.forkPositional delegates to its Legacy source;
            // the independent normal noise does not advance that wrapper count.
            assert_eq!(sample["independent_random_count"], 0);
            for row in sample["noise_bits"].as_array().unwrap() {
                let x = row[0].as_f64().unwrap();
                let y = row[1].as_f64().unwrap();
                let z = row[2].as_f64().unwrap();
                let expected = row[3].as_i64().unwrap() as u64;
                assert_eq!(
                    noise.value(x, y, z).to_bits(),
                    expected,
                    "world seed {seed}, ({x}, {y}, {z})"
                );
                count += 1;
            }
        }
        assert_eq!(count, 432);
    }
}
