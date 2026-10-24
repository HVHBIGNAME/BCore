//! Target definitions and state-property arithmetic for additional trees.
use super::fallen::Pos;
use super::standing::StandingTreeWorld;
use super::{
    ExtraFoliage, ExtraTrunk, FeatureSize, FoliagePlacer, IntProvider, TreeConfig, TreeRandom,
    TrunkPlacer,
};
use crate::block_predicate::{array, int, invalid, kind, string, FeatureResult};
use crate::feature_world::FeatureError;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
pub(super) struct Property {
    pub name: String,
    pub values: Vec<String>,
}
#[derive(Deserialize)]
pub(super) struct BlockDefinition {
    pub first: u32,
    pub end: u32,
    pub default: u32,
    pub properties: Vec<Property>,
    pub update_shape: String,
    pub can_survive: String,
    #[serde(default)]
    pub may_place_on: Option<Vec<[u32; 2]>>,
}
#[derive(Deserialize)]
pub(super) struct Catalog {
    pub blocks: BTreeMap<String, BlockDefinition>,
    pub tags: BTreeMap<String, Vec<[u32; 2]>>,
}

pub(super) fn catalog() -> &'static Catalog {
    static DATA: OnceLock<Catalog> = OnceLock::new();
    DATA.get_or_init(|| {
        crate::block_predicate::parse_catalog(include_str!(
            "../../data/extra_tree_catalog_26_2.json"
        ))
        .expect("native extra-tree catalog")
    })
}

pub(super) fn block(state: u32) -> &'static BlockDefinition {
    static LOOKUP: OnceLock<Vec<&'static BlockDefinition>> = OnceLock::new();
    let lookup = LOOKUP.get_or_init(|| {
        let mut blocks: Vec<_> = catalog().blocks.values().collect();
        blocks.sort_by_key(|b| b.first);
        let mut result = Vec::new();
        for block in blocks {
            assert_eq!(result.len(), block.first as usize);
            assert!(
                block.end > block.first && block.end as usize <= bcore_registry::BLOCK_STATE_COUNT
            );
            assert!((block.first..block.end).contains(&block.default));
            result.resize(block.end as usize, block);
        }
        assert_eq!(result.len(), bcore_registry::BLOCK_STATE_COUNT);
        result
    });
    lookup[state as usize]
}

pub(super) fn default_state(name: &str) -> u32 {
    catalog().blocks[name].default
}

pub(super) fn property(state: u32, name: &str) -> Option<&'static str> {
    let block = block(state);
    let mut index = (state - block.first) as usize;
    for property in block.properties.iter().rev() {
        let value = index % property.values.len();
        index /= property.values.len();
        if property.name == name {
            return Some(property.values[value].as_str());
        }
    }
    None
}

pub(super) fn with_property(state: u32, name: &str, value: &str) -> u32 {
    let block = block(state);
    let mut index = (state - block.first) as usize;
    let mut stride = 1;
    for property in block.properties.iter().rev() {
        let previous = index % property.values.len();
        if property.name == name {
            let next = property
                .values
                .iter()
                .position(|v| v == value)
                .expect("native property value");
            return (state as i64 + (next as i64 - previous as i64) * stride as i64) as u32;
        }
        index /= property.values.len();
        stride *= property.values.len();
    }
    panic!("native block has no property {name}")
}

pub(super) fn state(value: &Value) -> FeatureResult<u32> {
    crate::block_predicate::catalog().state(value)
}

pub(super) fn tagged(state: u32, name: &str) -> bool {
    let ranges = catalog().tags.get(name).expect("native block tag");
    let index = ranges.partition_point(|range| range[0] <= state);
    index > 0 && state < ranges[index - 1][1]
}

pub(super) fn tag_name(value: &Value) -> FeatureResult<String> {
    let name = string(value)?;
    let name = crate::block_predicate::qualified(name.strip_prefix('#').unwrap_or(name));
    if !catalog().tags.contains_key(name.as_ref()) {
        return Err(crate::block_predicate::missing(format!(
            "tree block tag {name}"
        )));
    }
    Ok(name.into_owned())
}

pub(super) fn number(value: &Value) -> FeatureResult<i32> {
    int(value)
}

pub(super) fn int_provider(value: &Value) -> FeatureResult<IntProvider> {
    if value.is_number() {
        return Ok(IntProvider::Constant(number(value)?));
    }
    match kind(value)? {
        "constant" => Ok(IntProvider::Constant(number(&value["value"])?)),
        "uniform" => {
            let min = number(&value["min_inclusive"])?;
            let max = number(&value["max_inclusive"])?;
            if min > max || i64::from(max) - i64::from(min) >= i64::from(i32::MAX) {
                return Err(invalid("tree integer provider bounds"));
            }
            Ok(IntProvider::Uniform { min, max })
        }
        other => Err(FeatureError::Unsupported(format!(
            "tree integer provider {other}"
        ))),
    }
}

pub(super) fn chance(value: &Value) -> FeatureResult<f32> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .map(|v| v as f32)
        .ok_or_else(|| invalid(format!("tree probability {value}")))
}

#[derive(Debug)]
pub(crate) enum Provider {
    Simple(u32),
    Weighted {
        choices: Vec<(u32, i32)>,
        total: i32,
    },
    Randomized {
        source: Box<Provider>,
        property: String,
        values: IntProvider,
    },
    Rules {
        fallback: Option<Box<Provider>>,
        rules: Vec<(Predicate, Provider)>,
    },
}

impl Provider {
    pub fn parse(value: &Value) -> FeatureResult<Self> {
        Ok(match kind(value)? {
            "simple_state_provider" => Self::Simple(state(&value["state"])?),
            "weighted_state_provider" => {
                let choices: Vec<_> = array(&value["entries"])?
                    .iter()
                    .map(|e| Ok((state(&e["data"])?, number(&e["weight"])?)))
                    .collect::<FeatureResult<_>>()?;
                let mut total = 0_i32;
                for &(_, weight) in &choices {
                    if weight < 0 {
                        return Err(invalid("negative tree provider weight"));
                    }
                    total = total
                        .checked_add(weight)
                        .ok_or_else(|| invalid("tree provider weights overflow"))?;
                }
                if total == 0 {
                    return Err(invalid("tree provider has no positive weight"));
                }
                Self::Weighted { choices, total }
            }
            "randomized_int_state_provider" => Self::Randomized {
                source: Box::new(Self::parse(&value["source"])?),
                property: string(&value["property"])?.to_owned(),
                values: int_provider(&value["values"])?,
            },
            "rule_based_state_provider" => Self::Rules {
                fallback: value
                    .get("fallback")
                    .map(Self::parse)
                    .transpose()?
                    .map(Box::new),
                rules: array(&value["rules"])?
                    .iter()
                    .map(|r| Ok((Predicate::parse(&r["if_true"])?, Self::parse(&r["then"])?)))
                    .collect::<FeatureResult<_>>()?,
            },
            other => {
                return Err(FeatureError::Unsupported(format!(
                    "tree state provider {other}"
                )))
            }
        })
    }

    pub fn first(&self) -> FeatureResult<u32> {
        match self {
            Self::Simple(s) => Ok(*s),
            Self::Weighted { choices, .. } => Ok(choices[0].0),
            _ => Err(FeatureError::Unsupported(
                "tree geometry requires a simple or weighted base state".into(),
            )),
        }
    }

    pub fn sample<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
        &self,
        world: &W,
        random: &mut R,
        pos: Pos,
    ) -> FeatureResult<Option<u32>> {
        Ok(match self {
            Self::Simple(state) => Some(*state),
            Self::Weighted { choices, total } => {
                let mut pick = random.next_i32_bounded(*total);
                for &(state, weight) in choices {
                    pick -= weight;
                    if pick < 0 {
                        return Ok(Some(state));
                    }
                }
                unreachable!("positive native weighted total")
            }
            Self::Randomized {
                source,
                property,
                values,
            } => {
                let state = source.sample_required(world, random, pos)?;
                let block = crate::block_predicate::catalog().block(state)?.1;
                let integer_property = block.properties.iter().any(|(name, choices)| {
                    name == property && choices.iter().all(|value| value.parse::<i32>().is_ok())
                });
                if !integer_property {
                    return Ok(Some(state));
                }
                Some(block.with_property(state, property, &values.sample(random).to_string())?)
            }
            Self::Rules { fallback, rules } => {
                for (condition, provider) in rules {
                    if condition.test(world, pos)? {
                        return provider.sample_required(world, random, pos).map(Some);
                    }
                }
                return fallback.as_ref().map_or(Ok(None), |provider| {
                    provider.sample_required(world, random, pos).map(Some)
                });
            }
        })
    }

    pub fn sample_required<W: StandingTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
        &self,
        world: &W,
        random: &mut R,
        pos: Pos,
    ) -> FeatureResult<u32> {
        Ok(self
            .sample(world, random, pos)?
            .unwrap_or_else(|| world.get_block(pos)))
    }
}

#[derive(Debug)]
pub(crate) enum Predicate {
    True,
    Tag(String, Pos),
    Blocks(Vec<(u32, u32)>, Pos),
    Biomes(std::collections::BTreeSet<u32>),
    Not(Box<Self>),
    All(Vec<Self>),
    Any(Vec<Self>),
}
impl Predicate {
    pub fn parse(value: &Value) -> FeatureResult<Self> {
        if kind(value)? == "matching_biomes" {
            return Ok(Self::Biomes(
                crate::block_predicate::catalog().biome_holder_set(&value["biomes"])?,
            ));
        }
        let offset = value
            .get("offset")
            .map(crate::block_predicate::position)
            .transpose()?
            .unwrap_or((0, 0, 0));
        Ok(match kind(value)? {
            "true" => Self::True,
            "matching_block_tag" => Self::Tag(tag_name(&value["tag"])?, offset),
            "matching_blocks" => {
                let names: Vec<_> = if let Some(name) = value["blocks"].as_str() {
                    vec![name]
                } else {
                    array(&value["blocks"])?
                        .iter()
                        .map(string)
                        .collect::<FeatureResult<_>>()?
                };
                Self::Blocks(
                    names
                        .into_iter()
                        .map(|name| {
                            let b = catalog()
                                .blocks
                                .get(crate::block_predicate::qualified(name).as_ref())
                                .ok_or_else(|| {
                                    crate::block_predicate::missing(format!("tree block {name}"))
                                })?;
                            Ok((b.first, b.end))
                        })
                        .collect::<FeatureResult<_>>()?,
                    offset,
                )
            }
            "not" => Self::Not(Box::new(Self::parse(&value["predicate"])?)),
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
            other => {
                return Err(FeatureError::Unsupported(format!(
                    "tree-provider predicate {other}"
                )))
            }
        })
    }
    pub fn test<W: StandingTreeWorld + ?Sized>(&self, world: &W, p: Pos) -> FeatureResult<bool> {
        match self {
            Self::True => Ok(true),
            Self::Biomes(biomes) => {
                let biome = world.standing_biome(p)?;
                if biome as usize >= bcore_registry::BIOME_COUNT {
                    return Err(crate::block_predicate::missing(format!(
                        "tree biome {biome}"
                    )));
                }
                Ok(biomes.contains(&biome))
            }
            Self::Tag(tag, o) => Ok(tagged(
                world.get_block(crate::block_predicate::offset(p, *o)),
                tag,
            )),
            Self::Blocks(blocks, o) => {
                let s = world.get_block(crate::block_predicate::offset(p, *o));
                Ok(blocks.iter().any(|&(a, b)| (a..b).contains(&s)))
            }
            Self::Not(predicate) => Ok(!predicate.test(world, p)?),
            Self::All(items) => {
                for predicate in items {
                    if !predicate.test(world, p)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Any(items) => {
                for predicate in items {
                    if predicate.test(world, p)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }
}

pub(super) struct TreeDefinition {
    pub shape: TreeConfig,
    pub below: Provider,
    pub foliage: Provider,
    pub ignore_vines: bool,
    pub grow_through: Option<String>,
    pub decorators: Vec<super::standing::extra_decorators::Decorator>,
    pub roots: Option<super::standing::extra_roots::Roots>,
}

type TreeCache = BTreeMap<&'static str, OnceLock<FeatureResult<TreeDefinition>>>;

pub(super) fn tree(name: &str) -> FeatureResult<Option<&'static TreeDefinition>> {
    static TREES: OnceLock<TreeCache> = OnceLock::new();
    let documents = crate::block_predicate::catalog().documents["configured_feature"]
        .as_object()
        .ok_or_else(|| invalid("configured tree registry"))?;
    let name = crate::block_predicate::qualified(name);
    let definition = TREES
        .get_or_init(|| {
            documents
                .iter()
                .filter(|(_, v)| v["type"] == "minecraft:tree")
                .map(|(name, _)| (name.as_str(), OnceLock::new()))
                .collect()
        })
        .get(name.as_ref());
    let Some(definition) = definition else {
        return Ok(None);
    };
    match definition.get_or_init(|| parse_tree(&documents[name.as_ref()]["config"])) {
        Ok(definition) => Ok(Some(definition)),
        Err(error) => Err(error.clone()),
    }
}

pub(super) fn check_tree(name: &str) -> crate::block_predicate::FeatureResult<bool> {
    Ok(tree(name)?.is_some())
}

fn minimum_size(value: &Value) -> crate::block_predicate::FeatureResult<FeatureSize> {
    let field = |name: &str, default: i32, max: i32| -> FeatureResult<i32> {
        let number = value.get(name).map(int).transpose()?.unwrap_or(default);
        if !(0..=max).contains(&number) {
            return Err(invalid(format!(
                "tree feature-size {name} outside 0..={max}"
            )));
        }
        Ok(number)
    };
    Ok(match kind(value)? {
        "two_layers_feature_size" => FeatureSize::TwoLayers {
            limit: field("limit", 1, 81)?,
            lower_size: field("lower_size", 0, 16)?,
            upper_size: field("upper_size", 1, 16)?,
        },
        "three_layers_feature_size" => FeatureSize::ThreeLayers {
            limit: field("limit", 1, 80)?,
            upper_limit: field("upper_limit", 1, 80)?,
            lower_size: field("lower_size", 0, 16)?,
            middle_size: field("middle_size", 1, 16)?,
            upper_size: field("upper_size", 1, 16)?,
        },
        other => {
            return Err(FeatureError::Unsupported(format!(
                "tree feature size {other}"
            )))
        }
    })
}

pub(super) fn parse_tree(config: &Value) -> FeatureResult<TreeDefinition> {
    let t = &config["trunk_placer"];
    let base_height = number(&t["base_height"])?;
    let height_rand_a = number(&t["height_rand_a"])?;
    let height_rand_b = number(&t["height_rand_b"])?;
    if !(0..=32).contains(&base_height)
        || !(0..=24).contains(&height_rand_a)
        || !(0..=24).contains(&height_rand_b)
    {
        return Err(invalid("tree trunk height fields outside codec bounds"));
    }
    let size = &config["minimum_size"];
    let minimum_size = minimum_size(size)?;
    let min_clipped_height = size.get("min_clipped_height").map(number).transpose()?;
    if min_clipped_height.is_some_and(|height| !(0..=80).contains(&height)) {
        return Err(invalid("tree min-clipped-height outside 0..=80"));
    }
    let below = Provider::parse(&config["below_trunk_provider"])?;
    let foliage = Provider::parse(&config["foliage_provider"])?;
    let leaves = foliage.first()?;
    if kind(&config["trunk_provider"])? != "simple_state_provider" {
        return Err(FeatureError::Unsupported(
            "non-simple tree trunk provider".into(),
        ));
    }
    let log = state(&config["trunk_provider"]["state"])?;
    let decorators = array(&config["decorators"])?
        .iter()
        .map(super::standing::extra_decorators::Decorator::parse)
        .collect::<FeatureResult<_>>()?;
    let roots = config
        .get("root_placer")
        .map(super::standing::extra_roots::Roots::parse)
        .transpose()?;
    // TreeConfiguration uses BOOL.fieldOf(...).orElse(false), including decode errors.
    let ignore_vines = config
        .get("ignore_vines")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let grow_through = t.get("can_grow_through").map(tag_name).transpose()?;
    let trunk = match t["type"]
        .as_str()
        .ok_or_else(|| invalid("tree trunk type"))?
    {
        "minecraft:straight_trunk_placer" => TrunkPlacer::Straight {
            base_height,
            height_rand_a,
            height_rand_b,
        },
        "minecraft:fancy_trunk_placer" => TrunkPlacer::Fancy {
            base_height,
            height_rand_a,
            height_rand_b,
        },
        "minecraft:forking_trunk_placer" => TrunkPlacer::Forking {
            base_height,
            height_rand_a,
            height_rand_b,
        },
        "minecraft:giant_trunk_placer" => TrunkPlacer::Giant {
            base_height,
            height_rand_a,
            height_rand_b,
        },
        "minecraft:dark_oak_trunk_placer" => TrunkPlacer::DarkOak {
            base_height,
            height_rand_a,
            height_rand_b,
        },
        _ => TrunkPlacer::Extended {
            base_height,
            height_rand_a,
            height_rand_b,
            placer: Box::leak(Box::new(ExtraTrunk::parse(t)?)),
        },
    };
    if matches!(trunk, TrunkPlacer::Fancy { .. })
        || matches!(
            trunk,
            TrunkPlacer::Extended {
                placer: ExtraTrunk::Cherry { .. },
                ..
            }
        )
    {
        let block = crate::block_predicate::catalog().block(log)?.1;
        for axis in ["x", "y", "z"] {
            block.with_property(log, "axis", axis)?;
        }
    }
    let f = &config["foliage_placer"];
    let radius = int_provider(&f["radius"])?;
    let offset = int_provider(&f["offset"])?;
    let foliage_placer = match f["type"]
        .as_str()
        .ok_or_else(|| invalid("tree foliage type"))?
    {
        "minecraft:blob_foliage_placer" => FoliagePlacer::Blob {
            radius,
            offset,
            height: int_provider(&f["height"])?,
        },
        "minecraft:spruce_foliage_placer" => FoliagePlacer::Spruce {
            radius,
            offset,
            trunk_height: int_provider(&f["trunk_height"])?,
        },
        "minecraft:pine_foliage_placer" => FoliagePlacer::Pine {
            radius,
            offset,
            height: int_provider(&f["height"])?,
        },
        "minecraft:acacia_foliage_placer" => FoliagePlacer::Acacia { radius, offset },
        "minecraft:dark_oak_foliage_placer" => FoliagePlacer::DarkOak { radius, offset },
        "minecraft:fancy_foliage_placer" => FoliagePlacer::Fancy {
            radius,
            offset,
            height: int_provider(&f["height"])?,
        },
        _ => FoliagePlacer::Extended {
            radius,
            offset,
            placer: Box::leak(Box::new(ExtraFoliage::parse(f)?)),
        },
    };
    Ok(TreeDefinition {
        shape: TreeConfig {
            trunk,
            foliage: foliage_placer,
            log,
            leaves,
            minimum_size,
            min_clipped_height,
            beehive_probability: None,
            leaf_litter: false,
        },
        below,
        foliage,
        ignore_vines,
        grow_through,
        decorators,
        roots,
    })
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
