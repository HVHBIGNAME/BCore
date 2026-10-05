//! Minecraft 26.1 pointed, cluster, and large dripstone configured features.
//!
//! Call [`place_configured`] with the **feature type** and its `config` object
//! after the external placed-feature driver has applied modifiers. In particular,
//! the registered `pointed_dripstone` is a selector containing two placed children;
//! its inline `minecraft:pointed_dripstone` bodies are handled here.
//!
//! Retain one [`CaveRandomState`] alongside each `WorldgenRandom`, across placed
//! attempts and feature reseeding. Vanilla WorldgenRandom does not clear its
//! inherited Gaussian cache in `setSeed`. This module never reseeds the RNG.
//! Reads outside the Overworld build height are air; unavailable in-range reads
//! are errors. Write flags and native return values are preserved independently
//! of whether the world's setter accepts a write. The world owns implicit block
//! postprocessing from writes; these three feature bodies emit no explicit ticks
//! or postprocessing marks.

use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::simplex::WorldgenRandom;
use crate::{mth, MAX_Y, MIN_Y};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The Marsaglia-polar cache missing from BCore's current WorldgenRandom.
/// Create it once per RNG, not once per configured feature invocation.
#[derive(Debug, Default, Clone)]
pub struct CaveRandomState {
    next_gaussian: Option<f64>,
}

impl CaveRandomState {
    pub fn next_gaussian(&mut self, random: &mut WorldgenRandom) -> f64 {
        if let Some(value) = self.next_gaussian.take() {
            return value;
        }
        loop {
            let x = 2.0 * random.next_double() - 1.0;
            let y = 2.0 * random.next_double() - 1.0;
            let square = x * x + y * y;
            if square >= 1.0 || square == 0.0 {
                continue;
            }
            let scale = (-2.0 * square.ln() / square).sqrt();
            self.next_gaussian = Some(y * scale);
            return x * scale;
        }
    }

    pub(crate) fn normal(&mut self, random: &mut WorldgenRandom, mean: f32, deviation: f32) -> f32 {
        mean + self.next_gaussian(random) as f32 * deviation
    }
}

/// Dispatch a native dripstone feature body. The config is validated before use.
pub fn place_configured(
    name: &str,
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    random_state: &mut CaveRandomState,
) -> Result<bool, FeatureError> {
    match key(name) {
        "pointed_dripstone" => place_pointed_dripstone(config, world, random, origin),
        "dripstone_cluster" => place_dripstone_cluster(config, world, random, origin, random_state),
        "large_dripstone" => place_large_dripstone(config, world, random, origin, random_state),
        _ => Err(FeatureError::Unsupported(format!(
            "dripstone feature {name}"
        ))),
    }
}

pub fn place_pointed_dripstone(
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
) -> Result<bool, FeatureError> {
    if !config.is_object() {
        return Err(FeatureError::InvalidConfig(
            "pointed dripstone config must be an object".into(),
        ));
    }
    let taller = float_default(config, "chance_of_taller_dripstone", 0.2, 0.0, 1.0)?;
    let spread = float_default(config, "chance_of_directional_spread", 0.7, 0.0, 1.0)?;
    let radius2 = float_default(config, "chance_of_spread_radius2", 0.5, 0.0, 1.0)?;
    let radius3 = float_default(config, "chance_of_spread_radius3", 0.5, 0.0, 1.0)?;
    if !world.can_write_feature(origin) {
        return Ok(false);
    }
    let above = dripstone_base(read(world, offset(origin, UP))?)?;
    let below = dripstone_base(read(world, offset(origin, DOWN))?)?;
    let direction = match (above, below) {
        (true, true) => {
            if random.next_bool() {
                DOWN
            } else {
                UP
            }
        }
        (true, false) => DOWN,
        (false, true) => UP,
        (false, false) => return Ok(false),
    };
    let base = offset(origin, opposite(direction));
    replace_dripstone(world, base)?;
    for side in HORIZONTAL {
        if random.next_float() > spread {
            continue;
        }
        let first = offset(base, side);
        replace_dripstone(world, first)?;
        if random.next_float() > radius2 {
            continue;
        }
        let second = offset(first, random.next_int(6));
        replace_dripstone(world, second)?;
        if random.next_float() > radius3 {
            continue;
        }
        let third = offset(second, random.next_int(6));
        replace_dripstone(world, third)?;
    }
    let height = if random.next_float() < taller
        && empty_or_water(read(world, offset(origin, direction))?)?
    {
        2
    } else {
        1
    };
    grow_pointed(world, origin, direction, height, false)?;
    Ok(true)
}

fn dripstone_base(state: u32) -> Result<bool, FeatureError> {
    Ok(state == block_id("dripstone_block")? || in_tag(state, "dripstone_replaceable")?)
}

fn replace_dripstone(world: &mut impl FeatureWorld, pos: Pos) -> Result<bool, FeatureError> {
    if !in_tag(read(world, pos)?, "dripstone_replaceable")? {
        return Ok(false);
    }
    world.set_feature_block(pos, block_id("dripstone_block")?, 2);
    Ok(true)
}

fn grow_pointed(
    world: &mut impl FeatureWorld,
    mut pos: Pos,
    direction: usize,
    height: i32,
    merge: bool,
) -> Result<(), FeatureError> {
    if !dripstone_base(read(world, offset(pos, opposite(direction)))?)? {
        return Ok(());
    }
    for index in 0..height {
        let thickness = if index == height - 1 {
            if merge {
                "tip_merge"
            } else {
                "tip"
            }
        } else if index == height - 2 {
            "frustum"
        } else if index == 0 {
            "base"
        } else {
            "middle"
        };
        let wet = state_info(read(world, pos)?)?.flags & 8 != 0;
        let state = state_with(
            "pointed_dripstone",
            &[
                (
                    "vertical_direction",
                    if direction == UP { "up" } else { "down" },
                ),
                ("thickness", thickness),
                ("waterlogged", if wet { "true" } else { "false" }),
            ],
        )?;
        world.set_feature_block(pos, state, 2);
        pos = offset(pos, direction);
    }
    Ok(())
}

struct ClusterConfig {
    search: i32,
    height: IntProvider,
    radius: IntProvider,
    difference: i32,
    deviation: i32,
    thickness: IntProvider,
    density: FloatProvider,
    wetness: FloatProvider,
    edge_chance: f32,
    edge_distance: i32,
    center_distance: i32,
}

impl ClusterConfig {
    fn parse(v: &Value) -> Result<Self, FeatureError> {
        Ok(Self {
            search: int_field(v, "floor_to_ceiling_search_range", 1, 512)?,
            height: IntProvider::parse(&v["height"], 1, 128)?,
            radius: IntProvider::parse(&v["radius"], 1, 128)?,
            difference: int_field(v, "max_stalagmite_stalactite_height_diff", 0, 64)?,
            deviation: int_field(v, "height_deviation", 1, 64)?,
            thickness: IntProvider::parse(&v["dripstone_block_layer_thickness"], 0, 128)?,
            density: FloatProvider::parse(&v["density"], 0.0, 2.0)?,
            wetness: FloatProvider::parse(&v["wetness"], 0.0, 2.0)?,
            edge_chance: float_field(
                v,
                "chance_of_dripstone_column_at_max_distance_from_center",
                0.0,
                1.0,
            )?,
            edge_distance: int_field(
                v,
                "max_distance_from_edge_affecting_chance_of_dripstone_column",
                1,
                64,
            )?,
            center_distance: int_field(v, "max_distance_from_center_affecting_height_bias", 1, 64)?,
        })
    }
}

pub fn place_dripstone_cluster(
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    random_state: &mut CaveRandomState,
) -> Result<bool, FeatureError> {
    let config = ClusterConfig::parse(config)?;
    if !world.can_write_feature(origin) || !empty_or_water(read(world, origin)?)? {
        return Ok(false);
    }
    let height = config.height.sample(random, random_state);
    let wetness = config.wetness.sample(random, random_state);
    let density = config.density.sample(random, random_state);
    let rx = config.radius.sample(random, random_state);
    let rz = config.radius.sample(random, random_state);
    for x in -rx..=rx {
        for z in -rz..=rz {
            let edge = (rx - x.abs()).min(rz - z.abs());
            let t = edge as f32 / config.edge_distance as f32;
            let chance = clamped_lerp(config.edge_chance, 1.0, t) as f64;
            let pos = (origin.0 + x, origin.1, origin.2 + z);
            let mut column = ClusterColumn {
                config: &config,
                height,
                wetness,
                density,
                chance,
                x,
                z,
            };
            column.place(world, random, random_state, pos)?;
        }
    }
    Ok(true)
}

struct ClusterColumn<'a> {
    config: &'a ClusterConfig,
    height: i32,
    wetness: f32,
    density: f32,
    chance: f64,
    x: i32,
    z: i32,
}

impl ClusterColumn<'_> {
    fn height(&self, random: &mut WorldgenRandom, state: &mut CaveRandomState, max: i32) -> i32 {
        if random.next_float() > self.density {
            return 0;
        }
        let distance = self.x.abs() + self.z.abs();
        let t = (distance as f64 / self.config.center_distance as f64).clamp(0.0, 1.0);
        let half = max as f64 / 2.0;
        let mean = (half + t * (0.0 - half)) as f32;
        state
            .normal(random, mean, self.config.deviation as f32)
            .clamp(0.0, max as f32) as i32
    }

    fn place(
        &mut self,
        world: &mut impl FeatureWorld,
        random: &mut WorldgenRandom,
        state: &mut CaveRandomState,
        pos: Pos,
    ) -> Result<(), FeatureError> {
        let Some((mut floor, ceiling)) = scan_column(world, pos, self.config.search, false)? else {
            return Ok(());
        };
        if floor.is_none() && ceiling.is_none() {
            return Ok(());
        }
        let wet = random.next_float() < self.wetness;
        if wet {
            if let Some(y) = floor {
                if can_place_pool(world, at_y(pos, y))? {
                    floor = Some(y - 1);
                    world.set_feature_block(at_y(pos, y), block_id("water")?, 2);
                }
            }
        }
        let top_roll = random.next_double() < self.chance;
        let top_height = if let Some(y) = ceiling.filter(|_| top_roll) {
            if state_info(read(world, at_y(pos, y))?)?.flags & 4 == 0 {
                let thickness = self.config.thickness.sample(random, state);
                replace_layer(world, at_y(pos, y), thickness, UP)?;
                self.height(
                    random,
                    state,
                    floor.map_or(self.height, |f| self.height.min(y - f)),
                )
            } else {
                0
            }
        } else {
            0
        };
        let bottom_roll = random.next_double() < self.chance;
        let bottom_height = if let Some(y) = floor.filter(|_| bottom_roll) {
            if state_info(read(world, at_y(pos, y))?)?.flags & 4 == 0 {
                let thickness = self.config.thickness.sample(random, state);
                replace_layer(world, at_y(pos, y), thickness, DOWN)?;
                if ceiling.is_some() {
                    (top_height + between(random, -self.config.difference, self.config.difference))
                        .max(0)
                } else {
                    self.height(random, state, self.height)
                }
            } else {
                0
            }
        } else {
            0
        };
        let (mut top, mut bottom) = (top_height, bottom_height);
        if let (Some(f), Some(c)) = (floor, ceiling) {
            if c - top_height <= f + bottom_height {
                let lo = (c - top_height).max(f + 1);
                let hi = (f + bottom_height).min(c - 1);
                let divide = between(random, lo, hi + 1);
                top = c - divide;
                bottom = divide - 1 - f;
            }
        }
        let merge_roll = random.next_bool();
        let merge = merge_roll
            && top > 0
            && bottom > 0
            && floor
                .zip(ceiling)
                .is_some_and(|(f, c)| top + bottom == c - f - 1);
        if let Some(y) = ceiling {
            grow_pointed(world, at_y(pos, y - 1), DOWN, top, merge)?;
        }
        if let Some(y) = floor {
            grow_pointed(world, at_y(pos, y + 1), UP, bottom, merge)?;
        }
        Ok(())
    }
}

fn can_place_pool(world: &impl FeatureWorld, pos: Pos) -> Result<bool, FeatureError> {
    let current = read(world, pos)?;
    if state_info(current)?.flags & 2 != 0
        || current == block_id("dripstone_block")?
        || state_info(current)?.block == block_id("pointed_dripstone")?
    {
        return Ok(false);
    }
    if state_info(read(world, offset(pos, UP))?)?.flags & 8 != 0 {
        return Ok(false);
    }
    for direction in HORIZONTAL.into_iter().chain([DOWN]) {
        let neighbour = read(world, offset(pos, direction))?;
        if !in_tag(neighbour, "base_stone_overworld")? && state_info(neighbour)?.flags & 8 == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn replace_layer(
    world: &mut impl FeatureWorld,
    mut pos: Pos,
    height: i32,
    direction: usize,
) -> Result<(), FeatureError> {
    for _ in 0..height {
        if !replace_dripstone(world, pos)? {
            break;
        }
        pos = offset(pos, direction);
    }
    Ok(())
}

type ColumnBounds = (Option<i32>, Option<i32>);
fn scan_column(
    world: &impl FeatureWorld,
    pos: Pos,
    range: i32,
    dripstone_only: bool,
) -> Result<Option<ColumnBounds>, FeatureError> {
    if !empty_or_water(read(world, pos)?)? {
        return Ok(None);
    }
    let mut limits = [None, None];
    for (index, direction) in [UP, DOWN].into_iter().enumerate() {
        let mut p = pos;
        for _ in 1..range {
            if !empty_or_water(read(world, p)?)? {
                break;
            }
            p = offset(p, direction);
        }
        let state = read(world, p)?;
        let boundary = if dripstone_only {
            dripstone_base(state)? || state_info(state)?.flags & 4 != 0
        } else {
            !empty_or_water(state)?
        };
        if boundary {
            limits[index] = Some(p.1);
        }
    }
    Ok(Some((limits[1], limits[0])))
}

struct LargeConfig {
    search: i32,
    radius: IntProvider,
    scale: FloatProvider,
    ratio: f32,
    top_blunt: FloatProvider,
    bottom_blunt: FloatProvider,
    wind: FloatProvider,
    wind_radius: i32,
    wind_blunt: f32,
}

impl LargeConfig {
    fn parse(v: &Value) -> Result<Self, FeatureError> {
        Ok(Self {
            search: v
                .get("floor_to_ceiling_search_range")
                .map_or(Ok(30), |v| integer(v, 1, 512))?,
            radius: IntProvider::parse(&v["column_radius"], 1, 60)?,
            scale: FloatProvider::parse(&v["height_scale"], 0.0, 20.0)?,
            ratio: float_field(v, "max_column_radius_to_cave_height_ratio", 0.1, 1.0)?,
            top_blunt: FloatProvider::parse(&v["stalactite_bluntness"], 0.1, 10.0)?,
            bottom_blunt: FloatProvider::parse(&v["stalagmite_bluntness"], 0.1, 10.0)?,
            wind: FloatProvider::parse(&v["wind_speed"], 0.0, 2.0)?,
            wind_radius: int_field(v, "min_radius_for_wind", 0, 100)?,
            wind_blunt: float_field(v, "min_bluntness_for_wind", 0.0, 5.0)?,
        })
    }
}

pub fn place_large_dripstone(
    config: &Value,
    world: &mut impl FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    random_state: &mut CaveRandomState,
) -> Result<bool, FeatureError> {
    let config = LargeConfig::parse(config)?;
    if !world.can_write_feature(origin) {
        return Ok(false);
    }
    let Some((Some(floor), Some(ceiling))) = scan_column(world, origin, config.search, true)?
    else {
        return Ok(false);
    };
    let height = ceiling - floor - 1;
    if height < 4 {
        return Ok(false);
    }
    let (min_radius, max_radius) = config.radius.bounds();
    let upper = ((height as f32 * config.ratio) as i32).clamp(min_radius, max_radius);
    let radius = between(random, min_radius, upper);
    let mut top = LargeDripstone {
        root: at_y(origin, ceiling - 1),
        direction: DOWN,
        radius,
        bluntness: config.top_blunt.sample(random, random_state) as f64,
        scale: config.scale.sample(random, random_state) as f64,
    };
    let mut bottom = LargeDripstone {
        root: at_y(origin, floor + 1),
        direction: UP,
        radius,
        bluntness: config.bottom_blunt.sample(random, random_state) as f64,
        scale: config.scale.sample(random, random_state) as f64,
    };
    let wind = if radius >= config.wind_radius
        && top.bluntness >= config.wind_blunt as f64
        && bottom.bluntness >= config.wind_blunt as f64
    {
        let speed = config.wind.sample(random, random_state);
        let angle = random.next_float() * std::f32::consts::PI;
        Wind {
            y: origin.1,
            x: (mth::cos_f32(angle) * speed) as f64,
            z: (mth::sin_f32(angle) * speed) as f64,
        }
    } else {
        Wind {
            y: 0,
            x: 0.0,
            z: 0.0,
        }
    };
    let place_top = top.embed(world, wind)?;
    let place_bottom = bottom.embed(world, wind)?;
    if place_top {
        top.place(world, random, wind)?;
    }
    if place_bottom {
        bottom.place(world, random, wind)?;
    }
    Ok(true)
}

#[derive(Clone, Copy)]
struct Wind {
    y: i32,
    x: f64,
    z: f64,
}
impl Wind {
    fn offset(self, pos: Pos) -> Pos {
        let height = (self.y - pos.1) as f64;
        (
            pos.0 + (self.x * height).floor() as i32,
            pos.1,
            pos.2 + (self.z * height).floor() as i32,
        )
    }
}

struct LargeDripstone {
    root: Pos,
    direction: usize,
    radius: i32,
    bluntness: f64,
    scale: f64,
}
impl LargeDripstone {
    fn height(&self, distance: f32) -> i32 {
        let distance = (distance as f64).max(self.bluntness);
        let u = distance / self.radius as f64 * 0.384;
        let curve = 0.75 * u.powf(1.3333333333333333)
            - u.powf(0.6666666666666666)
            - 0.3333333333333333 * u.ln();
        ((self.scale * curve).max(0.0) / 0.384 * self.radius as f64) as i32
    }

    fn embed(&mut self, world: &impl FeatureWorld, wind: Wind) -> Result<bool, FeatureError> {
        while self.radius > 1 {
            let mut p = self.root;
            for _ in 0..10.min(self.height(0.0)) {
                if state_info(read(world, p)?)?.flags & 4 != 0 {
                    return Ok(false);
                }
                if embedded_circle(world, wind.offset(p), self.radius)? {
                    self.root = p;
                    return Ok(true);
                }
                p = offset(p, opposite(self.direction));
            }
            self.radius /= 2;
        }
        Ok(false)
    }

    fn place(
        &self,
        world: &mut impl FeatureWorld,
        random: &mut WorldgenRandom,
        wind: Wind,
    ) -> Result<(), FeatureError> {
        for x in -self.radius..=self.radius {
            for z in -self.radius..=self.radius {
                let distance = ((x * x + z * z) as f32).sqrt();
                if distance > self.radius as f32 {
                    continue;
                }
                let mut height = self.height(distance);
                if height <= 0 {
                    continue;
                }
                if (random.next_float() as f64) < 0.2 {
                    height = (height as f32 * (0.8_f32 + random.next_float() * (1.0_f32 - 0.8_f32)))
                        as i32;
                }
                let mut p = (self.root.0 + x, self.root.1, self.root.2 + z);
                let max_y = if self.direction == UP {
                    world.feature_height(FeatureHeightmap::WorldSurfaceWg, p.0, p.2)
                } else {
                    i32::MAX
                };
                let mut started = false;
                for _ in 0..height {
                    if p.1 >= max_y {
                        break;
                    }
                    let target = wind.offset(p);
                    let current = read(world, target)?;
                    if state_info(current)?.flags & 7 != 0 {
                        started = true;
                        world.set_feature_block(target, block_id("dripstone_block")?, 2);
                    } else if started && in_tag(current, "base_stone_overworld")? {
                        break;
                    }
                    p = offset(p, self.direction);
                }
            }
        }
        Ok(())
    }
}

fn embedded_circle(world: &impl FeatureWorld, pos: Pos, radius: i32) -> Result<bool, FeatureError> {
    if state_info(read(world, pos)?)?.flags & 7 != 0 {
        return Ok(false);
    }
    let step = 6.0_f32 / radius as f32;
    let mut angle = 0.0_f32;
    while angle < std::f32::consts::TAU {
        let x = (mth::cos_f32(angle) * radius as f32) as i32;
        let z = (mth::sin_f32(angle) * radius as f32) as i32;
        if state_info(read(world, (pos.0 + x, pos.1, pos.2 + z))?)?.flags & 7 != 0 {
            return Ok(false);
        }
        angle += step;
    }
    Ok(true)
}

pub(crate) const DOWN: usize = 0;
pub(crate) const UP: usize = 1;
pub(crate) const HORIZONTAL: [usize; 4] = [2, 5, 3, 4];
pub(crate) const DIRECTIONS: [Pos; 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];
pub(crate) fn opposite(direction: usize) -> usize {
    direction ^ 1
}
pub(crate) fn offset(p: Pos, direction: usize) -> Pos {
    let d = DIRECTIONS[direction];
    (p.0 + d.0, p.1 + d.1, p.2 + d.2)
}
fn at_y(p: Pos, y: i32) -> Pos {
    (p.0, y, p.2)
}
pub(crate) fn key(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

pub(crate) fn read(world: &impl FeatureWorld, pos: Pos) -> Result<u32, FeatureError> {
    if !(MIN_Y..=MAX_Y).contains(&pos.1) {
        return Ok(0);
    }
    world
        .get_block(pos)
        .ok_or_else(|| FeatureError::MissingData(format!("cave feature read at {pos:?}")))
}

#[derive(Deserialize)]
pub(crate) struct CaveData {
    pub configs: BTreeMap<String, Value>,
    pub tags: BTreeMap<String, Vec<u32>>,
    pub blocks: BTreeMap<String, BlockSchema>,
    pub block_ids: BTreeMap<String, u32>,
    state_ranges: Vec<[i32; 7]>,
}

#[derive(Deserialize)]
pub(crate) struct BlockSchema {
    pub default: u32,
    pub states: Vec<StateSchema>,
}
#[derive(Deserialize)]
pub(crate) struct StateSchema {
    pub id: u32,
    pub properties: BTreeMap<String, String>,
}

pub(crate) fn data() -> &'static CaveData {
    static DATA: OnceLock<CaveData> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/cave_feature_data_26_1.json"))
            .expect("captured cave block data")
    })
}

/// Original configured-feature document from the pinned JAR, including its type.
pub fn configured_feature(name: &str) -> Result<&'static Value, FeatureError> {
    data()
        .configs
        .get(key(name))
        .ok_or_else(|| FeatureError::MissingData(format!("cave configured feature {name}")))
}

pub(crate) struct StateInfo {
    pub flags: i32,
    pub faces: i32,
    pub block: u32,
    pub attach: i32,
    pub center: i32,
}
pub(crate) fn state_info(state: u32) -> Result<StateInfo, FeatureError> {
    let ranges = &data().state_ranges;
    let i = ranges.partition_point(|row| row[1] <= state as i32);
    let row = ranges
        .get(i)
        .filter(|row| row[0] <= state as i32)
        .ok_or_else(|| FeatureError::MissingData(format!("cave block state {state}")))?;
    Ok(StateInfo {
        flags: row[2],
        faces: row[3],
        block: row[4] as u32,
        attach: row[5],
        center: row[6],
    })
}
pub(crate) fn block_id(name: &str) -> Result<u32, FeatureError> {
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    data()
        .block_ids
        .get(&name)
        .copied()
        .ok_or_else(|| FeatureError::MissingData(format!("block {name}")))
}
pub(crate) fn in_tag(state: u32, tag: &str) -> Result<bool, FeatureError> {
    let tag = key(tag.strip_prefix('#').unwrap_or(tag));
    let states = data()
        .tags
        .get(tag)
        .ok_or_else(|| FeatureError::MissingData(format!("cave block tag {tag}")))?;
    Ok(states.binary_search(&state).is_ok())
}
pub(crate) fn state_with(block: &str, properties: &[(&str, &str)]) -> Result<u32, FeatureError> {
    let schema = data()
        .blocks
        .get(key(block))
        .ok_or_else(|| FeatureError::Unsupported(format!("state properties for {block}")))?;
    let mut desired = schema
        .states
        .iter()
        .find(|s| s.id == schema.default)
        .expect("captured default state")
        .properties
        .clone();
    for &(name, value) in properties {
        desired.insert(name.to_owned(), value.to_owned());
    }
    schema
        .states
        .iter()
        .find(|s| s.properties == desired)
        .map(|s| s.id)
        .ok_or_else(|| FeatureError::InvalidConfig(format!("{block} properties {properties:?}")))
}
pub(crate) fn empty_or_water(state: u32) -> Result<bool, FeatureError> {
    Ok(state_info(state)?.flags & 3 != 0)
}

pub(crate) fn integer(v: &Value, min: i32, max: i32) -> Result<i32, FeatureError> {
    let value = v
        .as_i64()
        .filter(|v| *v >= min as i64 && *v <= max as i64)
        .ok_or_else(|| {
            FeatureError::InvalidConfig(format!("expected integer in {min}..={max}: {v}"))
        })?;
    Ok(value as i32)
}
pub(crate) fn int_field(v: &Value, name: &str, min: i32, max: i32) -> Result<i32, FeatureError> {
    integer(&v[name], min, max)
}
pub(crate) fn number(v: &Value, min: f32, max: f32) -> Result<f32, FeatureError> {
    v.as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite() && *v >= min && *v <= max)
        .ok_or_else(|| FeatureError::InvalidConfig(format!("expected float in {min}..={max}: {v}")))
}
pub(crate) fn float_field(v: &Value, name: &str, min: f32, max: f32) -> Result<f32, FeatureError> {
    number(&v[name], min, max)
}
pub(crate) fn float_default(
    v: &Value,
    name: &str,
    default: f32,
    min: f32,
    max: f32,
) -> Result<f32, FeatureError> {
    v.get(name).map_or(Ok(default), |v| number(v, min, max))
}
pub(crate) fn string<'a>(v: &'a Value, name: &str) -> Result<&'a str, FeatureError> {
    v[name]
        .as_str()
        .ok_or_else(|| FeatureError::InvalidConfig(format!("missing string {name}")))
}
pub(crate) fn between(random: &mut WorldgenRandom, min: i32, max: i32) -> i32 {
    min + random.next_int((max - min + 1) as usize) as i32
}
fn clamped_lerp(start: f32, end: f32, t: f32) -> f32 {
    if t < 0.0 {
        start
    } else if t > 1.0 {
        end
    } else {
        start + t * (end - start)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum IntProvider {
    Constant(i32),
    Uniform(i32, i32),
    Biased(i32, i32),
    Weighted(Vec<(IntProvider, i32)>),
    Clamped(Box<IntProvider>, i32, i32),
    Normal {
        mean: f32,
        deviation: f32,
        min: i32,
        max: i32,
    },
}
impl IntProvider {
    pub fn parse(v: &Value, min: i32, max: i32) -> Result<Self, FeatureError> {
        if v.is_number() {
            return Ok(Self::Constant(integer(v, min, max)?));
        }
        match key(string(v, "type")?) {
            "constant" => Ok(Self::Constant(integer(&v["value"], min, max)?)),
            "uniform" | "biased_to_bottom" => {
                let lo = int_field(v, "min_inclusive", min, max)?;
                let hi = int_field(v, "max_inclusive", lo, max)?;
                if hi as i64 - lo as i64 + 1 > i32::MAX as i64 {
                    return Err(FeatureError::InvalidConfig(
                        "native integer provider would overflow nextInt bound".into(),
                    ));
                }
                Ok(if key(string(v, "type")?) == "uniform" {
                    Self::Uniform(lo, hi)
                } else {
                    Self::Biased(lo, hi)
                })
            }
            "weighted_list" => {
                let mut entries = Vec::new();
                for entry in array(v, "distribution")? {
                    entries.push((
                        Self::parse(&entry["data"], min, max)?,
                        int_field(entry, "weight", 0, i32::MAX)?,
                    ));
                }
                check_weights(entries.iter().map(|(_, w)| *w))?;
                Ok(Self::Weighted(entries))
            }
            "clamped" => {
                let source = Self::parse(&v["source"], i32::MIN, i32::MAX)?;
                let lo = int_field(v, "min_inclusive", i32::MIN, i32::MAX)?;
                let hi = int_field(v, "max_inclusive", lo, i32::MAX)?;
                let provider = Self::Clamped(Box::new(source), lo, hi);
                let (a, b) = provider.bounds();
                if a < min || b > max {
                    return Err(FeatureError::InvalidConfig(
                        "clamped provider outside feature bounds".into(),
                    ));
                }
                Ok(provider)
            }
            "clamped_normal" => {
                let lo = int_field(v, "min_inclusive", min, max)?;
                Ok(Self::Normal {
                    mean: float_field(v, "mean", f32::MIN, f32::MAX)?,
                    deviation: float_field(v, "deviation", f32::MIN, f32::MAX)?,
                    min: lo,
                    max: int_field(v, "max_inclusive", lo, max)?,
                })
            }
            other => Err(FeatureError::Unsupported(format!(
                "cave IntProvider {other}"
            ))),
        }
    }
    pub fn sample(&self, random: &mut WorldgenRandom, state: &mut CaveRandomState) -> i32 {
        match self {
            Self::Constant(n) => *n,
            Self::Uniform(min, max) => between(random, *min, *max),
            Self::Biased(min, max) => {
                let bound = random.next_int((max - min + 1) as usize) + 1;
                min + random.next_int(bound) as i32
            }
            Self::Weighted(entries) => {
                let mut choice =
                    random.next_int(entries.iter().map(|(_, w)| *w as usize).sum()) as i32;
                for (provider, weight) in entries {
                    choice -= weight;
                    if choice < 0 {
                        return provider.sample(random, state);
                    }
                }
                unreachable!("validated weights")
            }
            Self::Clamped(source, min, max) => source.sample(random, state).clamp(*min, *max),
            Self::Normal {
                mean,
                deviation,
                min,
                max,
            } => state
                .normal(random, *mean, *deviation)
                .clamp(*min as f32, *max as f32) as i32,
        }
    }
    pub fn bounds(&self) -> (i32, i32) {
        match self {
            Self::Constant(n) => (*n, *n),
            Self::Uniform(a, b) | Self::Biased(a, b) => (*a, *b),
            Self::Weighted(entries) => entries
                .iter()
                .map(|(p, _)| p.bounds())
                .fold((i32::MAX, i32::MIN), |(a, b), (c, d)| (a.min(c), b.max(d))),
            Self::Clamped(source, min, max) => {
                let (a, b) = source.bounds();
                (a.max(*min), b.min(*max))
            }
            Self::Normal { min, max, .. } => (*min, *max),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum FloatProvider {
    Constant(f32),
    Uniform(f32, f32),
    Trapezoid(f32, f32, f32),
    Normal {
        mean: f32,
        deviation: f32,
        min: f32,
        max: f32,
    },
}
impl FloatProvider {
    fn parse(v: &Value, min: f32, max: f32) -> Result<Self, FeatureError> {
        if v.is_number() {
            return Ok(Self::Constant(number(v, min, max)?));
        }
        match key(string(v, "type")?) {
            "constant" => Ok(Self::Constant(number(&v["value"], min, max)?)),
            "uniform" => {
                let lo = float_field(v, "min_inclusive", min, max)?;
                let hi = float_field(v, "max_exclusive", lo, max)?;
                if lo == hi {
                    return Err(FeatureError::InvalidConfig(
                        "uniform float requires max > min".into(),
                    ));
                }
                Ok(Self::Uniform(lo, hi))
            }
            "clamped_normal" => {
                let lo = float_field(v, "min", min, max)?;
                Ok(Self::Normal {
                    mean: float_field(v, "mean", f32::MIN, f32::MAX)?,
                    deviation: float_field(v, "deviation", f32::MIN, f32::MAX)?,
                    min: lo,
                    max: float_field(v, "max", lo, max)?,
                })
            }
            "trapezoid" => {
                let lo = float_field(v, "min", min, max)?;
                let hi = float_field(v, "max", lo, max)?;
                Ok(Self::Trapezoid(
                    lo,
                    hi,
                    float_field(v, "plateau", f32::MIN, hi - lo)?,
                ))
            }
            other => Err(FeatureError::Unsupported(format!(
                "cave FloatProvider {other}"
            ))),
        }
    }
    fn sample(&self, random: &mut WorldgenRandom, state: &mut CaveRandomState) -> f32 {
        match *self {
            Self::Constant(value) => value,
            Self::Uniform(min, max) => min + random.next_float() * (max - min),
            Self::Trapezoid(min, max, plateau) => {
                let half = (max - min - plateau) / 2.0;
                min + random.next_float() * (max - min - half) + random.next_float() * half
            }
            Self::Normal {
                mean,
                deviation,
                min,
                max,
            } => state.normal(random, mean, deviation).clamp(min, max),
        }
    }
}

pub(crate) fn array<'a>(v: &'a Value, name: &str) -> Result<&'a [Value], FeatureError> {
    v[name]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| FeatureError::InvalidConfig(format!("missing array {name}")))
}
pub(crate) fn check_weights(weights: impl Iterator<Item = i32>) -> Result<i32, FeatureError> {
    let total: i64 = weights.map(i64::from).sum();
    if !(1..=i32::MAX as i64).contains(&total) {
        return Err(FeatureError::InvalidConfig(
            "weighted provider requires positive i32 total".into(),
        ));
    }
    Ok(total as i32)
}
