//! Minecraft 26.1 sculk patches, multiface veins, and worldgen charge cursors.
//!
//! The world owns normal `WorldGenRegion` write effects, including implicit
//! postprocessing and block-entity creation/removal. Adapters can use
//! [`generated_block_entity`] for the three generated sculk entity types. Direct
//! multiface marks are emitted here, including marks preceding rejected writes.
//! This module does not run catalyst gameplay ticks or consume deferred marks.
use crate::feature_world::{FeatureError, FeatureWorld, Pos};
use crate::simplex::WorldgenRandom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const AIR: u32 = 1;
const WATER_FLUID: u32 = 2;
const FLUID: u32 = 4;
const WATER_BLOCK: u32 = 8;
const REPLACEABLE: u32 = 16;
const WORLDGEN_SUBSTRATE: u32 = 32;
const SUBSTRATE: u32 = 64;
const FIRE: u32 = 128;
const SCULK: u32 = 256;
const VEIN: u32 = 512;
const CATALYST: u32 = 1024;
const MOVING_PISTON: u32 = 2048;
const GROWTH: u32 = 4096;
const BEHAVIOUR: u32 = 8192;
const FULL_COLLISION: u32 = 16384;
const KNOWN_SHAPE: u32 = 32768;

// Direction ordinal order, also used by native face masks and shuffle inputs.
const DIRECTIONS: [Pos; 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntProvider {
    Constant(i32),
    Uniform {
        min_inclusive: i32,
        max_inclusive: i32,
    },
}

impl IntProvider {
    fn parse(value: &Value) -> Result<Self, FeatureError> {
        if let Some(value) = value.as_i64().and_then(|n| i32::try_from(n).ok()) {
            return Ok(Self::Constant(value));
        }
        match value.get("type").and_then(Value::as_str).map(unqualified) {
            Some("constant") => Ok(Self::Constant(integer(value, "value")?)),
            Some("uniform") => Ok(Self::Uniform {
                min_inclusive: integer(value, "min_inclusive")?,
                max_inclusive: integer(value, "max_inclusive")?,
            }),
            _ => Err(FeatureError::Unsupported(format!(
                "sculk extra_rare_growths {value}"
            ))),
        }
    }

    fn validate(&self) -> Result<(), FeatureError> {
        if let Self::Uniform {
            min_inclusive,
            max_inclusive,
        } = *self
        {
            let width = i64::from(max_inclusive) - i64::from(min_inclusive) + 1;
            if !(1..=i64::from(i32::MAX)).contains(&width) {
                return Err(FeatureError::InvalidConfig(
                    "sculk uniform provider bounds".into(),
                ));
            }
        }
        Ok(())
    }

    fn sample(&self, random: &mut WorldgenRandom) -> i32 {
        match *self {
            Self::Constant(value) => value,
            Self::Uniform {
                min_inclusive,
                max_inclusive,
            } => {
                let width = (i64::from(max_inclusive) - i64::from(min_inclusive) + 1) as usize;
                min_inclusive + random.next_int(width) as i32
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchConfig {
    pub charge_count: i32,
    pub amount_per_charge: i32,
    pub spread_attempts: i32,
    pub growth_rounds: i32,
    pub spread_rounds: i32,
    pub extra_rare_growths: IntProvider,
    pub catalyst_chance: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VeinConfig {
    pub search_range: i32,
    pub can_place_on_floor: bool,
    pub can_place_on_ceiling: bool,
    pub can_place_on_wall: bool,
    pub chance_of_spreading: f32,
    /// Explicit native block names; all states of each block are accepted.
    pub can_be_placed_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Config {
    Patch(PatchConfig),
    Vein(VeinConfig),
}

impl Config {
    /// Registered 26.1 `sculk_patch_deep_dark`, `sculk_patch_ancient_city`, or `sculk_vein`.
    pub fn named(name: &str) -> Result<Self, FeatureError> {
        let definition = native()?
            .configurations
            .get(unqualified(name))
            .ok_or_else(|| FeatureError::Unsupported(format!("sculk configured feature {name}")))?;
        Self::from_json(string(definition, "type")?, &definition["config"])
    }

    /// Decode a configured feature's type and config object, before consuming RNG.
    /// Patch extra-growth providers support constant and uniform distributions.
    pub fn from_json(feature_type: &str, value: &Value) -> Result<Self, FeatureError> {
        let config = match unqualified(feature_type) {
            "sculk_patch" => Self::Patch(PatchConfig {
                charge_count: integer(value, "charge_count")?,
                amount_per_charge: integer(value, "amount_per_charge")?,
                spread_attempts: integer(value, "spread_attempts")?,
                growth_rounds: integer(value, "growth_rounds")?,
                spread_rounds: integer(value, "spread_rounds")?,
                extra_rare_growths: IntProvider::parse(&value["extra_rare_growths"])?,
                catalyst_chance: value["catalyst_chance"]
                    .as_f64()
                    .ok_or_else(|| FeatureError::InvalidConfig("sculk catalyst_chance".into()))?
                    as f32,
            }),
            "multiface_growth" => {
                if unqualified(string(value, "block")?) != "sculk_vein" {
                    return Err(FeatureError::Unsupported(
                        "non-sculk multiface block".into(),
                    ));
                }
                Self::Vein(
                    serde_json::from_value(value.clone())
                        .map_err(|e| FeatureError::InvalidConfig(format!("sculk vein: {e}")))?,
                )
            }
            _ => {
                return Err(FeatureError::Unsupported(format!(
                    "sculk feature type {feature_type}"
                )))
            }
        };
        config.validate(native()?)?;
        Ok(config)
    }

    fn validate(&self, data: &NativeData) -> Result<(), FeatureError> {
        match self {
            Self::Patch(c) => {
                for (name, value, min, max) in [
                    ("charge_count", c.charge_count, 1, 32),
                    ("amount_per_charge", c.amount_per_charge, 1, 500),
                    ("spread_attempts", c.spread_attempts, 1, 64),
                    ("growth_rounds", c.growth_rounds, 0, 8),
                    ("spread_rounds", c.spread_rounds, 0, 8),
                ] {
                    if !(min..=max).contains(&value) {
                        return Err(FeatureError::InvalidConfig(format!(
                            "sculk {name} must be {min}..={max}"
                        )));
                    }
                }
                probability(c.catalyst_chance, "catalyst_chance")?;
                c.extra_rare_growths.validate()
            }
            Self::Vein(c) => {
                if !(1..=64).contains(&c.search_range) {
                    return Err(FeatureError::InvalidConfig(
                        "sculk search_range must be 1..=64".into(),
                    ));
                }
                probability(c.chance_of_spreading, "chance_of_spreading")?;
                for name in &c.can_be_placed_on {
                    data.block(name)?;
                }
                Ok(())
            }
        }
    }
}

fn unqualified(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}
fn integer(value: &Value, key: &str) -> Result<i32, FeatureError> {
    value[key]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| FeatureError::InvalidConfig(format!("sculk integer {key}")))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, FeatureError> {
    value[key]
        .as_str()
        .ok_or_else(|| FeatureError::InvalidConfig(format!("sculk string {key}")))
}
fn probability(value: f32, key: &str) -> Result<(), FeatureError> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(FeatureError::InvalidConfig(format!(
            "sculk {key} must be 0..=1"
        )))
    }
}

/// Place with the caller's borrowed feature stream. Validation precedes all effects.
pub fn place<W: FeatureWorld + ?Sized>(
    world: &mut W,
    random: &mut WorldgenRandom,
    origin: Pos,
    config: &Config,
) -> Result<bool, FeatureError> {
    let data = native()?;
    config.validate(data)?;
    let mut engine = Engine {
        world,
        random,
        data,
    };
    match config {
        Config::Patch(config) => engine.patch(origin, config),
        Config::Vein(config) => engine.vein_feature(origin, config),
    }
}

pub fn place_named<W: FeatureWorld + ?Sized>(
    world: &mut W,
    random: &mut WorldgenRandom,
    origin: Pos,
    name: &str,
) -> Result<bool, FeatureError> {
    place(world, random, origin, &Config::named(name)?)
}

/// Generated defaults only. `typed_data` uses `[NBT tag ID, payload] and retains
/// the TAG_Long type of `listener.selector.tick`; plain JSON cannot retain it.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedBlockEntity {
    pub type_id: u32,
    pub full_data: Value,
    pub update_data: Value,
    pub typed_data: Value,
}

/// Required by a write adapter until its shared BlockEntity schema supports sculk.
/// Call on successful state writes, preserving existing compatible live entities.
pub fn generated_block_entity(
    state: u32,
    pos: Pos,
) -> Result<Option<GeneratedBlockEntity>, FeatureError> {
    let data = native()?;
    data.state(state)?;
    for template in &data.block_entities {
        let [start, end, _] = data.block(&template.block)?;
        if (start..end).contains(&state) {
            let mut full_data = template.nbt.clone();
            let mut typed_data = template.typed_nbt.clone();
            for (key, coordinate) in [("x", pos.0), ("y", pos.1), ("z", pos.2)] {
                full_data[key] = coordinate.into();
                typed_data[1][key][1] = coordinate.into();
            }
            return Ok(Some(GeneratedBlockEntity {
                type_id: template.type_id,
                full_data,
                typed_data,
                update_data: template.update.clone(),
            }));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChargeCursor {
    pub pos: Pos,
    pub charge: i32,
    pub update_delay: i32,
    pub decay_delay: i32,
    /// None is the initial same-space mode; Some(0) enables general spreading.
    pub facings: Option<u8>,
}

#[derive(Debug, Default)]
pub struct WorldgenSpreader {
    cursors: Vec<ChargeCursor>,
}

impl WorldgenSpreader {
    pub fn add_cursors(&mut self, pos: Pos, mut charge: i32) {
        while charge > 0 && self.cursors.len() < 32 {
            let amount = charge.min(1000);
            self.cursors.push(ChargeCursor {
                pos,
                charge: amount,
                update_delay: 0,
                decay_delay: 1,
                facings: None,
            });
            charge -= amount;
        }
    }

    pub fn cursors(&self) -> &[ChargeCursor] {
        &self.cursors
    }
    pub fn clear(&mut self) {
        self.cursors.clear();
    }

    pub fn update<W: FeatureWorld + ?Sized>(
        &mut self,
        world: &mut W,
        random: &mut WorldgenRandom,
        origin: Pos,
        spread_veins: bool,
    ) -> Result<(), FeatureError> {
        self.advance(
            &mut Engine {
                world,
                random,
                data: native()?,
            },
            origin,
            spread_veins,
        )
    }

    fn advance<W: FeatureWorld + ?Sized>(
        &mut self,
        engine: &mut Engine<'_, W>,
        origin: Pos,
        spread_veins: bool,
    ) -> Result<(), FeatureError> {
        for cursor in &mut self.cursors {
            if cursor
                .pos
                .0
                .abs_diff(origin.0)
                .max(cursor.pos.1.abs_diff(origin.1))
                .max(cursor.pos.2.abs_diff(origin.2))
                > 1024
            {
                cursor.charge = 0;
            } else {
                engine.update_cursor(cursor, origin, spread_veins)?;
            }
        }
        // Native worldgen cursors never merge, even when their positions coincide.
        self.cursors.retain(|cursor| cursor.charge > 0);
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct StateInfo {
    flags: u32,
    sturdy: u8,
    attach: u8,
    faces: u8,
}
impl StateInfo {
    fn has(self, mask: u32) -> bool {
        self.flags & mask != 0
    }
    fn face(self, dir: usize) -> bool {
        self.faces & (1 << dir) != 0
    }
    fn require_shape(self, state: u32) -> Result<Self, FeatureError> {
        if self.has(KNOWN_SHAPE) {
            Ok(self)
        } else {
            Err(FeatureError::Unsupported(format!(
                "sculk contextual collision/support shape for state {state}"
            )))
        }
    }
}

/// Native `MultifaceBlock.canAttachTo` for a neighbour's block state.
/// `toward_neighbour` is DOWN/UP/NORTH/SOUTH/WEST/EAST (0..6), measured from
/// the placement position toward the neighbour, not the neighbour's outward face.
/// Uncached shapes require the catalog's native-verified `KNOWN_SHAPE` flag;
/// stateful collision/support shapes remain explicit errors.
pub fn multiface_can_attach_to(
    neighbour_state: u32,
    toward_neighbour: usize,
) -> Result<bool, FeatureError> {
    if toward_neighbour >= DIRECTIONS.len() {
        return Err(FeatureError::InvalidConfig(format!(
            "multiface direction ordinal {toward_neighbour}"
        )));
    }
    let shape = native()?
        .state(neighbour_state)?
        .require_shape(neighbour_state)?;
    Ok(shape.attach & (1 << toward_neighbour) != 0)
}

#[derive(Deserialize)]
struct PlacementStates {
    sculk: u32,
    catalyst: u32,
    sensor: [u32; 2],
    shrieker: [u32; 2],
}
#[derive(Deserialize)]
struct EntityTemplate {
    block: String,
    type_id: u32,
    nbt: Value,
    typed_nbt: Value,
    update: Value,
}
#[derive(Deserialize)]
struct Capture {
    state_count: usize,
    ranges: Vec<[u32; 5]>,
    veins: Vec<[u32; 3]>,
    blocks: BTreeMap<String, [u32; 3]>,
    configurations: BTreeMap<String, Value>,
    placement_states: PlacementStates,
    block_entities: Vec<EntityTemplate>,
    non_corner_neighbours: [Pos; 18],
}
struct NativeData {
    states: Vec<StateInfo>,
    veins: [[u32; 64]; 2],
    blocks: BTreeMap<String, [u32; 3]>,
    configurations: BTreeMap<String, Value>,
    placement: PlacementStates,
    block_entities: Vec<EntityTemplate>,
    neighbours: [Pos; 18],
    air: u32,
    void_air: u32,
    water: u32,
}

impl NativeData {
    fn load() -> Result<Self, FeatureError> {
        let raw: Capture = serde_json::from_str(include_str!("../data/sculk_states_26_1.json"))
            .map_err(|e| FeatureError::MissingData(format!("sculk 26.1 capture: {e}")))?;
        let mut states = Vec::with_capacity(raw.state_count);
        for [start, end, flags, sturdy, attach] in raw.ranges {
            if start as usize != states.len() || end < start || end as usize > raw.state_count {
                return Err(FeatureError::MissingData(
                    "noncontiguous sculk state predicates".into(),
                ));
            }
            states.resize(
                end as usize,
                StateInfo {
                    flags,
                    sturdy: sturdy as u8,
                    attach: attach as u8,
                    faces: 0,
                },
            );
        }
        if states.len() != raw.state_count {
            return Err(FeatureError::MissingData(
                "incomplete sculk state predicates".into(),
            ));
        }
        let mut veins = [[u32::MAX; 64]; 2];
        for [state, faces, wet] in raw.veins {
            if state as usize >= states.len() || faces >= 64 || wet > 1 {
                return Err(FeatureError::MissingData("invalid sculk vein state".into()));
            }
            veins[wet as usize][faces as usize] = state;
            states[state as usize].faces = faces as u8;
        }
        if veins.iter().flatten().any(|&state| state == u32::MAX) {
            return Err(FeatureError::MissingData(
                "incomplete sculk vein faces".into(),
            ));
        }
        let default = |name: &str| {
            raw.blocks
                .get(name)
                .map(|row| row[2])
                .ok_or_else(|| FeatureError::MissingData(format!("sculk block {name}")))
        };
        let (air, void_air, water) = (
            default("minecraft:air")?,
            default("minecraft:void_air")?,
            default("minecraft:water")?,
        );
        Ok(Self {
            states,
            veins,
            blocks: raw.blocks,
            configurations: raw.configurations,
            placement: raw.placement_states,
            block_entities: raw.block_entities,
            neighbours: raw.non_corner_neighbours,
            air,
            void_air,
            water,
        })
    }

    fn state(&self, id: u32) -> Result<StateInfo, FeatureError> {
        self.states
            .get(id as usize)
            .copied()
            .ok_or_else(|| FeatureError::MissingData(format!("sculk state {id}")))
    }
    fn block(&self, name: &str) -> Result<[u32; 3], FeatureError> {
        let key = if name.contains(':') {
            name.to_owned()
        } else {
            format!("minecraft:{name}")
        };
        self.blocks.get(&key).copied().ok_or_else(|| {
            FeatureError::Unsupported(format!("sculk block name or holder set {name}"))
        })
    }
}

fn native() -> Result<&'static NativeData, FeatureError> {
    static DATA: OnceLock<Result<NativeData, FeatureError>> = OnceLock::new();
    DATA.get_or_init(NativeData::load)
        .as_ref()
        .map_err(Clone::clone)
}

struct Engine<'a, W: FeatureWorld + ?Sized> {
    world: &'a mut W,
    random: &'a mut WorldgenRandom,
    data: &'static NativeData,
}

fn offset(p: Pos, d: Pos) -> Pos {
    (
        p.0.wrapping_add(d.0),
        p.1.wrapping_add(d.1),
        p.2.wrapping_add(d.2),
    )
}
fn relative(p: Pos, dir: usize) -> Pos {
    offset(p, DIRECTIONS[dir])
}
fn distance_squared(a: Pos, b: Pos) -> f64 {
    let (x, y, z) = (
        f64::from(a.0) - f64::from(b.0),
        f64::from(a.1) - f64::from(b.1),
        f64::from(a.2) - f64::from(b.2),
    );
    x * x + y * y + z * z
}
fn shuffle<T>(values: &mut [T], random: &mut WorldgenRandom) {
    for i in (1..values.len()).rev() {
        values.swap(i, random.next_int(i + 1));
    }
}

impl<W: FeatureWorld + ?Sized> Engine<'_, W> {
    fn block(&self, pos: Pos) -> Result<u32, FeatureError> {
        if !(crate::MIN_Y..=crate::MAX_Y).contains(&pos.1) {
            return Ok(self.data.void_air);
        }
        self.world
            .get_block(pos)
            .ok_or_else(|| FeatureError::MissingData(format!("sculk neighbour at {pos:?}")))
    }
    fn info(&self, pos: Pos) -> Result<StateInfo, FeatureError> {
        self.data.state(self.block(pos)?)
    }
    fn shape(&self, pos: Pos) -> Result<StateInfo, FeatureError> {
        let state = self.block(pos)?;
        self.data.state(state)?.require_shape(state)
    }
    fn attach(&self, pos: Pos, dir: usize) -> Result<bool, FeatureError> {
        multiface_can_attach_to(self.block(relative(pos, dir))?, dir)
    }

    fn patch(&mut self, origin: Pos, config: &PatchConfig) -> Result<bool, FeatureError> {
        let start = self.info(origin)?;
        if !start.has(BEHAVIOUR) {
            if !start.has(AIR) && !(start.has(WATER_BLOCK) && start.has(WATER_FLUID)) {
                return Ok(false);
            }
            let mut supported = false;
            for dir in 0..6 {
                if self.shape(relative(origin, dir))?.has(FULL_COLLISION) {
                    supported = true;
                    break;
                }
            }
            if !supported {
                return Ok(false);
            }
        }
        let mut spreader = WorldgenSpreader::default();
        for round in 0..config.spread_rounds + config.growth_rounds {
            for _ in 0..config.charge_count {
                spreader.add_cursors(origin, config.amount_per_charge);
            }
            for _ in 0..config.spread_attempts {
                spreader.advance(self, origin, round < config.spread_rounds)?;
            }
            spreader.clear();
        }
        if self.random.next_float() <= config.catalyst_chance
            && self.shape(relative(origin, 0))?.has(FULL_COLLISION)
        {
            self.world
                .set_feature_block(origin, self.data.placement.catalyst, 3);
        }
        let extra = config.extra_rare_growths.sample(self.random);
        for _ in 0..extra {
            let candidate = offset(
                origin,
                (
                    self.random.next_int(5) as i32 - 2,
                    0,
                    self.random.next_int(5) as i32 - 2,
                ),
            );
            if self.info(candidate)?.has(AIR)
                && self.shape(relative(candidate, 0))?.sturdy & (1 << 1) != 0
            {
                self.world
                    .set_feature_block(candidate, self.data.placement.shrieker[0], 3);
            }
        }
        Ok(true)
    }

    fn placement_state(
        &self,
        old: u32,
        pos: Pos,
        face: usize,
    ) -> Result<Option<u32>, FeatureError> {
        let info = self.data.state(old)?;
        if info.has(VEIN) && info.face(face) || !self.attach(pos, face)? {
            return Ok(None);
        }
        let faces = if info.has(VEIN) { info.faces } else { 0 } | (1 << face);
        Ok(Some(
            self.data.veins[usize::from(info.has(WATER_FLUID))][faces as usize],
        ))
    }

    fn can_spread_into(&self, source: Pos, pos: Pos, face: usize) -> Result<bool, FeatureError> {
        let existing = self.info(pos)?;
        if self
            .info(relative(pos, face))?
            .has(SCULK | CATALYST | MOVING_PISTON)
        {
            return Ok(false);
        }
        let manhattan =
            source.0.abs_diff(pos.0) + source.1.abs_diff(pos.1) + source.2.abs_diff(pos.2);
        if manhattan == 2 && self.shape(relative(source, face ^ 1))?.sturdy & (1 << face) != 0 {
            return Ok(false);
        }
        if existing.has(FLUID) && !existing.has(WATER_FLUID) || existing.has(FIRE) {
            return Ok(false);
        }
        if !existing.has(REPLACEABLE | AIR | VEIN)
            && !(existing.has(WATER_BLOCK) && existing.has(WATER_FLUID))
        {
            return Ok(false);
        }
        if existing.has(VEIN) && existing.face(face) {
            return Ok(false);
        }
        self.attach(pos, face)
    }

    fn spread_direction(
        &mut self,
        state: u32,
        source: Pos,
        from: usize,
        toward: usize,
        same_space: bool,
    ) -> Result<bool, FeatureError> {
        let info = self.data.state(state)?;
        if from / 2 == toward / 2 || info.has(VEIN) && (!info.face(from) || info.face(toward)) {
            return Ok(false);
        }
        let candidates = [
            (source, toward),
            (relative(source, toward), from),
            (relative(relative(source, toward), from), toward ^ 1),
        ];
        for &(pos, face) in &candidates[..if same_space { 1 } else { 3 }] {
            if !self.can_spread_into(source, pos, face)? {
                continue;
            }
            if let Some(state) = self.placement_state(self.block(pos)?, pos, face)? {
                self.world.mark_feature_postprocessing(pos);
                return Ok(self.world.set_feature_block(pos, state, 2));
            }
            return Ok(false);
        }
        Ok(false)
    }

    fn spread_all(&mut self, state: u32, pos: Pos, same_space: bool) -> Result<bool, FeatureError> {
        let info = self.data.state(state)?;
        let mut placed = false;
        for from in 0..6 {
            if info.has(VEIN) && !info.face(from) {
                continue;
            }
            for toward in 0..6 {
                placed |= self.spread_direction(state, pos, from, toward, same_space)?;
            }
        }
        Ok(placed)
    }

    fn spread_cursor(&mut self, cursor: &ChargeCursor, state: u32) -> Result<bool, FeatureError> {
        let info = self.data.state(state)?;
        if info.has(BEHAVIOUR) || cursor.facings == Some(0) {
            return self.spread_all(state, cursor.pos, false);
        }
        let Some(faces) = cursor.facings else {
            return self.spread_all(state, cursor.pos, true);
        };
        if !info.has(AIR | WATER_FLUID) {
            return Ok(false);
        }
        let mut attached = 0;
        for dir in 0..6 {
            if faces & (1 << dir) != 0 && self.attach(cursor.pos, dir)? {
                attached |= 1 << dir;
            }
        }
        if attached == 0 {
            return Ok(false);
        }
        let state = self.data.veins[usize::from(info.has(FLUID))][attached];
        self.world.set_feature_block(cursor.pos, state, 3);
        // regrow reports attachment success, independent of setBlock's result.
        Ok(true)
    }

    fn discharge(&mut self, pos: Pos, state: u32) -> Result<(), FeatureError> {
        let info = self.data.state(state)?;
        if !info.has(VEIN) {
            return Ok(());
        }
        let mut faces = info.faces;
        for dir in 0..6 {
            if faces & (1 << dir) != 0 && self.info(relative(pos, dir))?.has(SCULK) {
                faces &= !(1 << dir);
            }
        }
        let state = if faces == 0 {
            if self.info(pos)?.has(FLUID) {
                self.data.water
            } else {
                self.data.air
            }
        } else {
            self.data.veins[usize::from(info.has(WATER_FLUID))][faces as usize]
        };
        self.world.set_feature_block(pos, state, 3);
        Ok(())
    }

    fn convert_substrate(&mut self, pos: Pos) -> Result<bool, FeatureError> {
        let info = self.info(pos)?;
        let mut directions = [0, 1, 2, 3, 4, 5];
        shuffle(&mut directions, self.random);
        for dir in directions {
            if !info.face(dir) {
                continue;
            }
            let target = relative(pos, dir);
            if !self.info(target)?.has(WORLDGEN_SUBSTRATE) {
                continue;
            }
            let sculk = self.data.placement.sculk;
            self.world.set_feature_block(target, sculk, 3);
            self.spread_all(sculk, target, false)?;
            for neighbour in 0..6 {
                if neighbour == dir ^ 1 {
                    continue;
                }
                let adjacent = relative(target, neighbour);
                let state = self.block(adjacent)?;
                if self.data.state(state)?.has(VEIN) {
                    self.discharge(adjacent, state)?;
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn can_grow(&self, pos: Pos) -> Result<bool, FeatureError> {
        let above = self.info(relative(pos, 1))?;
        if !above.has(AIR) && !(above.has(WATER_BLOCK) && above.has(WATER_FLUID)) {
            return Ok(false);
        }
        let mut growths = 0;
        for z in -4..=4 {
            for y in 0..=2 {
                for x in -4..=4 {
                    if self.info(offset(pos, (x, y, z)))?.has(GROWTH) {
                        growths += 1;
                    }
                    if growths > 2 {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    fn sculk_charge(&mut self, cursor: &ChargeCursor, origin: Pos) -> Result<i32, FeatureError> {
        let charge = cursor.charge;
        if charge == 0 || self.random.next_int(5) != 0 {
            return Ok(charge);
        }
        let distance = distance_squared(cursor.pos, origin);
        let near = distance < 1.0;
        if !near && self.can_grow(cursor.pos)? {
            if (self.random.next_int(50) as i32) < charge {
                let states = if self.random.next_int(11) == 0 {
                    self.data.placement.shrieker
                } else {
                    self.data.placement.sensor
                };
                let above = relative(cursor.pos, 1);
                let state = states[usize::from(self.info(above)?.has(FLUID))];
                self.world.set_feature_block(above, state, 3);
            }
            return Ok((charge - 50).max(0));
        }
        if self.random.next_int(10) != 0 {
            return Ok(charge);
        }
        let penalty = if near {
            1
        } else {
            let outside = distance.sqrt() as f32 - 1.0;
            let factor = (outside * outside / 529.0_f32).min(1.0);
            (charge as f32 * factor * 0.5_f32) as i32
        };
        Ok(charge - penalty.max(1))
    }

    fn substrate_access(&self, pos: Pos, info: StateInfo) -> Result<bool, FeatureError> {
        if !info.has(VEIN) {
            return Ok(false);
        }
        for dir in 0..6 {
            if info.face(dir) && self.info(relative(pos, dir))?.has(SUBSTRATE) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn open_step(&self, pos: Pos, dir: usize) -> Result<bool, FeatureError> {
        Ok(self.shape(relative(pos, dir))?.sturdy & (1 << (dir ^ 1)) == 0)
    }

    fn movement(&mut self, pos: Pos) -> Result<Option<Pos>, FeatureError> {
        let mut offsets = self.data.neighbours;
        shuffle(&mut offsets, self.random);
        let mut candidate = None;
        for delta in offsets {
            let to = offset(pos, delta);
            let info = self.info(to)?;
            if !info.has(BEHAVIOUR) {
                continue;
            }
            if delta.0.abs() + delta.1.abs() + delta.2.abs() != 1 {
                let x = if delta.0 < 0 { 4 } else { 5 };
                let y = if delta.1 < 0 { 0 } else { 1 };
                let z = if delta.2 < 0 { 2 } else { 3 };
                let (a, b) = if delta.0 == 0 {
                    (y, z)
                } else if delta.1 == 0 {
                    (x, z)
                } else {
                    (x, y)
                };
                if !self.open_step(pos, a)? && !self.open_step(pos, b)? {
                    continue;
                }
            }
            candidate = Some(to);
            if self.substrate_access(to, info)? {
                break;
            }
        }
        Ok(candidate)
    }

    fn update_cursor(
        &mut self,
        cursor: &mut ChargeCursor,
        origin: Pos,
        spread: bool,
    ) -> Result<(), FeatureError> {
        if cursor.charge <= 0 {
            return Ok(());
        }
        if cursor.update_delay > 0 {
            cursor.update_delay -= 1;
            return Ok(());
        }
        let mut state = self.block(cursor.pos)?;
        let mut info = self.data.state(state)?;
        if spread && self.spread_cursor(cursor, state)? && !info.has(SCULK) {
            state = self.block(cursor.pos)?;
            info = self.data.state(state)?;
        }
        cursor.charge = if info.has(SCULK) {
            self.sculk_charge(cursor, origin)?
        } else if info.has(VEIN) {
            if spread && self.convert_substrate(cursor.pos)? {
                cursor.charge - 1
            } else if self.random.next_int(5) == 0 {
                cursor.charge / 2
            } else {
                cursor.charge
            }
        } else if cursor.decay_delay > 0 {
            cursor.charge
        } else {
            0
        };
        if cursor.charge <= 0 {
            return self.discharge(cursor.pos, state);
        }
        if let Some(to) = self.movement(cursor.pos)? {
            self.discharge(cursor.pos, state)?;
            cursor.pos = to;
            if distance_squared(to, (origin.0, to.1, origin.2)) >= 225.0 {
                cursor.charge = 0;
                return Ok(());
            }
            state = self.block(to)?;
        }
        let current = self.data.state(state)?;
        if current.has(BEHAVIOUR) {
            cursor.facings = Some(current.faces);
        }
        cursor.decay_delay = if info.has(BEHAVIOUR) {
            1
        } else {
            (cursor.decay_delay - 1).max(0)
        };
        cursor.update_delay = 1;
        Ok(())
    }

    fn vein_feature(&mut self, origin: Pos, config: &VeinConfig) -> Result<bool, FeatureError> {
        if !self.info(origin)?.has(AIR | WATER_BLOCK) {
            return Ok(false);
        }
        let directions: Vec<usize> = [1, 0, 2, 5, 3, 4]
            .into_iter()
            .filter(|dir| match dir {
                0 => config.can_place_on_floor,
                1 => config.can_place_on_ceiling,
                _ => config.can_place_on_wall,
            })
            .collect();
        let supports = config
            .can_be_placed_on
            .iter()
            .map(|name| self.data.block(name))
            .collect::<Result<Vec<_>, _>>()?;
        let mut shuffled = directions.clone();
        shuffle(&mut shuffled, self.random);
        if self.place_vein_growth(origin, config.chance_of_spreading, &supports, &shuffled)? {
            return Ok(true);
        }
        for dir in shuffled {
            let mut faces: Vec<_> = directions
                .iter()
                .copied()
                .filter(|&face| face != dir ^ 1)
                .collect();
            shuffle(&mut faces, self.random);
            for _ in 0..config.search_range {
                // Native setWithOffset resets from origin on every iteration.
                let target = relative(origin, dir);
                if !self.info(target)?.has(AIR | WATER_BLOCK | VEIN) {
                    break;
                }
                if self.place_vein_growth(target, config.chance_of_spreading, &supports, &faces)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn place_vein_growth(
        &mut self,
        pos: Pos,
        chance: f32,
        supports: &[[u32; 3]],
        faces: &[usize],
    ) -> Result<bool, FeatureError> {
        let old = self.block(pos)?;
        for &face in faces {
            let support = self.block(relative(pos, face))?;
            if !supports
                .iter()
                .any(|&[start, end, _]| (start..end).contains(&support))
            {
                continue;
            }
            let Some(state) = self.placement_state(old, pos, face)? else {
                return Ok(false);
            };
            self.world.set_feature_block(pos, state, 3);
            self.world.mark_feature_postprocessing(pos);
            if self.random.next_float() < chance {
                let mut directions = [0, 1, 2, 3, 4, 5];
                shuffle(&mut directions, self.random);
                for dir in directions {
                    if self.spread_direction(state, pos, face, dir, false)? {
                        break;
                    }
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}
