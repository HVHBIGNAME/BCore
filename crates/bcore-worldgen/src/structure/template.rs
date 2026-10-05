//! Structure-template coordinates, palettes, typed NBT and worldgen placement.
//!
//! The bundled registry describes the pinned JAR's block states (including each
//! block's rotation/mirror implementation). Coordinates, ordering, processing,
//! clipping, liquid restoration and RNG ownership are evaluated at runtime.
//! This module uses the known-shape placement mode used by native pool elements.

use super::processors::{process_block_infos, Processor};
use crate::feature_world::{FeatureError, FeatureWorld, Pos};
use crate::random::get_seed;
use crate::simplex::JavaRandom;
use crate::tick_request::{TickRequest, TickTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type Result<T> = std::result::Result<T, FeatureError>;

pub(crate) fn invalid(detail: impl Into<String>) -> FeatureError {
    FeatureError::InvalidConfig(detail.into())
}

pub(crate) fn missing(detail: impl Into<String>) -> FeatureError {
    FeatureError::MissingData(detail.into())
}

pub(crate) fn id(name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    }
}

/// The small common surface of BCore's existing random sources. Assembly uses
/// JavaRandom; block placement accepts the caller's current WorldgenRandom.
pub trait TemplateRandom {
    fn next_int(&mut self, bound: usize) -> usize;
    fn next_long(&mut self) -> i64;
    fn next_float(&mut self) -> f32;
}

impl TemplateRandom for JavaRandom {
    fn next_int(&mut self, bound: usize) -> usize {
        self.next_int(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.next_float()
    }
}

impl TemplateRandom for crate::simplex::WorldgenRandom {
    fn next_int(&mut self, bound: usize) -> usize {
        self.next_int(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.next_float()
    }
}

impl TemplateRandom for crate::random::WorldgenRandom {
    fn next_int(&mut self, bound: usize) -> usize {
        self.next_i32_bounded(bound as i32) as usize
    }
    fn next_long(&mut self) -> i64 {
        self.next_i64()
    }
    fn next_float(&mut self) -> f32 {
        self.next_f32()
    }
}

pub(crate) fn shuffle<T>(values: &mut [T], random: &mut (impl TemplateRandom + ?Sized)) {
    for end in (1..values.len()).rev() {
        let other = random.next_int(end + 1);
        values.swap(end, other);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rotation {
    #[default]
    None,
    #[serde(rename = "CLOCKWISE_90")]
    Clockwise90,
    #[serde(rename = "CLOCKWISE_180")]
    Clockwise180,
    #[serde(rename = "COUNTERCLOCKWISE_90")]
    Counterclockwise90,
}

impl Rotation {
    pub const ALL: [Self; 4] = [
        Self::None,
        Self::Clockwise90,
        Self::Clockwise180,
        Self::Counterclockwise90,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::None => 0,
            Self::Clockwise90 => 1,
            Self::Clockwise180 => 2,
            Self::Counterclockwise90 => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Clockwise90 => "CLOCKWISE_90",
            Self::Clockwise180 => "CLOCKWISE_180",
            Self::Counterclockwise90 => "COUNTERCLOCKWISE_90",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Mirror {
    #[default]
    None,
    LeftRight,
    FrontBack,
}

impl Mirror {
    pub fn index(self) -> usize {
        match self {
            Self::None => 0,
            Self::LeftRight => 1,
            Self::FrontBack => 2,
        }
    }
}

pub(crate) fn add(a: Pos, b: Pos) -> Pos {
    (
        a.0.wrapping_add(b.0),
        a.1.wrapping_add(b.1),
        a.2.wrapping_add(b.2),
    )
}

pub(crate) fn sub(a: Pos, b: Pos) -> Pos {
    (
        a.0.wrapping_sub(b.0),
        a.1.wrapping_sub(b.1),
        a.2.wrapping_sub(b.2),
    )
}

/// Mirror about the local origin, then rotate about the specified block pivot.
pub fn transform_block(pos: Pos, mirror: Mirror, rotation: Rotation, pivot: Pos) -> Pos {
    let mirrored = match mirror {
        Mirror::None => pos,
        Mirror::LeftRight => (pos.0, pos.1, pos.2.wrapping_neg()),
        Mirror::FrontBack => (pos.0.wrapping_neg(), pos.1, pos.2),
    };
    let delta = sub(mirrored, pivot);
    let rotated = match rotation {
        Rotation::None => delta,
        Rotation::Clockwise90 => (delta.2.wrapping_neg(), delta.1, delta.0),
        Rotation::Clockwise180 => (delta.0.wrapping_neg(), delta.1, delta.2.wrapping_neg()),
        Rotation::Counterclockwise90 => (delta.2, delta.1, delta.0.wrapping_neg()),
    };
    add(rotated, pivot)
}

/// Entity positions rotate around block centres, rather than block corners.
pub fn transform_entity(pos: [f64; 3], mirror: Mirror, rotation: Rotation, pivot: Pos) -> [f64; 3] {
    let mut p = pos;
    match mirror {
        Mirror::None => {}
        Mirror::LeftRight => p[2] = 1.0 - p[2],
        Mirror::FrontBack => p[0] = 1.0 - p[0],
    }
    let x = p[0] - f64::from(pivot.0) - 0.5;
    let z = p[2] - f64::from(pivot.2) - 0.5;
    let (x, z) = match rotation {
        Rotation::None => (x, z),
        Rotation::Clockwise90 => (-z, x),
        Rotation::Clockwise180 => (-x, -z),
        Rotation::Counterclockwise90 => (z, -x),
    };
    [
        x + f64::from(pivot.0) + 0.5,
        p[1],
        z + f64::from(pivot.2) + 0.5,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: Pos,
    pub max: Pos,
}

impl BoundingBox {
    pub fn new(a: Pos, b: Pos) -> Self {
        Self {
            min: (a.0.min(b.0), a.1.min(b.1), a.2.min(b.2)),
            max: (a.0.max(b.0), a.1.max(b.1), a.2.max(b.2)),
        }
    }

    pub fn contains(self, p: Pos) -> bool {
        p.0 >= self.min.0
            && p.0 <= self.max.0
            && p.1 >= self.min.1
            && p.1 <= self.max.1
            && p.2 >= self.min.2
            && p.2 <= self.max.2
    }

    pub fn contains_box(self, other: Self) -> bool {
        self.contains(other.min) && self.contains(other.max)
    }

    pub fn intersects(self, other: Self) -> bool {
        self.max.0 >= other.min.0
            && self.min.0 <= other.max.0
            && self.max.1 >= other.min.1
            && self.min.1 <= other.max.1
            && self.max.2 >= other.min.2
            && self.min.2 <= other.max.2
    }

    pub fn moved(self, by: Pos) -> Self {
        Self {
            min: add(self.min, by),
            max: add(self.max, by),
        }
    }

    pub fn inflated(self, radius: i32) -> Self {
        Self {
            min: sub(self.min, (radius, radius, radius)),
            max: add(self.max, (radius, radius, radius)),
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: (
                self.min.0.min(other.min.0),
                self.min.1.min(other.min.1),
                self.min.2.min(other.min.2),
            ),
            max: (
                self.max.0.max(other.max.0),
                self.max.1.max(other.max.1),
                self.max.2.max(other.max.2),
            ),
        }
    }

    pub fn y_span(self) -> i32 {
        self.max.1 - self.min.1 + 1
    }

    /// Jigsaw uses truncating Java division here, including negative coordinates.
    pub fn center_xz(self) -> (i32, i32) {
        (
            self.min.0.wrapping_add(self.max.0) / 2,
            self.min.2.wrapping_add(self.max.2) / 2,
        )
    }

    pub fn as_array(self) -> [i32; 6] {
        [
            self.min.0, self.min.1, self.min.2, self.max.0, self.max.1, self.max.2,
        ]
    }
}

/// NBT keeps numeric widths and list element types across extraction, template
/// processing and generated block/entity hand-off. JSON numbers alone lose these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List { element_type: u8, values: Vec<Nbt> },
    Compound(BTreeMap<String, Nbt>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Nbt {
    /// Lossless generated-data handoff. Lists explicitly retain their element
    /// type, including empty lists; numeric tags keep their original widths.
    pub fn typed_json(&self) -> Value {
        use serde_json::json;
        let (tag, payload) = match self {
            Self::Byte(v) => (1, json!(v)),
            Self::Short(v) => (2, json!(v)),
            Self::Int(v) => (3, json!(v)),
            Self::Long(v) => (4, json!(v)),
            Self::Float(v) => (5, json!(f64::from(*v))),
            Self::Double(v) => (6, json!(v)),
            Self::ByteArray(v) => (7, json!(v)),
            Self::String(v) => (8, json!(v)),
            Self::List {
                element_type,
                values,
            } => (
                9,
                json!({
                    "element_type": element_type,
                    "values": values.iter().map(Self::typed_json).collect::<Vec<_>>()
                }),
            ),
            Self::Compound(values) => (
                10,
                Value::Object(
                    values
                        .iter()
                        .map(|(k, v)| (k.clone(), v.typed_json()))
                        .collect(),
                ),
            ),
            Self::IntArray(v) => (11, json!(v)),
            Self::LongArray(v) => (12, json!(v)),
        };
        json!([tag, payload])
    }

    pub fn from_typed_json(value: &Value) -> Result<Self> {
        fn read(value: &Value, depth: usize) -> Result<Nbt> {
            if depth > 64 {
                return Err(invalid("NBT nesting exceeds 64"));
            }
            let row = value
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or_else(|| invalid("typed NBT pair"))?;
            let tag = row[0].as_u64().ok_or_else(|| invalid("NBT tag ID"))?;
            let value = &row[1];
            let integer = || value.as_i64().ok_or_else(|| invalid("NBT integer"));
            let number = || {
                value
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| invalid("finite NBT float"))
            };
            Ok(match tag {
                1 => Nbt::Byte(i8::try_from(integer()?).map_err(|_| invalid("NBT byte range"))?),
                2 => Nbt::Short(i16::try_from(integer()?).map_err(|_| invalid("NBT short range"))?),
                3 => Nbt::Int(i32::try_from(integer()?).map_err(|_| invalid("NBT int range"))?),
                4 => Nbt::Long(integer()?),
                5 => {
                    let v = number()?;
                    let single = v as f32;
                    if !single.is_finite() || f64::from(single).to_bits() != v.to_bits() {
                        return Err(invalid("NBT float is not exactly representable"));
                    }
                    Nbt::Float(single)
                }
                6 => Nbt::Double(number()?),
                7 => Nbt::ByteArray(
                    serde_json::from_value(value.clone()).map_err(|_| invalid("NBT byte array"))?,
                ),
                8 => Nbt::String(
                    value
                        .as_str()
                        .ok_or_else(|| invalid("NBT string"))?
                        .to_owned(),
                ),
                9 => {
                    let element_type = value["element_type"]
                        .as_u64()
                        .filter(|v| *v <= 12)
                        .ok_or_else(|| invalid("NBT list type"))?
                        as u8;
                    let mut values = Vec::new();
                    for child in value["values"]
                        .as_array()
                        .ok_or_else(|| invalid("NBT list values"))?
                    {
                        if element_type == 0 || child[0].as_u64() != Some(u64::from(element_type)) {
                            return Err(invalid("heterogeneous NBT list"));
                        }
                        values.push(read(child, depth + 1)?);
                    }
                    Nbt::List {
                        element_type,
                        values,
                    }
                }
                10 => Nbt::Compound(
                    value
                        .as_object()
                        .ok_or_else(|| invalid("NBT compound"))?
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), read(v, depth + 1)?)))
                        .collect::<Result<_>>()?,
                ),
                11 => Nbt::IntArray(
                    serde_json::from_value(value.clone()).map_err(|_| invalid("NBT int array"))?,
                ),
                12 => Nbt::LongArray(
                    serde_json::from_value(value.clone()).map_err(|_| invalid("NBT long array"))?,
                ),
                _ => return Err(invalid("NBT tag outside 1..=12")),
            })
        }
        read(value, 0)
    }

    /// Convert the older logical ListTag capture format to physical NBT. 26.1
    /// permits heterogeneous logical lists and escapes wrapper-shaped compounds.
    /// This is deliberately separate from the explicit physical-list decoder.
    pub fn from_logical_typed_json(value: &Value) -> Result<Self> {
        fn physical(value: &Value, depth: usize) -> Result<Value> {
            if depth > 64 {
                return Err(invalid("NBT nesting exceeds 64"));
            }
            let row = value
                .as_array()
                .filter(|row| row.len() == 2)
                .ok_or_else(|| invalid("logical typed NBT pair"))?;
            let tag = row[0]
                .as_u64()
                .filter(|tag| (1..=12).contains(tag))
                .ok_or_else(|| invalid("logical NBT tag ID"))?;
            let payload = match tag {
                9 => {
                    let values = row[1]
                        .as_array()
                        .ok_or_else(|| invalid("logical NBT list"))?;
                    let mut values = values
                        .iter()
                        .map(|value| physical(value, depth + 1))
                        .collect::<Result<Vec<_>>>()?;
                    let first = values
                        .first()
                        .and_then(|value| value[0].as_u64())
                        .unwrap_or(0);
                    let element_type =
                        if values.iter().all(|value| value[0].as_u64() == Some(first)) {
                            first
                        } else {
                            10
                        };
                    if element_type == 10 {
                        for value in &mut values {
                            let wrapper = value[0] != 10
                                || value[1].as_object().is_some_and(|fields| {
                                    fields.len() == 1 && fields.contains_key("")
                                });
                            if wrapper {
                                *value = serde_json::json!([10, {"": value.clone()}]);
                            }
                        }
                    }
                    serde_json::json!({"element_type": element_type, "values": values})
                }
                10 => Value::Object(
                    row[1]
                        .as_object()
                        .ok_or_else(|| invalid("logical NBT compound"))?
                        .iter()
                        .map(|(key, value)| Ok((key.clone(), physical(value, depth + 1)?)))
                        .collect::<Result<_>>()?,
                ),
                _ => row[1].clone(),
            };
            Ok(serde_json::json!([tag, payload]))
        }
        Self::from_typed_json(&physical(value, 0)?)
    }

    pub fn compound(&self) -> Result<&BTreeMap<String, Self>> {
        match self {
            Self::Compound(value) => Ok(value),
            _ => Err(invalid("expected NBT compound")),
        }
    }

    pub fn compound_mut(&mut self) -> Result<&mut BTreeMap<String, Self>> {
        match self {
            Self::Compound(value) => Ok(value),
            _ => Err(invalid("expected NBT compound")),
        }
    }

    pub fn string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i32> {
        match self {
            Self::Byte(v) => Some(i32::from(*v)),
            Self::Short(v) => Some(i32::from(*v)),
            Self::Int(v) => Some(*v),
            Self::Long(v) => Some(*v as i32),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(f64::from(*v)),
            Self::Double(v) => Some(*v),
            Self::Long(v) => Some(*v as f64),
            _ => self.int().map(f64::from),
        }
    }

    pub fn list(&self) -> Option<&[Self]> {
        match self {
            Self::List { values, .. } => Some(values),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Compound(values) => values.get(key),
            _ => None,
        }
    }

    pub fn empty_compound() -> Self {
        Self::Compound(BTreeMap::new())
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Compound(values) => values
                .iter()
                .map(|(k, v)| (k.clone(), v.to_json()))
                .collect(),
            Self::List { values, .. } => values.iter().map(Self::to_json).collect(),
            Self::String(v) => Value::from(v.clone()),
            Self::Byte(v) => Value::from(*v),
            Self::Short(v) => Value::from(*v),
            Self::Int(v) => Value::from(*v),
            Self::Long(v) => Value::from(*v),
            Self::Float(v) => Value::from(*v),
            Self::Double(v) => Value::from(*v),
            Self::ByteArray(v) => v.iter().copied().map(Value::from).collect(),
            Self::IntArray(v) => v.iter().copied().map(Value::from).collect(),
            Self::LongArray(v) => v.iter().copied().map(Value::from).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct BlockState {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(
        rename = "Properties",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub properties: BTreeMap<String, String>,
}

impl BlockState {
    pub fn parse(text: &str) -> Result<Self> {
        let (name, props) = match text.split_once('[') {
            Some((name, props)) => (
                name,
                props
                    .strip_suffix(']')
                    .ok_or_else(|| invalid(format!("invalid block state {text}")))?,
            ),
            None => (text, ""),
        };
        if name.is_empty() || name.contains(['{', '}', ' ', ']']) {
            return Err(invalid(format!("invalid block state {text}")));
        }
        let mut properties = BTreeMap::new();
        if !props.is_empty() {
            for entry in props.split(',') {
                let (key, value) = entry
                    .split_once('=')
                    .ok_or_else(|| invalid(format!("invalid block property {entry}")))?;
                if key.is_empty()
                    || value.is_empty()
                    || properties
                        .insert(key.to_owned(), value.to_owned())
                        .is_some()
                {
                    return Err(invalid(format!("invalid/duplicate block property {entry}")));
                }
            }
        }
        Ok(Self {
            name: id(name),
            properties,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockEntityType {
    pub type_id: u32,
    pub id: String,
    pub randomizable: bool,
    pub nbt: Nbt,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StateRow(
    pub u32,
    pub BlockState,
    pub [u32; 4],
    pub [u32; 3],
    pub u8,
    pub u8,
    pub bool,
);

#[derive(Debug)]
pub struct BlockRegistry {
    rows: Vec<StateRow>,
    states: HashMap<BlockState, u32>,
    defaults: BTreeMap<String, u32>,
    pub block_entities: BTreeMap<String, BlockEntityType>,
    pub water_fluid_id: u32,
}

impl BlockRegistry {
    pub(crate) fn new(
        rows: Vec<StateRow>,
        defaults: BTreeMap<String, u32>,
        block_entities: BTreeMap<String, BlockEntityType>,
        water_fluid_id: u32,
    ) -> Result<Self> {
        let count = rows.len() as u32;
        for (i, row) in rows.iter().enumerate() {
            if row.0 != i as u32 || row.2.iter().chain(&row.3).any(|v| *v >= count) {
                return Err(invalid("non-dense or invalid block-state registry"));
            }
        }
        let states = rows.iter().map(|r| (r.1.clone(), r.0)).collect();
        Ok(Self {
            rows,
            states,
            defaults,
            block_entities,
            water_fluid_id,
        })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn state(&self, state: u32) -> Result<&BlockState> {
        self.rows
            .get(state as usize)
            .map(|r| &r.1)
            .ok_or_else(|| missing(format!("block state {state}")))
    }

    pub fn default_state(&self, name: &str) -> Result<u32> {
        self.defaults
            .get(&id(name))
            .copied()
            .ok_or_else(|| missing(format!("block {name}")))
    }

    pub fn resolve(&self, spec: &BlockState) -> Result<u32> {
        let mut full = self.state(self.default_state(&spec.name)?)?.clone();
        for (key, value) in &spec.properties {
            if !full.properties.contains_key(key) {
                return Err(invalid(format!("{} has no property {key}", spec.name)));
            }
            full.properties.insert(key.clone(), value.clone());
        }
        self.states
            .get(&full)
            .copied()
            .ok_or_else(|| invalid(format!("invalid block state {spec:?}")))
    }

    pub fn parse_state(&self, text: &str) -> Result<u32> {
        self.resolve(&BlockState::parse(text)?)
    }

    pub fn transform(&self, state: u32, mirror: Mirror, rotation: Rotation) -> Result<u32> {
        let row = self
            .rows
            .get(state as usize)
            .ok_or_else(|| missing(format!("block state {state}")))?;
        Ok(self.rows[row.3[mirror.index()] as usize].2[rotation.index()])
    }

    pub fn flags(&self, state: u32) -> Result<u8> {
        self.rows
            .get(state as usize)
            .map(|r| r.4)
            .ok_or_else(|| missing(format!("block state {state}")))
    }

    pub fn fluid(&self, state: u32) -> Result<(u8, bool)> {
        self.rows
            .get(state as usize)
            .map(|r| (r.5, r.6))
            .ok_or_else(|| missing(format!("block state {state}")))
    }

    pub fn with_property(&self, state: u32, key: &str, value: &str) -> Result<u32> {
        let mut spec = self.state(state)?.clone();
        spec.properties.insert(key.to_owned(), value.to_owned());
        self.resolve(&spec)
    }

    pub fn default_block_entity(
        &self,
        state: u32,
        pos: Pos,
    ) -> Result<Option<TemplateBlockEntity>> {
        if self.flags(state)? & 8 == 0 {
            return Ok(None);
        }
        let name = &self.state(state)?.name;
        let ty = self
            .block_entities
            .get(name)
            .ok_or_else(|| missing(format!("block entity defaults {name}")))?;
        Ok(Some(TemplateBlockEntity {
            pos,
            state,
            type_id: ty.type_id,
            id: ty.id.clone(),
            load_data: Nbt::empty_compound(),
            loot_table: None,
            loot_seed: None,
            defaults: ty.nbt.clone(),
            single_item: None,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockInfo {
    pub pos: Pos,
    pub state: u32,
    pub nbt: Option<Nbt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityInfo {
    pub pos: [f64; 3],
    pub block_pos: Pos,
    pub nbt: Nbt,
}

#[derive(Debug, Clone)]
pub struct StructureTemplate {
    pub size: Pos,
    palettes: Vec<Vec<BlockInfo>>,
    pub entities: Vec<EntityInfo>,
}

#[derive(Debug, Clone)]
pub struct PlacementSettings {
    pub rotation: Rotation,
    pub mirror: Mirror,
    pub pivot: Pos,
    pub clip: Option<BoundingBox>,
    pub processors: Vec<Processor>,
    pub ignore_entities: bool,
    pub finalize_entities: bool,
    pub apply_waterlogging: bool,
    pub known_shape: bool,
    pub flags: i32,
    /// Used only by processors that explicitly query the level seed (Capped).
    pub world_seed: i64,
}

impl Default for PlacementSettings {
    fn default() -> Self {
        Self {
            rotation: Rotation::None,
            mirror: Mirror::None,
            pivot: (0, 0, 0),
            clip: None,
            processors: Vec::new(),
            ignore_entities: false,
            finalize_entities: false,
            apply_waterlogging: true,
            known_shape: true,
            flags: 18,
            world_seed: 0,
        }
    }
}

/// Caller-supplied settings randomness is separate from placement/loot randomness.
/// Without it, each query gets a freshly seeded positional LegacyRandomSource.
pub struct ProcessorRandom<'a>(pub Option<&'a mut dyn TemplateRandom>);

impl ProcessorRandom<'_> {
    pub fn at<T>(&mut self, pos: Pos, operation: impl FnOnce(&mut dyn TemplateRandom) -> T) -> T {
        match &mut self.0 {
            Some(random) => operation(*random),
            None => operation(&mut JavaRandom::new(get_seed(pos.0, pos.1, pos.2))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateBlockEntity {
    pub pos: Pos,
    pub state: u32,
    pub type_id: u32,
    pub id: String,
    /// Data passed to native loadWithComponents, after processor and loot edits.
    pub load_data: Nbt,
    pub loot_table: Option<String>,
    pub loot_seed: Option<i64>,
    defaults: Nbt,
    single_item: Option<Nbt>,
}

impl TemplateBlockEntity {
    pub fn from_load(
        registry: &BlockRegistry,
        state: u32,
        pos: Pos,
        load_data: Nbt,
    ) -> Result<Self> {
        load_data.compound()?;
        let mut entity = registry
            .default_block_entity(state, pos)?
            .ok_or_else(|| invalid("template data on a non-block-entity state"))?;
        entity.loot_table = load_data
            .get("LootTable")
            .and_then(Nbt::string)
            .map(str::to_owned);
        entity.loot_seed = match load_data.get("LootTableSeed") {
            Some(Nbt::Long(seed)) => Some(*seed),
            _ => None,
        };
        if matches!(
            entity.id.as_str(),
            "minecraft:brushable_block" | "minecraft:decorated_pot"
        ) {
            entity.loot_table = load_data
                .get("LootTable")
                .and_then(Nbt::string)
                .and_then(codec_identifier);
            entity.loot_seed = load_data.get("LootTableSeed").and_then(nbt_long_value);
            if load_data
                .get("components")
                .is_some_and(|data| !matches!(data, Nbt::Compound(fields) if fields.is_empty()))
            {
                return Err(FeatureError::Unsupported(format!(
                    "{} block data-component codecs",
                    entity.id
                )));
            }
            if entity.loot_table.is_none() {
                entity.single_item = decode_item_stack(load_data.get("item"))?;
            }
        }
        entity.load_data = load_data;
        Ok(entity)
    }

    /// Saved worldgen NBT for template containers. Retains typed fields; no loot
    /// is unpacked. Other block entities also expose their original load_data.
    pub fn full_data(&self) -> Nbt {
        let mut data = self
            .defaults
            .compound()
            .expect("validated defaults")
            .clone();
        if matches!(
            self.id.as_str(),
            "minecraft:brushable_block" | "minecraft:decorated_pot"
        ) {
            // Both save loot OR their decoded item; unknown load keys vanish.
            // Brushable hit direction belongs only to its update tag.
            if let Some(table) = &self.loot_table {
                data.insert("LootTable".into(), Nbt::String(table.clone()));
                if let Some(seed) = self.loot_seed.filter(|&seed| seed != 0) {
                    data.insert("LootTableSeed".into(), Nbt::Long(seed));
                }
            } else if let Some(item) = &self.single_item {
                data.insert("item".into(), item.clone());
            }
            if self.id == "minecraft:decorated_pot" {
                if let Some(sherds) = pot_decorations(self.load_data.get("sherds")) {
                    data.insert("sherds".into(), sherds);
                }
            }
        } else {
            data.extend(
                self.load_data
                    .compound()
                    .expect("validated load data")
                    .clone(),
            );
        }
        data.insert("id".into(), Nbt::String(self.id.clone()));
        for (name, value) in [("x", self.pos.0), ("y", self.pos.1), ("z", self.pos.2)] {
            data.insert(name.into(), Nbt::Int(value));
        }
        if self.loot_table.is_some() {
            data.remove("Items");
        }
        // Placement still draws and loads a seed for fixed-inventory containers,
        // but RandomizableContainer.trySaveLootTable writes it only with a table.
        if self.loot_table.is_none() || self.loot_seed == Some(0) {
            data.remove("LootTableSeed");
        }
        // BannerBlockEntity.saveAdditional omits BannerPatternLayers.EMPTY;
        // templates can explicitly load an empty list without saving it back.
        if self.id == "minecraft:banner"
            && data
                .get("patterns")
                .and_then(Nbt::list)
                .is_some_and(|values| values.is_empty())
        {
            data.remove("patterns");
        }
        if self.id == "minecraft:vault" {
            if let Some(Nbt::Compound(config)) = data.get_mut("config") {
                // VaultConfig's lenient optional field omits the ordinary reward
                // table; ominous vaults retain their different registry key.
                if config.get("loot_table").and_then(Nbt::string)
                    == Some("minecraft:chests/trial_chambers/reward")
                {
                    config.remove("loot_table");
                }
            }
        }
        if matches!(
            self.id.as_str(),
            "minecraft:sign" | "minecraft:hanging_sign"
        ) {
            for side in ["front_text", "back_text"] {
                if let Some(Nbt::Compound(text)) = data.get_mut(side) {
                    for field in ["messages", "filtered_messages"] {
                        if let Some(Nbt::List {
                            element_type,
                            values,
                        }) = text.get_mut(field)
                        {
                            for message in values.iter_mut() {
                                // ComponentSerialization encodes an unstyled literal
                                // as TAG_String after native sign load/save.
                                if let Nbt::Compound(component) = message {
                                    if component.len() == 1 {
                                        if let Some(Nbt::String(literal)) = component.get("text") {
                                            *message = Nbt::String(literal.clone());
                                        }
                                    }
                                }
                            }
                            if values
                                .iter()
                                .all(|message| matches!(message, Nbt::String(_)))
                            {
                                *element_type = if values.is_empty() { 0 } else { 8 };
                            } else if values
                                .iter()
                                .any(|message| matches!(message, Nbt::String(_)))
                            {
                                // 26.1 ListTag writes heterogeneous elements using
                                // physical compound wrappers with an empty key.
                                for message in values.iter_mut() {
                                    if !matches!(message, Nbt::Compound(_)) {
                                        *message = Nbt::Compound(BTreeMap::from([(
                                            "".into(),
                                            message.clone(),
                                        )]));
                                    }
                                }
                                *element_type = 10;
                            }
                        }
                    }
                }
            }
        }
        Nbt::Compound(data)
    }
}

/// Codecs use Number.intValue (truncation), unlike NumericTag.longValue below.
pub(crate) fn nbt_codec_int(value: &Nbt) -> Option<i32> {
    match value {
        Nbt::Float(value) => Some(*value as i32),
        Nbt::Double(value) => Some(*value as i32),
        _ => value.int(),
    }
}

fn nbt_long_value(value: &Nbt) -> Option<i64> {
    match value {
        Nbt::Byte(value) => Some(i64::from(*value)),
        Nbt::Short(value) => Some(i64::from(*value)),
        Nbt::Int(value) => Some(i64::from(*value)),
        Nbt::Long(value) => Some(*value),
        // FloatTag uses f2l; DoubleTag floors before d2l in the pinned JAR.
        Nbt::Float(value) => Some(*value as i64),
        Nbt::Double(value) => Some(value.floor() as i64),
        _ => None,
    }
}

fn codec_identifier(name: &str) -> Option<String> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    (namespace.bytes().all(valid) && path.bytes().all(|c| valid(c) || c == b'/'))
        .then(|| format!("{namespace}:{path}"))
}

fn item_names() -> &'static BTreeSet<String> {
    static ITEMS: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    ITEMS.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!(
            "../../data/brushable_block_entities_26_1_v2.json"
        ))
        .expect("pinned item registry");
        serde_json::from_value(data["item_names"].clone()).expect("native item identifiers")
    })
}

fn pot_decorations(value: Option<&Nbt>) -> Option<Nbt> {
    // TagValueInput retains ListCodec's partial successes. Invalid entries are
    // removed, and PotDecorations takes the first four items in back/left/right/
    // front order. Missing sides are bricks; all-brick decorations are omitted.
    let mut items: Vec<_> = value?
        .list()?
        .iter()
        .filter_map(Nbt::string)
        .filter_map(codec_identifier)
        .filter(|name| item_names().contains(name))
        .take(4)
        .collect();
    items.resize(4, "minecraft:brick".into());
    items
        .iter()
        .any(|name| name != "minecraft:brick")
        .then(|| Nbt::List {
            element_type: 8,
            values: items.into_iter().map(Nbt::String).collect(),
        })
}

fn decode_item_stack(value: Option<&Nbt>) -> Result<Option<Nbt>> {
    let Some(Nbt::Compound(input)) = value else {
        return Ok(None);
    };
    let Some(name) = input
        .get("id")
        .and_then(Nbt::string)
        .and_then(codec_identifier)
    else {
        return Ok(None);
    };
    if name == "minecraft:air" || !item_names().contains(&name) {
        return Ok(None);
    }
    if input
        .get("components")
        .is_some_and(|data| !matches!(data, Nbt::Compound(fields) if fields.is_empty()))
    {
        return Err(FeatureError::Unsupported(
            "template item data-component codecs".into(),
        ));
    }
    // ItemStack.CODEC lenientOptionalFieldOf defaults invalid counts to one.
    let count = input
        .get("count")
        .and_then(nbt_codec_int)
        .filter(|n| (1..=99).contains(n))
        .unwrap_or(1);
    Ok(Some(Nbt::Compound(BTreeMap::from([
        ("id".into(), Nbt::String(name)),
        ("count".into(), Nbt::Int(count)),
    ]))))
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateEntity {
    pub pos: [f64; 3],
    pub block_pos: Pos,
    pub nbt: Nbt,
    /// The entity consumer must perform STRUCTURE spawn finalization for mobs.
    pub finalize: bool,
    /// Applied by the native entity factory, after loading NBT. Entity subclasses
    /// implement their own mirror/rotation; rotating raw yaw here is incorrect.
    pub rotation: Rotation,
    pub mirror: Mirror,
}

#[derive(Debug, Clone)]
pub enum TemplateEffect {
    ClearBlockEntity(Pos),
    BlockEntity(TemplateBlockEntity),
    Entity(TemplateEntity),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataMarker {
    pub pos: Pos,
    pub metadata: String,
    pub nbt: Nbt,
}

#[derive(Debug, Default)]
pub struct PlacementResult {
    /// Native success means a usable template, even if every block was clipped.
    pub placed: bool,
    pub blocks_written: usize,
    pub block_entities: BTreeMap<Pos, TemplateBlockEntity>,
    pub cleared_block_entities: BTreeSet<Pos>,
    pub entities: Vec<TemplateEntity>,
}

impl PlacementResult {
    pub fn merge(&mut self, other: Self) {
        self.placed |= other.placed;
        self.blocks_written += other.blocks_written;
        for pos in &other.cleared_block_entities {
            self.block_entities.remove(pos);
        }
        self.cleared_block_entities
            .extend(other.cleared_block_entities);
        self.block_entities.extend(other.block_entities);
        self.entities.extend(other.entities);
    }
}

impl StructureTemplate {
    /// Palette block lists may have different states/order. Each is sorted by
    /// full static cubes, other blocks, NBT blocks; then Y, X, Z within each group.
    pub fn new(
        size: Pos,
        mut palettes: Vec<Vec<BlockInfo>>,
        entities: Vec<EntityInfo>,
        registry: &BlockRegistry,
    ) -> Result<Self> {
        if size.0 < 0 || size.1 < 0 || size.2 < 0 {
            return Err(invalid("negative template size"));
        }
        for palette in &mut palettes {
            for info in palette.iter() {
                registry.state(info.state)?;
                if let Some(nbt) = &info.nbt {
                    nbt.compound()?;
                }
            }
            palette.sort_by_key(|info| {
                let group = if info.nbt.is_some() {
                    2
                } else if registry.rows[info.state as usize].4 & 4 != 0 {
                    0
                } else {
                    1
                };
                (group, info.pos.1, info.pos.0, info.pos.2)
            });
        }
        for entity in &entities {
            entity.nbt.compound()?;
        }
        Ok(Self {
            size,
            palettes,
            entities,
        })
    }

    pub fn palettes(&self) -> &[Vec<BlockInfo>] {
        &self.palettes
    }

    pub fn palette(&self, origin: Pos) -> Result<&[BlockInfo]> {
        self.palette_with_random(origin, &mut ProcessorRandom(None))
    }

    pub fn palette_with_random(
        &self,
        origin: Pos,
        random: &mut ProcessorRandom<'_>,
    ) -> Result<&[BlockInfo]> {
        if self.palettes.is_empty() {
            return Err(invalid("template has no palettes"));
        }
        let index = random.at(origin, |r| r.next_int(self.palettes.len()));
        Ok(&self.palettes[index])
    }

    pub fn bounding_box(
        &self,
        origin: Pos,
        rotation: Rotation,
        mirror: Mirror,
        pivot: Pos,
    ) -> BoundingBox {
        BoundingBox::new(
            transform_block((0, 0, 0), mirror, rotation, pivot),
            transform_block(
                (self.size.0 - 1, self.size.1 - 1, self.size.2 - 1),
                mirror,
                rotation,
                pivot,
            ),
        )
        .moved(origin)
    }

    pub fn connected_position(
        first: &PlacementSettings,
        first_pos: Pos,
        second: &PlacementSettings,
        second_pos: Pos,
    ) -> Pos {
        sub(
            transform_block(first_pos, first.mirror, first.rotation, first.pivot),
            transform_block(second_pos, second.mirror, second.rotation, second.pivot),
        )
    }

    pub fn data_markers(
        &self,
        registry: &BlockRegistry,
        origin: Pos,
        settings: &PlacementSettings,
    ) -> Result<Vec<DataMarker>> {
        let mut result = Vec::new();
        for info in self.palette(origin)? {
            if registry.state(info.state)?.name != "minecraft:structure_block" {
                continue;
            }
            let Some(nbt) = &info.nbt else {
                continue;
            };
            if nbt.get("mode").and_then(Nbt::string) != Some("DATA") {
                continue;
            }
            let pos = add(
                transform_block(info.pos, settings.mirror, settings.rotation, settings.pivot),
                origin,
            );
            if settings.clip.is_some_and(|bb| !bb.contains(pos)) {
                continue;
            }
            result.push(DataMarker {
                pos,
                metadata: nbt
                    .get("metadata")
                    .and_then(Nbt::string)
                    .unwrap_or("")
                    .to_owned(),
                nbt: nbt.clone(),
            });
        }
        Ok(result)
    }

    pub fn place_in_world<W: FeatureWorld + ?Sized, R: TemplateRandom + ?Sized>(
        &self,
        world: &mut W,
        random: &mut R,
        registry: &BlockRegistry,
        origin: Pos,
        reference: Pos,
        settings: &PlacementSettings,
    ) -> Result<PlacementResult> {
        self.place_with_randoms(
            world,
            random,
            &mut ProcessorRandom(None),
            registry,
            origin,
            reference,
            settings,
        )
    }

    pub fn place_with_randoms<W: FeatureWorld + ?Sized, R: TemplateRandom + ?Sized>(
        &self,
        world: &mut W,
        random: &mut R,
        processor_random: &mut ProcessorRandom<'_>,
        registry: &BlockRegistry,
        origin: Pos,
        reference: Pos,
        settings: &PlacementSettings,
    ) -> Result<PlacementResult> {
        self.place_with_effects(
            world,
            random,
            processor_random,
            registry,
            origin,
            reference,
            settings,
            &mut |_, _| Ok(()),
        )
    }

    /// Immediate effect delivery for a retained world. The collected result is
    /// still returned, but a later error cannot discard already emitted data.
    pub fn place_with_effects<W, R, F>(
        &self,
        world: &mut W,
        random: &mut R,
        processor_random: &mut ProcessorRandom<'_>,
        registry: &BlockRegistry,
        origin: Pos,
        reference: Pos,
        settings: &PlacementSettings,
        effects: &mut F,
    ) -> Result<PlacementResult>
    where
        W: FeatureWorld + ?Sized,
        R: TemplateRandom + ?Sized,
        F: FnMut(&mut W, TemplateEffect) -> Result<()>,
    {
        if !settings.known_shape {
            return Err(FeatureError::Unsupported(
                "template neighbour-shape callbacks (known_shape=false)".into(),
            ));
        }
        let mut result = PlacementResult::default();
        if self.palettes.is_empty() {
            return Ok(result);
        }
        let palette = self.palette_with_random(origin, processor_random)?;
        if (palette.is_empty() && (settings.ignore_entities || self.entities.is_empty()))
            || self.size.0 < 1
            || self.size.1 < 1
            || self.size.2 < 1
        {
            return Ok(result);
        }
        let infos = process_block_infos(
            world,
            registry,
            origin,
            reference,
            palette,
            settings,
            processor_random,
        )?;
        let mut pending = Vec::new();
        let mut new_sources = BTreeSet::new();
        for mut info in infos {
            let pos = info.pos;
            if settings.clip.is_some_and(|bb| !bb.contains(pos)) || !world.can_write_feature(pos) {
                continue;
            }
            let previous = world
                .get_block(pos)
                .ok_or_else(|| missing(format!("template placement read {pos:?}")))?;
            let old_fluid = if settings.apply_waterlogging {
                registry.fluid(previous)?
            } else {
                (0, false)
            };
            let state = registry.transform(info.state, settings.mirror, settings.rotation)?;
            if registry.flags(previous)? & 8 != 0 {
                result.cleared_block_entities.insert(pos);
                result.block_entities.remove(&pos);
                effects(world, TemplateEffect::ClearBlockEntity(pos))?;
            }
            if info.nbt.is_some() {
                world.set_feature_block(pos, registry.default_state("barrier")?, 820);
            }
            if !world.set_feature_block(pos, state, settings.flags) {
                continue;
            }
            result.blocks_written += 1;
            if registry.flags(state)? & 8 != 0 {
                let kind = &registry.state(state)?.name;
                let ty = registry
                    .block_entities
                    .get(kind)
                    .ok_or_else(|| missing(format!("template block entity {kind}")))?;
                if ty.randomizable && info.nbt.is_some() {
                    info.nbt
                        .as_mut()
                        .expect("checked")
                        .compound_mut()?
                        .insert("LootTableSeed".into(), Nbt::Long(random.next_long()));
                }
                let load_data = info.nbt.unwrap_or_else(Nbt::empty_compound);
                let entity = TemplateBlockEntity::from_load(registry, state, pos, load_data)?;
                effects(world, TemplateEffect::BlockEntity(entity.clone()))?;
                result.block_entities.insert(pos, entity);
            }
            if settings.apply_waterlogging {
                if registry.fluid(state)?.1 {
                    new_sources.insert(pos);
                } else if registry.flags(state)? & 16 != 0 {
                    restore_water(world, registry, pos, state, old_fluid)?;
                    if !old_fluid.1 {
                        pending.push(pos);
                    }
                }
            }
        }
        // Water can propagate from existing sources, never from a source that
        // this template introduced. Repeat until no pending container changes.
        let mut progress = true;
        while progress && !pending.is_empty() {
            progress = false;
            let mut remaining = Vec::new();
            for pos in pending {
                let state = world
                    .get_block(pos)
                    .ok_or_else(|| missing(format!("template fluid read {pos:?}")))?;
                let mut fluid = registry.fluid(state)?;
                for step in [(0, 1, 0), (0, 0, -1), (1, 0, 0), (0, 0, 1), (-1, 0, 0)] {
                    if fluid.1 {
                        break;
                    }
                    let neighbour = add(pos, step);
                    let other = world.get_block(neighbour).ok_or_else(|| {
                        missing(format!("template fluid neighbour {neighbour:?}"))
                    })?;
                    let candidate = registry.fluid(other)?;
                    if candidate.1 && !new_sources.contains(&neighbour) {
                        fluid = candidate;
                    }
                }
                if fluid.1 && registry.flags(state)? & 16 != 0 {
                    restore_water(world, registry, pos, state, fluid)?;
                    progress = true;
                } else {
                    remaining.push(pos);
                }
            }
            pending = remaining;
        }
        for entity in result.block_entities.values_mut() {
            entity.state = world
                .get_block(entity.pos)
                .ok_or_else(|| missing(format!("template entity state {:?}", entity.pos)))?;
        }
        if !settings.ignore_entities {
            result.entities = self.place_entities(origin, settings)?;
            for entity in &result.entities {
                effects(world, TemplateEffect::Entity(entity.clone()))?;
            }
        }
        result.placed = true;
        Ok(result)
    }

    pub fn place_entities(
        &self,
        origin: Pos,
        settings: &PlacementSettings,
    ) -> Result<Vec<TemplateEntity>> {
        let mut result = Vec::new();
        for entity in &self.entities {
            let block_pos = add(
                transform_block(
                    entity.block_pos,
                    settings.mirror,
                    settings.rotation,
                    settings.pivot,
                ),
                origin,
            );
            if settings.clip.is_some_and(|bb| !bb.contains(block_pos)) {
                continue;
            }
            let mut pos = transform_entity(
                entity.pos,
                settings.mirror,
                settings.rotation,
                settings.pivot,
            );
            pos[0] += f64::from(origin.0);
            pos[1] += f64::from(origin.1);
            pos[2] += f64::from(origin.2);
            let mut nbt = entity.nbt.clone();
            let data = nbt.compound_mut()?;
            data.remove("UUID");
            data.insert(
                "Pos".into(),
                Nbt::List {
                    element_type: 6,
                    values: pos.into_iter().map(Nbt::Double).collect(),
                },
            );
            result.push(TemplateEntity {
                pos,
                block_pos,
                nbt,
                finalize: settings.finalize_entities,
                rotation: settings.rotation,
                mirror: settings.mirror,
            });
        }
        Ok(result)
    }
}

fn restore_water<W: FeatureWorld + ?Sized>(
    world: &mut W,
    registry: &BlockRegistry,
    pos: Pos,
    state: u32,
    fluid: (u8, bool),
) -> Result<()> {
    if fluid != (1, true)
        || registry
            .state(state)?
            .properties
            .get("waterlogged")
            .map(String::as_str)
            != Some("false")
    {
        return Ok(());
    }
    let wet = registry.with_property(state, "waterlogged", "true")?;
    world.set_feature_block(pos, wet, 3);
    world.schedule_feature_tick(TickRequest {
        block_pos: [pos.0, pos.1, pos.2],
        target: TickTarget::Fluid(registry.water_fluid_id),
        delay: 5,
    });
    Ok(())
}
