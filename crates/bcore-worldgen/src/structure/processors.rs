//! Native template processing order and position-seeded rule evaluation.
//! All processors used by the bundled ancient-city and five village pool sets
//! are supported. Unknown codecs fail explicitly at the asset/config boundary.

use super::template::{
    add, id, invalid, missing, transform_block, BlockInfo, BlockRegistry, BlockState, Nbt,
    PlacementSettings, ProcessorRandom, Result, TemplateRandom,
};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::random::get_seed;
use crate::simplex::JavaRandom;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub type BlockTags = BTreeMap<String, BTreeSet<String>>;

#[derive(Debug, Clone)]
pub enum StatePredicate {
    Always,
    Block(String),
    State(u32),
    Tag(BTreeSet<String>),
    RandomBlock { block: String, probability: f32 },
    RandomState { state: u32, probability: f32 },
}

impl StatePredicate {
    pub fn test(
        &self,
        state: u32,
        registry: &BlockRegistry,
        random: &mut dyn TemplateRandom,
    ) -> Result<bool> {
        let name = &registry.state(state)?.name;
        Ok(match self {
            Self::Always => true,
            Self::Block(block) => name == block,
            Self::State(expected) => state == *expected,
            Self::Tag(blocks) => blocks.contains(name),
            Self::RandomBlock { block, probability } => {
                name == block && random.next_float() < *probability
            }
            Self::RandomState {
                state: expected,
                probability,
            } => state == *expected && random.next_float() < *probability,
        })
    }

    fn from_json(value: &Value, registry: &BlockRegistry, tags: &BlockTags) -> Result<Self> {
        Ok(
            match text(value, "predicate_type")?.trim_start_matches("minecraft:") {
                "always_true" => Self::Always,
                "block_match" => Self::Block(id(text(value, "block")?)),
                "blockstate_match" => Self::State(resolve_state(&value["block_state"], registry)?),
                "tag_match" => Self::Tag(tag(tags, text(value, "tag")?)?.clone()),
                "random_block_match" => Self::RandomBlock {
                    block: id(text(value, "block")?),
                    probability: probability(value, "probability", None)?,
                },
                "random_blockstate_match" => Self::RandomState {
                    state: resolve_state(&value["block_state"], registry)?,
                    probability: probability(value, "probability", None)?,
                },
                other => {
                    return Err(FeatureError::Unsupported(format!(
                        "structure rule predicate {other}"
                    )))
                }
            },
        )
    }
}

#[derive(Debug, Clone)]
pub enum PositionPredicate {
    Always,
    Linear {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
        axis: Option<usize>,
    },
}

impl PositionPredicate {
    fn test(&self, pos: Pos, reference: Pos, random: &mut dyn TemplateRandom) -> bool {
        match self {
            Self::Always => true,
            Self::Linear {
                min_chance,
                max_chance,
                min_dist,
                max_dist,
                axis,
            } => {
                let distances = [
                    pos.0.wrapping_sub(reference.0).wrapping_abs(),
                    pos.1.wrapping_sub(reference.1).wrapping_abs(),
                    pos.2.wrapping_sub(reference.2).wrapping_abs(),
                ];
                let distance = match axis {
                    Some(i) => distances[*i],
                    None => distances[0]
                        .wrapping_add(distances[1])
                        .wrapping_add(distances[2]),
                };
                let t = ((distance as f32 - *min_dist as f32)
                    / (*max_dist as f32 - *min_dist as f32))
                    .clamp(0.0, 1.0);
                random.next_float() <= *min_chance + t * (*max_chance - *min_chance)
            }
        }
    }

    fn from_json(value: &Value) -> Result<Self> {
        if value.is_null() {
            return Ok(Self::Always);
        }
        let kind = text(value, "predicate_type")?.trim_start_matches("minecraft:");
        if kind == "always_true" {
            return Ok(Self::Always);
        }
        if !matches!(kind, "linear_pos" | "axis_aligned_linear_pos") {
            return Err(FeatureError::Unsupported(format!(
                "structure position predicate {kind}"
            )));
        }
        let min_dist = integer(value, "min_dist", Some(0))?;
        let max_dist = integer(value, "max_dist", Some(0))?;
        if min_dist >= max_dist {
            return Err(invalid("position predicate needs min_dist < max_dist"));
        }
        let axis = if kind == "axis_aligned_linear_pos" {
            Some(match value["axis"].as_str().unwrap_or("y") {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                other => return Err(invalid(format!("axis {other}"))),
            })
        } else {
            None
        };
        Ok(Self::Linear {
            min_chance: probability(value, "min_chance", Some(0.0))?,
            max_chance: probability(value, "max_chance", Some(0.0))?,
            min_dist,
            max_dist,
            axis,
        })
    }
}

#[derive(Debug, Clone)]
pub enum BlockEntityModifier {
    Passthrough,
    Clear,
    AppendStatic(Nbt),
    AppendLoot(String),
}

impl BlockEntityModifier {
    fn apply(&self, nbt: Option<Nbt>, random: &mut dyn TemplateRandom) -> Result<Option<Nbt>> {
        match self {
            Self::Passthrough => Ok(nbt),
            Self::Clear => Ok(None),
            Self::AppendStatic(addition) => {
                let mut out = nbt.unwrap_or_else(Nbt::empty_compound);
                merge_compounds(out.compound_mut()?, addition.compound()?);
                Ok(Some(out))
            }
            Self::AppendLoot(table) => {
                let mut out = nbt.unwrap_or_else(Nbt::empty_compound);
                let compound = out.compound_mut()?;
                compound.insert("LootTable".into(), Nbt::String(table.clone()));
                compound.insert("LootTableSeed".into(), Nbt::Long(random.next_long()));
                Ok(Some(out))
            }
        }
    }

    fn from_json(value: &Value) -> Result<Self> {
        if value.is_null() {
            return Ok(Self::Passthrough);
        }
        Ok(
            match text(value, "type")?.trim_start_matches("minecraft:") {
                "passthrough" => Self::Passthrough,
                "clear" => Self::Clear,
                "append_loot" => Self::AppendLoot(id(text(value, "loot_table")?)),
                // Typed NBT is accepted for custom inputs; bundled village/city
                // rules only need passthrough. Untyped static NBT is rejected.
                "append_static" => Self::AppendStatic(
                    serde_json::from_value(value["data"].clone())
                        .map_err(|e| invalid(format!("typed append_static NBT: {e}")))?,
                ),
                other => {
                    return Err(FeatureError::Unsupported(format!(
                        "structure block entity modifier {other}"
                    )))
                }
            },
        )
    }
}

fn merge_compounds(target: &mut BTreeMap<String, Nbt>, source: &BTreeMap<String, Nbt>) {
    for (key, value) in source {
        if let (Some(Nbt::Compound(old)), Nbt::Compound(new)) = (target.get_mut(key), value) {
            merge_compounds(old, new);
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub input: StatePredicate,
    pub location: StatePredicate,
    pub position: PositionPredicate,
    pub output: u32,
    pub block_entity: BlockEntityModifier,
}

#[derive(Debug, Clone)]
pub enum IntProvider {
    Constant(i32),
    Uniform { min: i32, max: i32 },
}

impl IntProvider {
    fn sample(&self, random: &mut dyn TemplateRandom) -> i32 {
        match self {
            Self::Constant(value) => *value,
            Self::Uniform { min, max } => *min + random.next_int((max - min + 1) as usize) as i32,
        }
    }

    fn max(&self) -> i32 {
        match self {
            Self::Constant(value) => *value,
            Self::Uniform { max, .. } => *max,
        }
    }

    fn from_json(value: &Value) -> Result<Self> {
        if let Some(number) = value.as_i64() {
            let n = i32::try_from(number).map_err(|_| invalid("processor limit overflow"))?;
            if n < 0 {
                return Err(invalid("negative processor limit"));
            }
            return Ok(Self::Constant(n));
        }
        if text(value, "type")? != "minecraft:uniform" {
            return Err(FeatureError::Unsupported("processor limit provider".into()));
        }
        let min = integer(value, "min_inclusive", None)?;
        let max = integer(value, "max_inclusive", None)?;
        if min < 0 || min > max || i64::from(max) - i64::from(min) >= i64::from(i32::MAX) {
            return Err(invalid("invalid processor limit range"));
        }
        Ok(Self::Uniform { min, max })
    }
}

#[derive(Debug, Clone)]
pub enum Processor {
    Nop,
    Ignore(BTreeSet<String>),
    BlockRot {
        integrity: f32,
        rottable: Option<BTreeSet<String>>,
    },
    BlockAge {
        mossiness: f32,
        stairs: BTreeSet<String>,
        slabs: BTreeSet<String>,
        walls: BTreeSet<String>,
    },
    BlackstoneReplace,
    Rules(Vec<Rule>),
    Protected(BTreeSet<String>),
    Gravity {
        heightmap: FeatureHeightmap,
        offset: i32,
    },
    JigsawReplacement,
    LavaSubmerged,
    Capped {
        delegate: Box<Processor>,
        limit: IntProvider,
    },
}

impl Processor {
    pub fn ignore_structure() -> Self {
        Self::Ignore(BTreeSet::from(["minecraft:structure_block".into()]))
    }

    pub fn ignore_structure_and_air() -> Self {
        Self::Ignore(BTreeSet::from([
            "minecraft:structure_block".into(),
            "minecraft:air".into(),
        ]))
    }

    pub fn from_json(value: &Value, registry: &BlockRegistry, tags: &BlockTags) -> Result<Self> {
        Ok(
            match text(value, "processor_type")?.trim_start_matches("minecraft:") {
                "nop" => Self::Nop,
                "block_ignore" => Self::Ignore(
                    value["blocks"]
                        .as_array()
                        .ok_or_else(|| invalid("block_ignore blocks"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(id)
                                .ok_or_else(|| invalid("block_ignore block"))
                        })
                        .collect::<Result<_>>()?,
                ),
                "block_rot" => Self::BlockRot {
                    integrity: probability(value, "integrity", None)?,
                    rottable: if value["rottable_blocks"].is_null() {
                        None
                    } else {
                        Some(block_set(&value["rottable_blocks"], tags)?)
                    },
                },
                "block_age" => {
                    // The native codec is FLOAT, not a bounded probability codec.
                    let mossiness = value["mossiness"]
                        .as_f64()
                        .ok_or_else(|| invalid("block_age mossiness"))?
                        as f32;
                    if !mossiness.is_finite() {
                        return Err(invalid("nonfinite block_age mossiness"));
                    }
                    Self::BlockAge {
                        mossiness,
                        stairs: tag(tags, "stairs")?.clone(),
                        slabs: tag(tags, "slabs")?.clone(),
                        walls: tag(tags, "walls")?.clone(),
                    }
                }
                "blackstone_replace" => Self::BlackstoneReplace,
                "protected_blocks" => Self::Protected(tag(tags, text(value, "value")?)?.clone()),
                "gravity" => Self::Gravity {
                    heightmap: heightmap(
                        value["heightmap"].as_str().unwrap_or("WORLD_SURFACE_WG"),
                    )?,
                    offset: integer(value, "offset", Some(0))?,
                },
                "jigsaw_replacement" => Self::JigsawReplacement,
                "lava_submerged_block" => Self::LavaSubmerged,
                "capped" => Self::Capped {
                    delegate: Box::new(Self::from_json(&value["delegate"], registry, tags)?),
                    limit: IntProvider::from_json(&value["limit"])?,
                },
                "rule" => {
                    let rules = value["rules"]
                        .as_array()
                        .ok_or_else(|| invalid("structure rule list"))?;
                    Self::Rules(
                        rules
                            .iter()
                            .map(|v| {
                                Ok(Rule {
                                    input: StatePredicate::from_json(
                                        &v["input_predicate"],
                                        registry,
                                        tags,
                                    )?,
                                    location: StatePredicate::from_json(
                                        &v["location_predicate"],
                                        registry,
                                        tags,
                                    )?,
                                    position: PositionPredicate::from_json(
                                        &v["position_predicate"],
                                    )?,
                                    output: resolve_state(&v["output_state"], registry)?,
                                    block_entity: BlockEntityModifier::from_json(
                                        &v["block_entity_modifier"],
                                    )?,
                                })
                            })
                            .collect::<Result<_>>()?,
                    )
                }
                other => {
                    return Err(FeatureError::Unsupported(format!(
                        "structure processor {other}"
                    )))
                }
            },
        )
    }

    fn process<W: FeatureWorld + ?Sized>(
        &self,
        world: &W,
        registry: &BlockRegistry,
        reference: Pos,
        original: &BlockInfo,
        mut info: BlockInfo,
        random: &mut ProcessorRandom<'_>,
    ) -> Result<Option<BlockInfo>> {
        match self {
            Self::Nop | Self::Capped { .. } => {}
            Self::Ignore(blocks) => {
                if blocks.contains(&registry.state(info.state)?.name) {
                    return Ok(None);
                }
            }
            Self::BlockRot {
                integrity,
                rottable,
            } => {
                if rottable.as_ref().is_none_or(|blocks| {
                    blocks.contains(
                        &registry
                            .state(original.state)
                            .expect("validated palette")
                            .name,
                    )
                }) && random.at(info.pos, |r| r.next_float()) > *integrity
                {
                    return Ok(None);
                }
            }
            Self::BlockAge {
                mossiness,
                stairs,
                slabs,
                walls,
            } => {
                let name = &registry.state(info.state)?.name;
                info.state = random.at(info.pos, |random| {
                    age_state(
                        registry,
                        info.state,
                        name,
                        *mossiness,
                        stairs.contains(name),
                        slabs.contains(name),
                        walls.contains(name),
                        random,
                    )
                })?;
            }
            Self::BlackstoneReplace => {
                info.state = blackstone_state(registry, info.state)?;
            }
            Self::Protected(blocks) => {
                let current = read(world, info.pos)?;
                if blocks.contains(&registry.state(current)?.name) {
                    return Ok(None);
                }
            }
            Self::Gravity { heightmap, offset } => {
                info.pos.1 = world
                    .feature_height(*heightmap, info.pos.0, info.pos.2)
                    .wrapping_add(*offset)
                    .wrapping_add(original.pos.1);
            }
            Self::Rules(rules) => {
                // RuleProcessor always constructs its own positional Legacy RNG;
                // it does not use StructurePlaceSettings.random.
                let mut rule_random = JavaRandom::new(get_seed(info.pos.0, info.pos.1, info.pos.2));
                let current = read(world, info.pos)?;
                for rule in rules {
                    if rule.input.test(info.state, registry, &mut rule_random)?
                        && rule.location.test(current, registry, &mut rule_random)?
                        && rule.position.test(info.pos, reference, &mut rule_random)
                    {
                        info.state = rule.output;
                        info.nbt = rule.block_entity.apply(info.nbt, &mut rule_random)?;
                        break;
                    }
                }
            }
            Self::JigsawReplacement => {
                if registry.state(info.state)?.name == "minecraft:jigsaw" {
                    let final_state = info
                        .nbt
                        .as_ref()
                        .and_then(|n| n.get("final_state"))
                        .and_then(Nbt::string)
                        .ok_or_else(|| invalid("jigsaw without final_state"))?;
                    info.state = registry.parse_state(final_state)?;
                    if registry.state(info.state)?.name == "minecraft:structure_void" {
                        return Ok(None);
                    }
                    info.nbt = None;
                }
            }
            Self::LavaSubmerged => {
                if registry.state(read(world, info.pos)?)?.name == "minecraft:lava"
                    && registry.flags(info.state)? & 32 == 0
                {
                    info.state = registry.default_state("lava")?;
                }
            }
        }
        Ok(Some(info))
    }

    fn finalize<W: FeatureWorld + ?Sized>(
        &self,
        world: &W,
        registry: &BlockRegistry,
        origin: Pos,
        reference: Pos,
        originals: &[&BlockInfo],
        infos: &mut [BlockInfo],
        settings: &PlacementSettings,
        random: &mut ProcessorRandom<'_>,
    ) -> Result<()> {
        let Self::Capped { delegate, limit } = self else {
            return Ok(());
        };
        if limit.max() == 0 || infos.is_empty() || infos.len() != originals.len() {
            return Ok(());
        }
        let positional_seed = JavaRandom::new(settings.world_seed).next_long();
        let mut capped_random =
            JavaRandom::new(positional_seed ^ get_seed(origin.0, origin.1, origin.2));
        let count = (limit.sample(&mut capped_random) as usize).min(infos.len());
        if count == 0 {
            return Ok(());
        }
        let mut order: Vec<_> = (0..infos.len()).collect();
        super::template::shuffle(&mut order, &mut capped_random);
        let mut changed = 0;
        for index in order {
            if changed == count {
                break;
            }
            if let Some(replacement) = delegate.process(
                world,
                registry,
                reference,
                originals[index],
                infos[index].clone(),
                random,
            )? {
                if replacement != infos[index] {
                    infos[index] = replacement;
                    changed += 1;
                }
            }
        }
        Ok(())
    }
}

fn properties_of(
    registry: &BlockRegistry,
    block: &str,
    original: u32,
    selected: Option<&[&str]>,
) -> Result<u32> {
    let original = registry.state(original)?;
    let mut state = registry.state(registry.default_state(block)?)?.clone();
    for (key, value) in &original.properties {
        if state.properties.contains_key(key)
            && selected.is_none_or(|keys| keys.contains(&key.as_str()))
        {
            state.properties.insert(key.clone(), value.clone());
        }
    }
    registry.resolve(&state)
}

fn random_stairs(
    registry: &BlockRegistry,
    name: &str,
    random: &mut dyn TemplateRandom,
) -> Result<u32> {
    let state = registry.default_state(name)?;
    let state = registry.with_property(
        state,
        "facing",
        ["north", "east", "south", "west"][random.next_int(4)],
    )?;
    registry.with_property(state, "half", ["top", "bottom"][random.next_int(2)])
}

fn age_state(
    registry: &BlockRegistry,
    state: u32,
    name: &str,
    mossiness: f32,
    stairs: bool,
    slabs: bool,
    walls: bool,
    random: &mut dyn TemplateRandom,
) -> Result<u32> {
    if matches!(
        name,
        "minecraft:stone" | "minecraft:stone_bricks" | "minecraft:chiseled_stone_bricks"
    ) {
        if random.next_float() >= 0.5 {
            return Ok(state);
        }
        // Both arrays are evaluated before choosing mossy/non-mossy. Skipping the
        // unused stair orientation would consume four fewer native draws.
        let dry = [
            registry.default_state("cracked_stone_bricks")?,
            random_stairs(registry, "stone_brick_stairs", random)?,
        ];
        let mossy = [
            registry.default_state("mossy_stone_bricks")?,
            random_stairs(registry, "mossy_stone_brick_stairs", random)?,
        ];
        let choices = if random.next_float() < mossiness {
            mossy
        } else {
            dry
        };
        return Ok(choices[random.next_int(2)]);
    }
    if stairs {
        if random.next_float() >= 0.5 {
            return Ok(state);
        }
        let mossy = [
            properties_of(registry, "mossy_stone_brick_stairs", state, None)?,
            registry.default_state("mossy_stone_brick_slab")?,
        ];
        let dry = [
            registry.default_state("stone_slab")?,
            registry.default_state("stone_brick_slab")?,
        ];
        return Ok((if random.next_float() < mossiness {
            mossy
        } else {
            dry
        })[random.next_int(2)]);
    }
    if slabs || walls {
        if random.next_float() < mossiness {
            return properties_of(
                registry,
                if slabs {
                    "mossy_stone_brick_slab"
                } else {
                    "mossy_stone_brick_wall"
                },
                state,
                None,
            );
        }
    } else if name == "minecraft:obsidian" && random.next_float() < 0.15 {
        return registry.default_state("crying_obsidian");
    }
    Ok(state)
}

fn blackstone_state(registry: &BlockRegistry, state: u32) -> Result<u32> {
    let name = registry.state(state)?.name.trim_start_matches("minecraft:");
    let replacement = match name {
        "cobblestone" | "mossy_cobblestone" => "blackstone",
        "stone" => "polished_blackstone",
        "stone_bricks" | "mossy_stone_bricks" => "polished_blackstone_bricks",
        "cobblestone_stairs" | "mossy_cobblestone_stairs" => "blackstone_stairs",
        "stone_stairs" => "polished_blackstone_stairs",
        "stone_brick_stairs" | "mossy_stone_brick_stairs" => "polished_blackstone_brick_stairs",
        "cobblestone_slab" | "mossy_cobblestone_slab" => "blackstone_slab",
        "smooth_stone_slab" | "stone_slab" => "polished_blackstone_slab",
        "stone_brick_slab" | "mossy_stone_brick_slab" => "polished_blackstone_brick_slab",
        "stone_brick_wall" | "mossy_stone_brick_wall" => "polished_blackstone_brick_wall",
        "cobblestone_wall" | "mossy_cobblestone_wall" => "blackstone_wall",
        "chiseled_stone_bricks" => "chiseled_polished_blackstone",
        "cracked_stone_bricks" => "cracked_polished_blackstone_bricks",
        "iron_bars" => "iron_chain",
        _ => return Ok(state),
    };
    // Native deliberately resets shape/waterlogged/wall connections. Only these
    // three property objects are explicitly copied by BlackstoneReplaceProcessor.
    properties_of(
        registry,
        replacement,
        state,
        Some(&["facing", "half", "type"]),
    )
}

pub fn process_block_infos<W: FeatureWorld + ?Sized>(
    world: &W,
    registry: &BlockRegistry,
    origin: Pos,
    reference: Pos,
    blocks: &[BlockInfo],
    settings: &PlacementSettings,
    random: &mut ProcessorRandom<'_>,
) -> Result<Vec<BlockInfo>> {
    let mut originals = Vec::with_capacity(blocks.len());
    let mut result = Vec::with_capacity(blocks.len());
    for original in blocks {
        let mut current = Some(BlockInfo {
            pos: add(
                transform_block(
                    original.pos,
                    settings.mirror,
                    settings.rotation,
                    settings.pivot,
                ),
                origin,
            ),
            state: original.state,
            nbt: original.nbt.clone(),
        });
        for processor in &settings.processors {
            let Some(info) = current else {
                break;
            };
            current = processor.process(world, registry, reference, original, info, random)?;
        }
        if let Some(info) = current {
            originals.push(original);
            result.push(info);
        }
    }
    for processor in &settings.processors {
        processor.finalize(
            world,
            registry,
            origin,
            reference,
            &originals,
            &mut result,
            settings,
            random,
        )?;
    }
    Ok(result)
}

fn read<W: FeatureWorld + ?Sized>(world: &W, pos: Pos) -> Result<u32> {
    world
        .get_block(pos)
        .ok_or_else(|| missing(format!("structure processor read {pos:?}")))
}

pub(crate) fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .ok_or_else(|| invalid(format!("missing/string field {field}")))
}

pub(crate) fn integer(value: &Value, field: &str, default: Option<i32>) -> Result<i32> {
    match value.get(field) {
        None => default.ok_or_else(|| invalid(format!("missing integer {field}"))),
        Some(v) => v
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| invalid(format!("invalid integer {field}"))),
    }
}

fn probability(value: &Value, field: &str, default: Option<f32>) -> Result<f32> {
    let p = match value.get(field) {
        None => default.ok_or_else(|| invalid(format!("missing {field}")))?,
        Some(v) => v
            .as_f64()
            .ok_or_else(|| invalid(format!("invalid {field}")))? as f32,
    };
    if !(0.0..=1.0).contains(&p) {
        return Err(invalid(format!("{field} outside [0, 1]")));
    }
    Ok(p)
}

fn tag<'a>(tags: &'a BlockTags, name: &str) -> Result<&'a BTreeSet<String>> {
    tags.get(&id(name.trim_start_matches('#')))
        .ok_or_else(|| missing(format!("structure block tag {name}")))
}

fn block_set(value: &Value, tags: &BlockTags) -> Result<BTreeSet<String>> {
    if let Some(name) = value.as_str() {
        return if name.starts_with('#') {
            Ok(tag(tags, name)?.clone())
        } else {
            Ok(BTreeSet::from([id(name)]))
        };
    }
    value
        .as_array()
        .ok_or_else(|| invalid("block holder set"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(id)
                .ok_or_else(|| invalid("block holder name"))
        })
        .collect()
}

fn resolve_state(value: &Value, registry: &BlockRegistry) -> Result<u32> {
    registry.resolve(
        &serde_json::from_value::<BlockState>(value.clone())
            .map_err(|e| invalid(format!("processor state: {e}")))?,
    )
}

pub(crate) fn heightmap(name: &str) -> Result<FeatureHeightmap> {
    Ok(match name {
        "WORLD_SURFACE_WG" => FeatureHeightmap::WorldSurfaceWg,
        "WORLD_SURFACE" => FeatureHeightmap::WorldSurface,
        "OCEAN_FLOOR_WG" => FeatureHeightmap::OceanFloorWg,
        "OCEAN_FLOOR" => FeatureHeightmap::OceanFloor,
        "MOTION_BLOCKING" => FeatureHeightmap::MotionBlocking,
        "MOTION_BLOCKING_NO_LEAVES" => FeatureHeightmap::MotionBlockingNoLeaves,
        other => return Err(invalid(format!("heightmap {other}"))),
    })
}
