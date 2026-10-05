//! Seeded, priority-ordered jigsaw assembly and clipped template placement.
//!
//! Starts contain metadata for the structure/reference and beardifier stages.
//! `place_in_chunk_with_features` performs block placement later, using the
//! decoration driver's shared world and RNG. It does not choose a source schedule.

use super::placement::{
    large_feature_random, FrequencyReduction, RandomSpreadPlacement, SpreadType,
};
use super::pool_alias::PoolAliasBindings;
use super::processors::{heightmap, integer, text};
use super::template::{
    add, id, invalid, missing, shuffle, sub, BoundingBox, Nbt, PlacementResult, Result, Rotation,
    TemplateEffect, TemplateRandom,
};
use super::template_pool::{
    place_element_with_effects, JigsawConnector, PoolElement, PoolPlacement, Projection,
    StructureAssets,
};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use bcore_core::ChunkPos;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

pub struct HeightContext<'a> {
    pub min_y: i32,
    pub max_y: i32,
    /// The generator's first-free height, not a guessed terrain elevation.
    pub first_free: &'a dyn Fn(FeatureHeightmap, i32, i32) -> i32,
}

#[derive(Debug, Clone)]
pub enum VerticalAnchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
}

impl VerticalAnchor {
    fn resolve(&self, heights: &HeightContext<'_>) -> i32 {
        match self {
            Self::Absolute(y) => *y,
            Self::AboveBottom(y) => heights.min_y.wrapping_add(*y),
            Self::BelowTop(y) => heights.max_y.wrapping_sub(*y),
        }
    }

    fn parse(value: &Value) -> Result<Self> {
        for (key, constructor) in [
            ("absolute", Self::Absolute as fn(i32) -> Self),
            ("above_bottom", Self::AboveBottom),
            ("below_top", Self::BelowTop),
        ] {
            if value.get(key).is_some() {
                return Ok(constructor(integer(value, key, None)?));
            }
        }
        Err(invalid("jigsaw vertical anchor"))
    }
}

#[derive(Debug, Clone)]
pub enum StartHeight {
    Constant(VerticalAnchor),
    Uniform {
        min: VerticalAnchor,
        max: VerticalAnchor,
    },
}

impl StartHeight {
    pub fn sample(
        &self,
        heights: &HeightContext<'_>,
        random: &mut impl TemplateRandom,
    ) -> Result<i32> {
        match self {
            Self::Constant(anchor) => Ok(anchor.resolve(heights)),
            Self::Uniform { min, max } => {
                let min = min.resolve(heights);
                let max = max.resolve(heights);
                if max < min {
                    return Ok(min);
                } // Native empty-range fallback.
                let bound = i64::from(max) - i64::from(min) + 1;
                if bound > i64::from(i32::MAX) {
                    return Err(invalid("start height range overflow"));
                }
                Ok(min + random.next_int(bound as usize) as i32)
            }
        }
    }

    fn parse(value: &Value) -> Result<Self> {
        if value.get("type").is_none() {
            return Ok(Self::Constant(VerticalAnchor::parse(value)?));
        }
        Ok(
            match text(value, "type")?.trim_start_matches("minecraft:") {
                "constant" => Self::Constant(VerticalAnchor::parse(&value["value"])?),
                "uniform" => Self::Uniform {
                    min: VerticalAnchor::parse(&value["min_inclusive"])?,
                    max: VerticalAnchor::parse(&value["max_inclusive"])?,
                },
                other => {
                    return Err(FeatureError::Unsupported(format!(
                        "jigsaw start height provider {other}"
                    )))
                }
            },
        )
    }
}

#[derive(Debug, Clone)]
pub struct JigsawConfig {
    pub start_pool: String,
    pub start_jigsaw_name: Option<String>,
    pub max_depth: u32,
    pub start_height: StartHeight,
    pub project_start_to_heightmap: Option<FeatureHeightmap>,
    pub max_horizontal_distance: i32,
    pub max_vertical_distance: i32,
    pub use_expansion_hack: bool,
    pub padding_bottom: i32,
    pub padding_top: i32,
    pub waterlogging: bool,
    pub terrain_adaptation: String,
    pub decoration_step: String,
    pub pool_aliases: PoolAliasBindings,
}

impl JigsawConfig {
    pub fn from_assets(assets: &StructureAssets, name: &str) -> Result<Self> {
        Self::from_json(
            assets
                .structure_configs
                .get(&id(name))
                .ok_or_else(|| missing(format!("jigsaw structure {name}")))?,
        )
    }

    pub fn from_json(value: &Value) -> Result<Self> {
        if text(value, "type")? != "minecraft:jigsaw" {
            return Err(invalid("expected jigsaw structure configuration"));
        }
        let max_depth = integer(value, "size", None)?;
        if !(0..=20).contains(&max_depth) {
            return Err(invalid("jigsaw size outside [0, 20]"));
        }
        let distance = &value["max_distance_from_center"];
        let (horizontal, vertical) = if let Some(n) = distance.as_i64() {
            let n = i32::try_from(n).map_err(|_| invalid("jigsaw distance"))?;
            (n, n)
        } else {
            (
                integer(distance, "horizontal", None)?,
                integer(distance, "vertical", None)?,
            )
        };
        if !(1..=128).contains(&horizontal) || !(1..=4096).contains(&vertical) {
            return Err(invalid("jigsaw distance out of range"));
        }
        let padding = &value["dimension_padding"];
        let (bottom, top) = if padding.is_null() {
            (0, 0)
        } else if let Some(n) = padding.as_i64() {
            let n = i32::try_from(n).map_err(|_| invalid("dimension padding"))?;
            (n, n)
        } else {
            (
                integer(padding, "bottom", Some(0))?,
                integer(padding, "top", Some(0))?,
            )
        };
        if bottom < 0 || top < 0 {
            return Err(invalid("negative dimension padding"));
        }
        let waterlogging = match value["liquid_settings"]
            .as_str()
            .unwrap_or("apply_waterlogging")
        {
            "apply_waterlogging" => true,
            "ignore_waterlogging" => false,
            other => return Err(invalid(format!("liquid settings {other}"))),
        };
        Ok(Self {
            start_pool: id(text(value, "start_pool")?),
            start_jigsaw_name: value["start_jigsaw_name"].as_str().map(id),
            max_depth: max_depth as u32,
            start_height: StartHeight::parse(&value["start_height"])?,
            project_start_to_heightmap: value["project_start_to_heightmap"]
                .as_str()
                .map(heightmap)
                .transpose()?,
            max_horizontal_distance: horizontal,
            max_vertical_distance: vertical,
            use_expansion_hack: value["use_expansion_hack"]
                .as_bool()
                .ok_or_else(|| invalid("use_expansion_hack"))?,
            padding_bottom: bottom,
            padding_top: top,
            waterlogging,
            terrain_adaptation: value["terrain_adaptation"]
                .as_str()
                .unwrap_or("none")
                .to_owned(),
            decoration_step: text(value, "step")?.to_owned(),
            pool_aliases: PoolAliasBindings::from_structure_json(value)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Junction {
    pub source: Pos,
    pub delta_y: i32,
    pub destination_projection: Projection,
}

impl Junction {
    pub fn to_nbt(&self) -> Nbt {
        Nbt::Compound(BTreeMap::from([
            ("source_x".into(), Nbt::Int(self.source.0)),
            ("source_ground_y".into(), Nbt::Int(self.source.1)),
            ("source_z".into(), Nbt::Int(self.source.2)),
            ("delta_y".into(), Nbt::Int(self.delta_y)),
            (
                "dest_proj".into(),
                Nbt::String(self.destination_projection.name().into()),
            ),
        ]))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JigsawPiece {
    pub element: PoolElement,
    pub origin: Pos,
    pub rotation: Rotation,
    /// Includes the native expansion-hack reservation, as does the saved BB.
    pub bounds: BoundingBox,
    pub ground_level_delta: i32,
    pub depth: u32,
    pub junctions: Vec<Junction>,
    pub waterlogging: bool,
}

impl JigsawPiece {
    pub fn projection(&self) -> Projection {
        self.element.projection()
    }

    pub fn to_nbt(&self) -> Nbt {
        let mut data = BTreeMap::from([
            ("id".into(), Nbt::String("minecraft:jigsaw".into())),
            ("BB".into(), Nbt::IntArray(self.bounds.as_array().to_vec())),
            ("GD".into(), Nbt::Int(0)),
            ("O".into(), Nbt::Int(-1)),
            ("PosX".into(), Nbt::Int(self.origin.0)),
            ("PosY".into(), Nbt::Int(self.origin.1)),
            ("PosZ".into(), Nbt::Int(self.origin.2)),
            (
                "ground_level_delta".into(),
                Nbt::Int(self.ground_level_delta),
            ),
            ("rotation".into(), Nbt::String(self.rotation.name().into())),
            ("pool_element".into(), element_nbt(&self.element.to_json())),
            (
                "junctions".into(),
                Nbt::List {
                    element_type: if self.junctions.is_empty() { 0 } else { 10 },
                    values: self.junctions.iter().map(Junction::to_nbt).collect(),
                },
            ),
        ]);
        if !self.waterlogging {
            data.insert(
                "liquid_settings".into(),
                Nbt::String("ignore_waterlogging".into()),
            );
        }
        Nbt::Compound(data)
    }
}

fn element_nbt(value: &Value) -> Nbt {
    match value {
        Value::String(v) => Nbt::String(v.clone()),
        Value::Object(v) => {
            Nbt::Compound(v.iter().map(|(k, v)| (k.clone(), element_nbt(v))).collect())
        }
        Value::Array(v) => {
            let values: Vec<_> = v.iter().map(element_nbt).collect();
            Nbt::List {
                element_type: if values.is_empty() { 0 } else { 10 },
                values,
            }
        }
        Value::Bool(v) => Nbt::Byte(i8::from(*v)),
        Value::Number(v) => match v.as_i64() {
            Some(n) => Nbt::Int(n as i32),
            None => Nbt::Float(v.as_f64().expect("JSON number") as f32),
        },
        Value::Null => Nbt::empty_compound(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JigsawStart {
    pub generation_point: Pos,
    pub pieces: Vec<JigsawPiece>,
    pub terrain_adaptation: String,
    pub decoration_step: String,
    /// StructureStart's locate reference count, independent of chunk references.
    #[serde(default)]
    pub references: i32,
}

impl JigsawStart {
    pub fn to_nbt(&self, structure: &str, source: ChunkPos) -> Nbt {
        Nbt::Compound(BTreeMap::from([
            ("id".into(), Nbt::String(id(structure))),
            ("ChunkX".into(), Nbt::Int(source.x)),
            ("ChunkZ".into(), Nbt::Int(source.z)),
            ("references".into(), Nbt::Int(self.references)),
            (
                "Children".into(),
                Nbt::List {
                    element_type: if self.pieces.is_empty() { 0 } else { 10 },
                    values: self.pieces.iter().map(JigsawPiece::to_nbt).collect(),
                },
            ),
        ]))
    }

    pub fn valid_for(&self, structure: &str, source: ChunkPos) -> bool {
        let Ok(config) = JigsawConfig::from_assets(StructureAssets::bundled(), structure) else {
            return false;
        };
        if self.pieces.is_empty()
            || self.pieces.len() > 16384
            || self.references < 0
            || self.terrain_adaptation != config.terrain_adaptation
            || self.decoration_step != config.decoration_step
        {
            return false;
        }
        let origin = [i64::from(source.x) * 16, i64::from(source.z) * 16];
        let near = |p: Pos| {
            (i64::from(p.0) - origin[0]).abs() <= 256
                && (i64::from(p.2) - origin[1]).abs() <= 256
                && (crate::MIN_Y - 4096..=crate::MAX_Y + 4096).contains(&p.1)
        };
        near(self.generation_point)
            && self.pieces.iter().all(|piece| {
                let bb = piece.bounds;
                near(piece.origin)
                    && near(bb.min)
                    && near(bb.max)
                    && bb.min.0 <= bb.max.0
                    && bb.min.1 <= bb.max.1
                    && bb.min.2 <= bb.max.2
                    && piece.depth <= config.max_depth + 1
                    && piece.junctions.len() <= 4096
                    && piece.junctions.iter().all(|junction| near(junction.source))
            })
    }

    pub fn bounds(&self) -> Option<BoundingBox> {
        self.pieces
            .iter()
            .map(|p| p.bounds)
            .reduce(BoundingBox::union)
    }

    /// StructureStart's reference/locate box expands adapted structures by 12.
    /// Piece boxes themselves must remain unexpanded for density and placement.
    pub fn reference_bounds(&self) -> Option<BoundingBox> {
        self.bounds().map(|bb| {
            if self.terrain_adaptation == "none" {
                bb
            } else {
                bb.inflated(12)
            }
        })
    }

    pub fn reference_position(&self) -> Option<Pos> {
        self.pieces.first().map(|p| {
            let bb = p.bounds;
            (
                bb.min.0 + (bb.max.0 - bb.min.0 + 1) / 2,
                bb.min.1,
                bb.min.2 + (bb.max.2 - bb.min.2 + 1) / 2,
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct NamedStart {
    pub structure: String,
    pub source: ChunkPos,
    pub start: JigsawStart,
}

/// Random-spread settings extracted from the native structure set.
pub fn set_placement(assets: &StructureAssets, set: &str) -> Result<RandomSpreadPlacement> {
    let config = &assets
        .structure_sets
        .get(&id(set))
        .ok_or_else(|| missing(format!("structure set {set}")))?["placement"];
    if text(config, "type")? != "minecraft:random_spread" {
        return Err(FeatureError::Unsupported(
            "jigsaw structure-set placement type".into(),
        ));
    }
    if config.get("exclusion_zone").is_some() {
        return Err(FeatureError::Unsupported(
            "structure-set exclusion zones".into(),
        ));
    }
    let spread = match config["spread_type"].as_str().unwrap_or("linear") {
        "linear" => SpreadType::Linear,
        "triangular" => SpreadType::Triangular,
        other => return Err(invalid(format!("spread type {other}"))),
    };
    let reduction = match config["frequency_reduction_method"]
        .as_str()
        .unwrap_or("default")
    {
        "default" => FrequencyReduction::Default,
        "legacy_type_1" => FrequencyReduction::LegacyType1,
        "legacy_type_2" => FrequencyReduction::LegacyType2,
        "legacy_type_3" => FrequencyReduction::LegacyType3,
        other => return Err(invalid(format!("frequency reduction {other}"))),
    };
    RandomSpreadPlacement::new(
        integer(config, "spacing", None)?,
        integer(config, "separation", None)?,
        integer(config, "salt", None)?,
        spread,
        config["frequency"].as_f64().unwrap_or(1.0) as f32,
        reduction,
    )
    .ok_or_else(|| invalid("invalid jigsaw structure-set placement"))
}

/// Native weighted retry/admission path for `ancient_cities` and `villages`.
/// The biome callback takes absolute blocks and must return the generator's
/// noise-biome ID at that point (quart coordinates internally), not biome zoom.
pub fn for_chunk(
    assets: &StructureAssets,
    set: &str,
    seed: i64,
    chunk: ChunkPos,
    heights: &HeightContext<'_>,
    mut noise_biome: impl FnMut(Pos) -> u32,
) -> Result<Option<NamedStart>> {
    if !set_placement(assets, set)?.is_candidate(seed, chunk) {
        return Ok(None);
    }
    let config = &assets.structure_sets[&id(set)];
    let mut entries = Vec::new();
    for entry in config["structures"]
        .as_array()
        .ok_or_else(|| invalid("structure-set entries"))?
    {
        let name = id(text(entry, "structure")?);
        let weight = integer(entry, "weight", None)?;
        if weight < 1 {
            return Err(invalid("nonpositive structure-set weight"));
        }
        entries.push((name, weight));
    }
    let single = entries.len() == 1;
    let mut random = large_feature_random(seed, chunk);
    while !entries.is_empty() {
        let total: i64 = entries.iter().map(|e| i64::from(e.1)).sum();
        if total > i64::from(i32::MAX) {
            return Err(invalid("structure-set weight overflow"));
        }
        let index = if single {
            0
        } else {
            let mut choice = random.next_int(total as usize) as i32;
            entries
                .iter()
                .position(|(_, weight)| {
                    choice -= *weight;
                    choice < 0
                })
                .expect("validated weights")
        };
        let (name, _) = entries.remove(index);
        let config = JigsawConfig::from_assets(assets, &name)?;
        let metadata = assets
            .structure_metadata
            .get(&name)
            .ok_or_else(|| missing(format!("structure metadata {name}")))?;
        if let Some(start) = generate_start(assets, &config, seed, chunk, heights, |p| {
            metadata.biomes.contains(&noise_biome(p))
        })? {
            if !start.pieces.is_empty() {
                return Ok(Some(NamedStart {
                    structure: name,
                    source: chunk,
                    start,
                }));
            }
        }
    }
    Ok(None)
}

/// Generate an admitted start with the native large-feature seed. Candidate
/// random-spread selection is supplied by the structure-set owner, not repeated.
pub fn generate_start(
    assets: &StructureAssets,
    config: &JigsawConfig,
    world_seed: i64,
    chunk: ChunkPos,
    heights: &HeightContext<'_>,
    valid_biome: impl FnOnce(Pos) -> bool,
) -> Result<Option<JigsawStart>> {
    let mut random = large_feature_random(world_seed, chunk);
    let y = config.start_height.sample(heights, &mut random)?;
    let origin = (chunk.x.wrapping_mul(16), y, chunk.z.wrapping_mul(16));
    let aliases = config.pool_aliases.resolve(world_seed, origin)?;
    assemble_with_biome(
        assets,
        config,
        origin,
        heights,
        &mut random,
        &aliases,
        valid_biome,
    )
}

/// The actual placement algorithm, usable with caller-owned RNG and resolved
/// alias bindings. No seed-specific piece lists or bounding boxes are embedded.
pub fn assemble(
    assets: &StructureAssets,
    config: &JigsawConfig,
    origin: Pos,
    heights: &HeightContext<'_>,
    random: &mut impl TemplateRandom,
    aliases: &BTreeMap<String, String>,
) -> Result<Option<JigsawStart>> {
    assemble_with_biome(assets, config, origin, heights, random, aliases, |_| true)
}

fn assemble_with_biome(
    assets: &StructureAssets,
    config: &JigsawConfig,
    origin: Pos,
    heights: &HeightContext<'_>,
    random: &mut impl TemplateRandom,
    aliases: &BTreeMap<String, String>,
    valid_biome: impl FnOnce(Pos) -> bool,
) -> Result<Option<JigsawStart>> {
    if heights.min_y > heights.max_y {
        return Err(invalid("inverted dimension height"));
    }
    let rotation = Rotation::ALL[random.next_int(4)];
    let start_pool = resolve_alias(&config.start_pool, aliases);
    let element = assets.pool(start_pool)?.random_element(random)?.clone();
    if matches!(element, PoolElement::Empty) {
        return Ok(None);
    }
    let anchor = if let Some(name) = &config.start_jigsaw_name {
        let connectors = element.shuffled_jigsaws(assets, origin, rotation, random)?;
        let Some(connector) = connectors.iter().find(|c| c.name == *name) else {
            return Ok(None);
        };
        sub(connector.pos, origin)
    } else {
        (0, 0, 0)
    };
    let mut position = sub(origin, anchor);
    let mut bounds = element.bounding_box(assets, position, rotation)?;
    let (x, z) = bounds.center_xz();
    let ground = config.project_start_to_heightmap.map_or(position.1, |map| {
        origin.1.wrapping_add((heights.first_free)(map, x, z))
    });
    let ground_level_delta = element.ground_level_delta();
    let dy = ground.wrapping_sub(bounds.min.1.wrapping_add(ground_level_delta));
    position = add(position, (0, dy, 0));
    bounds = bounds.moved((0, dy, 0));
    if (config.padding_bottom != 0 || config.padding_top != 0)
        && (bounds.min.1 < heights.min_y + config.padding_bottom
            || bounds.max.1 > heights.max_y - config.padding_top)
    {
        return Ok(None);
    }
    let generation_point = (x, ground.wrapping_add(anchor.1), z);
    // Native validates the stub before executing its deferred pieces builder.
    if !valid_biome(generation_point) {
        return Ok(None);
    }
    let mut start = JigsawStart {
        generation_point,
        pieces: Vec::new(),
        terrain_adaptation: config.terrain_adaptation.clone(),
        decoration_step: config.decoration_step.clone(),
        references: 0,
    };
    // The pinned JAR's deferred builder returns before adding even the start
    // piece for size=0; this differs from generating with depth 1.
    if config.max_depth == 0 {
        return Ok(Some(start));
    }
    start.pieces.push(JigsawPiece {
        element,
        origin: position,
        rotation,
        bounds,
        ground_level_delta,
        depth: 0,
        junctions: Vec::new(),
        waterlogging: config.waterlogging,
    });
    let allowed = BoundingBox {
        min: (
            x - config.max_horizontal_distance,
            (generation_point.1 - config.max_vertical_distance)
                .max(heights.min_y + config.padding_bottom),
            z - config.max_horizontal_distance,
        ),
        max: (
            x + config.max_horizontal_distance,
            (generation_point.1 + config.max_vertical_distance)
                .min(heights.max_y - config.padding_top),
            z + config.max_horizontal_distance,
        ),
    };
    let mut state = Assembler {
        assets,
        config,
        heights,
        aliases,
        random,
        pieces: &mut start.pieces,
        spaces: vec![FreeSpace {
            allowed,
            occupied: vec![bounds],
        }],
        queue: BinaryHeap::new(),
        sequence: 0,
    };
    state.expand(0, 0)?;
    while let Some((_, _, piece, space)) = state.queue.pop() {
        state.expand(piece, space)?;
    }
    Ok(Some(start))
}

fn resolve_alias<'a>(name: &'a str, aliases: &'a BTreeMap<String, String>) -> &'a str {
    aliases.get(name).map_or(name, String::as_str)
}

struct FreeSpace {
    allowed: BoundingBox,
    occupied: Vec<BoundingBox>,
}

impl FreeSpace {
    fn accepts(&self, bounds: BoundingBox) -> bool {
        // All boxes lie on the integer block lattice. Native tests the candidate
        // AABB deflated by 0.25 against allowed-minus-occupied voxel space. For
        // lattice-aligned boxes this is exactly inclusive containment/non-overlap.
        self.allowed.contains_box(bounds) && self.occupied.iter().all(|b| !b.intersects(bounds))
    }
}

struct Assembler<'a, 'h, R> {
    assets: &'a StructureAssets,
    config: &'a JigsawConfig,
    heights: &'a HeightContext<'h>,
    aliases: &'a BTreeMap<String, String>,
    random: &'a mut R,
    pieces: &'a mut Vec<JigsawPiece>,
    spaces: Vec<FreeSpace>,
    queue: BinaryHeap<(i32, Reverse<u64>, usize, usize)>,
    sequence: u64,
}

impl<R: TemplateRandom> Assembler<'_, '_, R> {
    fn expand(&mut self, parent_index: usize, inherited_space: usize) -> Result<()> {
        let parent = self.pieces[parent_index].clone();
        let mut internal_space = None;
        let connectors = parent.element.shuffled_jigsaws(
            self.assets,
            parent.origin,
            parent.rotation,
            self.random,
        )?;
        for connector in connectors {
            let adjacent = add(connector.pos, connector.front.step());
            let name = resolve_alias(&connector.pool, self.aliases);
            let pool = self.assets.pool(name)?;
            if pool.weight() == 0 && name != "minecraft:empty" {
                continue;
            }
            let fallback = self.assets.pool(&pool.fallback)?;
            if fallback.weight() == 0 && pool.fallback != "minecraft:empty" {
                continue;
            }
            let space = if parent.bounds.contains(adjacent) {
                *internal_space.get_or_insert_with(|| {
                    let index = self.spaces.len();
                    self.spaces.push(FreeSpace {
                        allowed: parent.bounds,
                        occupied: Vec::new(),
                    });
                    index
                })
            } else {
                inherited_space
            };
            let mut candidates = if parent.depth == self.config.max_depth {
                Vec::new()
            } else {
                pool.shuffled_elements(self.random)
            };
            candidates.extend(fallback.shuffled_elements(self.random));
            let mut surface = None;
            'candidates: for candidate in candidates {
                if matches!(candidate, PoolElement::Empty) {
                    break;
                }
                let mut rotations = Rotation::ALL;
                shuffle(&mut rotations, self.random);
                for rotation in rotations {
                    let targets = candidate.shuffled_jigsaws(
                        self.assets,
                        (0, 0, 0),
                        rotation,
                        self.random,
                    )?;
                    let zero_bounds = candidate.bounding_box(self.assets, (0, 0, 0), rotation)?;
                    let expansion = self.expansion_height(&targets, zero_bounds)?;
                    for target in &targets {
                        if !connector.can_attach(target) {
                            continue;
                        }
                        let mut child = self.align(
                            &parent,
                            &connector,
                            candidate,
                            target,
                            rotation,
                            &mut surface,
                        )?;
                        if expansion > 0 {
                            child.bounds.max.1 = child.bounds.min.1
                                + (expansion + 1).max(child.bounds.max.1 - child.bounds.min.1);
                        }
                        if !self.spaces[space].accepts(child.bounds) {
                            continue;
                        }
                        self.spaces[space].occupied.push(child.bounds);
                        self.connect(
                            parent_index,
                            &parent,
                            &connector,
                            target,
                            &mut child,
                            &mut surface,
                        );
                        let index = self.pieces.len();
                        if child.depth <= self.config.max_depth {
                            self.queue.push((
                                connector.placement_priority,
                                Reverse(self.sequence),
                                index,
                                space,
                            ));
                            self.sequence += 1;
                        }
                        self.pieces.push(child);
                        break 'candidates;
                    }
                }
            }
        }
        Ok(())
    }

    fn expansion_height(&self, connectors: &[JigsawConnector], bounds: BoundingBox) -> Result<i32> {
        if !self.config.use_expansion_hack || bounds.y_span() > 16 {
            return Ok(0);
        }
        let mut result = 0;
        for connector in connectors {
            if !bounds.contains(add(connector.pos, connector.front.step())) {
                continue;
            }
            let name = resolve_alias(&connector.pool, self.aliases);
            // Native treats missing pools as height zero here. Actual expansion
            // attempts still report a missing asset at the lookup boundary.
            let Some(pool) = self.assets.pools.get(name) else {
                continue;
            };
            result = result.max(pool.max_size(self.assets)?);
            if let Some(fallback) = self.assets.pools.get(&pool.fallback) {
                result = result.max(fallback.max_size(self.assets)?);
            }
        }
        Ok(result)
    }

    fn surface_height(&self, connector: &JigsawConnector, cached: &mut Option<i32>) -> i32 {
        *cached.get_or_insert_with(|| {
            (self.heights.first_free)(
                FeatureHeightmap::WorldSurfaceWg,
                connector.pos.0,
                connector.pos.2,
            )
        })
    }

    fn align(
        &self,
        parent: &JigsawPiece,
        source: &JigsawConnector,
        element: &PoolElement,
        target: &JigsawConnector,
        rotation: Rotation,
        surface: &mut Option<i32>,
    ) -> Result<JigsawPiece> {
        let mut origin = sub(add(source.pos, source.front.step()), target.pos);
        let mut bounds = element.bounding_box(self.assets, origin, rotation)?;
        let source_local_y = source.pos.1 - parent.bounds.min.1;
        let delta_y = source_local_y - target.pos.1 + source.front.step().1;
        let rigid = element.projection() == Projection::Rigid;
        let min_y = if parent.projection() == Projection::Rigid && rigid {
            parent.bounds.min.1 + delta_y
        } else {
            self.surface_height(source, surface) - target.pos.1
        };
        let shift = (0, min_y - bounds.min.1, 0);
        origin = add(origin, shift);
        bounds = bounds.moved(shift);
        Ok(JigsawPiece {
            element: element.clone(),
            origin,
            rotation,
            bounds,
            ground_level_delta: if rigid {
                parent.ground_level_delta - delta_y
            } else {
                element.ground_level_delta()
            },
            depth: parent.depth + 1,
            junctions: Vec::new(),
            waterlogging: self.config.waterlogging,
        })
    }

    fn connect(
        &mut self,
        parent_index: usize,
        parent: &JigsawPiece,
        source: &JigsawConnector,
        target: &JigsawConnector,
        child: &mut JigsawPiece,
        surface: &mut Option<i32>,
    ) {
        let source_local_y = source.pos.1 - parent.bounds.min.1;
        let target_local_y = target.pos.1;
        let delta_y = source_local_y - target_local_y + source.front.step().1;
        let junction_y = if parent.projection() == Projection::Rigid {
            source.pos.1
        } else if child.projection() == Projection::Rigid {
            child.bounds.min.1 + target_local_y
        } else {
            self.surface_height(source, surface) + delta_y / 2
        };
        let adjacent = add(source.pos, source.front.step());
        self.pieces[parent_index].junctions.push(Junction {
            source: (
                adjacent.0,
                junction_y - source_local_y + parent.ground_level_delta,
                adjacent.2,
            ),
            delta_y,
            destination_projection: child.projection(),
        });
        child.junctions.push(Junction {
            source: (
                source.pos.0,
                junction_y - target_local_y + child.ground_level_delta,
                source.pos.2,
            ),
            delta_y: -delta_y,
            destination_projection: parent.projection(),
        });
    }
}

/// Place a complete template-only start. A start requiring a placed feature is
/// rejected before any writes; use the callback entry point for those starts.
pub fn place_in_chunk<W: FeatureWorld + ?Sized, R: TemplateRandom + ?Sized>(
    assets: &StructureAssets,
    start: &JigsawStart,
    world: &mut W,
    random: &mut R,
    clip: BoundingBox,
    world_seed: i64,
) -> Result<PlacementResult> {
    if start
        .pieces
        .iter()
        .any(|p| p.bounds.intersects(clip) && p.element.contains_features())
    {
        return Err(FeatureError::Unsupported(
            "jigsaw start contains placed-feature elements; use place_in_chunk_with_features"
                .into(),
        ));
    }
    place_in_chunk_with_features(
        assets,
        start,
        world,
        random,
        clip,
        world_seed,
        &mut |name, _, _, _| {
            Err(FeatureError::Unsupported(format!(
                "pool placed feature {name}"
            )))
        },
    )
}

pub fn place_in_chunk_with_features<W, R, F>(
    assets: &StructureAssets,
    start: &JigsawStart,
    world: &mut W,
    random: &mut R,
    clip: BoundingBox,
    world_seed: i64,
    feature: &mut F,
) -> Result<PlacementResult>
where
    W: FeatureWorld + ?Sized,
    R: TemplateRandom + ?Sized,
    F: FnMut(&str, &mut W, &mut R, Pos) -> Result<PlacementResult>,
{
    place_in_chunk_with_callbacks(
        assets,
        start,
        world,
        random,
        clip,
        world_seed,
        feature,
        &mut |_, _| Ok(()),
    )
}

pub fn place_in_chunk_with_callbacks<W, R, F, E>(
    assets: &StructureAssets,
    start: &JigsawStart,
    world: &mut W,
    random: &mut R,
    clip: BoundingBox,
    world_seed: i64,
    feature: &mut F,
    effects: &mut E,
) -> Result<PlacementResult>
where
    W: FeatureWorld + ?Sized,
    R: TemplateRandom + ?Sized,
    F: FnMut(&str, &mut W, &mut R, Pos) -> Result<PlacementResult>,
    E: FnMut(&mut W, TemplateEffect) -> Result<()>,
{
    let mut result = PlacementResult::default();
    let Some(reference) = start.reference_position() else {
        return Ok(result);
    };
    for piece in &start.pieces {
        if !piece.bounds.intersects(clip) {
            continue;
        }
        result.merge(place_element_with_effects(
            assets,
            &piece.element,
            world,
            random,
            PoolPlacement {
                origin: piece.origin,
                reference,
                rotation: piece.rotation,
                clip,
                world_seed,
                waterlogging: piece.waterlogging,
                keep_jigsaws: false,
            },
            feature,
            effects,
        )?);
    }
    Ok(result)
}

/// Complete ancient-city block path: templates, degradation, containers, and
/// actual sculk patch elements. Entity spawn/finalization requests are returned
/// to the owning world, as FeatureWorld has no entity-spawn method.
pub fn place_ancient_city_in_chunk<W: FeatureWorld + ?Sized>(
    assets: &StructureAssets,
    start: &JigsawStart,
    world: &mut W,
    random: &mut crate::simplex::WorldgenRandom,
    clip: BoundingBox,
    world_seed: i64,
) -> Result<PlacementResult> {
    let feature = assets
        .placed_features
        .get("minecraft:sculk_patch_ancient_city")
        .ok_or_else(|| missing("ancient-city sculk placed feature"))?;
    if !feature["placement"].as_array().is_some_and(Vec::is_empty) {
        return Err(invalid("ancient-city sculk placement modifiers changed"));
    }
    let configured = text(feature, "feature")?;
    place_in_chunk_with_features(
        assets,
        start,
        world,
        random,
        clip,
        world_seed,
        &mut |name, world, random, origin| {
            if name != "minecraft:sculk_patch_ancient_city" {
                return Err(FeatureError::Unsupported(format!(
                    "ancient-city pool feature {name}"
                )));
            }
            let mut effects = FeatureEffects {
                world,
                registry: &assets.blocks,
                result: PlacementResult::default(),
                error: None,
            };
            let placed = crate::sculk::place_named(&mut effects, random, origin, configured)?;
            if let Some(error) = effects.error {
                return Err(error);
            }
            effects.result.placed = placed;
            Ok(effects.result)
        },
    )
}

struct FeatureEffects<'a, W: ?Sized> {
    world: &'a mut W,
    registry: &'a super::template::BlockRegistry,
    result: PlacementResult,
    error: Option<FeatureError>,
}

impl<W: FeatureWorld + ?Sized> crate::ore::OreWorld for FeatureEffects<'_, W> {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.world.ocean_floor_wg(x, z)
    }
    fn get_block(&self, pos: Pos) -> Option<u32> {
        self.world.get_block(pos)
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.set_feature_block(pos, state, 2)
    }
}

impl<W: FeatureWorld + ?Sized> FeatureWorld for FeatureEffects<'_, W> {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.world.feature_biome(pos)
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.world.feature_height(kind, x, z)
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        self.world.can_write_feature(pos)
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        self.world.mark_feature_postprocessing(pos);
    }
    fn schedule_feature_tick(&mut self, request: crate::tick_request::TickRequest) -> bool {
        self.world.schedule_feature_tick(request)
    }

    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        if self.error.is_some() {
            return false;
        }
        let metadata = (|| -> Result<_> {
            let previous = self
                .world
                .get_block(pos)
                .ok_or_else(|| missing(format!("pool-feature effect read {pos:?}")))?;
            let changed_type =
                self.registry.state(previous)?.name != self.registry.state(state)?.name;
            Ok((
                changed_type && self.registry.flags(previous)? & 8 != 0,
                if changed_type {
                    self.registry.default_block_entity(state, pos)?
                } else {
                    None
                },
            ))
        })();
        let (clear, entity) = match metadata {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return false;
            }
        };
        if !self.world.set_feature_block(pos, state, flags) {
            return false;
        }
        self.result.blocks_written += 1;
        if clear {
            self.result.cleared_block_entities.insert(pos);
            self.result.block_entities.remove(&pos);
        }
        if let Some(entity) = entity {
            self.result.block_entities.insert(pos, entity);
        } else if let Some(entity) = self.result.block_entities.get_mut(&pos) {
            entity.state = state;
        }
        true
    }
}
