//! Interpreter for vanilla `surface_rule` datapack trees.
use crate::biome::BiomeId;
use crate::simplex::NoiseRegistry;
use crate::surface::{vertical_gradient, BlockState};
use crate::{block, MIN_Y};
use serde_json::Value;

#[derive(Clone)]
pub struct SurfaceContext<'a> {
    pub biome: BiomeId,
    pub stone_depth_above: i32,
    pub stone_depth_below: i32,
    pub water_height: i32,
    pub surface_depth: i32,
    pub preliminary_surface_level: i32,
    pub sea_level: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub seed: i64,
    pub noise: Option<&'a NoiseRegistry>,
}

#[derive(Clone, Debug)]
pub enum SurfaceRule {
    Sequence(Vec<SurfaceRule>),
    Condition(SurfaceCondition, Box<SurfaceRule>),
    Block(BlockState),
    Empty,
}
#[derive(Clone, Debug)]
pub enum SurfaceCondition {
    Biome(Vec<BiomeId>),
    StoneDepth {
        offset: i32,
        add_surface_depth: bool,
        secondary: i32,
        surface_type: String,
    },
    Water {
        offset: i32,
        add_stone_depth: bool,
        multiplier: i32,
    },
    AbovePreliminarySurface,
    VerticalGradient {
        name: String,
        below: i32,
        above: i32,
    },
    YAbove {
        anchor: i32,
        multiplier: i32,
        add_stone_depth: bool,
    },
    Not(Box<SurfaceCondition>),
    Hole,
    Noise {
        name: String,
        min: f64,
        max: f64,
    },
    Unsupported,
}

impl SurfaceRule {
    pub fn parse(value: &Value) -> Self {
        let Some(typ) = value.get("type").and_then(Value::as_str) else {
            return Self::Empty;
        };
        let typ = typ.rsplit(':').next().unwrap_or(typ);
        match typ {
            "sequence" => Self::Sequence(
                value
                    .get("sequence")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(Self::parse).collect())
                    .unwrap_or_default(),
            ),
            "condition" => match (value.get("if_true"), value.get("then_run")) {
                (Some(c), Some(r)) => Self::Condition(parse_condition(c), Box::new(Self::parse(r))),
                _ => Self::Empty,
            },
            "block" => Self::Block(block_id(value.get("result_state"))),
            _ => Self::Empty,
        }
    }
    pub fn from_json_str(s: &str) -> Result<Self, serde_json::Error> {
        Ok(Self::parse(&serde_json::from_str(s)?))
    }
    pub fn evaluate(&self, c: &SurfaceContext<'_>) -> Option<BlockState> {
        match self {
            Self::Block(b) => Some(*b),
            Self::Sequence(xs) => xs.iter().find_map(|x| x.evaluate(c)),
            Self::Condition(cond, rule) if cond.test(c) => rule.evaluate(c),
            _ => None,
        }
    }
}
impl SurfaceCondition {
    pub fn test(&self, c: &SurfaceContext<'_>) -> bool {
        match self {
            Self::Biome(ids) => ids.contains(&c.biome),
            Self::StoneDepth {
                offset,
                add_surface_depth,
                secondary,
                surface_type,
                ..
            } => {
                let depth = if surface_type == "ceiling" {
                    c.stone_depth_below
                } else {
                    c.stone_depth_above
                };
                let secondary_depth = if *secondary == 0 {
                    0
                } else {
                    let v = c
                        .noise
                        .map(|n| {
                            n.sample(
                                "minecraft:surface_secondary",
                                c.seed,
                                c.x as f64,
                                0.0,
                                c.z as f64,
                            )
                        })
                        .unwrap_or(0.0);
                    (((v.clamp(-1.0, 1.0) + 1.0) * 0.5) * *secondary as f64) as i32
                };
                depth
                    <= 1 + *offset
                        + if *add_surface_depth {
                            c.surface_depth
                        } else {
                            0
                        }
                        + secondary_depth
            }
            Self::Water {
                offset,
                add_stone_depth,
                multiplier,
            } => {
                if c.water_height == i32::MIN {
                    return true;
                }
                c.y + if *add_stone_depth {
                    c.stone_depth_above
                } else {
                    0
                } >= c.water_height + *offset + c.surface_depth * *multiplier
            }
            Self::AbovePreliminarySurface => {
                c.y >= c.preliminary_surface_level + c.surface_depth - 8
            }
            Self::VerticalGradient { name, below, above } => {
                vertical_gradient(name, c.x, c.y, c.z, c.seed, *below, *above)
            }
            Self::YAbove {
                anchor,
                multiplier,
                add_stone_depth,
            } => {
                c.y + if *add_stone_depth {
                    c.stone_depth_above
                } else {
                    0
                } >= *anchor + c.surface_depth * *multiplier
            }
            Self::Not(x) => !x.test(c),
            Self::Hole => c.surface_depth <= 0,
            Self::Noise { name, min, max } => c
                .noise
                .map(|n| {
                    let v = n.sample(name, c.seed, c.x as f64, c.y as f64, c.z as f64);
                    v >= *min && v <= *max
                })
                .unwrap_or(false),
            Self::Unsupported => false,
        }
    }
}
fn parse_condition(v: &Value) -> SurfaceCondition {
    let Some(typ) = v.get("type").and_then(Value::as_str) else {
        return SurfaceCondition::Unsupported;
    };
    let typ = typ.rsplit(':').next().unwrap_or(typ);
    match typ {
        "biome" => SurfaceCondition::Biome(v.get("biome_is").map(parse_biomes).unwrap_or_default()),
        "stone_depth" => SurfaceCondition::StoneDepth {
            offset: i32v(v, "offset", 0),
            add_surface_depth: boolv(v, "add_surface_depth", false),
            secondary: i32v(v, "secondary_depth_range", 0),
            surface_type: v
                .get("surface_type")
                .and_then(Value::as_str)
                .unwrap_or("floor")
                .to_string(),
        },
        "water" => SurfaceCondition::Water {
            offset: i32v(v, "offset", 0),
            add_stone_depth: boolv(v, "add_stone_depth", false),
            multiplier: i32v(v, "surface_depth_multiplier", 0),
        },
        "above_preliminary_surface" => SurfaceCondition::AbovePreliminarySurface,
        "vertical_gradient" => SurfaceCondition::VerticalGradient {
            name: strv(v, "random_name"),
            below: anchor(v.get("true_at_and_below")),
            above: anchor(v.get("false_at_and_above")),
        },
        "y_above" => SurfaceCondition::YAbove {
            anchor: anchor(v.get("anchor")),
            multiplier: i32v(v, "surface_depth_multiplier", 0),
            add_stone_depth: boolv(v, "add_stone_depth", false),
        },
        "not" => SurfaceCondition::Not(Box::new(
            v.get("invert")
                .map(parse_condition)
                .unwrap_or(SurfaceCondition::Unsupported),
        )),
        "hole" => SurfaceCondition::Hole,
        "noise_threshold" => SurfaceCondition::Noise {
            name: strv(v, "noise"),
            min: f64v(v, "min_threshold", f64::MIN),
            max: f64v(v, "max_threshold", f64::MAX),
        },
        _ => SurfaceCondition::Unsupported,
    }
}
fn parse_biomes(v: &Value) -> Vec<BiomeId> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(biome_id)).collect(),
        Value::String(s) => vec![biome_id(s)],
        _ => vec![],
    }
}
fn biome_id(s: &str) -> BiomeId {
    match s.rsplit(':').next().unwrap_or(s) {
        "badlands" => 2,
        "beach" => 3,
        "desert" => 14,
        "eroded_badlands" => 18,
        "frozen_ocean" => 22,
        "mushroom_fields" => 34,
        "ocean" => 35,
        "river" => 41,
        "snowy_plains" => 46,
        "snowy_slopes" => 47,
        "wooded_badlands" => 64,
        "swamp" => 54,
        "mangrove_swamp" => 55,
        _ => u32::MAX,
    }
}
fn block_id(v: Option<&Value>) -> BlockState {
    let n = v
        .and_then(|x| x.get("Name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match n.rsplit(':').next().unwrap_or(n) {
        "air" => block::AIR,
        "stone" => block::STONE,
        "dirt" => block::DIRT,
        "grass_block" => block::GRASS_BLOCK,
        "coarse_dirt" => block::COARSE_DIRT,
        "podzol" => block::PODZOL,
        "bedrock" => block::BEDROCK,
        "water" => block::WATER,
        "sand" => block::SAND,
        "gravel" => block::GRAVEL,
        "sandstone" => block::SANDSTONE,
        "snow_block" => block::SNOW_BLOCK,
        "deepslate" => block::DEEPSLATE,
        "tuff" => block::TUFF,
        "terracotta" => 12912,
        "orange_terracotta" => 12912,
        "red_sand" => 123,
        _ => block::AIR,
    }
}
fn anchor(v: Option<&Value>) -> i32 {
    let Some(x) = v else { return MIN_Y };
    if let Some(n) = x.get("absolute").and_then(Value::as_i64) {
        return n as i32;
    }
    if let Some(n) = x.get("above_bottom").and_then(Value::as_i64) {
        return MIN_Y + n as i32;
    }
    if let Some(n) = x.get("below_top").and_then(Value::as_i64) {
        return crate::MAX_Y - n as i32;
    }
    MIN_Y
}
fn i32v(v: &Value, k: &str, d: i32) -> i32 {
    v.get(k)
        .and_then(Value::as_i64)
        .map(|x| x as i32)
        .unwrap_or(d)
}
fn boolv(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn f64v(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn strv(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_tree() {
        let s = std::fs::read_to_string(
            "../../target/datapack/data/minecraft/worldgen/noise_settings/overworld.json",
        )
        .unwrap();
        let d: Value = serde_json::from_str(&s).unwrap();
        let r = SurfaceRule::parse(&d["surface_rule"]);
        let c = SurfaceContext {
            biome: 0,
            stone_depth_above: 0,
            stone_depth_below: 0,
            water_height: 63,
            surface_depth: 0,
            preliminary_surface_level: 70,
            sea_level: 63,
            x: 0,
            y: 70,
            z: 0,
            seed: 1234,
            noise: None,
        };
        assert!(r.evaluate(&c).is_some());
    }
}
