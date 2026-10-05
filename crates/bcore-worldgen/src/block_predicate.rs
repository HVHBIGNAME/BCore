//! Block predicates and plant survival for the pinned Java 26.1 registry.
//!
//! State IDs, property ordering, tags, material flags and vegetation support
//! tables are captured by `BaseFeatureReference`, not inferred from ID ranges.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use crate::feature_world::{FeatureError, FeatureWorld, Pos};
use crate::{MAX_Y, MIN_Y};

pub type FeatureResult<T> = Result<T, FeatureError>;

/// Extra observations which the shared feature world does not expose.
/// Implementations must supply native light when a survival test requests it.
pub trait FeatureEnvironment {
    fn raw_brightness(&self, _world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<i32> {
        Err(FeatureError::MissingData(format!(
            "raw brightness at {pos:?}"
        )))
    }

    fn biome_has_feature(&self, biome: u32, feature: &str) -> FeatureResult<bool> {
        catalog().biome_has_feature(biome, feature)
    }

    fn generation_bounds(&self) -> (i32, i32) {
        (MIN_Y, MAX_Y + 1)
    }

    /// Pinned overworld defaults; custom dimensions/worlds override these.
    fn sea_level(&self) -> i32 {
        crate::SEA_LEVEL
    }

    /// IcebergFeature asks the generator rather than WorldGenLevel.
    fn generator_sea_level(&self) -> i32 {
        crate::SEA_LEVEL
    }

    /// Lake freezing is a biome/light query, independent of feature randomness.
    fn should_freeze(&self, world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
        self.should_freeze_with_edge(world, pos, false)
    }

    fn should_freeze_with_edge(
        &self,
        _world: &dyn FeatureWorld,
        pos: Pos,
        _must_be_at_edge: bool,
    ) -> FeatureResult<bool> {
        Err(FeatureError::MissingData(format!(
            "biome freezing test at {pos:?}"
        )))
    }

    /// SnowAndFreezeFeature samples the biome above the position being frozen.
    /// Forward only when that biome is also the local biome; a vertical biome
    /// boundary requires an implementation which honors the supplied identity.
    fn should_freeze_in_biome(
        &self,
        world: &dyn FeatureWorld,
        biome: u32,
        pos: Pos,
        must_be_at_edge: bool,
    ) -> FeatureResult<bool> {
        if world.feature_biome(pos) == biome {
            self.should_freeze_with_edge(world, pos, must_be_at_edge)
        } else {
            Err(FeatureError::MissingData(format!(
                "freezing test for biome {biome} at {pos:?}"
            )))
        }
    }

    fn should_snow(&self, _world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
        Err(FeatureError::MissingData(format!(
            "biome snow test at {pos:?}"
        )))
    }
}

/// Registry-backed biome checks; light-dependent operations report missing data.
#[derive(Default)]
pub struct RegistryEnvironment;
impl FeatureEnvironment for RegistryEnvironment {}

#[derive(Debug, Deserialize)]
pub struct BlockDefinition {
    pub first: u32,
    pub count: u32,
    #[serde(rename = "default")]
    pub default_state: u32,
    pub properties: Vec<(String, Vec<String>)>,
    pub survival: String,
    pub class: String,
    pub double_plant: bool,
    #[serde(default)]
    pub support: Vec<[u32; 2]>,
}

impl BlockDefinition {
    pub fn supports(&self, below: u32) -> bool {
        let index = self.support.partition_point(|range| range[1] <= below);
        self.support
            .get(index)
            .is_some_and(|range| range[0] <= below)
    }

    pub fn property<'a>(&'a self, state: u32, name: &str) -> Option<&'a str> {
        let mut index = state.checked_sub(self.first)?;
        if index >= self.count {
            return None;
        }
        for (key, values) in self.properties.iter().rev() {
            let value = &values[index as usize % values.len()];
            if key == name {
                return Some(value);
            }
            index /= values.len() as u32;
        }
        None
    }

    pub fn with_property(&self, state: u32, name: &str, value: &str) -> FeatureResult<u32> {
        let mut stride = 1;
        for (key, values) in self.properties.iter().rev() {
            if key == name {
                let new =
                    values.iter().position(|v| v == value).ok_or_else(|| {
                        invalid(format!("invalid {name}={value} for state {state}"))
                    })? as u32;
                let old = (state - self.first) / stride % values.len() as u32;
                return Ok(state - old * stride + new * stride);
            }
            stride *= values.len() as u32;
        }
        Err(invalid(format!("state {state} has no property {name}")))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StateInfo {
    flags: u8,
    pub fluid: u32,
    pub fluid_amount: u8,
    sturdy_faces: u8,
    attachment_faces: u8,
    center_faces: u8,
    pub flammable: bool,
}

impl StateInfo {
    pub fn is_air(self) -> bool {
        self.flags & 16 != 0
    }
    pub fn is_solid(self) -> bool {
        self.flags & 8 != 0
    }
    pub fn replaceable(self) -> bool {
        self.flags & 4 != 0
    }
    pub fn liquid(self) -> bool {
        self.flags & 2 != 0
    }
    pub fn blocks_motion(self) -> bool {
        self.flags & 1 != 0
    }
    pub fn sturdy(self, direction: Direction) -> bool {
        self.sturdy_faces & (1 << direction as u8) != 0
    }
    pub fn can_attach_from(self, direction_to_neighbor: Direction) -> bool {
        self.attachment_faces & (1 << direction_to_neighbor as u8) != 0
    }
    pub fn supports_center(self, face: Direction) -> bool {
        self.center_faces & (1 << face as u8) != 0
    }
}

/// Native collision/occlusion observations which are distinct from sturdy faces.
/// Captured independently by the misc-feature probe; the shared catalog is intact.
#[derive(Clone, Copy, Debug)]
pub struct ShapeInfo {
    occlusion_faces: u8,
    pub collision_top_full: bool,
    pub collision_top_nonempty: bool,
}

impl ShapeInfo {
    pub fn occludes(self, face: Direction) -> bool {
        self.occlusion_faces & (1 << face as u8) != 0
    }
}

struct MiscMetadata {
    shapes: Vec<ShapeInfo>,
    ordered_tags: BTreeMap<String, Vec<u32>>,
}

fn misc_metadata() -> &'static MiscMetadata {
    static DATA: OnceLock<MiscMetadata> = OnceLock::new();
    DATA.get_or_init(|| {
        let value: Value =
            serde_json::from_str(include_str!("../data/misc_feature_states_26_1.json"))
                .expect("native misc shape metadata");
        let count = value["state_count"].as_u64().expect("native state count") as usize;
        let mut shapes = Vec::with_capacity(count);
        for row in value["shape_ranges"]
            .as_array()
            .expect("native shape ranges")
        {
            let [start, end, faces, full, nonempty]: [usize; 5] =
                serde_json::from_value(row.clone()).expect("native shape row");
            assert!(start == shapes.len() && end > start && end <= count);
            shapes.resize(
                end,
                ShapeInfo {
                    occlusion_faces: faces as u8,
                    collision_top_full: full != 0,
                    collision_top_nonempty: nonempty != 0,
                },
            );
        }
        assert_eq!(shapes.len(), count);
        MiscMetadata {
            shapes,
            ordered_tags: serde_json::from_value(value["ordered_tags"].clone())
                .expect("native ordered coral tags"),
        }
    })
}

pub fn shape_info(state: u32) -> FeatureResult<ShapeInfo> {
    misc_metadata()
        .shapes
        .get(state as usize)
        .copied()
        .ok_or_else(|| missing(format!("native shape for state {state}")))
}

/// Native HolderSet order, used when selection consumes a random tag index.
/// Only independently captured tags are available here; sets are never sorted.
pub fn ordered_block_tag(tag: &str) -> FeatureResult<&'static [u32]> {
    misc_metadata()
        .ordered_tags
        .get(tag.strip_prefix("minecraft:").unwrap_or(tag))
        .map(Vec::as_slice)
        .ok_or_else(|| missing(format!("native ordered block tag {tag}")))
}

/// The exact resources read out of `server-26.1.jar` by the native probe.
pub struct FeatureCatalog {
    pub documents: Value,
    pub blocks: BTreeMap<String, BlockDefinition>,
    pub block_tags: BTreeMap<String, BTreeSet<u32>>,
    pub fluid_tags: BTreeMap<String, BTreeSet<u32>>,
    pub fluids: BTreeMap<String, u32>,
    state_info: Vec<StateInfo>,
    block_starts: Vec<(u32, String)>,
    block_ids: BTreeMap<String, u32>,
    biome_features: BTreeMap<u32, BTreeSet<String>>,
}

pub fn catalog() -> &'static FeatureCatalog {
    static CATALOG: OnceLock<FeatureCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        FeatureCatalog::from_value(
            serde_json::from_str(include_str!("../data/base_feature_catalog_26_1.json"))
                .expect("valid captured 26.1 feature catalog"),
        )
        .expect("consistent captured 26.1 registries")
    })
}

impl FeatureCatalog {
    pub fn from_value(documents: Value) -> FeatureResult<Self> {
        let blocks: BTreeMap<String, BlockDefinition> =
            serde_json::from_value(documents["blocks"].clone())
                .map_err(|e| invalid(e.to_string()))?;
        let fluids: BTreeMap<String, u32> = serde_json::from_value(documents["fluids"].clone())
            .map_err(|e| invalid(e.to_string()))?;
        let mut block_starts: Vec<_> = blocks
            .iter()
            .map(|(name, block)| (block.first, name.clone()))
            .collect();
        block_starts.sort_unstable();
        let count = integer(&documents, "state_count")? as usize;
        let mut state_info = Vec::with_capacity(count);
        for row in array(&documents["state_ranges"])? {
            let values: Vec<u32> = array(row)?
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| invalid("state range"))
                })
                .collect::<FeatureResult<_>>()?;
            if values.len() != 6
                || values[0] as usize != state_info.len()
                || values[1] as usize > count
                || values[1] <= values[0]
            {
                return Err(invalid("non-contiguous state metadata"));
            }
            state_info.resize(
                values[1] as usize,
                StateInfo {
                    flags: values[2] as u8,
                    fluid: values[3],
                    fluid_amount: values[4] as u8,
                    sturdy_faces: values[5] as u8,
                    ..StateInfo::default()
                },
            );
        }
        if state_info.len() != count {
            return Err(invalid("incomplete state metadata"));
        }
        let mut end = 0;
        for row in array(&documents["shape_ranges"])? {
            let row: [usize; 5] =
                serde_json::from_value(row.clone()).map_err(|e| invalid(e.to_string()))?;
            if row[0] != end || row[1] <= end || row[1] > count {
                return Err(invalid("non-contiguous shape metadata"));
            }
            for state in &mut state_info[row[0]..row[1]] {
                state.attachment_faces = row[2] as u8;
                state.center_faces = row[3] as u8;
                state.flammable = row[4] != 0;
            }
            end = row[1];
        }
        if end != count {
            return Err(invalid("incomplete shape metadata"));
        }
        let block_ids = blocks
            .iter()
            .map(|(name, block)| (name.clone(), block.first))
            .collect();
        let block_tags = resolve_tags(&documents["block_tags"], &block_ids)?;
        let fluid_tags = resolve_tags(&documents["fluid_tags"], &fluids)?;
        let mut biome_features = BTreeMap::new();
        for (name, id) in documents["biome_ids"]
            .as_object()
            .ok_or_else(|| invalid("biome ids"))?
        {
            let features = array(&documents["biome"][name]["features"])?;
            let mut names = BTreeSet::new();
            for step in features {
                for name in array(step)? {
                    names.insert(string(name)?.to_owned());
                }
            }
            let id = id.as_u64().ok_or_else(|| invalid("biome id"))? as u32;
            biome_features.insert(id, names);
        }
        Ok(Self {
            documents,
            blocks,
            fluids,
            block_tags,
            fluid_tags,
            state_info,
            block_starts,
            block_ids,
            biome_features,
        })
    }

    pub fn definition(&self, name: &str) -> FeatureResult<&BlockDefinition> {
        self.blocks
            .get(qualified(name).as_ref())
            .ok_or_else(|| missing(format!("block {name}")))
    }

    pub fn block(&self, state: u32) -> FeatureResult<(&str, &BlockDefinition)> {
        let index = self
            .block_starts
            .partition_point(|(first, _)| *first <= state);
        if index == 0 {
            return Err(missing(format!("block state {state}")));
        }
        let name = &self.block_starts[index - 1].1;
        let block = &self.blocks[name];
        if state >= block.first + block.count {
            return Err(missing(format!("block state {state}")));
        }
        Ok((name, block))
    }

    pub fn info(&self, state: u32) -> FeatureResult<StateInfo> {
        self.state_info
            .get(state as usize)
            .copied()
            .ok_or_else(|| missing(format!("block state {state}")))
    }

    pub fn default_state(&self, name: &str) -> FeatureResult<u32> {
        Ok(self.definition(name)?.default_state)
    }

    pub fn fluid_state(&self, value: &Value) -> FeatureResult<(u32, u32)> {
        let name = qualified(string(&value["Name"])?);
        let mut state = self.documents["fluid_defaults"]
            .get(name.as_ref())
            .cloned()
            .ok_or_else(|| missing(format!("fluid {name}")))?;
        if let Some(properties) = value.get("Properties") {
            let defaults = state["Properties"]
                .as_object_mut()
                .ok_or_else(|| invalid("fluid has no properties"))?;
            for (key, value) in properties
                .as_object()
                .ok_or_else(|| invalid("fluid Properties"))?
            {
                if !defaults.contains_key(key) {
                    return Err(invalid(format!("fluid property {key}")));
                }
                defaults.insert(key.clone(), value.clone());
            }
        }
        for entry in array(&self.documents["fluid_states"])? {
            if entry["state"] == state {
                return Ok((
                    integer(entry, "block")? as u32,
                    integer(entry, "fluid")? as u32,
                ));
            }
        }
        Err(invalid(format!("fluid state {value}")))
    }

    pub fn state(&self, value: &Value) -> FeatureResult<u32> {
        let name = string(&value["Name"])?;
        let block = self.definition(name)?;
        let mut state = block.default_state;
        if let Some(properties) = value.get("Properties") {
            for (key, value) in properties
                .as_object()
                .ok_or_else(|| invalid("state Properties"))?
            {
                state = block.with_property(state, key, string(value)?)?;
            }
        }
        Ok(state)
    }

    pub fn is_block(&self, state: u32, name: &str) -> FeatureResult<bool> {
        let block = self.definition(name)?;
        Ok((block.first..block.first + block.count).contains(&state))
    }

    pub fn in_block_tag(&self, state: u32, tag: &str) -> FeatureResult<bool> {
        let members = self
            .block_tags
            .get(qualified(tag).as_ref())
            .ok_or_else(|| missing(format!("block tag {tag}")))?;
        Ok(members.contains(&self.block(state)?.1.first))
    }

    pub fn in_fluid_tag(&self, state: u32, tag: &str) -> FeatureResult<bool> {
        let members = self
            .fluid_tags
            .get(qualified(tag).as_ref())
            .ok_or_else(|| missing(format!("fluid tag {tag}")))?;
        Ok(members.contains(&self.info(state)?.fluid))
    }

    pub fn configured(&self, name: &str) -> FeatureResult<&Value> {
        self.resource("configured_feature", name)
    }
    pub fn placed(&self, name: &str) -> FeatureResult<&Value> {
        self.resource("placed_feature", name)
    }

    fn resource(&self, directory: &str, name: &str) -> FeatureResult<&Value> {
        self.documents[directory]
            .get(qualified(name).as_ref())
            .ok_or_else(|| missing(format!("{directory}/{name}")))
    }

    pub fn biome_has_feature(&self, biome: u32, feature: &str) -> FeatureResult<bool> {
        let names = self
            .biome_features
            .get(&biome)
            .ok_or_else(|| missing(format!("biome {biome}")))?;
        Ok(names.contains(qualified(feature).as_ref()))
    }

    pub fn block_holder_set(&self, value: &Value) -> FeatureResult<BTreeSet<u32>> {
        holder_set(value, &self.block_ids, &self.block_tags)
    }

    pub fn fluid_holder_set(&self, value: &Value) -> FeatureResult<BTreeSet<u32>> {
        holder_set(value, &self.fluids, &self.fluid_tags)
    }
}

fn resolve_tags(
    values: &Value,
    registry: &BTreeMap<String, u32>,
) -> FeatureResult<BTreeMap<String, BTreeSet<u32>>> {
    fn visit(
        name: &str,
        values: &Value,
        registry: &BTreeMap<String, u32>,
        stack: &mut BTreeSet<String>,
    ) -> FeatureResult<BTreeSet<u32>> {
        if !stack.insert(name.to_owned()) {
            return Err(invalid(format!("cyclic tag {name}")));
        }
        let mut result = BTreeSet::new();
        let entries = values
            .get(name)
            .ok_or_else(|| missing(format!("tag {name}")))?;
        for entry in array(&entries["values"])? {
            let id = if entry.is_string() {
                string(entry)?
            } else {
                string(&entry["id"])?
            };
            let required = entry
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if let Some(tag) = id.strip_prefix('#') {
                if required || values.get(qualified(tag).as_ref()).is_some() {
                    result.extend(visit(qualified(tag).as_ref(), values, registry, stack)?);
                }
            } else if let Some(id) = registry.get(qualified(id).as_ref()) {
                result.insert(*id);
            } else if required {
                return Err(missing(format!("tag entry {id}")));
            }
        }
        stack.remove(name);
        Ok(result)
    }
    values
        .as_object()
        .ok_or_else(|| invalid("tags"))?
        .keys()
        .map(|name| {
            Ok((
                name.clone(),
                visit(name, values, registry, &mut BTreeSet::new())?,
            ))
        })
        .collect()
}

fn holder_set(
    value: &Value,
    registry: &BTreeMap<String, u32>,
    tags: &BTreeMap<String, BTreeSet<u32>>,
) -> FeatureResult<BTreeSet<u32>> {
    if let Some(name) = value.as_str() {
        if let Some(tag) = name.strip_prefix('#') {
            return tags
                .get(qualified(tag).as_ref())
                .cloned()
                .ok_or_else(|| missing(format!("tag {tag}")));
        }
        return Ok(BTreeSet::from([*registry
            .get(qualified(name).as_ref())
            .ok_or_else(|| missing(name))?]));
    }
    let mut result = BTreeSet::new();
    for entry in array(value)? {
        result.extend(holder_set(entry, registry, tags)?);
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Direction {
    pub const ALL: [Self; 6] = [
        Self::Down,
        Self::Up,
        Self::North,
        Self::South,
        Self::West,
        Self::East,
    ];
    pub const HORIZONTAL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

    pub fn opposite(self) -> Self {
        match self {
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::East => Self::West,
        }
    }

    pub fn parse(value: &Value) -> FeatureResult<Self> {
        match string(value)? {
            "down" => Ok(Self::Down),
            "up" => Ok(Self::Up),
            "north" => Ok(Self::North),
            "south" => Ok(Self::South),
            "west" => Ok(Self::West),
            "east" => Ok(Self::East),
            other => Err(invalid(format!("direction {other}"))),
        }
    }

    pub fn step(self, pos: Pos) -> Pos {
        offset(
            pos,
            match self {
                Self::Down => (0, -1, 0),
                Self::Up => (0, 1, 0),
                Self::North => (0, 0, -1),
                Self::South => (0, 0, 1),
                Self::West => (-1, 0, 0),
                Self::East => (1, 0, 0),
            },
        )
    }
}

#[derive(Debug, Clone)]
pub enum BlockPredicate {
    True,
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
    Blocks(BTreeSet<u32>, Pos),
    Fluids(BTreeSet<u32>, Pos),
    Solid(Pos),
    Replaceable(Pos),
    Sturdy(Direction, Pos),
    Bounds(Pos),
    Survives(u32, Pos),
}

impl BlockPredicate {
    pub fn parse(value: &Value) -> FeatureResult<Self> {
        let displacement = value
            .get("offset")
            .map(position)
            .transpose()?
            .unwrap_or((0, 0, 0));
        Ok(match kind(value)? {
            "true" => Self::True,
            "all_of" => Self::All(
                array(&value["predicates"])?
                    .iter()
                    .map(Self::parse)
                    .collect::<FeatureResult<_>>()?,
            ),
            "any_of" => Self::Any(
                array(&value["predicates"])?
                    .iter()
                    .map(Self::parse)
                    .collect::<FeatureResult<_>>()?,
            ),
            "not" => Self::Not(Box::new(Self::parse(&value["predicate"])?)),
            "matching_blocks" => {
                Self::Blocks(catalog().block_holder_set(&value["blocks"])?, displacement)
            }
            "matching_block_tag" => Self::Blocks(
                catalog()
                    .block_holder_set(&Value::String(format!("#{}", string(&value["tag"])?)))?,
                displacement,
            ),
            "matching_fluids" => {
                Self::Fluids(catalog().fluid_holder_set(&value["fluids"])?, displacement)
            }
            "solid" => Self::Solid(displacement),
            "replaceable" => Self::Replaceable(displacement),
            "has_sturdy_face" => Self::Sturdy(Direction::parse(&value["direction"])?, displacement),
            "inside_world_bounds" => Self::Bounds(displacement),
            "would_survive" => Self::Survives(catalog().state(&value["state"])?, displacement),
            other => {
                return Err(FeatureError::Unsupported(format!(
                    "block predicate {other}"
                )))
            }
        })
    }

    pub fn test(
        &self,
        world: &dyn FeatureWorld,
        pos: Pos,
        environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        match self {
            Self::True => Ok(true),
            Self::All(predicates) => {
                for predicate in predicates {
                    if !predicate.test(world, pos, environment)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Any(predicates) => {
                for predicate in predicates {
                    if predicate.test(world, pos, environment)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(predicate) => Ok(!predicate.test(world, pos, environment)?),
            Self::Blocks(blocks, delta) => Ok(blocks.contains(
                &catalog()
                    .block(read_block(world, offset(pos, *delta))?)?
                    .1
                    .first,
            )),
            Self::Fluids(fluids, delta) => Ok(fluids.contains(
                &catalog()
                    .info(read_block(world, offset(pos, *delta))?)?
                    .fluid,
            )),
            Self::Solid(delta) => Ok(catalog()
                .info(read_block(world, offset(pos, *delta))?)?
                .is_solid()),
            Self::Replaceable(delta) => Ok(catalog()
                .info(read_block(world, offset(pos, *delta))?)?
                .replaceable()),
            Self::Sturdy(direction, delta) => Ok(catalog()
                .info(read_block(world, offset(pos, *delta))?)?
                .sturdy(*direction)),
            Self::Bounds(delta) => Ok((MIN_Y..=MAX_Y).contains(&offset(pos, *delta).1)),
            Self::Survives(state, delta) => {
                would_survive(world, *state, offset(pos, *delta), environment)
            }
        }
    }
}

pub fn test(
    value: &Value,
    world: &dyn FeatureWorld,
    pos: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    BlockPredicate::parse(value)?.test(world, pos, environment)
}

/// `BlockState.canSurvive`, including upper halves and local fluid/light rules.
/// Unsupported specialised block behaviours are reported rather than accepted.
pub fn survival_requires_brightness(state: u32) -> FeatureResult<bool> {
    let (name, block) = catalog().block(state)?;
    match block.survival.as_str() {
        "MushroomBlock" => Ok(true),
        "BlockBehaviour" | "CactusBlock" | "VegetationBlock" | "DoublePlantBlock"
        | "TallSeagrassBlock" | "SmallDripleafBlock" | "BambooStalkBlock"
        | "BambooSaplingBlock" | "SeaPickleBlock" | "SnowLayerBlock" | "VineBlock"
        | "SugarCaneBlock" | "CarpetBlock" | "LeafLitterBlock" | "MossyCarpetBlock"
        | "SporeBlossomBlock" | "SoulFireBlock" | "FireBlock" | "HangingRootsBlock" => Ok(false),
        "GrowingPlantBlock" if matches!(name, "minecraft:kelp" | "minecraft:kelp_plant") => {
            Ok(false)
        }
        other => Err(FeatureError::Unsupported(format!(
            "canSurvive {name} ({other})"
        ))),
    }
}

pub fn would_survive(
    world: &dyn FeatureWorld,
    state: u32,
    pos: Pos,
    environment: &dyn FeatureEnvironment,
) -> FeatureResult<bool> {
    let data = catalog();
    let (name, block) = data.block(state)?;
    if block.survival == "BlockBehaviour" {
        return Ok(true);
    }
    if block.survival == "CactusBlock" {
        return cactus_survives(world, pos);
    }
    if block.survival == "VineBlock" {
        return vine_survives(world, state, pos);
    }
    let below_pos = Direction::Down.step(pos);
    let below = read_block(world, below_pos)?;
    match block.survival.as_str() {
        "VegetationBlock" => {
            if !block.supports(below) {
                return Ok(false);
            }
            if block.class == "LilyPadBlock" {
                return Ok(
                    data.info(read_block(world, pos)?)?.fluid == data.fluids["minecraft:empty"]
                );
            }
            Ok(true)
        }
        "DoublePlantBlock" | "TallSeagrassBlock" | "SmallDripleafBlock" => {
            if block.property(state, "half") == Some("upper") {
                return Ok(
                    data.is_block(below, name)? && block.property(below, "half") == Some("lower")
                );
            }
            if block.survival == "SmallDripleafBlock" {
                if data.in_block_tag(below, "supports_small_dripleaf")? {
                    return Ok(true);
                }
                let fluid = data.info(read_block(world, pos)?)?;
                return Ok(fluid.fluid == data.fluids["minecraft:water"]
                    && data.in_block_tag(below, "supports_vegetation")?);
            }
            if !block.supports(below) {
                return Ok(false);
            }
            if block.survival == "TallSeagrassBlock" {
                let current = read_block(world, pos)?;
                return Ok(
                    data.in_fluid_tag(current, "water")? && data.info(current)?.fluid_amount == 8
                );
            }
            Ok(true)
        }
        "MushroomBlock" => {
            if data.in_block_tag(below, "overrides_mushroom_light_requirement")? {
                return Ok(true);
            }
            Ok(environment.raw_brightness(world, pos)? < 13 && block.supports(below))
        }
        "BambooStalkBlock" | "BambooSaplingBlock" => data.in_block_tag(below, "supports_bamboo"),
        "GrowingPlantBlock" if matches!(name, "minecraft:kelp" | "minecraft:kelp_plant") => {
            Ok(!data.in_block_tag(below, "cannot_support_kelp")?
                && (data.is_block(below, "kelp")?
                    || data.is_block(below, "kelp_plant")?
                    || data.info(below)?.sturdy(Direction::Up)))
        }
        "SeaPickleBlock" => Ok(
            shape_info(below)?.collision_top_nonempty || data.info(below)?.sturdy(Direction::Up)
        ),
        "SnowLayerBlock" => {
            if data.in_block_tag(below, "cannot_support_snow_layer")? {
                return Ok(false);
            }
            Ok(data.in_block_tag(below, "support_override_snow_layer")?
                || shape_info(below)?.collision_top_full
                || (data.is_block(below, "snow")?
                    && data.block(below)?.1.property(below, "layers") == Some("8")))
        }
        "SugarCaneBlock" => {
            if data.is_block(below, name)? {
                return Ok(true);
            }
            if !data.in_block_tag(below, "supports_sugar_cane")? {
                return Ok(false);
            }
            for direction in Direction::HORIZONTAL {
                let neighbor = read_block(world, direction.step(below_pos))?;
                if data.in_fluid_tag(neighbor, "supports_sugar_cane_adjacently")?
                    || data.in_block_tag(neighbor, "supports_sugar_cane_adjacently")?
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        "CarpetBlock" => Ok(!data.info(below)?.is_air()),
        "LeafLitterBlock" => Ok(data.info(below)?.sturdy(Direction::Up)),
        "MossyCarpetBlock" => {
            if block.property(state, "bottom") == Some("true") {
                return Ok(!data.info(below)?.is_air());
            }
            Ok(data.is_block(below, name)? && block.property(below, "bottom") == Some("true"))
        }
        "SporeBlossomBlock" => Ok(data
            .info(read_block(world, Direction::Up.step(pos))?)?
            .supports_center(Direction::Down)
            && !data.in_fluid_tag(read_block(world, pos)?, "water")?),
        "SoulFireBlock" => data.in_block_tag(below, "soul_fire_base_blocks"),
        "FireBlock" => {
            if data.info(below)?.sturdy(Direction::Up) {
                return Ok(true);
            }
            for direction in Direction::ALL {
                if data
                    .info(read_block(world, direction.step(pos))?)?
                    .flammable
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        "HangingRootsBlock" => Ok(data
            .info(read_block(world, Direction::Up.step(pos))?)?
            .sturdy(Direction::Down)),
        other => Err(FeatureError::Unsupported(format!(
            "canSurvive {name} ({other})"
        ))),
    }
}

fn vine_survives(world: &dyn FeatureWorld, state: u32, pos: Pos) -> FeatureResult<bool> {
    let data = catalog();
    let vine = data.definition("vine")?;
    let above = Direction::Up.step(pos);
    let mut has_face = vine.property(state, "up") == Some("true")
        // Native getUpdatedState intentionally uses DOWN for this test, unlike
        // the UP-facing initial attachment query in VinesFeature.place.
        && data.info(read_block(world, above)?)?.can_attach_from(Direction::Down);
    for (direction, property) in [
        (Direction::North, "north"),
        (Direction::East, "east"),
        (Direction::South, "south"),
        (Direction::West, "west"),
    ] {
        if vine.property(state, property) != Some("true") {
            continue;
        }
        if data
            .info(read_block(world, direction.step(pos))?)?
            .can_attach_from(direction)
        {
            has_face = true;
            continue;
        }
        let above = read_block(world, above)?;
        if data.is_block(above, "vine")? && vine.property(above, property) == Some("true") {
            has_face = true;
        }
    }
    Ok(has_face)
}

fn cactus_survives(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
    let data = catalog();
    for direction in Direction::HORIZONTAL {
        let state = read_block(world, direction.step(pos))?;
        if data.info(state)?.is_solid() || data.in_fluid_tag(state, "lava")? {
            return Ok(false);
        }
    }
    let below = read_block(world, Direction::Down.step(pos))?;
    if !data.is_block(below, "cactus")? && !data.in_block_tag(below, "supports_cactus")? {
        return Ok(false);
    }
    Ok(!data
        .info(read_block(world, Direction::Up.step(pos))?)?
        .liquid())
}

pub fn read_block(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<u32> {
    if !(MIN_Y..=MAX_Y).contains(&pos.1) {
        return catalog().default_state("void_air");
    }
    world
        .get_block(pos)
        .ok_or_else(|| missing(format!("feature block at {pos:?}")))
}

pub fn is_air(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
    Ok(catalog().info(read_block(world, pos)?)?.is_air())
}

pub fn offset(pos: Pos, delta: Pos) -> Pos {
    (
        pos.0.wrapping_add(delta.0),
        pos.1.wrapping_add(delta.1),
        pos.2.wrapping_add(delta.2),
    )
}

pub(crate) fn qualified(name: &str) -> std::borrow::Cow<'_, str> {
    if name.contains(':') {
        name.into()
    } else {
        format!("minecraft:{name}").into()
    }
}

pub(crate) fn kind(value: &Value) -> FeatureResult<&str> {
    let name = string(&value["type"])?;
    Ok(name.strip_prefix("minecraft:").unwrap_or(name))
}

pub(crate) fn string(value: &Value) -> FeatureResult<&str> {
    value
        .as_str()
        .ok_or_else(|| invalid(format!("expected string, got {value}")))
}

pub(crate) fn array(value: &Value) -> FeatureResult<&[Value]> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| invalid(format!("expected array, got {value}")))
}

pub(crate) fn int(value: &Value) -> FeatureResult<i32> {
    value
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| invalid(format!("expected i32, got {value}")))
}

pub(crate) fn integer(value: &Value, key: &str) -> FeatureResult<i32> {
    int(&value[key])
}

pub(crate) fn number(value: &Value, key: &str) -> FeatureResult<f64> {
    value[key]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid(format!("expected finite {key}")))
}

pub(crate) fn position(value: &Value) -> FeatureResult<Pos> {
    let values = array(value)?;
    if values.len() != 3 {
        return Err(invalid("position requires three coordinates"));
    }
    Ok((int(&values[0])?, int(&values[1])?, int(&values[2])?))
}

pub(crate) fn invalid(message: impl Into<String>) -> FeatureError {
    FeatureError::InvalidConfig(message.into())
}
pub(crate) fn missing(message: impl Into<String>) -> FeatureError {
    FeatureError::MissingData(message.into())
}
