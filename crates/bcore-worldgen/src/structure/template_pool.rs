//! Template-pool assets and weighted selection. Each weight contributes that
//! many entries to the native shuffled candidate list, including duplicates.

use super::processors::{text, BlockTags, Processor};
use super::template::{
    add, id, invalid, missing, shuffle, transform_block, BlockEntityType, BlockInfo, BlockRegistry,
    BlockState, BoundingBox, EntityInfo, Mirror, Nbt, PlacementResult, PlacementSettings,
    ProcessorRandom, Result, Rotation, StateRow, StructureTemplate, TemplateEffect, TemplateRandom,
};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    #[default]
    Rigid,
    TerrainMatching,
}

impl Projection {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rigid => "rigid",
            Self::TerrainMatching => "terrain_matching",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Direction {
    pub fn step(self) -> Pos {
        match self {
            Self::Down => (0, -1, 0),
            Self::Up => (0, 1, 0),
            Self::North => (0, 0, -1),
            Self::South => (0, 0, 1),
            Self::West => (-1, 0, 0),
            Self::East => (1, 0, 0),
        }
    }

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

    fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "down" => Self::Down,
            "up" => Self::Up,
            "north" => Self::North,
            "south" => Self::South,
            "west" => Self::West,
            "east" => Self::East,
            _ => return Err(invalid(format!("jigsaw direction {name}"))),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JigsawConnector {
    pub pos: Pos,
    pub front: Direction,
    pub top: Direction,
    pub name: String,
    pub target: String,
    pub pool: String,
    pub aligned: bool,
    pub selection_priority: i32,
    pub placement_priority: i32,
}

impl JigsawConnector {
    pub fn can_attach(&self, other: &Self) -> bool {
        self.front.opposite() == other.front
            && (!self.aligned || self.top == other.top)
            && self.target == other.name
    }

    fn from_info(
        info: &BlockInfo,
        assets: &StructureAssets,
        origin: Pos,
        rotation: Rotation,
    ) -> Result<Self> {
        let state = assets
            .blocks
            .transform(info.state, Mirror::None, rotation)?;
        let orientation = assets
            .blocks
            .state(state)?
            .properties
            .get("orientation")
            .ok_or_else(|| invalid("jigsaw without orientation"))?;
        let (front, top) = orientation
            .split_once('_')
            .ok_or_else(|| invalid("invalid jigsaw orientation"))?;
        let front = Direction::parse(front)?;
        let top = Direction::parse(top)?;
        let nbt = info
            .nbt
            .as_ref()
            .ok_or_else(|| invalid("jigsaw without NBT"))?;
        let string = |key| {
            nbt.get(key)
                .and_then(Nbt::string)
                .unwrap_or("minecraft:empty")
        };
        let aligned = match nbt.get("joint").and_then(Nbt::string) {
            Some("aligned") => true,
            Some("rollable") => false,
            _ => !matches!(front, Direction::Down | Direction::Up),
        };
        Ok(Self {
            pos: add(
                origin,
                transform_block(info.pos, Mirror::None, rotation, (0, 0, 0)),
            ),
            front,
            top,
            name: id(string("name")),
            target: id(string("target")),
            pool: id(string("pool")),
            aligned,
            selection_priority: nbt
                .get("selection_priority")
                .and_then(Nbt::int)
                .unwrap_or(0),
            placement_priority: nbt
                .get("placement_priority")
                .and_then(Nbt::int)
                .unwrap_or(0),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SingleElement {
    pub location: String,
    pub processors: Vec<Processor>,
    pub processor_config: Value,
    pub projection: Projection,
    pub legacy: bool,
    pub override_waterlogging: Option<bool>,
}

#[derive(Debug, Clone)]
pub enum PoolElement {
    Empty,
    Single(SingleElement),
    List {
        elements: Vec<PoolElement>,
        projection: Projection,
    },
    Feature {
        feature: String,
        projection: Projection,
    },
}

// Persist the native pool-element codec, not the parsed processor implementation.
// Restoring resolves the same pinned templates/processors and rejects missing assets.
impl Serialize for PoolElement {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PoolElement {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_json(&value, StructureAssets::bundled()).map_err(serde::de::Error::custom)
    }
}

impl PartialEq for PoolElement {
    fn eq(&self, other: &Self) -> bool {
        self.to_json() == other.to_json()
    }
}
impl Eq for PoolElement {}

impl PoolElement {
    pub fn from_json(value: &Value, assets: &StructureAssets) -> Result<Self> {
        let projection = match value.get("projection") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|e| invalid(format!("pool projection: {e}")))?,
            None => Projection::Rigid,
        };
        Ok(
            match text(value, "element_type")?.trim_start_matches("minecraft:") {
                "empty_pool_element" => Self::Empty,
                "single_pool_element" | "legacy_single_pool_element" => {
                    let location = id(text(value, "location")?);
                    assets.template(&location)?;
                    let processor_config = value
                        .get("processors")
                        .ok_or_else(|| invalid("pool processors"))?
                        .clone();
                    let processors = assets.resolve_processors(&processor_config)?;
                    let override_waterlogging = match value["override_liquid_settings"].as_str() {
                        None => None,
                        Some("apply_waterlogging") => Some(true),
                        Some("ignore_waterlogging") => Some(false),
                        Some(other) => return Err(invalid(format!("liquid settings {other}"))),
                    };
                    Self::Single(SingleElement {
                        location,
                        processors,
                        processor_config,
                        projection,
                        legacy: text(value, "element_type")?
                            .ends_with("legacy_single_pool_element"),
                        override_waterlogging,
                    })
                }
                "list_pool_element" => {
                    let mut elements = value["elements"]
                        .as_array()
                        .ok_or_else(|| invalid("pool elements"))?
                        .iter()
                        .map(|v| Self::from_json(v, assets))
                        .collect::<Result<Vec<_>>>()?;
                    if elements.is_empty() {
                        return Err(invalid("empty ListPoolElement"));
                    }
                    for element in &mut elements {
                        element.set_projection(projection);
                    }
                    Self::List {
                        elements,
                        projection,
                    }
                }
                "feature_pool_element" => Self::Feature {
                    feature: id(text(value, "feature")?),
                    projection,
                },
                other => return Err(FeatureError::Unsupported(format!("pool element {other}"))),
            },
        )
    }

    fn set_projection(&mut self, value: Projection) {
        match self {
            Self::Empty => {}
            Self::Single(single) => single.projection = value,
            Self::Feature { projection, .. } => *projection = value,
            Self::List {
                elements,
                projection,
            } => {
                *projection = value;
                for element in elements {
                    element.set_projection(value);
                }
            }
        }
    }

    pub fn projection(&self) -> Projection {
        match self {
            Self::Empty => Projection::Rigid,
            Self::Single(single) => single.projection,
            Self::List { projection, .. } | Self::Feature { projection, .. } => *projection,
        }
    }

    pub fn ground_level_delta(&self) -> i32 {
        1
    }

    pub fn contains_features(&self) -> bool {
        match self {
            Self::Feature { .. } => true,
            Self::List { elements, .. } => elements.iter().any(Self::contains_features),
            _ => false,
        }
    }

    pub fn bounding_box(
        &self,
        assets: &StructureAssets,
        origin: Pos,
        rotation: Rotation,
    ) -> Result<BoundingBox> {
        match self {
            Self::Empty => Err(invalid("EmptyPoolElement has no bounding box")),
            Self::Single(single) => Ok(assets.template(&single.location)?.bounding_box(
                origin,
                rotation,
                Mirror::None,
                (0, 0, 0),
            )),
            Self::Feature { .. } => Ok(BoundingBox::new(origin, origin)),
            Self::List { elements, .. } => {
                let mut bounds: Option<BoundingBox> = None;
                for element in elements {
                    if matches!(element, Self::Empty) {
                        continue;
                    }
                    let next = element.bounding_box(assets, origin, rotation)?;
                    bounds = Some(bounds.map_or(next, |bb| bb.union(next)));
                }
                bounds.ok_or_else(|| invalid("ListPoolElement contains only empty elements"))
            }
        }
    }

    pub fn shuffled_jigsaws(
        &self,
        assets: &StructureAssets,
        origin: Pos,
        rotation: Rotation,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<Vec<JigsawConnector>> {
        match self {
            Self::Empty => Ok(Vec::new()),
            Self::List { elements, .. } => {
                elements[0].shuffled_jigsaws(assets, origin, rotation, random)
            }
            // FeaturePoolElement does not shuffle or rotate its synthetic marker.
            Self::Feature { .. } => Ok(vec![JigsawConnector {
                pos: origin,
                front: Direction::Down,
                top: Direction::South,
                name: "minecraft:bottom".into(),
                target: "minecraft:empty".into(),
                pool: "minecraft:empty".into(),
                aligned: false,
                selection_priority: 0,
                placement_priority: 0,
            }]),
            Self::Single(single) => {
                let mut jigsaws = Vec::new();
                for info in assets.template(&single.location)?.palette(origin)? {
                    if assets.blocks.state(info.state)?.name == "minecraft:jigsaw" {
                        jigsaws.push(JigsawConnector::from_info(info, assets, origin, rotation)?);
                    }
                }
                shuffle(&mut jigsaws, random);
                jigsaws.sort_by_key(|j| Reverse(j.selection_priority));
                Ok(jigsaws)
            }
        }
    }

    pub fn settings(
        &self,
        rotation: Rotation,
        clip: BoundingBox,
        waterlogging: bool,
        world_seed: i64,
        keep_jigsaws: bool,
    ) -> Result<PlacementSettings> {
        let Self::Single(single) = self else {
            return Err(invalid("placement settings require a single pool element"));
        };
        let mut processors = Vec::new();
        if !single.legacy {
            processors.push(Processor::ignore_structure());
        }
        if !keep_jigsaws {
            processors.push(Processor::JigsawReplacement);
        }
        processors.extend(single.processors.clone());
        if single.projection == Projection::TerrainMatching {
            processors.push(Processor::Gravity {
                heightmap: FeatureHeightmap::WorldSurfaceWg,
                offset: -1,
            });
        }
        // LegacySingle removes the first ignore processor and appends the
        // structure+air filter after the projection processors.
        if single.legacy {
            processors.push(Processor::ignore_structure_and_air());
        }
        Ok(PlacementSettings {
            rotation,
            clip: Some(clip),
            processors,
            finalize_entities: true,
            apply_waterlogging: single.override_waterlogging.unwrap_or(waterlogging),
            world_seed,
            ..PlacementSettings::default()
        })
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Empty => json!({"element_type": "minecraft:empty_pool_element"}),
            Self::Feature {
                feature,
                projection,
            } => {
                json!({"element_type": "minecraft:feature_pool_element", "feature": feature, "projection": projection.name()})
            }
            Self::List {
                elements,
                projection,
            } => {
                json!({"element_type": "minecraft:list_pool_element", "elements": elements.iter().map(Self::to_json).collect::<Vec<_>>(), "projection": projection.name()})
            }
            Self::Single(single) => {
                let mut value = json!({"element_type": if single.legacy { "minecraft:legacy_single_pool_element" } else { "minecraft:single_pool_element" },
                    "location": single.location, "processors": single.processor_config, "projection": single.projection.name()});
                if let Some(mode) = single.override_waterlogging {
                    value["override_liquid_settings"] = json!(if mode {
                        "apply_waterlogging"
                    } else {
                        "ignore_waterlogging"
                    });
                }
                value
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WeightedElement {
    pub weight: u32,
    pub element: PoolElement,
}

#[derive(Debug, Clone)]
pub struct TemplatePool {
    pub fallback: String,
    pub elements: Vec<WeightedElement>,
}

impl TemplatePool {
    pub fn from_json(value: &Value, assets: &StructureAssets) -> Result<Self> {
        let mut elements = Vec::new();
        let mut total = 0_u64;
        for row in value["elements"]
            .as_array()
            .ok_or_else(|| invalid("template pool elements"))?
        {
            let weight = row["weight"]
                .as_u64()
                .filter(|w| (1..=150).contains(w))
                .ok_or_else(|| invalid("template pool weight outside [1, 150]"))?;
            total += weight;
            if total > i32::MAX as u64 {
                return Err(invalid("template pool weight overflow"));
            }
            elements.push(WeightedElement {
                weight: weight as u32,
                element: PoolElement::from_json(&row["element"], assets)?,
            });
        }
        Ok(Self {
            fallback: id(text(value, "fallback")?),
            elements,
        })
    }

    pub fn weight(&self) -> usize {
        self.elements.iter().map(|e| e.weight as usize).sum()
    }

    pub fn random_element(
        &self,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Result<&PoolElement> {
        let total = self.weight();
        if total == 0 {
            return Err(invalid("cannot select an element from an empty start pool"));
        }
        let mut index = random.next_int(total);
        for entry in &self.elements {
            if index < entry.weight as usize {
                return Ok(&entry.element);
            }
            index -= entry.weight as usize;
        }
        unreachable!("validated weighted selection")
    }

    pub fn shuffled_elements(
        &self,
        random: &mut (impl TemplateRandom + ?Sized),
    ) -> Vec<&PoolElement> {
        let mut elements = Vec::with_capacity(self.weight());
        for entry in &self.elements {
            for _ in 0..entry.weight {
                elements.push(&entry.element);
            }
        }
        shuffle(&mut elements, random);
        elements
    }

    pub fn max_size(&self, assets: &StructureAssets) -> Result<i32> {
        let mut max = 0;
        for element in &self.elements {
            if !matches!(element.element, PoolElement::Empty) {
                max = max.max(
                    element
                        .element
                        .bounding_box(assets, (0, 0, 0), Rotation::None)?
                        .y_span(),
                );
            }
        }
        Ok(max)
    }
}

#[derive(Debug, Deserialize)]
struct JarAsset {
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawBlock {
    WithNbt(i32, i32, i32, usize, Nbt),
    Plain(i32, i32, i32, usize),
}

#[derive(Debug, Deserialize)]
struct RawTemplate {
    size: Pos,
    palettes: Vec<Vec<BlockState>>,
    blocks: Vec<RawBlock>,
    entities: Vec<EntityInfo>,
}

#[derive(Deserialize)]
struct AssetData {
    jar_sha256: String,
    states: Vec<StateRow>,
    defaults: BTreeMap<String, u32>,
    block_entities: BTreeMap<String, BlockEntityType>,
    water_fluid_id: u32,
    structure_metadata: BTreeMap<String, StructureMetadata>,
    templates: BTreeMap<String, RawTemplate>,
    jar_assets: BTreeMap<String, JarAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StructureMetadata {
    pub step: i32,
    pub feature_index: i32,
    pub biomes: Vec<u32>,
}

#[derive(Debug)]
pub struct StructureAssets {
    pub blocks: BlockRegistry,
    pub templates: BTreeMap<String, StructureTemplate>,
    pub pools: BTreeMap<String, TemplatePool>,
    pub tags: BlockTags,
    pub structure_configs: BTreeMap<String, Value>,
    pub structure_sets: BTreeMap<String, Value>,
    pub structure_metadata: BTreeMap<String, StructureMetadata>,
    pub placed_features: BTreeMap<String, Value>,
    processor_configs: BTreeMap<String, Value>,
}

impl StructureAssets {
    pub const JAR_SHA256: &'static str =
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52";

    pub fn bundled() -> &'static Self {
        static ASSETS: OnceLock<StructureAssets> = OnceLock::new();
        ASSETS.get_or_init(|| {
            let mut assets = Self::from_json(include_str!(
                "../../data/pool_alias_trial_combined_assets_26_1_v2.json"
            ))
            .expect("validated pinned jigsaw assets");
            // The independent scattered capture supplies the dispenser defaults
            // absent from the jigsaw bundle. Existing immutable captures remain
            // authoritative; join by resource identity only for missing entries.
            let containers: Value =
                serde_json::from_str(include_str!("../../data/scattered_containers_26_1.json"))
                    .expect("native container defaults");
            assert_eq!(containers["jar_sha256"], Self::JAR_SHA256);
            for (name, data) in containers["containers"]["defaults"].as_object().unwrap() {
                assets
                    .blocks
                    .block_entities
                    .entry(name.clone())
                    .or_insert_with(|| BlockEntityType {
                        id: data["id"].as_str().unwrap().into(),
                        type_id: data["type_id"].as_u64().unwrap() as u32,
                        randomizable: true,
                        nbt: serde_json::from_value(data["full"]["nbt"].clone()).unwrap(),
                    });
            }
            assets
        })
    }

    pub fn from_json(source: &str) -> Result<Self> {
        let data: AssetData =
            serde_json::from_str(source).map_err(|e| invalid(format!("jigsaw assets: {e}")))?;
        if data.jar_sha256 != Self::JAR_SHA256 {
            return Err(invalid("jigsaw asset JAR version/hash mismatch"));
        }
        let blocks = BlockRegistry::new(
            data.states,
            data.defaults,
            data.block_entities,
            data.water_fluid_id,
        )?;
        let mut templates = BTreeMap::new();
        for (name, raw) in data.templates {
            let mut palettes = Vec::new();
            for palette in raw.palettes {
                let states = palette
                    .iter()
                    .map(|v| blocks.resolve(v))
                    .collect::<Result<Vec<_>>>()?;
                let mut infos = Vec::with_capacity(raw.blocks.len());
                for block in &raw.blocks {
                    let (pos, index, nbt) = match block {
                        RawBlock::Plain(x, y, z, index) => ((*x, *y, *z), *index, None),
                        RawBlock::WithNbt(x, y, z, index, nbt) => {
                            ((*x, *y, *z), *index, Some(nbt.clone()))
                        }
                    };
                    infos.push(BlockInfo {
                        pos,
                        state: *states.get(index).ok_or_else(|| {
                            invalid(format!("{name}: invalid palette index {index}"))
                        })?,
                        nbt,
                    });
                }
                palettes.push(infos);
            }
            templates.insert(
                name,
                StructureTemplate::new(raw.size, palettes, raw.entities, &blocks)?,
            );
        }
        let mut tags_raw = BTreeMap::new();
        let mut pool_configs = BTreeMap::new();
        let mut processor_configs = BTreeMap::new();
        let mut structure_configs = BTreeMap::new();
        let mut structure_sets = BTreeMap::new();
        let mut placed_features = BTreeMap::new();
        for (path, asset) in data.jar_assets {
            for (prefix, destination) in [
                ("data/minecraft/tags/block/", &mut tags_raw),
                ("data/minecraft/worldgen/template_pool/", &mut pool_configs),
                (
                    "data/minecraft/worldgen/processor_list/",
                    &mut processor_configs,
                ),
                ("data/minecraft/worldgen/structure/", &mut structure_configs),
                (
                    "data/minecraft/worldgen/structure_set/",
                    &mut structure_sets,
                ),
                (
                    "data/minecraft/worldgen/placed_feature/",
                    &mut placed_features,
                ),
            ] {
                if let Some(name) = path
                    .strip_prefix(prefix)
                    .and_then(|p| p.strip_suffix(".json"))
                {
                    destination.insert(id(name), asset.value.clone());
                }
            }
        }
        let mut tags = BTreeMap::new();
        for name in tags_raw.keys() {
            resolve_tag(name, &tags_raw, &mut tags, &mut BTreeSet::new())?;
        }
        let mut assets = Self {
            blocks,
            templates,
            pools: BTreeMap::new(),
            tags,
            structure_configs,
            processor_configs,
            structure_sets,
            structure_metadata: data.structure_metadata,
            placed_features,
        };
        for (name, config) in pool_configs {
            assets
                .pools
                .insert(name, TemplatePool::from_json(&config, &assets)?);
        }
        for pool in assets.pools.values() {
            assets.pool(&pool.fallback)?;
        }
        Ok(assets)
    }

    pub fn template(&self, name: &str) -> Result<&StructureTemplate> {
        self.templates
            .get(&id(name))
            .ok_or_else(|| missing(format!("structure template {name}")))
    }

    pub fn pool(&self, name: &str) -> Result<&TemplatePool> {
        self.pools
            .get(&id(name))
            .ok_or_else(|| missing(format!("template pool {name}")))
    }

    pub fn resolve_processors(&self, value: &Value) -> Result<Vec<Processor>> {
        let value = if let Some(name) = value.as_str() {
            self.processor_configs
                .get(&id(name))
                .ok_or_else(|| missing(format!("processor list {name}")))?
        } else {
            value
        };
        value["processors"]
            .as_array()
            .ok_or_else(|| invalid("processor list"))?
            .iter()
            .map(|v| Processor::from_json(v, &self.blocks, &self.tags))
            .collect()
    }
}

fn resolve_tag(
    name: &str,
    raw: &BTreeMap<String, Value>,
    resolved: &mut BlockTags,
    stack: &mut BTreeSet<String>,
) -> Result<()> {
    if resolved.contains_key(name) {
        return Ok(());
    }
    if !stack.insert(name.to_owned()) {
        return Err(invalid(format!("cyclic block tag {name}")));
    }
    let data = raw
        .get(name)
        .ok_or_else(|| missing(format!("block tag {name}")))?;
    let mut entries = BTreeSet::new();
    for value in data["values"]
        .as_array()
        .ok_or_else(|| invalid(format!("block tag values {name}")))?
    {
        let reference = value
            .as_str()
            .or_else(|| value["id"].as_str())
            .ok_or_else(|| invalid("tag entry"))?;
        if let Some(nested) = reference.strip_prefix('#') {
            let nested = id(nested);
            if !raw.contains_key(&nested) && value["required"] == false {
                continue;
            }
            resolve_tag(&nested, raw, resolved, stack)?;
            entries.extend(resolved[&nested].clone());
        } else {
            entries.insert(id(reference));
        }
    }
    stack.remove(name);
    resolved.insert(name.to_owned(), entries);
    Ok(())
}

/// Context shared by a pool element and each element of an overlay list.
#[derive(Debug, Clone, Copy)]
pub struct PoolPlacement {
    pub origin: Pos,
    pub reference: Pos,
    pub rotation: Rotation,
    pub clip: BoundingBox,
    pub waterlogging: bool,
    pub world_seed: i64,
    pub keep_jigsaws: bool,
}

/// Execute feature elements synchronously in piece order using the same RNG.
/// The callback is the integration seam for the shared placed-feature driver;
/// returning a deferred list or substituting a no-op changes downstream loot RNG.
pub fn place_element<W, R, F>(
    assets: &StructureAssets,
    element: &PoolElement,
    world: &mut W,
    random: &mut R,
    placement: PoolPlacement,
    feature: &mut F,
) -> Result<PlacementResult>
where
    W: FeatureWorld + ?Sized,
    R: TemplateRandom + ?Sized,
    F: FnMut(&str, &mut W, &mut R, Pos) -> Result<PlacementResult>,
{
    place_element_with_effects(
        assets,
        element,
        world,
        random,
        placement,
        feature,
        &mut |_, _| Ok(()),
    )
}

/// Retained-world variant; template effects are delivered at their native point
/// in the write stream, including before a later list/feature element fails.
pub fn place_element_with_effects<W, R, F, E>(
    assets: &StructureAssets,
    element: &PoolElement,
    world: &mut W,
    random: &mut R,
    placement: PoolPlacement,
    feature: &mut F,
    effects: &mut E,
) -> Result<PlacementResult>
where
    W: FeatureWorld + ?Sized,
    R: TemplateRandom + ?Sized,
    F: FnMut(&str, &mut W, &mut R, Pos) -> Result<PlacementResult>,
    E: FnMut(&mut W, TemplateEffect) -> Result<()>,
{
    match element {
        PoolElement::Empty => Ok(PlacementResult {
            placed: true,
            ..PlacementResult::default()
        }),
        PoolElement::Feature { feature: name, .. } => {
            feature(name, world, random, placement.origin)
        }
        PoolElement::List { elements, .. } => {
            let mut result = PlacementResult::default();
            for child in elements {
                let child_result = place_element_with_effects(
                    assets, child, world, random, placement, feature, effects,
                )?;
                let success = child_result.placed;
                result.merge(child_result);
                if !success {
                    result.placed = false;
                    break;
                }
            }
            Ok(result)
        }
        PoolElement::Single(single) => {
            let settings = element.settings(
                placement.rotation,
                placement.clip,
                placement.waterlogging,
                placement.world_seed,
                placement.keep_jigsaws,
            )?;
            assets.template(&single.location)?.place_with_effects(
                world,
                random,
                &mut ProcessorRandom(None),
                &assets.blocks,
                placement.origin,
                placement.reference,
                &settings,
                effects,
            )
        }
    }
}
