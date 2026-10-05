//! Pinned 26.1 definitions and state-property arithmetic for additional trees.
use super::fallen::{FallenTreeWorld, Pos};
use super::{
    ExtraFoliage, ExtraTrunk, FeatureSize, FoliagePlacer, IntProvider, TreeConfig, TreeRandom,
    TrunkPlacer,
};
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
    pub trees: BTreeMap<String, Value>,
    pub blocks: BTreeMap<String, BlockDefinition>,
    pub tags: BTreeMap<String, Vec<[u32; 2]>>,
}

pub(super) fn catalog() -> &'static Catalog {
    static DATA: OnceLock<Catalog> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/extra_tree_catalog_26_1.json"))
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
            result.resize(block.end as usize, block);
        }
        assert_eq!(result.len(), 29_873);
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

pub(super) fn state(value: &Value) -> u32 {
    let mut result = default_state(value["Name"].as_str().expect("block name"));
    if let Some(properties) = value["Properties"].as_object() {
        for (name, value) in properties {
            result = with_property(result, name, value.as_str().expect("property string"));
        }
    }
    result
}

pub(super) fn tagged(state: u32, name: &str) -> bool {
    let ranges = catalog().tags.get(name).expect("native block tag");
    let index = ranges.partition_point(|range| range[0] <= state);
    index > 0 && state < ranges[index - 1][1]
}

pub(super) fn number(value: &Value) -> i32 {
    let value = value.as_f64().expect("native integer provider value");
    assert!(value.fract() == 0.0 && value >= i32::MIN as f64 && value <= i32::MAX as f64);
    value as i32
}

pub(super) fn int_provider(value: &Value) -> IntProvider {
    if value.is_number() {
        return IntProvider::Constant(number(value));
    }
    assert!(
        value["type"] == "minecraft:uniform",
        "unsupported native integer provider {value}"
    );
    IntProvider::Uniform {
        min: number(&value["min_inclusive"]),
        max: number(&value["max_inclusive"]),
    }
}

pub(super) fn chance(value: &Value) -> f32 {
    value.as_f64().expect("native float") as f32
}

#[derive(Debug)]
pub(super) enum Provider {
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
    Rules(Vec<(Predicate, Provider)>),
}

impl Provider {
    pub fn parse(value: &Value) -> Self {
        match value["type"].as_str().expect("state provider type") {
            "minecraft:simple_state_provider" => Self::Simple(state(&value["state"])),
            "minecraft:weighted_state_provider" => {
                let choices: Vec<_> = value["entries"]
                    .as_array()
                    .expect("weighted entries")
                    .iter()
                    .map(|e| (state(&e["data"]), number(&e["weight"])))
                    .collect();
                let total = choices.iter().map(|c| c.1).sum();
                Self::Weighted { choices, total }
            }
            "minecraft:randomized_int_state_provider" => Self::Randomized {
                source: Box::new(Self::parse(&value["source"])),
                property: value["property"]
                    .as_str()
                    .expect("randomized property")
                    .to_owned(),
                values: int_provider(&value["values"]),
            },
            "minecraft:rule_based_state_provider" => Self::Rules(
                value["rules"]
                    .as_array()
                    .expect("state rules")
                    .iter()
                    .map(|r| (Predicate::parse(&r["if_true"]), Self::parse(&r["then"])))
                    .collect(),
            ),
            kind => panic!("unsupported native state provider {kind}"),
        }
    }

    pub fn first(&self) -> u32 {
        match self {
            Self::Simple(s) => *s,
            Self::Weighted { choices, .. } => choices[0].0,
            _ => panic!("tree geometry requires a simple or weighted base state"),
        }
    }

    pub fn sample<W: FallenTreeWorld + ?Sized, R: TreeRandom + ?Sized>(
        &self,
        world: &W,
        random: &mut R,
        pos: Pos,
    ) -> Option<u32> {
        match self {
            Self::Simple(state) => Some(*state),
            Self::Weighted { choices, total } => {
                let mut pick = random.next_i32_bounded(*total);
                for &(state, weight) in choices {
                    pick -= weight;
                    if pick < 0 {
                        return Some(state);
                    }
                }
                unreachable!("positive native weighted total")
            }
            Self::Randomized {
                source,
                property,
                values,
            } => {
                let state = source.sample(world, random, pos)?;
                Some(with_property(
                    state,
                    property,
                    &values.sample(random).to_string(),
                ))
            }
            Self::Rules(rules) => rules
                .iter()
                .find(|(condition, _)| condition.test(world, pos))
                .and_then(|(_, provider)| provider.sample(world, random, pos)),
        }
    }
}

#[derive(Debug)]
pub(super) enum Predicate {
    Tag(String, Pos),
    Blocks(Vec<(u32, u32)>, Pos),
    Not(Box<Self>),
    All(Vec<Self>),
    Any(Vec<Self>),
}
impl Predicate {
    pub fn parse(value: &Value) -> Self {
        let offset = value["offset"]
            .as_array()
            .map_or((0, 0, 0), |p| (number(&p[0]), number(&p[1]), number(&p[2])));
        match value["type"].as_str().expect("tree predicate type") {
            "minecraft:matching_block_tag" => {
                Self::Tag(value["tag"].as_str().expect("block tag").to_owned(), offset)
            }
            "minecraft:matching_blocks" => {
                let names: Vec<_> = if let Some(name) = value["blocks"].as_str() {
                    vec![name]
                } else {
                    value["blocks"]
                        .as_array()
                        .expect("block names")
                        .iter()
                        .map(|v| v.as_str().expect("block name"))
                        .collect()
                };
                Self::Blocks(
                    names
                        .into_iter()
                        .map(|name| {
                            let b = &catalog().blocks[name];
                            (b.first, b.end)
                        })
                        .collect(),
                    offset,
                )
            }
            "minecraft:not" => Self::Not(Box::new(Self::parse(&value["predicate"]))),
            "minecraft:all_of" => Self::All(
                value["predicates"]
                    .as_array()
                    .expect("predicates")
                    .iter()
                    .map(Self::parse)
                    .collect(),
            ),
            "minecraft:any_of" => Self::Any(
                value["predicates"]
                    .as_array()
                    .expect("predicates")
                    .iter()
                    .map(Self::parse)
                    .collect(),
            ),
            kind => panic!("unsupported tree-provider predicate {kind}"),
        }
    }
    pub fn test<W: FallenTreeWorld + ?Sized>(&self, world: &W, p: Pos) -> bool {
        match self {
            Self::Tag(tag, o) => tagged(world.get_block((p.0 + o.0, p.1 + o.1, p.2 + o.2)), tag),
            Self::Blocks(blocks, o) => {
                let s = world.get_block((p.0 + o.0, p.1 + o.1, p.2 + o.2));
                blocks.iter().any(|&(a, b)| (a..b).contains(&s))
            }
            Self::Not(predicate) => !predicate.test(world, p),
            Self::All(items) => items.iter().all(|predicate| predicate.test(world, p)),
            Self::Any(items) => items.iter().any(|predicate| predicate.test(world, p)),
        }
    }
}

pub(super) struct TreeDefinition {
    pub shape: TreeConfig,
    pub below: Provider,
    pub foliage: Provider,
    pub ignore_vines: bool,
    pub grow_through: Option<String>,
    pub decorators: &'static [Value],
    pub roots: Option<&'static Value>,
}

pub(super) fn tree(name: &str) -> Option<&'static TreeDefinition> {
    static TREES: OnceLock<BTreeMap<String, TreeDefinition>> = OnceLock::new();
    TREES
        .get_or_init(|| {
            catalog()
                .trees
                .iter()
                .filter(|(_, v)| v["type"] == "minecraft:tree")
                .map(|(name, v)| (name.clone(), parse_tree(&v["config"])))
                .collect()
        })
        .get(name)
}

fn parse_tree(config: &'static Value) -> TreeDefinition {
    let t = &config["trunk_placer"];
    let base_height = number(&t["base_height"]);
    let height_rand_a = number(&t["height_rand_a"]);
    let height_rand_b = number(&t["height_rand_b"]);
    let trunk = match t["type"].as_str().expect("trunk type") {
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
            placer: Box::leak(Box::new(ExtraTrunk::parse(t))),
        },
    };
    let f = &config["foliage_placer"];
    let radius = int_provider(&f["radius"]);
    let offset = int_provider(&f["offset"]);
    let foliage_placer = match f["type"].as_str().expect("foliage type") {
        "minecraft:blob_foliage_placer" => FoliagePlacer::Blob {
            radius,
            offset,
            height: int_provider(&f["height"]),
        },
        "minecraft:spruce_foliage_placer" => FoliagePlacer::Spruce {
            radius,
            offset,
            trunk_height: int_provider(&f["trunk_height"]),
        },
        "minecraft:pine_foliage_placer" => FoliagePlacer::Pine {
            radius,
            offset,
            height: int_provider(&f["height"]),
        },
        "minecraft:acacia_foliage_placer" => FoliagePlacer::Acacia { radius, offset },
        "minecraft:dark_oak_foliage_placer" => FoliagePlacer::DarkOak { radius, offset },
        "minecraft:fancy_foliage_placer" => FoliagePlacer::Fancy {
            radius,
            offset,
            height: int_provider(&f["height"]),
        },
        _ => FoliagePlacer::Extended {
            radius,
            offset,
            placer: Box::leak(Box::new(ExtraFoliage::parse(f))),
        },
    };
    let size = &config["minimum_size"];
    let minimum_size = match size["type"].as_str().expect("feature size type") {
        "minecraft:two_layers_feature_size" => FeatureSize::TwoLayers {
            limit: number(&size["limit"]),
            lower_size: number(&size["lower_size"]),
            upper_size: number(&size["upper_size"]),
        },
        "minecraft:three_layers_feature_size" => FeatureSize::ThreeLayers {
            limit: number(&size["limit"]),
            upper_limit: number(&size["upper_limit"]),
            lower_size: number(&size["lower_size"]),
            middle_size: number(&size["middle_size"]),
            upper_size: number(&size["upper_size"]),
        },
        kind => panic!("unsupported tree feature size {kind}"),
    };
    assert_eq!(
        config["trunk_provider"]["type"],
        "minecraft:simple_state_provider"
    );
    let foliage = Provider::parse(&config["foliage_provider"]);
    TreeDefinition {
        shape: TreeConfig {
            trunk,
            foliage: foliage_placer,
            log: state(&config["trunk_provider"]["state"]),
            leaves: foliage.first(),
            minimum_size,
            min_clipped_height: size.get("min_clipped_height").map(number),
            beehive_probability: None,
            leaf_litter: false,
        },
        below: Provider::parse(&config["below_trunk_provider"]),
        foliage,
        ignore_vines: config["ignore_vines"].as_bool().expect("ignore_vines"),
        grow_through: t.get("can_grow_through").map(|v| {
            v.as_str()
                .expect("mangrove trunk tag")
                .trim_start_matches('#')
                .to_owned()
        }),
        decorators: config["decorators"].as_array().expect("tree decorators"),
        roots: config.get("root_placer"),
    }
}
