//! Native non-jigsaw overworld starts and source-clipped piece placement.
//!
//! The scheduler owns start/reference retention and the per-step structure RNG.
//! Mob effects are explicit factory/finalization requests, not spawned entities.

use super::jigsaw::HeightContext;
use super::placement::{
    large_feature_random, FrequencyReduction, RandomSpreadPlacement, SpreadType,
};
use super::template::{
    add, invalid, missing, BlockRegistry, BoundingBox, Mirror, Nbt, Result, Rotation,
    TemplateRandom,
};
use super::template_pool::StructureAssets;
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::ore::OreWorld;
use crate::tick_request::{TickRequest, TickTarget};
use bcore_core::ChunkPos;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[path = "desert_pyramid.rs"]
pub mod desert_pyramid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScatteredKind {
    BuriedTreasure,
    SwampHut,
    JungleTemple,
    DesertPyramid,
}

impl ScatteredKind {
    pub const ALL: [Self; 4] = [
        Self::BuriedTreasure,
        Self::SwampHut,
        Self::JungleTemple,
        Self::DesertPyramid,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::BuriedTreasure => "minecraft:buried_treasure",
            Self::SwampHut => "minecraft:swamp_hut",
            Self::JungleTemple => "minecraft:jungle_pyramid",
            Self::DesertPyramid => "minecraft:desert_pyramid",
        }
    }

    pub fn from_name(name: &str) -> Result<Self> {
        match name.trim_start_matches("minecraft:") {
            "buried_treasure" => Ok(Self::BuriedTreasure),
            "swamp_hut" => Ok(Self::SwampHut),
            "jungle_pyramid" => Ok(Self::JungleTemple),
            "desert_pyramid" => Ok(Self::DesertPyramid),
            _ => Err(FeatureError::Unsupported(format!(
                "scattered structure {name}"
            ))),
        }
    }

    fn piece_id(self) -> &'static str {
        match self {
            Self::BuriedTreasure => "minecraft:btp",
            Self::SwampHut => "minecraft:tesh",
            Self::JungleTemple => "minecraft:tejp",
            Self::DesertPyramid => "minecraft:tedp",
        }
    }

    pub fn admission_heightmap(self) -> FeatureHeightmap {
        match self {
            Self::BuriedTreasure => FeatureHeightmap::OceanFloorWg,
            Self::SwampHut | Self::JungleTemple | Self::DesertPyramid => {
                FeatureHeightmap::WorldSurfaceWg
            }
        }
    }

    fn dimensions(self) -> Option<(i32, i32, i32)> {
        match self {
            Self::BuriedTreasure => None,
            Self::SwampHut => Some((7, 7, 9)),
            Self::JungleTemple => Some((12, 10, 15)),
            Self::DesertPyramid => Some((21, 15, 21)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScatteredConfig {
    pub kind: ScatteredKind,
    pub structure_set: String,
    pub structure_id: u32,
    pub decoration_step: i32,
    /// Index among STRUCTURES in this step, never a biome placed-feature index.
    pub structure_index: i32,
    pub biomes: Vec<u32>,
    pub placement: RandomSpreadPlacement,
    pub locate_offset: Pos,
    /// Includes the native piece-bounded witch/cat spawn overrides for swamp huts.
    pub native_config: Value,
}

#[derive(Debug, Clone, Copy)]
pub struct ScatteredBlockProperties {
    pub air: bool,
    pub liquid: bool,
    pub solid_render: bool,
    pub replaceable_by_structures: bool,
    pub ocean_floor: bool,
    pub motion_blocking_no_leaves: bool,
    pub shape_check: bool,
    /// BuiltInRegistries.FLUID ID; zero is empty.
    pub fluid_id: u32,
}

#[derive(Debug)]
pub struct ScatteredCatalog {
    configs: BTreeMap<ScatteredKind, ScatteredConfig>,
    properties: Vec<ScatteredBlockProperties>,
}

impl ScatteredCatalog {
    pub fn bundled() -> &'static Self {
        static DATA: OnceLock<ScatteredCatalog> = OnceLock::new();
        DATA.get_or_init(|| {
            // Keep the published three-family catalog immutable. The additional
            // native family carries its own independently repeated provenance.
            let mut data: Value = serde_json::from_str(include_str!(
                "../../data/scattered_structure_catalog_26_1_v2.json"
            ))
            .expect("pinned scattered catalog");
            let pyramid: Value =
                serde_json::from_str(include_str!("../../data/desert_pyramid_catalog_26_1.json"))
                    .expect("pinned desert-pyramid catalog");
            assert_eq!(pyramid["jar_sha256"], data["jar_sha256"]);
            data["catalog"]
                .as_array_mut()
                .unwrap()
                .extend(pyramid["catalog"].as_array().unwrap().iter().cloned());
            Self::from_json(&data.to_string()).expect("pinned native scattered-structure catalog")
        })
    }

    pub fn from_json(source: &str) -> Result<Self> {
        let data: Value = serde_json::from_str(source).map_err(|e| invalid(e.to_string()))?;
        if data["jar_sha256"] != StructureAssets::JAR_SHA256 {
            return Err(invalid("scattered catalog JAR differs from pinned 26.1"));
        }
        let mut configs = BTreeMap::new();
        for row in data["catalog"]
            .as_array()
            .ok_or_else(|| invalid("scattered catalog"))?
        {
            let kind = ScatteredKind::from_name(text(row, "kind")?)?;
            let config = &row["config"]["value"];
            let expected_type = if kind == ScatteredKind::JungleTemple {
                "minecraft:jungle_temple"
            } else {
                kind.name()
            };
            if text(config, "type")? != expected_type
                || config["terrain_adaptation"].as_str().unwrap_or("none") != "none"
            {
                return Err(FeatureError::Unsupported(
                    "scattered structure terrain adaptation".into(),
                ));
            }
            let set = &row["placement"]["value"];
            let entries = set["structures"]
                .as_array()
                .ok_or_else(|| invalid("scattered structure set"))?;
            if entries.len() != 1
                || entries[0]["structure"] != kind.name()
                || integer(&entries[0], "weight")? != 1
            {
                return Err(FeatureError::Unsupported(
                    "weighted scattered structure set".into(),
                ));
            }
            let placement = &set["placement"];
            if text(placement, "type")? != "minecraft:random_spread"
                || placement.get("exclusion_zone").is_some()
            {
                return Err(FeatureError::Unsupported(
                    "scattered placement or exclusion zone".into(),
                ));
            }
            let reduction = match placement["frequency_reduction_method"]
                .as_str()
                .unwrap_or("default")
            {
                "default" => FrequencyReduction::Default,
                "legacy_type_1" => FrequencyReduction::LegacyType1,
                "legacy_type_2" => FrequencyReduction::LegacyType2,
                "legacy_type_3" => FrequencyReduction::LegacyType3,
                _ => return Err(invalid("scattered frequency reduction")),
            };
            let spread = match placement["spread_type"].as_str().unwrap_or("linear") {
                "linear" => SpreadType::Linear,
                "triangular" => SpreadType::Triangular,
                _ => return Err(invalid("scattered spread type")),
            };
            let locate_offset = if let Some(offset) = placement.get("locate_offset") {
                json_pos(offset)?
            } else {
                (0, 0, 0)
            };
            let biomes = row["biomes"]
                .as_array()
                .ok_or_else(|| invalid("scattered biomes"))?
                .iter()
                .map(|b| {
                    b[0].as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| invalid("scattered biome ID"))
                })
                .collect::<Result<Vec<_>>>()?;
            let configuration = ScatteredConfig {
                kind,
                structure_set: format!("minecraft:{}", text(row, "set")?),
                structure_id: u32::try_from(integer(row, "structure_id")?)
                    .map_err(|_| invalid("structure ID"))?,
                decoration_step: integer(row, "step")?,
                structure_index: integer(row, "index")?,
                biomes,
                placement: RandomSpreadPlacement::new(
                    integer(placement, "spacing")?,
                    integer(placement, "separation")?,
                    integer(placement, "salt")?,
                    spread,
                    placement["frequency"].as_f64().unwrap_or(1.0) as f32,
                    reduction,
                )
                .ok_or_else(|| invalid("scattered random-spread settings"))?,
                locate_offset,
                native_config: config.clone(),
            };
            if configuration.structure_index < 0
                || !(0..11).contains(&configuration.decoration_step)
                || configs.insert(kind, configuration).is_some()
            {
                return Err(invalid("scattered registry metadata"));
            }
        }
        if ScatteredKind::ALL
            .iter()
            .any(|kind| !configs.contains_key(kind))
        {
            return Err(missing("scattered structure metadata"));
        }
        let state_count = data["block_properties"]["state_count"]
            .as_u64()
            .ok_or_else(|| invalid("scattered state count"))?;
        if state_count != 29_873 {
            return Err(invalid("scattered state count differs from 26.1"));
        }
        let mut properties = Vec::with_capacity(state_count as usize);
        for row in data["block_properties"]["ranges"]
            .as_array()
            .ok_or_else(|| invalid("scattered predicates"))?
        {
            let [first, end, flags, fluid_id]: [u32; 4] = serde_json::from_value(row.clone())
                .map_err(|_| invalid("scattered predicate range"))?;
            if first as usize != properties.len()
                || end <= first
                || u64::from(end) > state_count
                || flags >= 128
                || fluid_id >= 5
            {
                return Err(invalid("scattered predicate range bounds"));
            }
            properties.resize(
                end as usize,
                ScatteredBlockProperties {
                    air: flags & 1 != 0,
                    liquid: flags & 2 != 0,
                    solid_render: flags & 4 != 0,
                    replaceable_by_structures: flags & 8 != 0,
                    ocean_floor: flags & 16 != 0,
                    motion_blocking_no_leaves: flags & 32 != 0,
                    shape_check: flags & 64 != 0,
                    fluid_id,
                },
            );
        }
        if properties.len() != state_count as usize {
            return Err(missing("incomplete scattered block predicates"));
        }
        Ok(Self {
            configs,
            properties,
        })
    }

    pub fn config(&self, kind: ScatteredKind) -> &ScatteredConfig {
        &self.configs[&kind]
    }

    pub fn block_properties(&self, state: u32) -> Result<ScatteredBlockProperties> {
        self.properties
            .get(state as usize)
            .copied()
            .ok_or_else(|| missing(format!("scattered block state {state}")))
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| invalid(format!("scattered {key}")))
}

fn integer(value: &Value, key: &str) -> Result<i32> {
    json_integer(&value[key])
}

fn json_integer(value: &Value) -> Result<i32> {
    let value = value.as_f64().ok_or_else(|| invalid("scattered integer"))?;
    if !value.is_finite()
        || value != value.trunc()
        || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value)
    {
        return Err(invalid("scattered integer range"));
    }
    Ok(value as i32)
}

fn json_pos(value: &Value) -> Result<Pos> {
    let a = value
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| invalid("scattered position"))?;
    Ok((
        json_integer(&a[0])?,
        json_integer(&a[1])?,
        json_integer(&a[2])?,
    ))
}

/// Native Direction.Plane.HORIZONTAL draw order differs from the saved O value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HorizontalDirection {
    North,
    East,
    South,
    West,
}

impl HorizontalDirection {
    pub const ALL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

    pub fn name(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::East => "east",
            Self::South => "south",
            Self::West => "west",
        }
    }

    pub fn data_value(self) -> i32 {
        match self {
            Self::South => 0,
            Self::West => 1,
            Self::North => 2,
            Self::East => 3,
        }
    }

    fn from_data(value: i32) -> Result<Option<Self>> {
        Ok(match value {
            -1 => None,
            0 => Some(Self::South),
            1 => Some(Self::West),
            2 => Some(Self::North),
            3 => Some(Self::East),
            _ => return Err(invalid("scattered orientation")),
        })
    }

    fn step(self) -> Pos {
        match self {
            Self::North => (0, 0, -1),
            Self::East => (1, 0, 0),
            Self::South => (0, 0, 1),
            Self::West => (-1, 0, 0),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::North => Self::South,
            Self::East => Self::West,
            Self::South => Self::North,
            Self::West => Self::East,
        }
    }

    fn clockwise(self) -> Self {
        match self {
            Self::North => Self::East,
            Self::East => Self::South,
            Self::South => Self::West,
            Self::West => Self::North,
        }
    }

    fn transform(self) -> (Mirror, Rotation) {
        match self {
            Self::North => (Mirror::None, Rotation::None),
            Self::South => (Mirror::LeftRight, Rotation::None),
            Self::West => (Mirror::LeftRight, Rotation::Clockwise90),
            Self::East => (Mirror::None, Rotation::Clockwise90),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScatteredPieceData {
    BuriedTreasure,
    SwampHut {
        height_position: i32,
        spawned_witch: bool,
        spawned_cat: bool,
    },
    JungleTemple {
        height_position: i32,
        placed_main_chest: bool,
        placed_hidden_chest: bool,
        placed_trap1: bool,
        placed_trap2: bool,
    },
    DesertPyramid {
        height_position: i32,
        /// Native Direction.get2DDataValue order: south, west, north, east.
        has_placed_chest: [bool; 4],
        /// Retained in live holders; deliberately absent from native piece NBT.
        archaeology: desert_pyramid::ArchaeologyState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScatteredPiece {
    pub bounds: BoundingBox,
    pub orientation: Option<HorizontalDirection>,
    pub generation_depth: i32,
    pub data: ScatteredPieceData,
}

impl ScatteredPiece {
    pub fn kind(&self) -> ScatteredKind {
        match self.data {
            ScatteredPieceData::BuriedTreasure => ScatteredKind::BuriedTreasure,
            ScatteredPieceData::SwampHut { .. } => ScatteredKind::SwampHut,
            ScatteredPieceData::JungleTemple { .. } => ScatteredKind::JungleTemple,
            ScatteredPieceData::DesertPyramid { .. } => ScatteredKind::DesertPyramid,
        }
    }

    pub fn world_pos(&self, (x, y, z): Pos) -> Pos {
        let b = self.bounds;
        match self.orientation {
            None => (x, y, z),
            Some(HorizontalDirection::North) => (
                b.min.0.wrapping_add(x),
                b.min.1.wrapping_add(y),
                b.max.2.wrapping_sub(z),
            ),
            Some(HorizontalDirection::South) => (
                b.min.0.wrapping_add(x),
                b.min.1.wrapping_add(y),
                b.min.2.wrapping_add(z),
            ),
            Some(HorizontalDirection::West) => (
                b.max.0.wrapping_sub(z),
                b.min.1.wrapping_add(y),
                b.min.2.wrapping_add(x),
            ),
            Some(HorizontalDirection::East) => (
                b.min.0.wrapping_add(z),
                b.min.1.wrapping_add(y),
                b.min.2.wrapping_add(x),
            ),
        }
    }

    pub fn to_nbt(&self) -> Nbt {
        let mut data = BTreeMap::from([
            ("id".into(), Nbt::String(self.kind().piece_id().into())),
            ("BB".into(), Nbt::IntArray(self.bounds.as_array().to_vec())),
            (
                "O".into(),
                Nbt::Int(self.orientation.map_or(-1, HorizontalDirection::data_value)),
            ),
            ("GD".into(), Nbt::Int(self.generation_depth)),
        ]);
        if let Some((width, height, depth)) = self.kind().dimensions() {
            let height_position = match self.data {
                ScatteredPieceData::SwampHut {
                    height_position, ..
                }
                | ScatteredPieceData::JungleTemple {
                    height_position, ..
                }
                | ScatteredPieceData::DesertPyramid {
                    height_position, ..
                } => height_position,
                ScatteredPieceData::BuriedTreasure => unreachable!(),
            };
            for (key, value) in [
                ("Width", width),
                ("Height", height),
                ("Depth", depth),
                ("HPos", height_position),
            ] {
                data.insert(key.into(), Nbt::Int(value));
            }
        }
        match self.data {
            ScatteredPieceData::BuriedTreasure => {}
            ScatteredPieceData::SwampHut {
                spawned_witch,
                spawned_cat,
                ..
            } => {
                data.insert("Witch".into(), Nbt::Byte(i8::from(spawned_witch)));
                data.insert("Cat".into(), Nbt::Byte(i8::from(spawned_cat)));
            }
            ScatteredPieceData::JungleTemple {
                placed_main_chest,
                placed_hidden_chest,
                placed_trap1,
                placed_trap2,
                ..
            } => {
                for (key, flag) in [
                    ("placedMainChest", placed_main_chest),
                    ("placedHiddenChest", placed_hidden_chest),
                    ("placedTrap1", placed_trap1),
                    ("placedTrap2", placed_trap2),
                ] {
                    data.insert(key.into(), Nbt::Byte(i8::from(flag)));
                }
            }
            ScatteredPieceData::DesertPyramid {
                has_placed_chest, ..
            } => {
                for (index, flag) in has_placed_chest.into_iter().enumerate() {
                    data.insert(format!("hasPlacedChest{index}"), Nbt::Byte(i8::from(flag)));
                }
            }
        }
        Nbt::Compound(data)
    }

    pub fn from_nbt(nbt: &Nbt) -> Result<Self> {
        let name = nbt
            .get("id")
            .and_then(Nbt::string)
            .ok_or_else(|| invalid("scattered piece ID"))?;
        let bounds = match nbt.get("BB") {
            Some(Nbt::IntArray(a)) if a.len() == 6 => BoundingBox {
                min: (a[0], a[1], a[2]),
                max: (a[3], a[4], a[5]),
            },
            _ => return Err(invalid("scattered piece bounding box")),
        };
        let data = match name {
            "minecraft:btp" => ScatteredPieceData::BuriedTreasure,
            "minecraft:tesh" => {
                if [("Width", 7), ("Height", 7), ("Depth", 9)]
                    .into_iter()
                    .any(|(key, expected)| nbt_int(nbt, key) != Ok(expected))
                {
                    return Err(FeatureError::Unsupported(
                        "non-native swamp-hut dimensions".into(),
                    ));
                }
                ScatteredPieceData::SwampHut {
                    height_position: nbt_int(nbt, "HPos")?,
                    spawned_witch: nbt_bool(nbt, "Witch")?,
                    spawned_cat: nbt_bool(nbt, "Cat")?,
                }
            }
            "minecraft:tejp" => {
                if [("Width", 12), ("Height", 10), ("Depth", 15)]
                    .into_iter()
                    .any(|(key, expected)| nbt_int(nbt, key) != Ok(expected))
                {
                    return Err(FeatureError::Unsupported(
                        "non-native jungle-temple dimensions".into(),
                    ));
                }
                ScatteredPieceData::JungleTemple {
                    height_position: nbt_int(nbt, "HPos")?,
                    placed_main_chest: nbt_bool(nbt, "placedMainChest")?,
                    placed_hidden_chest: nbt_bool(nbt, "placedHiddenChest")?,
                    placed_trap1: nbt_bool(nbt, "placedTrap1")?,
                    placed_trap2: nbt_bool(nbt, "placedTrap2")?,
                }
            }
            "minecraft:tedp" => {
                if [("Width", 21), ("Height", 15), ("Depth", 21)]
                    .into_iter()
                    .any(|(key, expected)| nbt_int(nbt, key) != Ok(expected))
                {
                    return Err(FeatureError::Unsupported(
                        "non-native desert-pyramid dimensions".into(),
                    ));
                }
                ScatteredPieceData::DesertPyramid {
                    height_position: nbt_int(nbt, "HPos")?,
                    has_placed_chest: [
                        nbt_bool(nbt, "hasPlacedChest0")?,
                        nbt_bool(nbt, "hasPlacedChest1")?,
                        nbt_bool(nbt, "hasPlacedChest2")?,
                        nbt_bool(nbt, "hasPlacedChest3")?,
                    ],
                    archaeology: Default::default(),
                }
            }
            _ => return Err(FeatureError::Unsupported(format!("scattered piece {name}"))),
        };
        let piece = Self {
            bounds,
            orientation: HorizontalDirection::from_data(nbt_int(nbt, "O")?)?,
            generation_depth: nbt_int(nbt, "GD")?,
            data,
        };
        if !piece.valid() {
            return Err(invalid("scattered piece geometry"));
        }
        Ok(piece)
    }

    fn valid(&self) -> bool {
        let b = self.bounds;
        if b.min.0 > b.max.0 || b.min.1 > b.max.1 || b.min.2 > b.max.2 || self.generation_depth != 0
        {
            return false;
        }
        let spans = [
            i64::from(b.max.0) - i64::from(b.min.0) + 1,
            i64::from(b.max.1) - i64::from(b.min.1) + 1,
            i64::from(b.max.2) - i64::from(b.min.2) + 1,
        ];
        match self.kind().dimensions() {
            None => self.orientation.is_none() && spans == [1, 1, 1],
            Some((width, height, depth)) => match self.orientation {
                Some(HorizontalDirection::North | HorizontalDirection::South) => {
                    spans == [i64::from(width), i64::from(height), i64::from(depth)]
                }
                Some(HorizontalDirection::East | HorizontalDirection::West) => {
                    spans == [i64::from(depth), i64::from(height), i64::from(width)]
                }
                None => false,
            },
        }
    }
}

fn nbt_int(nbt: &Nbt, name: &str) -> Result<i32> {
    match nbt.get(name) {
        Some(Nbt::Int(n)) => Ok(*n),
        _ => Err(invalid(format!("scattered NBT integer {name}"))),
    }
}

fn nbt_bool(nbt: &Nbt, name: &str) -> Result<bool> {
    match nbt.get(name) {
        Some(Nbt::Byte(0)) => Ok(false),
        Some(Nbt::Byte(1)) => Ok(true),
        _ => Err(invalid(format!("scattered NBT boolean {name}"))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScatteredStart {
    pub kind: ScatteredKind,
    pub source: [i32; 2],
    /// Native start NBT does not save the generation stub position.
    pub generation_point: Option<Pos>,
    pub piece: ScatteredPiece,
    pub references: i32,
    /// Native getBoundingBox caches before the mutable piece moves during FEATURES.
    #[serde(default)]
    cached_reference_bounds: Option<BoundingBox>,
}

impl ScatteredStart {
    pub fn bounds(&self) -> BoundingBox {
        self.piece.bounds
    }

    pub fn reference_bounds(&mut self) -> BoundingBox {
        *self
            .cached_reference_bounds
            .get_or_insert(self.piece.bounds)
    }

    /// STRUCTURE_REFERENCES intersects X/Z only, not the piece's initial Y=90.
    pub fn references_chunk(&mut self, target: ChunkPos) -> bool {
        if (i64::from(target.x) - i64::from(self.source[0])).abs() > 8
            || (i64::from(target.z) - i64::from(self.source[1])).abs() > 8
        {
            return false;
        }
        let b = self.reference_bounds();
        let x = target.x.wrapping_mul(16);
        let z = target.z.wrapping_mul(16);
        b.max.0 >= x
            && b.min.0 <= x.wrapping_add(15)
            && b.max.2 >= z
            && b.min.2 <= z.wrapping_add(15)
    }

    pub fn to_nbt(&self) -> Nbt {
        Nbt::Compound(BTreeMap::from([
            ("id".into(), Nbt::String(self.kind.name().into())),
            ("ChunkX".into(), Nbt::Int(self.source[0])),
            ("ChunkZ".into(), Nbt::Int(self.source[1])),
            ("references".into(), Nbt::Int(self.references)),
            (
                "Children".into(),
                Nbt::List {
                    element_type: 10,
                    values: vec![self.piece.to_nbt()],
                },
            ),
        ]))
    }

    /// A native disk reload clears the cached reference BB, as StructureStart does.
    /// Serde retains it separately for live-holder snapshots.
    pub fn from_nbt(nbt: &Nbt) -> Result<Self> {
        let kind = ScatteredKind::from_name(
            nbt.get("id")
                .and_then(Nbt::string)
                .ok_or_else(|| invalid("scattered start ID"))?,
        )?;
        let children = nbt
            .get("Children")
            .and_then(Nbt::list)
            .filter(|a| a.len() == 1)
            .ok_or_else(|| invalid("scattered start children"))?;
        let start = Self {
            kind,
            source: [nbt_int(nbt, "ChunkX")?, nbt_int(nbt, "ChunkZ")?],
            generation_point: None,
            piece: ScatteredPiece::from_nbt(&children[0])?,
            references: nbt_int(nbt, "references")?,
            cached_reference_bounds: None,
        };
        if !start.valid_for(kind.name(), ChunkPos::new(start.source[0], start.source[1])) {
            return Err(invalid("scattered start geometry"));
        }
        Ok(start)
    }

    pub fn valid_for(&self, name: &str, owner: ChunkPos) -> bool {
        let near = |p: Pos| {
            (i64::from(p.0) - i64::from(owner.x) * 16).abs() <= 256
                && (i64::from(p.2) - i64::from(owner.z) * 16).abs() <= 256
                && (crate::MIN_Y - 4096..=crate::MAX_Y + 4096).contains(&p.1)
        };
        self.source == [owner.x, owner.z]
            && self.kind.name() == name
            && self.kind == self.piece.kind()
            && self.references >= 0
            && self.piece.valid()
            && near(self.piece.bounds.min)
            && near(self.piece.bounds.max)
            && self.generation_point.is_none_or(near)
            && match &self.piece.data {
                ScatteredPieceData::DesertPyramid { archaeology, .. } => {
                    archaeology.potential.iter().copied().all(near)
                        && (archaeology.roof == (0, 0, 0) || near(archaeology.roof))
                }
                _ => true,
            }
            && self.cached_reference_bounds.is_none_or(|b| {
                near(b.min)
                    && near(b.max)
                    && b.min.0 == self.piece.bounds.min.0
                    && b.max.0 == self.piece.bounds.max.0
                    && b.min.2 == self.piece.bounds.min.2
                    && b.max.2 == self.piece.bounds.max.2
                    && b.y_span() == self.piece.bounds.y_span()
            })
    }
}

/// Candidate/frequency gate followed by native centre/height/noise-biome admission.
/// The callback accepts absolute block coordinates and must query the generator's
/// quart noise biome, not the zoomed block biome. These native sets have one entry.
pub fn for_chunk(
    kind: ScatteredKind,
    seed: i64,
    chunk: ChunkPos,
    heights: &HeightContext<'_>,
    noise_biome: impl FnMut(Pos) -> u32,
) -> Result<Option<ScatteredStart>> {
    for_chunk_with_sea_level(kind, seed, chunk, heights, crate::SEA_LEVEL, noise_biome)
}

/// Explicit generator sea level for flat/custom generators. The standard entry
/// point uses the pinned overworld sea level (63).
pub fn for_chunk_with_sea_level(
    kind: ScatteredKind,
    seed: i64,
    chunk: ChunkPos,
    heights: &HeightContext<'_>,
    sea_level: i32,
    mut noise_biome: impl FnMut(Pos) -> u32,
) -> Result<Option<ScatteredStart>> {
    let config = ScatteredCatalog::bundled().config(kind);
    if !config.placement.is_candidate(seed, chunk) {
        return Ok(None);
    }
    assemble_with_sea_level(
        kind,
        chunk,
        heights,
        sea_level,
        &mut large_feature_random(seed, chunk),
        |p| config.biomes.contains(&noise_biome(p)),
    )
}

/// Assembly excludes the structure-set gate, just like Structure.generate.
pub fn generate_start(
    kind: ScatteredKind,
    seed: i64,
    chunk: ChunkPos,
    heights: &HeightContext<'_>,
    valid_biome: impl FnOnce(Pos) -> bool,
) -> Result<Option<ScatteredStart>> {
    assemble(
        kind,
        chunk,
        heights,
        &mut large_feature_random(seed, chunk),
        valid_biome,
    )
}

/// Caller-owned legacy RNG entry point for native generation-context continuations.
pub fn assemble(
    kind: ScatteredKind,
    chunk: ChunkPos,
    heights: &HeightContext<'_>,
    random: &mut (impl TemplateRandom + ?Sized),
    valid_biome: impl FnOnce(Pos) -> bool,
) -> Result<Option<ScatteredStart>> {
    assemble_with_sea_level(kind, chunk, heights, crate::SEA_LEVEL, random, valid_biome)
}

pub fn assemble_with_sea_level(
    kind: ScatteredKind,
    chunk: ChunkPos,
    heights: &HeightContext<'_>,
    sea_level: i32,
    random: &mut (impl TemplateRandom + ?Sized),
    valid_biome: impl FnOnce(Pos) -> bool,
) -> Result<Option<ScatteredStart>> {
    let x = chunk.x.wrapping_mul(16);
    let z = chunk.z.wrapping_mul(16);
    if matches!(
        kind,
        ScatteredKind::JungleTemple | ScatteredKind::DesertPyramid
    ) {
        // SinglePieceStructure uses width/depth, not width-1/depth-1, before
        // selecting the piece orientation. It samples all four corners.
        let (width, _, depth) = kind.dimensions().unwrap();
        let corners = [(0, 0), (0, depth), (width, 0), (width, depth)].map(|(dx, dz)| {
            (heights.first_free)(
                FeatureHeightmap::WorldSurfaceWg,
                x.wrapping_add(dx),
                z.wrapping_add(dz),
            )
            .wrapping_sub(1)
        });
        if *corners.iter().min().expect("four corners") < sea_level {
            return Ok(None);
        }
    }
    let (cx, cz) = (x.wrapping_add(8), z.wrapping_add(8));
    let point = (
        cx,
        (heights.first_free)(kind.admission_heightmap(), cx, cz).wrapping_sub(1),
        cz,
    );
    // findValidGenerationPoint filters the lazy stub before constructing its piece.
    if !valid_biome(point) {
        return Ok(None);
    }
    let piece = match kind {
        ScatteredKind::BuriedTreasure => {
            let p = (x.wrapping_add(9), 90, z.wrapping_add(9));
            ScatteredPiece {
                bounds: BoundingBox::new(p, p),
                orientation: None,
                generation_depth: 0,
                data: ScatteredPieceData::BuriedTreasure,
            }
        }
        ScatteredKind::SwampHut | ScatteredKind::JungleTemple | ScatteredKind::DesertPyramid => {
            let direction = HorizontalDirection::ALL[random.next_int(4)];
            let (width, height, depth) = kind.dimensions().expect("scattered feature dimensions");
            let (width, depth) = match direction {
                HorizontalDirection::North | HorizontalDirection::South => (width, depth),
                _ => (depth, width),
            };
            ScatteredPiece {
                bounds: BoundingBox {
                    min: (x, 64, z),
                    max: (
                        x.wrapping_add(width - 1),
                        64 + height - 1,
                        z.wrapping_add(depth - 1),
                    ),
                },
                orientation: Some(direction),
                generation_depth: 0,
                data: if kind == ScatteredKind::SwampHut {
                    ScatteredPieceData::SwampHut {
                        height_position: -1,
                        spawned_witch: false,
                        spawned_cat: false,
                    }
                } else if kind == ScatteredKind::JungleTemple {
                    ScatteredPieceData::JungleTemple {
                        height_position: -1,
                        placed_main_chest: false,
                        placed_hidden_chest: false,
                        placed_trap1: false,
                        placed_trap2: false,
                    }
                } else {
                    ScatteredPieceData::DesertPyramid {
                        height_position: -1,
                        has_placed_chest: [false; 4],
                        archaeology: Default::default(),
                    }
                },
            }
        }
    };
    Ok(Some(ScatteredStart {
        kind,
        source: [chunk.x, chunk.z],
        generation_point: Some(point),
        piece,
        references: 0,
        cached_reference_bounds: None,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScatteredMob {
    Witch,
    Cat,
}

impl ScatteredMob {
    pub fn name(self) -> &'static str {
        match self {
            Self::Witch => "minecraft:witch",
            Self::Cat => "minecraft:cat",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatteredMobRequest {
    pub mob: ScatteredMob,
    pub block_pos: Pos,
    pub position: [f64; 3],
    pub rotation: [f32; 2],
    pub persistence_required: bool,
    /// Must call the real equivalent of finalizeSpawn at local difficulty.
    pub finalize: bool,
    pub spawn_reason: String,
}

impl ScatteredMobRequest {
    fn new(mob: ScatteredMob, p: Pos) -> Self {
        Self {
            mob,
            block_pos: p,
            position: [f64::from(p.0) + 0.5, f64::from(p.1), f64::from(p.2) + 0.5],
            rotation: [0.0, 0.0],
            persistence_required: true,
            finalize: true,
            spawn_reason: "STRUCTURE".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScatteredEffect {
    /// Modify the already-created block entity without unpacking its loot table.
    LootTable {
        pos: Pos,
        block_entity: String,
        table: String,
        seed: i64,
    },
    MobRequest(ScatteredMobRequest),
}

#[derive(Debug, Deserialize)]
struct NativeContainerDefaults {
    id: String,
    type_id: u32,
    full: NativeContainerTag,
    update: NativeContainerTag,
}

#[derive(Debug, Deserialize)]
struct NativeContainerTag {
    nbt: Nbt,
}

fn container_defaults() -> &'static BTreeMap<String, NativeContainerDefaults> {
    static DEFAULTS: OnceLock<BTreeMap<String, NativeContainerDefaults>> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        let data: Value =
            serde_json::from_str(include_str!("../../data/scattered_containers_26_1.json"))
                .expect("native scattered container capture");
        assert_eq!(data["jar_sha256"], StructureAssets::JAR_SHA256);
        serde_json::from_value(data["containers"]["defaults"].clone())
            .expect("native scattered container defaults")
    })
}

/// Fresh native chest/dispenser data for a ScatteredWorld adapter. Dispenser
/// defaults are absent from the existing template catalog. This keeps native
/// numeric widths, empty item-list type, components, and zero-loot-seed omission.
/// It does not unpack loot or implement inventory/gameplay mutations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatteredContainer {
    pub pos: Pos,
    pub state: u32,
    pub id: String,
    pub type_id: u32,
    pub loot: Option<(String, i64)>,
    defaults: Nbt,
    update: Nbt,
}

impl ScatteredContainer {
    pub fn for_state(state: u32, pos: Pos) -> Result<Option<Self>> {
        let blocks = &StructureAssets::bundled().blocks;
        if blocks.flags(state)? & 8 == 0 {
            return Ok(None);
        }
        let name = &blocks.state(state)?.name;
        let data = container_defaults().get(name).ok_or_else(|| {
            FeatureError::Unsupported(format!(
                "scattered container defaults {name}; use the world's block-entity factory"
            ))
        })?;
        Ok(Some(Self {
            pos,
            state,
            id: data.id.clone(),
            type_id: data.type_id,
            loot: None,
            defaults: data.full.nbt.clone(),
            update: data.update.nbt.clone(),
        }))
    }

    pub fn set_loot_table(&mut self, table: String, seed: i64) {
        self.loot = Some((table, seed));
    }

    pub fn full_data(&self) -> Nbt {
        let mut data = self
            .defaults
            .compound()
            .expect("native container compound")
            .clone();
        for (key, value) in [("x", self.pos.0), ("y", self.pos.1), ("z", self.pos.2)] {
            data.insert(key.into(), Nbt::Int(value));
        }
        if let Some((table, seed)) = &self.loot {
            data.remove("Items");
            data.insert("LootTable".into(), Nbt::String(table.clone()));
            if *seed != 0 {
                data.insert("LootTableSeed".into(), Nbt::Long(*seed));
            } else {
                data.remove("LootTableSeed");
            }
        }
        Nbt::Compound(data)
    }

    pub fn update_data(&self) -> Nbt {
        self.update.clone()
    }
}

/// set_feature_block must synchronously create/remove native block entities and
/// apply WorldGenRegion's state-selected postprocessing mark. Explicit piece marks
/// and tick requests follow it. Missing reads must never be substituted with air.
pub trait ScatteredWorld: FeatureWorld {
    fn structure_min_y(&self) -> i32;
    fn has_structure_block_entity(&self, pos: Pos, id: &str) -> bool;
    /// Must apply or retain the effect in order, or return Unsupported. A mob request
    /// may be queued explicitly; returning Ok without retaining it is not supported.
    fn apply_scattered_effect(&mut self, effect: ScatteredEffect) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScatteredStatus {
    OutsideClip,
    NoGroundSamples,
    NoTreasureSubstrate,
    Processed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScatteredPlacement {
    pub status: ScatteredStatus,
    pub block_attempts: usize,
    pub blocks_written: usize,
    pub loot_assignments: usize,
    /// Counts requests, not instantiated or finalized mobs.
    pub mob_requests: usize,
}

impl ScatteredPlacement {
    fn new(status: ScatteredStatus) -> Self {
        Self {
            status,
            block_attempts: 0,
            blocks_written: 0,
            loot_assignments: 0,
            mob_requests: 0,
        }
    }
}

/// Does not reseed or choose source order. Reuse the same start across calls.
/// Ok means the native operation was evaluated; consult status/counts for actual
/// work. No-substrate and nonintersecting cases deliberately report no placement.
pub fn place_in_chunk<W: ScatteredWorld + ?Sized, R: TemplateRandom + ?Sized>(
    start: &mut ScatteredStart,
    world: &mut W,
    random: &mut R,
    clip: BoundingBox,
) -> Result<ScatteredPlacement> {
    if start.kind == ScatteredKind::DesertPyramid {
        return Err(FeatureError::Unsupported(
            "desert pyramid requires desert_pyramid::place_in_chunk with the source region RNG and world seed".into(),
        ));
    }
    if start.kind != start.piece.kind() || !start.piece.valid() {
        return Err(invalid("scattered piece before placement"));
    }
    let piece = &mut start.piece;
    if !piece.bounds.intersects(clip) {
        return Ok(ScatteredPlacement::new(ScatteredStatus::OutsideClip));
    }
    let mut report = ScatteredPlacement::new(ScatteredStatus::Processed);
    let catalog = ScatteredCatalog::bundled();
    let blocks = &StructureAssets::bundled().blocks;
    let mut placer = Placer {
        world,
        catalog,
        blocks,
        clip,
        report: &mut report,
    };
    match piece.data {
        ScatteredPieceData::BuriedTreasure => placer.treasure(piece, random)?,
        ScatteredPieceData::SwampHut { .. } => placer.swamp_hut(piece)?,
        ScatteredPieceData::JungleTemple { .. } => placer.jungle_temple(piece, random)?,
        ScatteredPieceData::DesertPyramid { .. } => unreachable!(),
    }
    Ok(report)
}

fn read(world: &(impl OreWorld + ?Sized), p: Pos) -> Result<u32> {
    world
        .get_block(p)
        .ok_or_else(|| missing(format!("scattered world block {p:?}")))
}

/// StructurePiece.reorient, including its early return on neighbouring chests
/// and early break at the second solid-render neighbour.
pub fn reorient_chest(world: &(impl OreWorld + ?Sized), p: Pos) -> Result<u32> {
    let catalog = ScatteredCatalog::bundled();
    let blocks = &StructureAssets::bundled().blocks;
    let chest = blocks.default_state("chest")?;
    let mut solid = None;
    for direction in HorizontalDirection::ALL {
        let state = read(world, add(p, direction.step()))?;
        if blocks.state(state)?.name == "minecraft:chest" {
            return Ok(chest);
        }
        if catalog.block_properties(state)?.solid_render {
            if solid.is_some() {
                solid = None;
                break;
            }
            solid = Some(direction);
        }
    }
    if let Some(direction) = solid {
        return blocks.with_property(chest, "facing", direction.opposite().name());
    }
    let mut direction = HorizontalDirection::North;
    let is_solid = |d: HorizontalDirection| -> Result<bool> {
        Ok(catalog
            .block_properties(read(world, add(p, d.step()))?)?
            .solid_render)
    };
    if is_solid(direction)? {
        direction = direction.opposite();
    }
    if is_solid(direction)? {
        direction = direction.clockwise();
    }
    if is_solid(direction)? {
        direction = direction.opposite();
    }
    blocks.with_property(chest, "facing", direction.name())
}

struct Placer<'a, W: ?Sized> {
    world: &'a mut W,
    catalog: &'a ScatteredCatalog,
    blocks: &'a BlockRegistry,
    clip: BoundingBox,
    report: &'a mut ScatteredPlacement,
}

impl<W: ScatteredWorld + ?Sized> Placer<'_, W> {
    fn write(&mut self, p: Pos, state: u32, flags: i32) {
        self.report.block_attempts += 1;
        self.report.blocks_written += usize::from(self.world.set_feature_block(p, state, flags));
    }

    fn piece_block(&mut self, piece: &ScatteredPiece, p: Pos, state: u32) -> Result<()> {
        let p = piece.world_pos(p);
        if !self.clip.contains(p) {
            return Ok(());
        }
        let (mirror, rotation) = piece.orientation.map_or(
            (Mirror::None, Rotation::None),
            HorizontalDirection::transform,
        );
        let state = self.blocks.transform(state, mirror, rotation)?;
        self.write(p, state, 2);
        // Native ignores setBlock's return before querying the resulting fluid.
        let fluid = self
            .catalog
            .block_properties(read(self.world, p)?)?
            .fluid_id;
        if fluid != 0 {
            self.world.schedule_feature_tick(TickRequest {
                block_pos: [p.0, p.1, p.2],
                target: TickTarget::Fluid(fluid),
                delay: 0,
            });
        }
        if self.catalog.block_properties(state)?.shape_check {
            self.world.mark_feature_postprocessing(p);
        }
        Ok(())
    }

    fn box_fill(&mut self, piece: &ScatteredPiece, min: Pos, max: Pos, state: u32) -> Result<()> {
        for y in min.1..=max.1 {
            for x in min.0..=max.0 {
                for z in min.2..=max.2 {
                    self.piece_block(piece, (x, y, z), state)?;
                }
            }
        }
        Ok(())
    }

    fn column_down(&mut self, piece: &ScatteredPiece, local: Pos, state: u32) -> Result<()> {
        let mut p = piece.world_pos(local);
        if !self.clip.contains(p) {
            return Ok(());
        }
        while self
            .catalog
            .block_properties(read(self.world, p)?)?
            .replaceable_by_structures
            && p.1 > self.world.structure_min_y().saturating_add(1)
        {
            // Only the first position is clip-tested; the native column can run
            // below a custom vertical clip until minY+1 or an obstruction.
            self.write(p, state, 2);
            p.1 -= 1;
        }
        Ok(())
    }

    fn update_ground_height(&mut self, piece: &mut ScatteredPiece) -> bool {
        let height_position = match &mut piece.data {
            ScatteredPieceData::SwampHut {
                height_position, ..
            }
            | ScatteredPieceData::JungleTemple {
                height_position, ..
            } => height_position,
            ScatteredPieceData::BuriedTreasure | ScatteredPieceData::DesertPyramid { .. } => {
                unreachable!()
            }
        };
        // A negative native HPos remains the sentinel, even after a void placement.
        if *height_position < 0 {
            let mut total = 0_i32;
            let mut count = 0_i32;
            for z in piece.bounds.min.2..=piece.bounds.max.2 {
                for x in piece.bounds.min.0..=piece.bounds.max.0 {
                    if self.clip.contains((x, 64, z)) {
                        total = total.wrapping_add(self.world.feature_height(
                            FeatureHeightmap::MotionBlockingNoLeaves,
                            x,
                            z,
                        ));
                        count += 1;
                    }
                }
            }
            if count == 0 {
                self.report.status = ScatteredStatus::NoGroundSamples;
                return false;
            }
            *height_position = total / count;
            piece.bounds =
                piece
                    .bounds
                    .moved((0, height_position.wrapping_sub(piece.bounds.min.1), 0));
        }
        true
    }

    fn swamp_hut(&mut self, piece: &mut ScatteredPiece) -> Result<()> {
        if !self.update_ground_height(piece) {
            return Ok(());
        }
        let planks = self.blocks.default_state("spruce_planks")?;
        for (min, max) in [
            ((1, 1, 1), (5, 1, 7)),
            ((1, 4, 2), (5, 4, 7)),
            ((2, 1, 0), (4, 1, 0)),
            ((2, 2, 2), (3, 3, 2)),
            ((1, 2, 3), (1, 3, 6)),
            ((5, 2, 3), (5, 3, 6)),
            ((2, 2, 7), (4, 3, 7)),
        ] {
            self.box_fill(piece, min, max, planks)?;
        }
        let log = self.blocks.default_state("oak_log")?;
        for (x, z) in [(1, 2), (5, 2), (1, 7), (5, 7)] {
            self.box_fill(piece, (x, 0, z), (x, 3, z), log)?;
        }
        for (p, name) in [
            ((2, 3, 2), "oak_fence"),
            ((3, 3, 7), "oak_fence"),
            ((1, 3, 4), "air"),
            ((5, 3, 4), "air"),
            ((5, 3, 5), "air"),
            ((1, 3, 5), "potted_red_mushroom"),
            ((3, 2, 6), "crafting_table"),
            ((4, 2, 6), "cauldron"),
            ((1, 2, 1), "oak_fence"),
            ((5, 2, 1), "oak_fence"),
        ] {
            self.piece_block(piece, p, self.blocks.default_state(name)?)?;
        }
        let stairs = self.blocks.default_state("spruce_stairs")?;
        for (direction, min, max) in [
            ("north", (0, 4, 1), (6, 4, 1)),
            ("east", (0, 4, 2), (0, 4, 7)),
            ("west", (6, 4, 2), (6, 4, 7)),
            ("south", (0, 4, 8), (6, 4, 8)),
        ] {
            self.box_fill(
                piece,
                min,
                max,
                self.blocks.with_property(stairs, "facing", direction)?,
            )?;
        }
        for (p, facing, shape) in [
            ((0, 4, 1), "north", "outer_right"),
            ((6, 4, 1), "north", "outer_left"),
            ((0, 4, 8), "south", "outer_left"),
            ((6, 4, 8), "south", "outer_right"),
        ] {
            let state = self.blocks.with_property(
                self.blocks.with_property(stairs, "facing", facing)?,
                "shape",
                shape,
            )?;
            self.piece_block(piece, p, state)?;
        }
        for z in [2, 7] {
            for x in [1, 5] {
                self.column_down(piece, (x, -1, z), log)?;
            }
        }
        let p = piece.world_pos((2, 2, 5));
        let ScatteredPieceData::SwampHut {
            spawned_witch,
            spawned_cat,
            ..
        } = &mut piece.data
        else {
            unreachable!()
        };
        for (flag, mob) in [
            (spawned_witch, ScatteredMob::Witch),
            (spawned_cat, ScatteredMob::Cat),
        ] {
            if !*flag && self.clip.contains(p) {
                *flag = true;
                self.world
                    .apply_scattered_effect(ScatteredEffect::MobRequest(
                        ScatteredMobRequest::new(mob, p),
                    ))?;
                self.report.mob_requests += 1;
            }
        }
        Ok(())
    }

    fn masonry(
        &mut self,
        piece: &ScatteredPiece,
        random: &mut (impl TemplateRandom + ?Sized),
        min: Pos,
        max: Pos,
    ) -> Result<()> {
        let stone = self.blocks.default_state("cobblestone")?;
        let moss = self.blocks.default_state("mossy_cobblestone")?;
        for y in min.1..=max.1 {
            for x in min.0..=max.0 {
                for z in min.2..=max.2 {
                    // The selector runs before placeBlock's clip test, even for
                    // blocks wholly outside the current source chunk.
                    let state = if random.next_float() < 0.4_f32 {
                        stone
                    } else {
                        moss
                    };
                    self.piece_block(piece, (x, y, z), state)?;
                }
            }
        }
        Ok(())
    }

    fn state_at(&mut self, piece: &ScatteredPiece, p: Pos, spec: &str) -> Result<()> {
        self.piece_block(piece, p, self.blocks.parse_state(spec)?)
    }

    fn assign_loot(
        &mut self,
        p: Pos,
        block_entity: &str,
        table: &str,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<()> {
        if self.world.has_structure_block_entity(p, block_entity) {
            self.world
                .apply_scattered_effect(ScatteredEffect::LootTable {
                    pos: p,
                    block_entity: block_entity.into(),
                    table: table.into(),
                    seed: random.next_long(),
                })?;
            self.report.loot_assignments += 1;
        }
        Ok(())
    }

    fn chest_at(
        &mut self,
        p: Pos,
        table: &str,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<bool> {
        if !self.clip.contains(p)
            || self.blocks.state(read(self.world, p)?)?.name == "minecraft:chest"
        {
            return Ok(false);
        }
        self.write(p, reorient_chest(self.world, p)?, 2);
        self.assign_loot(p, "minecraft:chest", table, random)?;
        // Native success means the placement branch ran, even without a BE.
        Ok(true)
    }

    fn dispenser_at(
        &mut self,
        piece: &ScatteredPiece,
        local: Pos,
        facing: &str,
        table: &str,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<bool> {
        let p = piece.world_pos(local);
        if !self.clip.contains(p)
            || self.blocks.state(read(self.world, p)?)?.name == "minecraft:dispenser"
        {
            return Ok(false);
        }
        let state =
            self.blocks
                .with_property(self.blocks.default_state("dispenser")?, "facing", facing)?;
        self.piece_block(piece, local, state)?;
        self.assign_loot(p, "minecraft:dispenser", table, random)?;
        Ok(true)
    }

    fn jungle_temple(
        &mut self,
        piece: &mut ScatteredPiece,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<()> {
        if !self.update_ground_height(piece) {
            return Ok(());
        }
        for (min, max) in [
            ((0, -4, 0), (11, 0, 14)),
            ((2, 1, 2), (9, 2, 2)),
            ((2, 1, 12), (9, 2, 12)),
            ((2, 1, 3), (2, 2, 11)),
            ((9, 1, 3), (9, 2, 11)),
            ((1, 3, 1), (10, 6, 1)),
            ((1, 3, 13), (10, 6, 13)),
            ((1, 3, 2), (1, 6, 12)),
            ((10, 3, 2), (10, 6, 12)),
            ((2, 3, 2), (9, 3, 12)),
            ((2, 6, 2), (9, 6, 12)),
            ((3, 7, 3), (8, 7, 11)),
            ((4, 8, 4), (7, 8, 10)),
        ] {
            self.masonry(piece, random, min, max)?;
        }
        let air = self.blocks.default_state("air")?;
        for (min, max) in [
            ((3, 1, 3), (8, 2, 11)),
            ((4, 3, 6), (7, 3, 9)),
            ((2, 4, 2), (9, 5, 12)),
            ((4, 6, 5), (7, 6, 9)),
            ((5, 7, 6), (6, 7, 8)),
            ((5, 1, 2), (6, 2, 2)),
            ((5, 2, 12), (6, 2, 12)),
            ((5, 5, 1), (6, 5, 1)),
            ((5, 5, 13), (6, 5, 13)),
        ] {
            self.box_fill(piece, min, max, air)?;
        }
        for p in [(1, 5, 5), (10, 5, 5), (1, 5, 9), (10, 5, 9)] {
            self.piece_block(piece, p, air)?;
        }
        for z in [0, 14] {
            for x in [2, 4, 7, 9] {
                self.masonry(piece, random, (x, 4, z), (x, 5, z))?;
            }
        }
        self.masonry(piece, random, (5, 6, 0), (6, 6, 0))?;
        for x in [0, 11] {
            for z in (2..=12).step_by(2) {
                self.masonry(piece, random, (x, 4, z), (x, 5, z))?;
            }
            for z in [5, 9] {
                self.masonry(piece, random, (x, 6, z), (x, 6, z))?;
            }
        }
        for (min, max) in [
            ((2, 7, 2), (2, 9, 2)),
            ((9, 7, 2), (9, 9, 2)),
            ((2, 7, 12), (2, 9, 12)),
            ((9, 7, 12), (9, 9, 12)),
            ((4, 9, 4), (4, 9, 4)),
            ((7, 9, 4), (7, 9, 4)),
            ((4, 9, 10), (4, 9, 10)),
            ((7, 9, 10), (7, 9, 10)),
            ((5, 9, 7), (6, 9, 7)),
        ] {
            self.masonry(piece, random, min, max)?;
        }
        let stairs = self.blocks.default_state("cobblestone_stairs")?;
        let north = self.blocks.with_property(stairs, "facing", "north")?;
        let south = self.blocks.with_property(stairs, "facing", "south")?;
        for (p, state) in [
            ((5, 9, 6), north),
            ((6, 9, 6), north),
            ((5, 9, 8), south),
            ((6, 9, 8), south),
        ] {
            self.piece_block(piece, p, state)?;
        }
        for x in 4..=7 {
            self.piece_block(piece, (x, 0, 0), north)?;
        }
        for x in [4, 7] {
            for (y, z) in [(1, 8), (2, 9), (3, 10)] {
                self.piece_block(piece, (x, y, z), north)?;
            }
        }
        for (min, max) in [
            ((4, 1, 9), (4, 1, 9)),
            ((7, 1, 9), (7, 1, 9)),
            ((4, 1, 10), (7, 2, 10)),
            ((5, 4, 5), (6, 4, 5)),
        ] {
            self.masonry(piece, random, min, max)?;
        }
        self.state_at(piece, (4, 4, 5), "cobblestone_stairs[facing=east]")?;
        self.state_at(piece, (7, 4, 5), "cobblestone_stairs[facing=west]")?;
        for i in 0..4 {
            self.piece_block(piece, (5, -i, 6 + i), south)?;
            self.piece_block(piece, (6, -i, 6 + i), south)?;
            self.box_fill(piece, (5, -i, 7 + i), (6, -i, 9 + i), air)?;
        }
        for (min, max) in [
            ((1, -3, 12), (10, -1, 13)),
            ((1, -3, 1), (3, -1, 13)),
            ((1, -3, 1), (9, -1, 5)),
        ] {
            self.box_fill(piece, min, max, air)?;
        }
        for z in (1..=13).step_by(2) {
            self.masonry(piece, random, (1, -3, z), (1, -2, z))?;
        }
        for z in (2..=12).step_by(2) {
            self.masonry(piece, random, (1, -1, z), (3, -1, z))?;
        }
        for (min, max) in [
            ((2, -2, 1), (5, -2, 1)),
            ((7, -2, 1), (9, -2, 1)),
            ((6, -3, 1), (6, -3, 1)),
            ((6, -1, 1), (6, -1, 1)),
        ] {
            self.masonry(piece, random, min, max)?;
        }

        self.state_at(
            piece,
            (1, -3, 8),
            "tripwire_hook[facing=east,attached=true]",
        )?;
        self.state_at(
            piece,
            (4, -3, 8),
            "tripwire_hook[facing=west,attached=true]",
        )?;
        for x in [2, 3] {
            self.state_at(
                piece,
                (x, -3, 8),
                "tripwire[east=true,west=true,attached=true]",
            )?;
        }
        let wire_ns = self
            .blocks
            .parse_state("redstone_wire[north=side,south=side]")?;
        for z in (2..=7).rev() {
            self.piece_block(piece, (5, -3, z), wire_ns)?;
        }
        self.state_at(piece, (5, -3, 1), "redstone_wire[north=side,west=side]")?;
        self.state_at(piece, (4, -3, 1), "redstone_wire[east=side,west=side]")?;
        let moss = self.blocks.default_state("mossy_cobblestone")?;
        self.piece_block(piece, (3, -3, 1), moss)?;
        if matches!(
            piece.data,
            ScatteredPieceData::JungleTemple {
                placed_trap1: false,
                ..
            }
        ) {
            let placed = self.dispenser_at(
                piece,
                (3, -2, 1),
                "north",
                "minecraft:chests/jungle_temple_dispenser",
                random,
            )?;
            if let ScatteredPieceData::JungleTemple { placed_trap1, .. } = &mut piece.data {
                *placed_trap1 = placed;
            }
        }
        self.state_at(piece, (3, -2, 2), "vine[south=true]")?;

        self.state_at(
            piece,
            (7, -3, 1),
            "tripwire_hook[facing=north,attached=true]",
        )?;
        self.state_at(
            piece,
            (7, -3, 5),
            "tripwire_hook[facing=south,attached=true]",
        )?;
        for z in 2..=4 {
            self.state_at(
                piece,
                (7, -3, z),
                "tripwire[north=true,south=true,attached=true]",
            )?;
        }
        self.state_at(piece, (8, -3, 6), "redstone_wire[east=side,west=side]")?;
        self.state_at(piece, (9, -3, 6), "redstone_wire[west=side,south=side]")?;
        self.state_at(piece, (9, -3, 5), "redstone_wire[north=side,south=up]")?;
        self.piece_block(piece, (9, -3, 4), moss)?;
        self.piece_block(piece, (9, -2, 4), wire_ns)?;
        if matches!(
            piece.data,
            ScatteredPieceData::JungleTemple {
                placed_trap2: false,
                ..
            }
        ) {
            let placed = self.dispenser_at(
                piece,
                (9, -2, 3),
                "west",
                "minecraft:chests/jungle_temple_dispenser",
                random,
            )?;
            if let ScatteredPieceData::JungleTemple { placed_trap2, .. } = &mut piece.data {
                *placed_trap2 = placed;
            }
        }
        for y in [-1, -2] {
            self.state_at(piece, (8, y, 3), "vine[east=true]")?;
        }
        if matches!(
            piece.data,
            ScatteredPieceData::JungleTemple {
                placed_main_chest: false,
                ..
            }
        ) {
            let placed = self.chest_at(
                piece.world_pos((8, -3, 3)),
                "minecraft:chests/jungle_temple",
                random,
            )?;
            if let ScatteredPieceData::JungleTemple {
                placed_main_chest, ..
            } = &mut piece.data
            {
                *placed_main_chest = placed;
            }
        }
        for p in [
            (9, -3, 2),
            (8, -3, 1),
            (4, -3, 5),
            (5, -2, 5),
            (5, -1, 5),
            (6, -3, 5),
            (7, -2, 5),
            (7, -1, 5),
            (8, -3, 5),
        ] {
            self.piece_block(piece, p, moss)?;
        }
        self.masonry(piece, random, (9, -1, 1), (9, -1, 5))?;

        self.box_fill(piece, (8, -3, 8), (10, -1, 10), air)?;
        for x in 8..=10 {
            self.state_at(piece, (x, -2, 11), "chiseled_stone_bricks")?;
        }
        for x in 8..=10 {
            self.state_at(piece, (x, -2, 12), "lever[facing=north,face=wall]")?;
        }
        self.masonry(piece, random, (8, -3, 8), (8, -3, 10))?;
        self.masonry(piece, random, (10, -3, 8), (10, -3, 10))?;
        self.piece_block(piece, (10, -2, 9), moss)?;
        self.piece_block(piece, (8, -2, 9), wire_ns)?;
        self.piece_block(piece, (8, -2, 10), wire_ns)?;
        self.state_at(
            piece,
            (10, -1, 9),
            "redstone_wire[north=side,south=side,east=side,west=side]",
        )?;
        self.state_at(piece, (9, -2, 8), "sticky_piston[facing=up]")?;
        self.state_at(piece, (10, -2, 8), "sticky_piston[facing=west]")?;
        self.state_at(piece, (10, -1, 8), "sticky_piston[facing=west]")?;
        self.state_at(piece, (10, -2, 10), "repeater[facing=north]")?;
        if matches!(
            piece.data,
            ScatteredPieceData::JungleTemple {
                placed_hidden_chest: false,
                ..
            }
        ) {
            let placed = self.chest_at(
                piece.world_pos((9, -3, 10)),
                "minecraft:chests/jungle_temple",
                random,
            )?;
            if let ScatteredPieceData::JungleTemple {
                placed_hidden_chest,
                ..
            } = &mut piece.data
            {
                *placed_hidden_chest = placed;
            }
        }
        Ok(())
    }

    fn treasure(
        &mut self,
        piece: &mut ScatteredPiece,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<()> {
        let (x, _, z) = piece.bounds.min;
        let mut y = self
            .world
            .feature_height(FeatureHeightmap::OceanFloorWg, x, z);
        let blocks = self.blocks;
        let catalog = self.catalog;
        let liquid_or_air = |state: u32| -> Result<bool> {
            Ok(catalog.block_properties(state)?.air
                || matches!(
                    blocks.state(state)?.name.as_str(),
                    "minecraft:water" | "minecraft:lava"
                ))
        };
        while y > self.world.structure_min_y() {
            let p = (x, y, z);
            let current = read(self.world, p)?;
            let below = read(self.world, (x, y - 1, z))?;
            if matches!(
                self.blocks.state(below)?.name.as_str(),
                "minecraft:sandstone"
                    | "minecraft:stone"
                    | "minecraft:andesite"
                    | "minecraft:granite"
                    | "minecraft:diorite"
            ) {
                let cover = if liquid_or_air(current)? {
                    self.blocks.default_state("sand")?
                } else {
                    current
                };
                for direction in [
                    (0, -1, 0),
                    (0, 1, 0),
                    (0, 0, -1),
                    (0, 0, 1),
                    (-1, 0, 0),
                    (1, 0, 0),
                ] {
                    let at = add(p, direction);
                    if liquid_or_air(read(self.world, at)?)? {
                        let down = read(self.world, (at.0, at.1 - 1, at.2))?;
                        let state = if liquid_or_air(down)? && direction != (0, 1, 0) {
                            below
                        } else {
                            cover
                        };
                        // Native uses direct WorldGenLevel writes, not placeBlock:
                        // neighbour repair can cross the structure clip by one block.
                        self.write(at, state, 3);
                    }
                }
                piece.bounds = BoundingBox::new(p, p);
                self.chest_at(p, "minecraft:chests/buried_treasure", random)?;
                return Ok(());
            }
            y -= 1;
        }
        self.report.status = ScatteredStatus::NoTreasureSubstrate;
        Ok(())
    }
}
