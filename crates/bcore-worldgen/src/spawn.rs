//! Java 26.1 fresh-chunk creature generation (`NaturalSpawner`, protocol 775).
//!
//! Placement has a legacy RNG, independent from the region RNG used by spawn
//! rules/finalization and from each entity's externally supplied entropy. The
//! latter produces the UUID; a world seed alone cannot determine native UUIDs.
//! Runtime stage ownership, conversion to LevelChunk, and protocol delivery are
//! deliberately left to the generation scheduler. Only finalized mobs are output.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::biome::BiomeId;
use crate::block_predicate::{catalog as blocks, FeatureResult};
use crate::feature_world::{FeatureHeightmap, FeatureWorld, Pos};
use crate::noise_perlin::Xoroshiro;
use crate::simplex::JavaRandom;

pub type SpawnResult<T> = Result<T, SpawnError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    MissingData(String),
    Unsupported(String),
    InvalidSettings(String),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingData(s) => write!(f, "missing generation-spawn data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported generation-spawn callback: {s}"),
            Self::InvalidSettings(s) => write!(f, "invalid generation-spawn settings: {s}"),
        }
    }
}
impl std::error::Error for SpawnError {}

fn feature<T>(result: FeatureResult<T>) -> SpawnResult<T> {
    result.map_err(|e| SpawnError::MissingData(e.to_string()))
}

/// NBT types are retained, including float vs double, shorts, and int arrays.
/// This is an entity result, not protocol encoding or a queued factory request.
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnTag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Self>),
    Compound(BTreeMap<String, Self>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl SpawnTag {
    pub fn from_native_json(value: &Value) -> SpawnResult<Self> {
        let invalid = || SpawnError::MissingData("invalid native typed NBT".into());
        let row = value
            .as_array()
            .filter(|v| v.len() == 2)
            .ok_or_else(invalid)?;
        let p = &row[1];
        Ok(match row[0].as_u64().ok_or_else(invalid)? {
            1 => Self::Byte(i8::try_from(p.as_i64().ok_or_else(invalid)?).map_err(|_| invalid())?),
            2 => {
                Self::Short(i16::try_from(p.as_i64().ok_or_else(invalid)?).map_err(|_| invalid())?)
            }
            3 => Self::Int(i32::try_from(p.as_i64().ok_or_else(invalid)?).map_err(|_| invalid())?),
            4 => Self::Long(p.as_i64().ok_or_else(invalid)?),
            5 => Self::Float(p.as_f64().ok_or_else(invalid)? as f32),
            6 => Self::Double(p.as_f64().ok_or_else(invalid)?),
            7 => Self::ByteArray(serde_json::from_value(p.clone()).map_err(|_| invalid())?),
            8 => Self::String(p.as_str().ok_or_else(invalid)?.into()),
            9 => Self::List(
                p.as_array()
                    .ok_or_else(invalid)?
                    .iter()
                    .map(Self::from_native_json)
                    .collect::<SpawnResult<_>>()?,
            ),
            10 => Self::Compound(
                p.as_object()
                    .ok_or_else(invalid)?
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), Self::from_native_json(v)?)))
                    .collect::<SpawnResult<_>>()?,
            ),
            11 => Self::IntArray(serde_json::from_value(p.clone()).map_err(|_| invalid())?),
            12 => Self::LongArray(serde_json::from_value(p.clone()).map_err(|_| invalid())?),
            _ => return Err(invalid()),
        })
    }

    /// Native probe's `[tag ID, payload]` representation.
    pub fn native_json(&self) -> Value {
        match self {
            Self::Byte(v) => json!([1, v]),
            Self::Short(v) => json!([2, v]),
            Self::Int(v) => json!([3, v]),
            Self::Long(v) => json!([4, v]),
            Self::Float(v) => json!([5, v]),
            Self::Double(v) => json!([6, v]),
            Self::ByteArray(v) => json!([7, v]),
            Self::String(v) => json!([8, v]),
            Self::List(v) => json!([9, v.iter().map(Self::native_json).collect::<Vec<_>>()]),
            Self::Compound(v) => json!([
                10,
                v.iter()
                    .map(|(k, v)| (k, v.native_json()))
                    .collect::<BTreeMap<_, _>>()
            ]),
            Self::IntArray(v) => json!([11, v]),
            Self::LongArray(v) => json!([12, v]),
        }
    }

    pub fn data(&self) -> Value {
        match self {
            Self::List(v) => v.iter().map(Self::data).collect(),
            Self::Compound(v) => v.iter().map(|(k, v)| (k.clone(), v.data())).collect(),
            _ => self.native_json()[1].clone(),
        }
    }

    fn put(&mut self, key: &str, value: Self) {
        let Self::Compound(values) = self else {
            panic!("native compound template");
        };
        values.insert(key.into(), value);
    }

    fn field(&self, key: &str) -> &Self {
        let Self::Compound(values) = self else {
            panic!("native compound template");
        };
        &values[key]
    }

    fn field_mut(&mut self, key: &str) -> &mut Self {
        let Self::Compound(values) = self else {
            panic!("native compound template");
        };
        values.get_mut(key).expect("native compound field")
    }
}

#[derive(Clone)]
enum RandomSource {
    Legacy(JavaRandom),
    Xoroshiro(Xoroshiro),
}

/// Exact primitive draws, for optional differential execution evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnDraw {
    Int { bound: i32, value: i32 },
    Long(i64),
    Float(u32),
    Double(u64),
    Boolean(bool),
}

impl SpawnDraw {
    pub fn native_trace(&self) -> String {
        match self {
            Self::Int { bound, value } => format!("i:{bound}:{value}"),
            Self::Long(v) => format!("l:{v}"),
            Self::Float(v) => format!("f:{v}"),
            Self::Double(v) => format!("d:{v}"),
            Self::Boolean(v) => format!("b:{v}"),
        }
    }
}

/// A continuation-owning adapter over existing native-compatible generators.
/// Tracing does not consume extra draws. `peek_next_long` clones the stream.
#[derive(Clone)]
pub struct SpawnRandom {
    source: RandomSource,
    trace: Option<Vec<SpawnDraw>>,
}

impl SpawnRandom {
    pub fn legacy(seed: i64) -> Self {
        Self {
            source: RandomSource::Legacy(JavaRandom::new(seed)),
            trace: None,
        }
    }

    pub fn xoroshiro(seed: i64) -> Self {
        Self {
            source: RandomSource::Xoroshiro(Xoroshiro::new(seed)),
            trace: None,
        }
    }

    /// `WorldGenRegion` creates a fresh named positional stream for each stage.
    pub fn for_region(world_seed: i64, chunk: [i32; 2]) -> Self {
        let source = Xoroshiro::new(world_seed)
            .fork_positional()
            .from_hash_of("minecraft:worldgen_region_random")
            .fork_positional()
            .at(chunk[0].wrapping_mul(16), 0, chunk[1].wrapping_mul(16));
        Self {
            source: RandomSource::Xoroshiro(source),
            trace: None,
        }
    }

    /// `WorldgenRandom(LegacyRandomSource).setDecorationSeed`, not Xoroshiro.
    pub fn for_chunk(world_seed: i64, chunk: [i32; 2]) -> (i64, Self) {
        let mut seed_random = JavaRandom::new(world_seed);
        let x_scale = seed_random.next_long() | 1;
        let z_scale = seed_random.next_long() | 1;
        let seed = i64::from(chunk[0].wrapping_mul(16))
            .wrapping_mul(x_scale)
            .wrapping_add(i64::from(chunk[1].wrapping_mul(16)).wrapping_mul(z_scale))
            ^ world_seed;
        (seed, Self::legacy(seed))
    }

    pub fn trace(mut self) -> Self {
        self.trace = Some(Vec::new());
        self
    }

    pub fn draws(&self) -> &[SpawnDraw] {
        self.trace.as_deref().unwrap_or_default()
    }

    pub fn peek_next_long(&self) -> i64 {
        self.clone().next_long()
    }

    fn record(&mut self, draw: SpawnDraw) {
        if let Some(trace) = &mut self.trace {
            trace.push(draw);
        }
    }

    pub fn next_int(&mut self, bound: i32) -> i32 {
        assert!(bound > 0);
        let value = match &mut self.source {
            RandomSource::Legacy(r) => r.next_int(bound as usize) as i32,
            RandomSource::Xoroshiro(r) => r.next_int(bound as u32) as i32,
        };
        self.record(SpawnDraw::Int { bound, value });
        value
    }

    pub fn next_long(&mut self) -> i64 {
        let value = match &mut self.source {
            RandomSource::Legacy(r) => r.next_long(),
            RandomSource::Xoroshiro(r) => r.next_long() as i64,
        };
        self.record(SpawnDraw::Long(value));
        value
    }

    pub fn next_float(&mut self) -> f32 {
        let value = match &mut self.source {
            RandomSource::Legacy(r) => r.next_float(),
            RandomSource::Xoroshiro(r) => r.next_float(),
        };
        self.record(SpawnDraw::Float(value.to_bits()));
        value
    }

    pub fn next_double(&mut self) -> f64 {
        let value = match &mut self.source {
            RandomSource::Legacy(r) => r.next_double(),
            RandomSource::Xoroshiro(r) => r.next_double(),
        };
        self.record(SpawnDraw::Double(value.to_bits()));
        value
    }

    pub fn next_bool(&mut self) -> bool {
        let value = match &mut self.source {
            RandomSource::Legacy(r) => r.next_bool(),
            RandomSource::Xoroshiro(r) => r.next_long() & 1 != 0,
        };
        self.record(SpawnDraw::Boolean(value));
        value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobKind {
    Armadillo,
    Camel,
    Chicken,
    Cow,
    Donkey,
    Fox,
    Frog,
    Goat,
    Horse,
    Llama,
    Mooshroom,
    Panda,
    Parrot,
    Pig,
    PolarBear,
    Rabbit,
    Sheep,
    Strider,
    Turtle,
    Wolf,
}

impl MobKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Armadillo => "minecraft:armadillo",
            Self::Camel => "minecraft:camel",
            Self::Chicken => "minecraft:chicken",
            Self::Cow => "minecraft:cow",
            Self::Donkey => "minecraft:donkey",
            Self::Fox => "minecraft:fox",
            Self::Frog => "minecraft:frog",
            Self::Goat => "minecraft:goat",
            Self::Horse => "minecraft:horse",
            Self::Llama => "minecraft:llama",
            Self::Mooshroom => "minecraft:mooshroom",
            Self::Panda => "minecraft:panda",
            Self::Parrot => "minecraft:parrot",
            Self::Pig => "minecraft:pig",
            Self::PolarBear => "minecraft:polar_bear",
            Self::Rabbit => "minecraft:rabbit",
            Self::Sheep => "minecraft:sheep",
            Self::Strider => "minecraft:strider",
            Self::Turtle => "minecraft:turtle",
            Self::Wolf => "minecraft:wolf",
        }
    }

    pub fn from_name(name: &str) -> SpawnResult<Self> {
        serde_json::from_value(json!(name.strip_prefix("minecraft:").unwrap_or(name)))
            .map_err(|_| SpawnError::Unsupported(format!("entity type {name}")))
    }

    pub fn supports_finalization(self) -> bool {
        matches!(
            self,
            Self::Armadillo
                | Self::Camel
                | Self::Chicken
                | Self::Cow
                | Self::Donkey
                | Self::Frog
                | Self::Goat
                | Self::Horse
                | Self::Llama
                | Self::Mooshroom
                | Self::Parrot
                | Self::Pig
                | Self::PolarBear
                | Self::Rabbit
                | Self::Sheep
                | Self::Turtle
                | Self::Wolf
                | Self::Fox
                | Self::Panda
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnerData {
    pub kind: MobKind,
    pub weight: i32,
    pub min_count: i32,
    pub max_count: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BiomeSpawns {
    pub probability: f32,
    /// Native list order and duplicate entries are significant.
    pub groups: Vec<SpawnerData>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl SpawnBox {
    fn at(self, position: [f64; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i] + position[i]),
            max: std::array::from_fn(|i| self.max[i] + position[i]),
        }
    }

    fn intersects(self, other: Self, epsilon: f64) -> bool {
        (0..3).all(|i| self.max[i].min(other.max[i]) - self.min[i].max(other.min[i]) > epsilon)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnPlacement {
    OnGround,
    NoRestrictions,
    InWater,
    InLava,
}

#[derive(Debug, Clone)]
pub struct SpawnType {
    pub kind: MobKind,
    pub registry_id: u32,
    pub width: f32,
    pub height: f32,
    pub follow_range: f64,
    pub spawn_box: SpawnBox,
    pub can_summon: bool,
    pub placement: SpawnPlacement,
    pub heightmap: FeatureHeightmap,
    mask: u64,
}

#[derive(Clone, Copy)]
struct StateData {
    shape: usize,
    flags: u8,
    floor: u64,
    empty: u64,
}

struct SpawnCatalog {
    biomes: BTreeMap<BiomeId, BiomeSpawns>,
    biome_tags: BTreeMap<BiomeId, Vec<String>>,
    types: BTreeMap<MobKind, SpawnType>,
    templates: BTreeMap<MobKind, SpawnTag>,
    sounds: BTreeMap<String, Vec<String>>,
    variants: BTreeMap<String, BTreeMap<BiomeId, Vec<String>>>,
    sensors: BTreeMap<MobKind, Vec<(String, i32)>>,
    memory_intervals: BTreeMap<String, [i32; 2]>,
    states: Vec<StateData>,
    shapes: Vec<Vec<SpawnBox>>,
    offsets: BTreeMap<u32, [f32; 2]>,
}

fn catalog() -> &'static SpawnCatalog {
    static CATALOG: OnceLock<SpawnCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let data: Value =
            serde_json::from_str(include_str!("../data/generation_spawn_assets_26_1_v2.json"))
                .expect("native generation-spawn catalog");
        let callbacks: Value = serde_json::from_str(include_str!(
            "../data/generation_spawn_callbacks_26_1_v1.json"
        ))
        .expect("native generation-spawn callback catalog");
        let mut biomes = BTreeMap::new();
        let mut biome_tags = BTreeMap::new();
        for row in data["catalog"]["biomes"].as_array().expect("native biomes") {
            let id =
                crate::biome::id(row["name"].as_str().unwrap()).expect("shared biome identity");
            // Dynamic registry ordering is a native server input; join it to
            // BCore's shared biome identities by resource key, never raw ID.
            let groups = row["spawns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| SpawnerData {
                    kind: MobKind::from_name(g["type"].as_str().unwrap()).unwrap(),
                    weight: g["weight"].as_i64().unwrap() as i32,
                    min_count: g["min"].as_i64().unwrap() as i32,
                    max_count: g["max"].as_i64().unwrap() as i32,
                })
                .collect();
            biomes.insert(
                id,
                BiomeSpawns {
                    probability: row["probability"].as_f64().unwrap() as f32,
                    groups,
                },
            );
            biome_tags.insert(id, serde_json::from_value(row["tags"].clone()).unwrap());
        }
        let mut types = BTreeMap::new();
        for (index, row) in data["catalog"]["types"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let kind = MobKind::from_name(row["name"].as_str().unwrap()).unwrap();
            let b: [f64; 6] = serde_json::from_value(row["spawn_box"].clone()).unwrap();
            types.insert(
                kind,
                SpawnType {
                    kind,
                    registry_id: row["id"].as_u64().unwrap() as u32,
                    width: row["width"].as_f64().unwrap() as f32,
                    height: row["height"].as_f64().unwrap() as f32,
                    follow_range: row["follow_range"].as_f64().unwrap(),
                    spawn_box: SpawnBox {
                        min: [b[0], b[1], b[2]],
                        max: [b[3], b[4], b[5]],
                    },
                    can_summon: row["summon"].as_bool().unwrap(),
                    placement: match row["placement"].as_str().unwrap() {
                        "ON_GROUND" => SpawnPlacement::OnGround,
                        "NO_RESTRICTIONS" => SpawnPlacement::NoRestrictions,
                        "IN_WATER" => SpawnPlacement::InWater,
                        "IN_LAVA" => SpawnPlacement::InLava,
                        _ => panic!("unknown native placement"),
                    },
                    heightmap: match row["heightmap"].as_str().unwrap() {
                        "MOTION_BLOCKING_NO_LEAVES" => FeatureHeightmap::MotionBlockingNoLeaves,
                        "MOTION_BLOCKING" => FeatureHeightmap::MotionBlocking,
                        _ => panic!("unknown native creature heightmap"),
                    },
                    mask: 1 << index,
                },
            );
        }
        let templates = data["constructors"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, row)| {
                (
                    MobKind::from_name(name).unwrap(),
                    SpawnTag::from_native_json(&row["entity"]["typed_nbt"]).unwrap(),
                )
            })
            .collect();
        let mut states = Vec::new();
        for row in data["blocks"]["runs"].as_array().unwrap() {
            let [start, end, shape, flags, floor, empty]: [u64; 6] =
                serde_json::from_value(row.clone()).unwrap();
            assert_eq!(start as usize, states.len());
            assert!(end > start);
            states.resize(
                end as usize,
                StateData {
                    shape: shape as usize,
                    flags: flags as u8,
                    floor,
                    empty,
                },
            );
        }
        assert_eq!(
            states.len(),
            data["blocks"]["state_count"].as_u64().unwrap() as usize
        );
        let shapes = data["blocks"]["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|shape| {
                shape
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| {
                        let v: [f64; 6] = serde_json::from_value(b.clone()).unwrap();
                        SpawnBox {
                            min: [v[0], v[1], v[2]],
                            max: [v[3], v[4], v[5]],
                        }
                    })
                    .collect()
            })
            .collect();
        let variants = callbacks["catalog"]["variant_candidates"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, by_biome)| {
                (
                    key.clone(),
                    by_biome
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(biome, names)| {
                            (
                                crate::biome::id(biome).unwrap(),
                                serde_json::from_value(names.clone()).unwrap(),
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        let sensors = callbacks["constructors"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, row)| {
                let sensors = row["entity"]["sensors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|sensor| {
                        (
                            sensor["name"].as_str().unwrap().to_owned(),
                            sensor["scan_rate"].as_i64().unwrap() as i32,
                        )
                    })
                    .collect();
                (MobKind::from_name(name).unwrap(), sensors)
            })
            .collect();
        SpawnCatalog {
            biomes,
            biome_tags,
            types,
            templates,
            states,
            shapes,
            variants,
            sensors,
            memory_intervals: serde_json::from_value(callbacks["memory_intervals"].clone())
                .unwrap(),
            offsets: serde_json::from_value(data["blocks"]["offsets"].clone()).unwrap(),
            sounds: serde_json::from_value(data["catalog"]["sound_variants"].clone()).unwrap(),
        }
    })
}

pub fn biome_spawns(id: BiomeId) -> SpawnResult<&'static BiomeSpawns> {
    catalog()
        .biomes
        .get(&id)
        .ok_or_else(|| SpawnError::MissingData(format!("biome {id}")))
}

pub fn spawn_type(kind: MobKind) -> &'static SpawnType {
    &catalog().types[&kind]
}

fn state_data(state: u32) -> SpawnResult<StateData> {
    catalog()
        .states
        .get(state as usize)
        .copied()
        .ok_or_else(|| SpawnError::MissingData(format!("state {state}")))
}

fn biome_tag(biome: BiomeId, tag: &str) -> SpawnResult<bool> {
    let tags = catalog()
        .biome_tags
        .get(&biome)
        .ok_or_else(|| SpawnError::MissingData(format!("biome tags {biome}")))?;
    Ok(tags
        .iter()
        .any(|t| t.strip_prefix("minecraft:").unwrap_or(t) == tag))
}

/// Exact WorldBorder block-position test (max edges exclusive, min edges open
/// against the block's upper face). Y is not part of this predicate.
#[derive(Debug, Clone, Copy)]
pub struct SpawnBorder {
    pub min_x: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_z: f64,
}

impl SpawnBorder {
    pub fn contains(self, (x, _, z): Pos) -> bool {
        f64::from(x) + 1.0 > self.min_x
            && f64::from(x) < self.max_x
            && f64::from(z) + 1.0 > self.min_z
            && f64::from(z) < self.max_z
    }
}

impl Default for SpawnBorder {
    fn default() -> Self {
        Self {
            min_x: -29_999_984.0,
            min_z: -29_999_984.0,
            max_x: 29_999_984.0,
            max_z: 29_999_984.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnDifficulty {
    /// Native Difficulty ID: peaceful=0, easy=1, normal=2, hard=3.
    pub difficulty: u8,
    pub overworld_time: i64,
    pub moon_brightness: f32,
    /// WorldGenRegion supplies zero inhabited time, even on a populated neighbour.
    pub inhabited_time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryAdmission {
    Available,
    Null,
    Exception(String),
}

/// Missing inputs fail explicitly; no pending factory request counts as a mob.
/// Default entity collision semantics are those of a fresh WorldGenRegion:
/// proto-entity NBT does not enter the live entity collision index.
pub trait SpawnEnvironment {
    fn spawn_mobs(&self) -> bool;
    fn random(&mut self) -> &mut SpawnRandom;
    /// Native `RandomSource.create()` input, independent of world/region seeds.
    fn entity_seed(&mut self, kind: MobKind) -> SpawnResult<i64>;
    fn raw_brightness(
        &self,
        world: &dyn FeatureWorld,
        pos: Pos,
        sky_darken: i32,
    ) -> SpawnResult<i32>;
    fn current_difficulty_at(&self, pos: Pos) -> SpawnResult<SpawnDifficulty>;

    /// The generation region's real addFreshEntity handoff, after finalization.
    /// A retained-world adapter stores the proto save here so a later error or
    /// unwind cannot discard mobs which were already successfully added.
    fn add_fresh_mob(&mut self, _mob: &SpawnedMob) -> SpawnResult<()> {
        Ok(())
    }

    /// `ServerLevel.getGameTime()`, used by Camel's initial standing pose. This
    /// is a separate input from DifficultyInstance's overworld-time calculation.
    fn game_time(&self) -> SpawnResult<i64> {
        Err(SpawnError::MissingData("ServerLevel game time".into()))
    }

    fn bounds(&self) -> (i32, i32) {
        (crate::MIN_Y, crate::MAX_Y + 1)
    }
    fn has_ceiling(&self) -> bool {
        false
    }
    fn border(&self) -> SpawnBorder {
        SpawnBorder::default()
    }
    fn sea_level(&self) -> i32 {
        crate::SEA_LEVEL
    }
    fn sky_darken(&self) -> i32 {
        0
    }
    fn ambient_light(&self) -> f32 {
        0.0
    }
    fn factory_admission(&mut self, _kind: MobKind) -> SpawnResult<FactoryAdmission> {
        Ok(FactoryAdmission::Available)
    }

    fn collision_boxes(
        &self,
        _world: &dyn FeatureWorld,
        pos: Pos,
        state: u32,
    ) -> SpawnResult<Vec<SpawnBox>> {
        let data = state_data(state)?;
        if data.flags & 4 != 0 {
            return Err(SpawnError::Unsupported(format!(
                "position-dependent empty-context collision shape, state {state} at {pos:?}"
            )));
        }
        let mut shapes = catalog().shapes[data.shape].clone();
        if data.flags & 8 != 0 {
            let [horizontal, vertical] = catalog().offsets[&state];
            let hash = crate::random::get_seed(pos.0, 0, pos.2);
            let component =
                |shift: u32| (f64::from(((hash >> shift) & 15_i64) as f32 / 15.0) - 0.5) * 0.5;
            let max = f64::from(horizontal);
            let offset = [
                component(0).clamp(-max, max),
                (f64::from(((hash >> 4) & 15) as f32 / 15.0) - 1.0) * f64::from(vertical),
                component(8).clamp(-max, max),
            ];
            for shape in &mut shapes {
                *shape = shape.at(offset);
            }
        }
        Ok(shapes)
    }

    /// Native PriorityProvider.select for the pinned biome-only variant rules.
    /// A datapack with additional selector inputs must override this query.
    fn variant_candidates(
        &self,
        world: &dyn FeatureWorld,
        kind: MobKind,
        pos: Pos,
    ) -> SpawnResult<Vec<String>> {
        let key = format!(
            "{}_variant",
            kind.name().strip_prefix("minecraft:").unwrap()
        );
        catalog()
            .variants
            .get(&key)
            .and_then(|v| v.get(&world.feature_biome(pos)))
            .cloned()
            .ok_or_else(|| SpawnError::MissingData(format!("{key} candidates at {pos:?}")))
    }

    fn pathfinding_cost_from_light(&self, world: &dyn FeatureWorld, pos: Pos) -> SpawnResult<f32> {
        let brightness = self.raw_brightness(world, pos, self.sky_darken())? as f32 / 15.0;
        let magic = brightness / (4.0 - 3.0 * brightness);
        Ok(magic + self.ambient_light() * (1.0 - magic) - 0.5)
    }

    fn no_collision(&self, world: &dyn FeatureWorld, bounds: SpawnBox) -> SpawnResult<bool> {
        no_block_collision(world, self, bounds)
    }

    fn is_unobstructed(&self, _kind: MobKind, _bounds: SpawnBox) -> SpawnResult<bool> {
        Ok(true)
    }

    fn contains_any_liquid(&self, world: &dyn FeatureWorld, bounds: SpawnBox) -> SpawnResult<bool> {
        for z in bounds.min[2].floor() as i32..bounds.max[2].ceil() as i32 {
            for y in bounds.min[1].floor() as i32..bounds.max[1].ceil() as i32 {
                for x in bounds.min[0].floor() as i32..bounds.max[0].ceil() as i32 {
                    if feature(blocks().info(read(world, self, (x, y, z))?))?.liquid() {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

fn read(
    world: &dyn FeatureWorld,
    env: &(impl SpawnEnvironment + ?Sized),
    pos: Pos,
) -> SpawnResult<u32> {
    let (min, end) = env.bounds();
    if pos.1 < min || pos.1 >= end {
        return Ok(crate::block::AIR);
    }
    world
        .get_block(pos)
        .ok_or_else(|| SpawnError::MissingData(format!("block at {pos:?}")))
}

/// Native BlockCollisions traversal, including the one-block shape padding and
/// the special face/edge/corner admission rules. Missing chunks are not air.
pub fn no_block_collision(
    world: &dyn FeatureWorld,
    env: &(impl SpawnEnvironment + ?Sized),
    bounds: SpawnBox,
) -> SpawnResult<bool> {
    let min: [i32; 3] = std::array::from_fn(|i| (bounds.min[i] - 1.0e-7).floor() as i32 - 1);
    let max: [i32; 3] = std::array::from_fn(|i| (bounds.max[i] + 1.0e-7).floor() as i32 + 1);
    for z in min[2]..=max[2] {
        for y in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                let p = [x, y, z];
                let edges = (0..3).filter(|&i| p[i] == min[i] || p[i] == max[i]).count();
                if edges == 3 {
                    continue;
                }
                let pos = (x, y, z);
                let state = read(world, env, pos)?;
                let data = state_data(state)?;
                if edges == 1 && data.flags & 2 == 0 {
                    continue;
                }
                if edges == 2 && feature(blocks().block(state))?.0 != "minecraft:moving_piston" {
                    continue;
                }
                for shape in env.collision_boxes(world, pos, state)? {
                    let epsilon = if data.flags & 16 != 0 { 0.0 } else { 1.0e-7 };
                    if bounds.intersects(
                        shape.at([f64::from(x), f64::from(y), f64::from(z)]),
                        epsilon,
                    ) {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}

pub fn top_non_colliding_pos(
    world: &dyn FeatureWorld,
    env: &(impl SpawnEnvironment + ?Sized),
    kind: MobKind,
    x: i32,
    z: i32,
) -> SpawnResult<Pos> {
    // This module's runtime contract is fresh overworld. Do not emulate a ceiling
    // dimension with the overworld catalog or risk the native unbounded descent.
    if env.has_ceiling() {
        return Err(SpawnError::Unsupported(
            "ceiling-dimension top position".into(),
        ));
    }
    let ty = spawn_type(kind);
    let y = world.feature_height(ty.heightmap, x, z);
    let mut pos = (x, y, z);
    if ty.placement == SpawnPlacement::OnGround
        && state_data(read(world, env, (x, y - 1, z))?)?.flags & 1 != 0
    {
        pos.1 -= 1;
    }
    Ok(pos)
}

pub fn is_spawn_position_ok(
    world: &dyn FeatureWorld,
    env: &(impl SpawnEnvironment + ?Sized),
    kind: MobKind,
    pos: Pos,
) -> SpawnResult<bool> {
    let ty = spawn_type(kind);
    if ty.placement == SpawnPlacement::NoRestrictions {
        return Ok(true);
    }
    if !env.border().contains(pos) {
        return Ok(false);
    }
    if ty.placement != SpawnPlacement::OnGround {
        return Err(SpawnError::Unsupported(format!(
            "{:?} placement for {}",
            ty.placement,
            kind.name()
        )));
    }
    if state_data(read(world, env, (pos.0, pos.1 - 1, pos.2))?)?.floor & ty.mask == 0 {
        return Ok(false);
    }
    for p in [pos, (pos.0, pos.1 + 1, pos.2)] {
        if state_data(read(world, env, p)?)?.empty & ty.mask == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn ground_tag(
    world: &dyn FeatureWorld,
    env: &(impl SpawnEnvironment + ?Sized),
    pos: Pos,
    tag: &str,
) -> SpawnResult<bool> {
    feature(blocks().in_block_tag(read(world, env, (pos.0, pos.1 - 1, pos.2))?, tag))
}

fn bright(
    world: &dyn FeatureWorld,
    env: &(impl SpawnEnvironment + ?Sized),
    pos: Pos,
) -> SpawnResult<bool> {
    Ok(env.raw_brightness(world, pos, 0)? > 8)
}

/// `SpawnPlacements.checkSpawnRules(..., CHUNK_GENERATION, region.getRandom())`.
pub fn check_spawn_rules(
    world: &dyn FeatureWorld,
    env: &mut (impl SpawnEnvironment + ?Sized),
    kind: MobKind,
    pos: Pos,
) -> SpawnResult<bool> {
    use MobKind::*;
    Ok(match kind {
        Sheep | Pig | Cow | Chicken | Horse | Donkey | Llama | Panda => {
            let light = bright(world, env, pos)?;
            ground_tag(world, env, pos, "animals_spawnable_on")? && light
        }
        Rabbit => ground_tag(world, env, pos, "rabbits_spawnable_on")? && bright(world, env, pos)?,
        Mooshroom => {
            ground_tag(world, env, pos, "mooshrooms_spawnable_on")? && bright(world, env, pos)?
        }
        Parrot => ground_tag(world, env, pos, "parrots_spawnable_on")? && bright(world, env, pos)?,
        Wolf => ground_tag(world, env, pos, "wolves_spawnable_on")? && bright(world, env, pos)?,
        Fox => ground_tag(world, env, pos, "foxes_spawnable_on")? && bright(world, env, pos)?,
        Goat => ground_tag(world, env, pos, "goats_spawnable_on")? && bright(world, env, pos)?,
        Armadillo => {
            ground_tag(world, env, pos, "armadillo_spawnable_on")? && bright(world, env, pos)?
        }
        Frog => ground_tag(world, env, pos, "frogs_spawnable_on")? && bright(world, env, pos)?,
        Camel => ground_tag(world, env, pos, "camels_spawnable_on")? && bright(world, env, pos)?,
        Turtle => {
            pos.1 < env.sea_level() + 4
                && ground_tag(world, env, pos, "sand")?
                && bright(world, env, pos)?
        }
        PolarBear => {
            let biome = world.feature_biome(pos);
            if biome_tag(biome, "polar_bears_spawn_on_alternate_blocks")? {
                bright(world, env, pos)?
                    && ground_tag(world, env, pos, "polar_bears_spawnable_on_alternate")?
            } else {
                let light = bright(world, env, pos)?;
                ground_tag(world, env, pos, "animals_spawnable_on")? && light
            }
        }
        _ => {
            return Err(SpawnError::Unsupported(format!(
                "SpawnPlacements predicate for {}",
                kind.name()
            )))
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FarmVariant {
    Temperate,
    Warm,
    Cold,
}

impl FarmVariant {
    fn name(self) -> &'static str {
        match self {
            Self::Temperate => "minecraft:temperate",
            Self::Warm => "minecraft:warm",
            Self::Cold => "minecraft:cold",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrogVariant {
    Temperate,
    Warm,
    Cold,
}

impl FrogVariant {
    pub fn name(self) -> &'static str {
        match self {
            Self::Temperate => "minecraft:temperate",
            Self::Warm => "minecraft:warm",
            Self::Cold => "minecraft:cold",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum RabbitVariant {
    Brown = 0,
    White = 1,
    Black = 2,
    WhiteSplotched = 3,
    Gold = 4,
    Salt = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WolfVariant {
    Pale,
    Spotted,
    Snowy,
    Ashen,
    Black,
    Chestnut,
    Rusty,
    Striped,
    Woods,
}

impl WolfVariant {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pale => "minecraft:pale",
            Self::Spotted => "minecraft:spotted",
            Self::Snowy => "minecraft:snowy",
            Self::Ashen => "minecraft:ashen",
            Self::Black => "minecraft:black",
            Self::Chestnut => "minecraft:chestnut",
            Self::Rusty => "minecraft:rusty",
            Self::Striped => "minecraft:striped",
            Self::Woods => "minecraft:woods",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoxVariant {
    Red,
    Snow,
}

impl FoxVariant {
    fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Snow => "snow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PandaGene {
    Normal,
    Lazy,
    Worried,
    Playful,
    Brown,
    Weak,
    Aggressive,
}

impl PandaGene {
    fn sample(random: &mut SpawnRandom) -> Self {
        match random.next_int(16) {
            0 => Self::Lazy,
            1 => Self::Worried,
            2 => Self::Playful,
            4 => Self::Aggressive,
            3 | 5..=8 => Self::Weak,
            9..=10 => Self::Brown,
            _ => Self::Normal,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Lazy => "lazy",
            Self::Worried => "worried",
            Self::Playful => "playful",
            Self::Brown => "brown",
            Self::Weak => "weak",
            Self::Aggressive => "aggressive",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MobProperties {
    None,
    Armadillo {
        scute_time: i32,
    },
    Camel {
        last_pose_tick: i64,
    },
    Frog {
        variant: FrogVariant,
        long_jump_cooldown: i32,
    },
    Goat {
        screaming: bool,
        has_left_horn: bool,
        has_right_horn: bool,
        long_jump_cooldown: i32,
        ram_cooldown: i32,
    },
    Sheep {
        color: u8,
    },
    Farm {
        variant: FarmVariant,
        sound: String,
        egg_lay_time: Option<i32>,
    },
    Rabbit {
        variant: RabbitVariant,
        idle_animation_timeout: i32,
    },
    Horse {
        variant: u8,
        markings: u8,
    },
    Llama {
        variant: u8,
        strength: u8,
    },
    Parrot {
        variant: u8,
    },
    Turtle {
        home: Pos,
    },
    Wolf {
        variant: WolfVariant,
        sound: String,
    },
    Fox {
        variant: FoxVariant,
        held_item: Option<&'static str>,
        sleep_countdown: i32,
    },
    Panda {
        main_gene: PandaGene,
        hidden_gene: PandaGene,
    },
}

/// Constructor-only Brain state, not saved in native NBT. Sensor declaration
/// order and each `nextInt(scanRate)` on the entity-local RNG are significant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSensor {
    pub name: String,
    pub scan_rate: i32,
    pub time_to_tick: i64,
}

/// Group metadata shared only by successfully finalized individuals. Failed
/// attempts never advance the baby counter or select a replacement group variant.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnGroupData {
    pub size: u32,
    pub spawn_babies: bool,
    pub baby_chance: f32,
    pub variant: GroupVariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupVariant {
    None,
    Rabbit(RabbitVariant),
    Horse(u8),
    Llama(u8),
    Wolf(WolfVariant),
    Fox(FoxVariant),
}

impl SpawnGroupData {
    fn new(spawn_babies: bool, baby_chance: f32, variant: GroupVariant) -> Self {
        Self {
            size: 0,
            spawn_babies,
            baby_chance,
            variant,
        }
    }
}

/// A finalized, addFreshEntityWithPassengers-eligible mob. No entity simulation
/// or UUID generation from the world seed is performed by the caller-facing API.
#[derive(Clone)]
pub struct SpawnedMob {
    pub kind: MobKind,
    pub position: [f64; 3],
    pub yaw: f32,
    pub head_yaw: f32,
    pub uuid: [i32; 4],
    pub age: i32,
    pub left_handed: bool,
    pub follow_range_bonus: f64,
    pub properties: MobProperties,
    pub entropy_seed: i64,
    metadata: SpawnTag,
    entity_random: SpawnRandom,
    sensors: Vec<SpawnSensor>,
}

impl std::fmt::Debug for SpawnedMob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnedMob")
            .field("kind", &self.kind)
            .field("position", &self.position)
            .field("age", &self.age)
            .field("properties", &self.properties)
            .finish_non_exhaustive()
    }
}

impl SpawnedMob {
    pub fn type_id(&self) -> u32 {
        spawn_type(self.kind).registry_id
    }
    pub fn metadata(&self) -> &SpawnTag {
        &self.metadata
    }
    pub fn data(&self) -> Value {
        self.metadata.data()
    }
    pub fn random(&self) -> &SpawnRandom {
        &self.entity_random
    }
    pub fn sensors(&self) -> &[SpawnSensor] {
        &self.sensors
    }
    /// Frog overrides isBaby even when AgeableMob's group draw sets a negative
    /// saved Age. Consumers must not infer its visual baby flag from Age alone.
    pub fn is_baby(&self) -> bool {
        self.kind != MobKind::Frog && self.age < 0
    }
    pub fn block_pos(&self) -> Pos {
        (
            self.position[0].floor() as i32,
            self.position[1].floor() as i32,
            self.position[2].floor() as i32,
        )
    }
}

struct CreatedMob {
    kind: MobKind,
    entropy_seed: i64,
    uuid: [i32; 4],
    head_yaw: f32,
    random: SpawnRandom,
    metadata: SpawnTag,
    egg_lay_time: Option<i32>,
    idle_animation_timeout: i32,
    scute_time: i32,
    sensors: Vec<SpawnSensor>,
}

fn create_mob(
    env: &mut (impl SpawnEnvironment + ?Sized),
    kind: MobKind,
    tracing: bool,
) -> SpawnResult<CreatedMob> {
    if !kind.supports_finalization() {
        return Err(SpawnError::Unsupported(format!(
            "EntityType.create(NATURAL)/{}.finalizeSpawn(CHUNK_GENERATION)",
            kind.name()
        )));
    }
    let entropy_seed = env.entity_seed(kind)?;
    let mut random = SpawnRandom::legacy(entropy_seed);
    if tracing {
        random = random.trace();
    }
    // Entity constructor -> Mth.createInsecureUUID(entity.random). This stream
    // starts with externally supplied entropy, never the decoration/region seed.
    let most = (random.next_long() as u64 & 0xffff_ffff_ffff_0fff) | 0x4000;
    let least = (random.next_long() as u64 & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    let uuid = [
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ];
    let head_yaw = random.next_float() * std::f32::consts::TAU;
    let mut metadata = catalog().templates[&kind].clone();
    metadata.put("UUID", SpawnTag::IntArray(uuid.to_vec()));
    let sensors = catalog().sensors[&kind]
        .iter()
        .map(|(name, rate)| SpawnSensor {
            name: name.clone(),
            scan_rate: *rate,
            time_to_tick: i64::from(random.next_int(*rate)),
        })
        .collect();
    let mut egg_lay_time = None;
    let mut idle_animation_timeout = 0;
    let mut scute_time = 0;
    if kind == MobKind::Chicken {
        let ticks = random.next_int(6000) + 6000;
        egg_lay_time = Some(ticks);
        metadata.put("EggLayTime", SpawnTag::Int(ticks));
    }
    if kind == MobKind::Rabbit {
        idle_animation_timeout = random.next_int(40) + 180;
    }
    if kind == MobKind::Fox {
        // Fox.registerGoals -> SleepGoal.countdown, reduced tick delay 140 / 2.
        idle_animation_timeout = random.next_int(70);
    }
    if kind == MobKind::Armadillo {
        scute_time = random.next_int(6000) + 6000;
        metadata.put("scute_time", SpawnTag::Int(scute_time));
    }
    Ok(CreatedMob {
        kind,
        entropy_seed,
        uuid,
        head_yaw,
        random,
        metadata,
        egg_lay_time,
        idle_animation_timeout,
        scute_time,
        sensors,
    })
}

fn mob_spawn_rule(
    world: &dyn FeatureWorld,
    env: &(impl SpawnEnvironment + ?Sized),
    kind: MobKind,
    pos: Pos,
) -> SpawnResult<bool> {
    let ground = read(world, env, (pos.0, pos.1 - 1, pos.2))?;
    let preferred = match kind {
        MobKind::Mooshroom => "minecraft:mycelium",
        MobKind::Turtle => {
            if ground_tag(world, env, pos, "sand")?
                || feature(blocks().in_fluid_tag(read(world, env, pos)?, "water"))?
            {
                return Ok(true);
            }
            ""
        }
        _ => "minecraft:grass_block",
    };
    if feature(blocks().block(ground))?.0 == preferred {
        return Ok(true);
    }
    // PathfinderMob.checkSpawnRules -> Animal.getWalkTargetValue -> native
    // light-level-dependent magic value, with float (not double) arithmetic.
    Ok(env.pathfinding_cost_from_light(world, pos)? >= 0.0)
}

fn attribute(id: &str, base: f64, bonus: Option<f64>) -> SpawnTag {
    let mut fields = BTreeMap::from([
        ("id".into(), SpawnTag::String(format!("minecraft:{id}"))),
        ("base".into(), SpawnTag::Double(base)),
    ]);
    if let Some(bonus) = bonus {
        fields.insert(
            "modifiers".into(),
            SpawnTag::List(vec![SpawnTag::Compound(BTreeMap::from([
                ("amount".into(), SpawnTag::Double(bonus)),
                (
                    "id".into(),
                    SpawnTag::String("minecraft:random_spawn_bonus".into()),
                ),
                (
                    "operation".into(),
                    SpawnTag::String("add_multiplied_base".into()),
                ),
            ]))]),
        );
    }
    SpawnTag::Compound(fields)
}

fn sheep_color(biome: BiomeId, random: &mut SpawnRandom) -> SpawnResult<u8> {
    let colors = if biome_tag(biome, "spawns_warm_variant_farm_animals")? {
        [7, 8, 0, 15, 12]
    } else if biome_tag(biome, "spawns_cold_variant_farm_animals")? {
        [8, 7, 0, 12, 15]
    } else {
        [15, 7, 8, 12, 0]
    };
    let draw = random.next_int(100);
    Ok(if draw < 5 {
        colors[0]
    } else if draw < 10 {
        colors[1]
    } else if draw < 15 {
        colors[2]
    } else if draw < 18 {
        colors[3]
    } else if random.next_int(500) < 499 {
        colors[4]
    } else {
        6
    })
}

fn rabbit_variant(biome: BiomeId, random: &mut SpawnRandom) -> SpawnResult<RabbitVariant> {
    let draw = random.next_int(100); // even gold biomes and existing groups draw
    Ok(if biome_tag(biome, "spawns_white_rabbits")? {
        if draw < 80 {
            RabbitVariant::White
        } else {
            RabbitVariant::WhiteSplotched
        }
    } else if biome_tag(biome, "spawns_gold_rabbits")? {
        RabbitVariant::Gold
    } else if draw < 50 {
        RabbitVariant::Brown
    } else if draw < 90 {
        RabbitVariant::Salt
    } else {
        RabbitVariant::Black
    })
}

fn initial_memory(mob: &mut CreatedMob, name: &str, value: i32) {
    mob.metadata.field_mut("Brain").field_mut("memories").put(
        name,
        SpawnTag::Compound(BTreeMap::from([("value".into(), SpawnTag::Int(value))])),
    );
}

fn memory_interval(random: &mut SpawnRandom, key: &str) -> i32 {
    let [min, max] = catalog().memory_intervals[key];
    min + random.next_int(max - min + 1)
}

fn finalize_mob(
    world: &dyn FeatureWorld,
    env: &mut (impl SpawnEnvironment + ?Sized),
    mut mob: CreatedMob,
    position: [f64; 3],
    yaw: f32,
    group: &mut Option<SpawnGroupData>,
) -> SpawnResult<SpawnedMob> {
    use MobKind::*;
    let pos = (
        position[0].floor() as i32,
        position[1].floor() as i32,
        position[2].floor() as i32,
    );
    let mut properties = MobProperties::None;
    let mut forced_age = 0;
    let mut attributes = match mob.metadata.field("attributes") {
        SpawnTag::List(v) => v.clone(),
        _ => unreachable!("native attribute list"),
    };
    match mob.kind {
        Armadillo => {
            properties = MobProperties::Armadillo {
                scute_time: mob.scute_time,
            };
        }
        Camel => {
            // Java long subtraction wraps before Math.max, including MIN_VALUE.
            let last_pose_tick = env.game_time()?.wrapping_sub(52).wrapping_sub(1).max(0);
            mob.metadata
                .put("LastPoseTick", SpawnTag::Long(last_pose_tick));
            properties = MobProperties::Camel { last_pose_tick };
        }
        Frog => {
            let choices = env.variant_candidates(world, mob.kind, pos)?;
            let variant = if choices.is_empty() {
                FrogVariant::Temperate
            } else {
                match choices[env.random().next_int(choices.len() as i32) as usize].as_str() {
                    "minecraft:temperate" => FrogVariant::Temperate,
                    "minecraft:warm" => FrogVariant::Warm,
                    "minecraft:cold" => FrogVariant::Cold,
                    other => return Err(SpawnError::Unsupported(format!("frog variant {other}"))),
                }
            };
            mob.metadata
                .put("variant", SpawnTag::String(variant.name().into()));
            let long_jump_cooldown = memory_interval(env.random(), "frog_jump");
            initial_memory(
                &mut mob,
                "minecraft:long_jump_cooling_down",
                long_jump_cooldown,
            );
            properties = MobProperties::Frog {
                variant,
                long_jump_cooldown,
            };
        }
        Goat => {
            let long_jump_cooldown = memory_interval(env.random(), "goat_jump");
            let ram_cooldown = memory_interval(env.random(), "goat_ram");
            initial_memory(
                &mut mob,
                "minecraft:long_jump_cooling_down",
                long_jump_cooldown,
            );
            initial_memory(&mut mob, "minecraft:ram_cooldown_ticks", ram_cooldown);
            let screaming = env.random().next_double() < 0.02;
            let mut has_left_horn = true;
            let mut has_right_horn = true;
            // This occurs while still adult, before AgeableMob's group baby draw.
            if env.random().next_float() < 0.1 {
                if env.random().next_bool() {
                    has_left_horn = false;
                } else {
                    has_right_horn = false;
                }
            }
            mob.metadata
                .put("IsScreamingGoat", SpawnTag::Byte(i8::from(screaming)));
            mob.metadata
                .put("HasLeftHorn", SpawnTag::Byte(i8::from(has_left_horn)));
            mob.metadata
                .put("HasRightHorn", SpawnTag::Byte(i8::from(has_right_horn)));
            properties = MobProperties::Goat {
                screaming,
                has_left_horn,
                has_right_horn,
                long_jump_cooldown,
                ram_cooldown,
            };
        }
        Sheep => {
            let color = sheep_color(world.feature_biome(pos), env.random())?;
            mob.metadata.put("Color", SpawnTag::Byte(color as i8));
            properties = MobProperties::Sheep { color };
        }
        Pig | Cow | Chicken => {
            let candidates = env.variant_candidates(world, mob.kind, pos)?;
            let variant = if candidates.is_empty() {
                FarmVariant::Temperate
            } else {
                // PriorityProvider.pick draws even when there is only one candidate.
                match candidates[env.random().next_int(candidates.len() as i32) as usize].as_str() {
                    "minecraft:warm" => FarmVariant::Warm,
                    "minecraft:cold" => FarmVariant::Cold,
                    "minecraft:temperate" => FarmVariant::Temperate,
                    other => return Err(SpawnError::Unsupported(format!("farm variant {other}"))),
                }
            };
            let sound_key = format!(
                "{}_sound_variant",
                mob.kind.name().strip_prefix("minecraft:").unwrap()
            );
            let choices = &catalog().sounds[&sound_key];
            let sound = choices[env.random().next_int(choices.len() as i32) as usize].clone();
            mob.metadata
                .put("variant", SpawnTag::String(variant.name().into()));
            mob.metadata
                .put("sound_variant", SpawnTag::String(sound.clone()));
            properties = MobProperties::Farm {
                variant,
                sound,
                egg_lay_time: mob.egg_lay_time,
            };
        }
        Rabbit => {
            let rolled = rabbit_variant(world.feature_biome(pos), env.random())?;
            let variant = match group.as_ref().map(|g| g.variant) {
                Some(GroupVariant::Rabbit(v)) => v,
                _ => {
                    *group = Some(SpawnGroupData::new(true, 1.0, GroupVariant::Rabbit(rolled)));
                    rolled
                }
            };
            mob.metadata
                .put("RabbitType", SpawnTag::Int(variant as i32));
            attributes.push(attribute("attack_damage", 3.0, None));
            properties = MobProperties::Rabbit {
                variant,
                idle_animation_timeout: mob.idle_animation_timeout,
            };
        }
        Horse => {
            let variant = match group.as_ref().map(|g| g.variant) {
                Some(GroupVariant::Horse(v)) => v,
                _ => {
                    let v = env.random().next_int(7) as u8;
                    *group = Some(SpawnGroupData::new(true, 0.05, GroupVariant::Horse(v)));
                    v
                }
            };
            let markings = env.random().next_int(5) as u8;
            mob.metadata.put(
                "Variant",
                SpawnTag::Int(i32::from(variant) | (i32::from(markings) << 8)),
            );
            properties = MobProperties::Horse { variant, markings };
        }
        Llama => {
            let bound = if env.random().next_float() < 0.04 {
                5
            } else {
                3
            };
            let strength = (1 + env.random().next_int(bound)) as u8;
            let variant = match group.as_ref().map(|g| g.variant) {
                Some(GroupVariant::Llama(v)) => v,
                _ => {
                    let v = env.random().next_int(4) as u8;
                    *group = Some(SpawnGroupData::new(true, 0.05, GroupVariant::Llama(v)));
                    v
                }
            };
            mob.metadata
                .put("Strength", SpawnTag::Int(i32::from(strength)));
            mob.metadata
                .put("Variant", SpawnTag::Int(i32::from(variant)));
            properties = MobProperties::Llama { variant, strength };
        }
        Parrot => {
            let variant = env.random().next_int(5) as u8;
            mob.metadata
                .put("Variant", SpawnTag::Int(i32::from(variant)));
            properties = MobProperties::Parrot { variant };
            if group.is_none() {
                *group = Some(SpawnGroupData::new(false, 0.05, GroupVariant::None));
            }
        }
        Turtle => {
            mob.metadata
                .put("home_pos", SpawnTag::IntArray(vec![pos.0, pos.1, pos.2]));
            properties = MobProperties::Turtle { home: pos };
        }
        Wolf => {
            let variant = match group.as_ref().map(|g| g.variant) {
                Some(GroupVariant::Wolf(v)) => v,
                _ => {
                    let choices = env.variant_candidates(world, mob.kind, pos)?;
                    if choices.is_empty() {
                        WolfVariant::Pale
                    } else {
                        let selected =
                            &choices[env.random().next_int(choices.len() as i32) as usize];
                        let v: WolfVariant = serde_json::from_value(json!(selected
                            .strip_prefix("minecraft:")
                            .unwrap_or(selected)))
                        .map_err(|_| SpawnError::Unsupported(format!("wolf variant {selected}")))?;
                        *group = Some(SpawnGroupData::new(false, 0.05, GroupVariant::Wolf(v)));
                        v
                    }
                }
            };
            let sounds = &catalog().sounds["wolf_sound_variant"];
            let sound = sounds[env.random().next_int(sounds.len() as i32) as usize].clone();
            mob.metadata
                .put("variant", SpawnTag::String(variant.name().into()));
            mob.metadata
                .put("sound_variant", SpawnTag::String(sound.clone()));
            properties = MobProperties::Wolf { variant, sound };
        }
        Fox => {
            let local = if biome_tag(world.feature_biome(pos), "spawns_snow_foxes")? {
                FoxVariant::Snow
            } else {
                FoxVariant::Red
            };
            let variant = match group.as_ref().map(|g| g.variant) {
                Some(GroupVariant::Fox(v)) => {
                    if group.as_ref().unwrap().size >= 2 {
                        forced_age = -24_000;
                    }
                    v
                }
                _ => {
                    *group = Some(SpawnGroupData::new(false, 0.05, GroupVariant::Fox(local)));
                    local
                }
            };
            mob.metadata
                .put("Type", SpawnTag::String(variant.name().into()));
            let held_item = if env.random().next_float() < 0.2 {
                let sample = env.random().next_float();
                Some(if sample < 0.05 {
                    "minecraft:emerald"
                } else if sample < 0.2 {
                    "minecraft:egg"
                } else if sample < 0.4 {
                    if env.random().next_bool() {
                        "minecraft:rabbit_foot"
                    } else {
                        "minecraft:rabbit_hide"
                    }
                } else if sample < 0.6 {
                    "minecraft:wheat"
                } else if sample < 0.8 {
                    "minecraft:leather"
                } else {
                    "minecraft:feather"
                })
            } else {
                None
            };
            if let Some(item) = held_item {
                mob.metadata.put(
                    "equipment",
                    SpawnTag::Compound(BTreeMap::from([(
                        "mainhand".into(),
                        SpawnTag::Compound(BTreeMap::from([
                            ("id".into(), SpawnTag::String(item.into())),
                            ("count".into(), SpawnTag::Int(1)),
                        ])),
                    )])),
                );
            }
            properties = MobProperties::Fox {
                variant,
                held_item,
                sleep_countdown: mob.idle_animation_timeout,
            };
        }
        Panda => {
            let main_gene = PandaGene::sample(env.random());
            let hidden_gene = PandaGene::sample(env.random());
            mob.metadata
                .put("MainGene", SpawnTag::String(main_gene.name().into()));
            mob.metadata
                .put("HiddenGene", SpawnTag::String(hidden_gene.name().into()));
            if main_gene == PandaGene::Weak && hidden_gene == PandaGene::Weak {
                attributes.push(attribute("max_health", 10.0, None));
            }
            if main_gene == PandaGene::Lazy {
                attributes.push(attribute("movement_speed", f64::from(0.07_f32), None));
            }
            if group.is_none() {
                *group = Some(SpawnGroupData::new(true, 0.2, GroupVariant::None));
            }
            properties = MobProperties::Panda {
                main_gene,
                hidden_gene,
            };
        }
        _ => {}
    }
    if group.is_none() {
        let chance = match mob.kind {
            PolarBear => 1.0,
            Donkey | Camel => 0.2,
            _ => 0.05,
        };
        *group = Some(SpawnGroupData::new(true, chance, GroupVariant::None));
    }
    if matches!(mob.kind, Horse | Donkey | Llama) {
        // Health is NOT clamped here: the native initial entity still saves 53.
        let health = 15.0_f32 + env.random().next_int(8) as f32 + env.random().next_int(9) as f32;
        attributes.push(attribute("max_health", f64::from(health), None));
        if mob.kind == Horse {
            let speed = (f64::from(0.45_f32)
                + env.random().next_double() * 0.3
                + env.random().next_double() * 0.3
                + env.random().next_double() * 0.3)
                * 0.25;
            let jump = f64::from(0.4_f32)
                + env.random().next_double() * 0.2
                + env.random().next_double() * 0.2
                + env.random().next_double() * 0.2;
            attributes.push(attribute("movement_speed", speed, None));
            attributes.push(attribute("jump_strength", jump, None));
        }
    }
    let group = group.as_mut().expect("native ageable group");
    let age =
        if group.spawn_babies && group.size > 0 && env.random().next_float() <= group.baby_chance {
            -24_000
        } else {
            forced_age
        };
    group.size += 1;
    if mob.kind == Goat {
        // ageBoundaryReached executes once before the baby roll and again only
        // on an actual sign change; it updates the same attribute instance.
        attributes.push(attribute(
            "attack_damage",
            if age < 0 { 1.0 } else { 2.0 },
            None,
        ));
    }
    let follow_range_bonus =
        0.0 + 0.114_850_000_000_000_01 * (env.random().next_double() - env.random().next_double());
    let left_handed = env.random().next_float() < 0.05;
    attributes.push(attribute(
        "follow_range",
        spawn_type(mob.kind).follow_range,
        Some(follow_range_bonus),
    ));
    // Native AttributeMap uses an identity-keyed map. Preserve all typed entries;
    // canonical order avoids asserting a world-seeded JVM object identity order.
    attributes.sort_by(|a, b| {
        let (SpawnTag::String(a), SpawnTag::String(b)) = (a.field("id"), b.field("id")) else {
            unreachable!()
        };
        a.cmp(b)
    });
    mob.metadata.put("attributes", SpawnTag::List(attributes));
    mob.metadata.put("Age", SpawnTag::Int(age));
    mob.metadata
        .put("LeftHanded", SpawnTag::Byte(i8::from(left_handed)));
    mob.metadata.put(
        "Pos",
        SpawnTag::List(position.map(SpawnTag::Double).to_vec()),
    );
    mob.metadata.put(
        "Rotation",
        SpawnTag::List(vec![SpawnTag::Float(yaw), SpawnTag::Float(0.0)]),
    );
    Ok(SpawnedMob {
        kind: mob.kind,
        position,
        yaw,
        head_yaw: mob.head_yaw,
        uuid: mob.uuid,
        age,
        left_handed,
        follow_range_bonus,
        properties,
        entropy_seed: mob.entropy_seed,
        metadata: mob.metadata,
        entity_random: mob.random,
        sensors: mob.sensors,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnAttemptOutcome {
    NotSummonable,
    PlacementRejected,
    CollisionRejected,
    StaticRuleRejected,
    FactoryReturnedNull,
    FactoryException(String),
    MobRuleRejected,
    Obstructed,
    Spawned { index: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnAttempt {
    pub group: usize,
    pub individual: i32,
    pub retry: u8,
    pub kind: MobKind,
    pub top: Pos,
    pub position: Option<[f64; 3]>,
    pub outcome: SpawnAttemptOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnGroup {
    pub kind: MobKind,
    pub requested_count: i32,
    pub origin: [i32; 2],
    pub final_data: Option<SpawnGroupData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnSkip {
    UpgradingChunk,
    MobGenerationDisabled,
    EmptyBiomeList,
    SpawnMobsGameRule,
}

#[derive(Debug, Clone, Default)]
pub struct SpawnReport {
    pub mobs: Vec<SpawnedMob>,
    pub groups: Vec<SpawnGroup>,
    pub attempts: Vec<SpawnAttempt>,
    pub skipped: Option<SpawnSkip>,
    pub biome: Option<BiomeId>,
    pub decoration_seed: Option<i64>,
    pub placement_draws: Vec<SpawnDraw>,
    pub placement_next_long: Option<i64>,
    /// Created-but-rejected entities also consume external entropy and draws.
    pub rejected_entity_draws: Vec<(i64, Vec<SpawnDraw>, i64)>,
}

/// A partial execution must not be marked as a completed SPAWN stage. It may
/// already contain genuinely finalized mobs; its error is never a fake rejection.
#[derive(Debug)]
pub struct SpawnFailure {
    pub error: SpawnError,
    pub partial: SpawnReport,
}

impl std::fmt::Display for SpawnFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for SpawnFailure {}

#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnOptions {
    pub upgrading_chunk: bool,
    pub disable_mob_generation: bool,
    pub trace: bool,
}

/// Native ChunkStatusTasks.generateSpawn -> NoiseBasedChunkGenerator entry path.
/// The biome is queried at the minimum chunk X/Z and inclusive maximum build Y.
pub fn spawn_original_mobs(
    world: &dyn FeatureWorld,
    env: &mut (impl SpawnEnvironment + ?Sized),
    world_seed: i64,
    chunk: [i32; 2],
    options: SpawnOptions,
) -> Result<SpawnReport, SpawnFailure> {
    let skipped = if options.upgrading_chunk {
        Some(SpawnSkip::UpgradingChunk)
    } else if options.disable_mob_generation {
        Some(SpawnSkip::MobGenerationDisabled)
    } else {
        None
    };
    if skipped.is_some() {
        return Ok(SpawnReport {
            skipped,
            ..SpawnReport::default()
        });
    }
    let biome = world.feature_biome((
        chunk[0].wrapping_mul(16),
        env.bounds().1 - 1,
        chunk[1].wrapping_mul(16),
    ));
    let settings = biome_spawns(biome).map_err(|error| SpawnFailure {
        error,
        partial: SpawnReport::default(),
    })?;
    let (seed, mut random) = SpawnRandom::for_chunk(world_seed, chunk);
    if options.trace {
        random = random.trace();
    }
    let mut result =
        spawn_mobs_for_chunk_generation(world, env, settings, chunk, &mut random, options.trace);
    let report = match &mut result {
        Ok(report) => report,
        Err(failure) => &mut failure.partial,
    };
    report.biome = Some(biome);
    report.decoration_seed = Some(seed);
    result
}

/// Direct native method entry, useful for custom biome settings and continuing
/// an existing placement stream. `Ok` means all required callbacks executed.
pub fn spawn_mobs_for_chunk_generation(
    world: &dyn FeatureWorld,
    env: &mut (impl SpawnEnvironment + ?Sized),
    settings: &BiomeSpawns,
    chunk: [i32; 2],
    random: &mut SpawnRandom,
    trace: bool,
) -> Result<SpawnReport, SpawnFailure> {
    let mut report = SpawnReport::default();
    let result = spawn_loop(world, env, settings, chunk, random, trace, &mut report);
    report.placement_draws = random.draws().to_vec();
    report.placement_next_long = Some(random.peek_next_long());
    match result {
        Ok(()) => Ok(report),
        Err(error) => Err(SpawnFailure {
            error,
            partial: report,
        }),
    }
}

fn spawn_loop(
    world: &dyn FeatureWorld,
    env: &mut (impl SpawnEnvironment + ?Sized),
    settings: &BiomeSpawns,
    chunk: [i32; 2],
    random: &mut SpawnRandom,
    trace: bool,
    report: &mut SpawnReport,
) -> SpawnResult<()> {
    if settings.groups.is_empty() {
        report.skipped = Some(SpawnSkip::EmptyBiomeList);
        return Ok(());
    }
    if !env.spawn_mobs() {
        report.skipped = Some(SpawnSkip::SpawnMobsGameRule);
        return Ok(());
    }
    if !(0.0..1.0).contains(&settings.probability) {
        return Err(SpawnError::InvalidSettings(
            "creature probability must be finite and in [0,1)".into(),
        ));
    }
    let mut total_weight = 0_i32;
    for group in &settings.groups {
        if group.weight < 0
            || group.min_count < 0
            || group.max_count < group.min_count
            || group.max_count == i32::MAX
        {
            return Err(SpawnError::InvalidSettings(format!("spawner {group:?}")));
        }
        total_weight = total_weight
            .checked_add(group.weight)
            .ok_or_else(|| SpawnError::InvalidSettings("weight overflow".into()))?;
    }
    let min_x = chunk[0].wrapping_mul(16);
    let min_z = chunk[1].wrapping_mul(16);
    while random.next_float() < settings.probability {
        if total_weight == 0 {
            continue;
        }
        let mut choice = random.next_int(total_weight);
        let selected = settings
            .groups
            .iter()
            .find(|group| {
                choice -= group.weight;
                choice < 0
            })
            .expect("positive native weight");
        let kind = selected.kind;
        let count =
            selected.min_count + random.next_int(selected.max_count + 1 - selected.min_count);
        let mut x = min_x.wrapping_add(random.next_int(16));
        let mut z = min_z.wrapping_add(random.next_int(16));
        let origin = [x, z];
        let group_index = report.groups.len();
        report.groups.push(SpawnGroup {
            kind,
            requested_count: count,
            origin,
            final_data: None,
        });
        let mut group_data = None;
        for individual in 0..count {
            for retry in 0..4 {
                let top = top_non_colliding_pos(world, env, kind, x, z)?;
                let ty = spawn_type(kind);
                let mut position = None;
                let mut jitter = true;
                let outcome = if !ty.can_summon {
                    SpawnAttemptOutcome::NotSummonable
                } else if !is_spawn_position_ok(world, env, kind, top)? {
                    SpawnAttemptOutcome::PlacementRejected
                } else {
                    // Native clamps the integer candidate itself: there is NO +0.5.
                    let p = [
                        (f64::from(x)).clamp(
                            f64::from(min_x) + f64::from(ty.width),
                            f64::from(min_x) + 16.0 - f64::from(ty.width),
                        ),
                        f64::from(top.1),
                        (f64::from(z)).clamp(
                            f64::from(min_z) + f64::from(ty.width),
                            f64::from(min_z) + 16.0 - f64::from(ty.width),
                        ),
                    ];
                    position = Some(p);
                    let block_pos = (
                        p[0].floor() as i32,
                        p[1].floor() as i32,
                        p[2].floor() as i32,
                    );
                    let bounds = ty.spawn_box.at(p);
                    if !env.no_collision(world, bounds)? {
                        jitter = false;
                        SpawnAttemptOutcome::CollisionRejected
                    } else if !check_spawn_rules(world, env, kind, block_pos)? {
                        jitter = false;
                        SpawnAttemptOutcome::StaticRuleRejected
                    } else {
                        match env.factory_admission(kind)? {
                            FactoryAdmission::Null => {
                                jitter = false;
                                SpawnAttemptOutcome::FactoryReturnedNull
                            }
                            FactoryAdmission::Exception(error) => {
                                jitter = false;
                                SpawnAttemptOutcome::FactoryException(error)
                            }
                            FactoryAdmission::Available => {
                                let mob = create_mob(env, kind, trace)?;
                                let yaw = random.next_float() * 360.0;
                                let outcome = if !mob_spawn_rule(world, env, kind, block_pos)? {
                                    Some(SpawnAttemptOutcome::MobRuleRejected)
                                } else if env.contains_any_liquid(world, bounds)?
                                    || !env.is_unobstructed(kind, bounds)?
                                {
                                    Some(SpawnAttemptOutcome::Obstructed)
                                } else {
                                    None
                                };
                                if let Some(outcome) = outcome {
                                    report.rejected_entity_draws.push((
                                        mob.entropy_seed,
                                        mob.random.draws().to_vec(),
                                        mob.random.peek_next_long(),
                                    ));
                                    outcome
                                } else {
                                    // The difficulty callback executes even for entities which do not use its value.
                                    env.current_difficulty_at(block_pos)?;
                                    let finalized =
                                        finalize_mob(world, env, mob, p, yaw, &mut group_data)?;
                                    env.add_fresh_mob(&finalized)?;
                                    let index = report.mobs.len();
                                    report.mobs.push(finalized);
                                    report.groups[group_index].final_data = group_data.clone();
                                    SpawnAttemptOutcome::Spawned { index }
                                }
                            }
                        }
                    }
                };
                let spawned = matches!(outcome, SpawnAttemptOutcome::Spawned { .. });
                report.attempts.push(SpawnAttempt {
                    group: group_index,
                    individual,
                    retry,
                    kind,
                    top,
                    position,
                    outcome,
                });
                if jitter {
                    x = x.wrapping_add(random.next_int(5) - random.next_int(5));
                    z = z.wrapping_add(random.next_int(5) - random.next_int(5));
                    while x < min_x
                        || x >= min_x.wrapping_add(16)
                        || z < min_z
                        || z >= min_z.wrapping_add(16)
                    {
                        x = origin[0]
                            .wrapping_add(random.next_int(5))
                            .wrapping_sub(random.next_int(5));
                        z = origin[1]
                            .wrapping_add(random.next_int(5))
                            .wrapping_sub(random.next_int(5));
                    }
                }
                if spawned {
                    break;
                }
            }
        }
    }
    Ok(())
}
