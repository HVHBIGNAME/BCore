//! Interpreter for vanilla `surface_rule` datapack trees.
use crate::biome::BiomeId;
use crate::simplex::NoiseRegistry;
use crate::surface::{vertical_gradient, BlockState};
use crate::{block, GeneratedChunk, MIN_Y};
use serde_json::Value;
use std::sync::OnceLock;

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
    Bandlands,
    Empty,
    Unsupported(String),
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
    Steep,
    Temperature,
    Noise {
        name: String,
        min: f64,
        max: f64,
    },
    Unsupported(String),
}

impl SurfaceRule {
    pub fn parse(value: &Value) -> Self {
        let Some(typ) = value.get("type").and_then(Value::as_str) else {
            return Self::Empty;
        };
        let typ = typ.strip_prefix("minecraft:").unwrap_or(typ);
        match typ {
            "sequence" => Self::Sequence(
                value
                    .get("sequence")
                    .and_then(Value::as_array)
                    .expect("surface rule sequence")
                    .iter()
                    .map(Self::parse)
                    .collect(),
            ),
            "condition" => match (value.get("if_true"), value.get("then_run")) {
                (Some(c), Some(r)) => Self::Condition(parse_condition(c), Box::new(Self::parse(r))),
                _ => Self::Empty,
            },
            "block" => Self::Block(block_id(value.get("result_state"))),
            "bandlands" => Self::Bandlands,
            _ => Self::Unsupported(typ.to_owned()),
        }
    }
    pub fn from_json_str(s: &str) -> Result<Self, serde_json::Error> {
        Ok(Self::parse(&serde_json::from_str(s)?))
    }

    pub fn check_supported(&self) -> crate::block_predicate::FeatureResult<()> {
        use crate::block_predicate::invalid;
        use crate::feature_world::FeatureError;

        match self {
            Self::Sequence(rules) => {
                for rule in rules {
                    rule.check_supported()?;
                }
                Ok(())
            }
            Self::Condition(condition, rule) => {
                condition.check_supported()?;
                rule.check_supported()
            }
            Self::Unsupported(name) => {
                Err(FeatureError::Unsupported(format!("surface rule {name}")))
            }
            Self::Empty => Err(invalid("missing surface rule")),
            _ => Ok(()),
        }
    }

    pub fn evaluate(&self, c: &SurfaceContext<'_>) -> Option<BlockState> {
        self.evaluate_with_steep(c, &mut || false)
    }

    pub(crate) fn evaluate_in_chunk(
        &self,
        c: &SurfaceContext<'_>,
        chunk: &GeneratedChunk,
    ) -> Option<BlockState> {
        self.evaluate_with_steep(c, &mut || steep(chunk, c.x, c.z))
    }

    /// Evaluate with the caller's lazily cached, column-local steepness condition.
    /// Native `LazyXZCondition` keeps the first value until the next column.
    pub fn evaluate_with_steep(
        &self,
        c: &SurfaceContext<'_>,
        steep: &mut impl FnMut() -> bool,
    ) -> Option<BlockState> {
        match self {
            Self::Block(b) => Some(*b),
            Self::Bandlands => Some(band(c)),
            Self::Sequence(xs) => xs.iter().find_map(|x| x.evaluate_with_steep(c, steep)),
            Self::Condition(cond, rule) if cond.test_with_steep(c, steep) => {
                rule.evaluate_with_steep(c, steep)
            }
            Self::Unsupported(name) => panic!("unsupported surface rule {name}"),
            _ => None,
        }
    }
}
impl SurfaceCondition {
    fn check_supported(&self) -> crate::block_predicate::FeatureResult<()> {
        match self {
            Self::Not(condition) => condition.check_supported(),
            Self::Unsupported(name) => Err(crate::feature_world::FeatureError::Unsupported(
                format!("surface condition {name}"),
            )),
            Self::Biome(ids) if ids.contains(&u32::MAX) => {
                Err(crate::block_predicate::invalid("unknown surface biome"))
            }
            _ => Ok(()),
        }
    }

    pub fn test(&self, c: &SurfaceContext<'_>) -> bool {
        self.test_with_steep(c, &mut || false)
    }

    fn test_with_steep(&self, c: &SurfaceContext<'_>, steep: &mut impl FnMut() -> bool) -> bool {
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
                    (((v + 1.0) / 2.0) * *secondary as f64) as i32
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
            Self::Not(x) => !x.test_with_steep(c, steep),
            Self::Hole => c.surface_depth <= 0,
            Self::Steep => steep(),
            Self::Temperature => c.temperature() < 0.15_f32,
            Self::Noise { name, min, max } => c
                .noise
                .map(|n| {
                    let v = n.sample(name, c.seed, c.x as f64, 0.0, c.z as f64);
                    v >= *min && v <= *max
                })
                .unwrap_or(false),
            Self::Unsupported(name) => panic!("unsupported surface condition {name}"),
        }
    }
}
fn parse_condition(v: &Value) -> SurfaceCondition {
    let Some(typ) = v.get("type").and_then(Value::as_str) else {
        return SurfaceCondition::Unsupported("missing condition type".into());
    };
    let typ = typ.strip_prefix("minecraft:").unwrap_or(typ);
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
            v.get("invert").map(parse_condition).unwrap_or_else(|| {
                SurfaceCondition::Unsupported("missing inverted condition".into())
            }),
        )),
        "hole" => SurfaceCondition::Hole,
        "steep" => SurfaceCondition::Steep,
        "temperature" => SurfaceCondition::Temperature,
        "noise_threshold" => SurfaceCondition::Noise {
            name: strv(v, "noise"),
            min: f64v(v, "min_threshold", f64::MIN),
            max: f64v(v, "max_threshold", f64::MAX),
        },
        _ => SurfaceCondition::Unsupported(typ.to_owned()),
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
    crate::biome::id(s).unwrap_or(u32::MAX)
}
fn block_id(v: Option<&Value>) -> BlockState {
    crate::block_predicate::catalog()
        .state(v.expect("surface block state"))
        .expect("native surface block name and properties")
}

/// Native steepness is directional, with both neighbor coordinates clamped to this chunk.
pub fn steep(chunk: &GeneratedChunk, x: i32, z: i32) -> bool {
    let x = (x & 15) as usize;
    let z = (z & 15) as usize;
    let height = |x, z| chunk.surface_y(x, z).unwrap_or(MIN_Y - 1);
    height(x, (z + 1).min(15)) >= height(x, z.saturating_sub(1)) + 4
        || height(x.saturating_sub(1), z) >= height((x + 1).min(15), z) + 4
}

/// SurfaceSystem receives RandomState's root positional factory directly.
pub(crate) fn surface_depth(seed: i64, x: i32, z: i32) -> i32 {
    surface_depth_with_noise(crate::density::noise_registry(), seed, x, z)
}

/// Surface depth from the supplied RandomState noise registry and root positional RNG.
pub fn surface_depth_with_noise(noises: &NoiseRegistry, seed: i64, x: i32, z: i32) -> i32 {
    let noise = noises.sample("minecraft:surface", seed, x as f64, 0.0, z as f64);
    let mut random = crate::noise_perlin::Xoroshiro::new(seed)
        .fork_positional()
        .at(x, 0, z);
    (noise * 2.75 + 3.0 + random.next_double() * 0.25) as i32
}

fn band(c: &SurfaceContext<'_>) -> BlockState {
    thread_local! {
        static BANDS: std::cell::RefCell<Option<(i64, [u32; 192])>> = const { std::cell::RefCell::new(None) };
    }
    let noise = c.noise.expect("badlands band noise registry").sample(
        "minecraft:clay_bands_offset",
        c.seed,
        c.x as f64,
        0.0,
        c.z as f64,
    );
    // Java Math.round rounds negative ties toward positive infinity.
    let offset = (noise * 4.0 + 0.5).floor() as i32;
    BANDS.with_borrow_mut(|cache| {
        if !matches!(cache.as_ref(), Some((seed, _)) if *seed == c.seed) {
            *cache = Some((c.seed, clay_bands(c.seed)));
        }
        cache.as_ref().unwrap().1[(c.y + offset + 192).rem_euclid(192) as usize]
    })
}

fn clay_bands(seed: i64) -> [u32; 192] {
    let mut random = crate::noise_perlin::Xoroshiro::new(seed)
        .fork_positional()
        .from_hash_of("minecraft:clay_bands");
    let mut bands = [block::TERRACOTTA; 192];
    let mut i = 0;
    while i < bands.len() {
        i += random.next_int(5) as usize + 1;
        if i < bands.len() {
            bands[i] = block::ORANGE_TERRACOTTA;
        }
        i += 1;
    }
    for (min_width, state) in [
        (1, block::YELLOW_TERRACOTTA),
        (2, block::BROWN_TERRACOTTA),
        (1, block::RED_TERRACOTTA),
    ] {
        let count = random.next_int(10) + 6;
        for _ in 0..count {
            let width = min_width + random.next_int(3) as usize;
            let start = random.next_int(192) as usize;
            bands[start..(start + width).min(192)].fill(state);
        }
    }
    let count = random.next_int(7) + 9;
    let mut start = 0;
    for _ in 0..count {
        if start >= bands.len() {
            break;
        }
        bands[start] = block::WHITE_TERRACOTTA;
        if start > 1 && random.next_long() & 1 != 0 {
            bands[start - 1] = block::LIGHT_GRAY_TERRACOTTA;
        }
        if start + 1 < bands.len() && random.next_long() & 1 != 0 {
            bands[start + 1] = block::LIGHT_GRAY_TERRACOTTA;
        }
        start += random.next_int(16) as usize + 4;
    }
    bands
}

struct TemperatureNoise {
    temperature: [u8; 256],
    frozen: [[u8; 256]; 3],
    biome_info: [u8; 256],
}

fn temperature_noise() -> &'static TemperatureNoise {
    static NOISE: OnceLock<TemperatureNoise> = OnceLock::new();
    NOISE.get_or_init(|| {
        let permutation = |random: &mut crate::simplex::JavaRandom| {
            for _ in 0..3 {
                random.next_double();
            }
            let mut p = std::array::from_fn(|i| i as u8);
            for i in 0..256 {
                p.swap(i, i + random.next_int(256 - i));
            }
            p
        };
        let mut frozen = crate::simplex::JavaRandom::new(3456);
        TemperatureNoise {
            temperature: permutation(&mut crate::simplex::JavaRandom::new(1234)),
            frozen: std::array::from_fn(|_| permutation(&mut frozen)),
            biome_info: permutation(&mut crate::simplex::JavaRandom::new(2345)),
        }
    })
}

/// SimplexNoise's two-dimensional overload, used without coordinate offsets by Biome.
fn simplex_2d(p: &[u8; 256], x: f64, z: f64) -> f64 {
    let f2 = 0.5 * (3.0_f64.sqrt() - 1.0);
    let g2 = (3.0 - 3.0_f64.sqrt()) / 6.0;
    let skew = (x + z) * f2;
    let ix = (x + skew).floor() as i32;
    let iz = (z + skew).floor() as i32;
    let unskew = ix.wrapping_add(iz) as f64 * g2;
    let x = x - (ix as f64 - unskew);
    let z = z - (iz as f64 - unskew);
    let (dx, dz) = if x > z { (1, 0) } else { (0, 1) };
    let corner = |dx: i32, dz: i32, x: f64, z: f64| {
        const GRADIENT: [[f64; 2]; 12] = [
            [1.0, 1.0],
            [-1.0, 1.0],
            [1.0, -1.0],
            [-1.0, -1.0],
            [1.0, 0.0],
            [-1.0, 0.0],
            [1.0, 0.0],
            [-1.0, 0.0],
            [0.0, 1.0],
            [0.0, -1.0],
            [0.0, 1.0],
            [0.0, -1.0],
        ];
        let t = 0.5 - x * x - z * z;
        if t < 0.0 {
            return 0.0;
        }
        let h = p[((ix + dx + p[((iz + dz) & 255) as usize] as i32) & 255) as usize];
        let [gx, gz] = GRADIENT[h as usize % 12];
        let t = t * t;
        t * t * (gx * x + gz * z + 0.0)
    };
    70.0 * (corner(0, 0, x, z)
        + corner(dx, dz, x - dx as f64 + g2, z - dz as f64 + g2)
        + corner(1, 1, x - 1.0 + 2.0 * g2, z - 1.0 + 2.0 * g2))
}

impl SurfaceContext<'_> {
    /// Native biome temperature, including frozen patches and the height adjustment.
    pub fn temperature(&self) -> f32 {
        static CLIMATES: OnceLock<std::collections::BTreeMap<String, (f32, bool)>> =
            OnceLock::new();
        let climates = CLIMATES.get_or_init(|| {
            let climates: Value =
                serde_json::from_str(include_str!("../data/surface_climates_26_2.json"))
                    .expect("native surface climates");
            let rows = climates.as_object().expect("native climate map");
            assert_eq!(rows.len(), bcore_registry::BIOME_COUNT);
            rows.iter()
                .map(|(name, row)| {
                    assert!(
                        crate::biome::id(name).is_some(),
                        "unknown native climate {name}"
                    );
                    let bits =
                        u32::from_str_radix(row[0].as_str().expect("native temperature bits"), 16)
                            .expect("hex temperature");
                    assert!(
                        f32::from_bits(bits).is_finite(),
                        "non-finite native temperature"
                    );
                    (
                        name.strip_prefix("minecraft:").unwrap().to_owned(),
                        (
                            f32::from_bits(bits),
                            row[1].as_bool().expect("native temperature modifier"),
                        ),
                    )
                })
                .collect()
        });
        let name = crate::biome::name(self.biome);
        let (mut temperature, frozen) = *climates
            .get(name)
            .unwrap_or_else(|| panic!("missing native climate: {name}"));
        let noise = temperature_noise();
        if frozen {
            let mut value = 0.0;
            let mut frequency = 1.0;
            let mut amplitude = 1.0 / 7.0;
            for p in &noise.frozen {
                value += simplex_2d(
                    p,
                    self.x as f64 * 0.05 * frequency,
                    self.z as f64 * 0.05 * frequency,
                ) * amplitude;
                frequency /= 2.0;
                amplitude *= 2.0;
            }
            if value * 7.0 + simplex_2d(&noise.biome_info, self.x as f64 * 0.2, self.z as f64 * 0.2)
                < 0.3
                && simplex_2d(
                    &noise.biome_info,
                    self.x as f64 * 0.09,
                    self.z as f64 * 0.09,
                ) < 0.8
            {
                temperature = 0.2;
            }
        }
        let threshold = self.sea_level + 17;
        if self.y > threshold {
            let value = (simplex_2d(
                &noise.temperature,
                (self.x as f32 / 8.0) as f64,
                (self.z as f32 / 8.0) as f64,
            ) * 8.0) as f32;
            temperature -= (value + self.y as f32 - threshold as f32) * 0.05_f32 / 40.0_f32;
        }
        temperature
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
#[path = "surface_builder_pipeline_tests.rs"]
mod pipeline_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_surface_palette_and_bands() {
        let fixture: Value =
            serde_json::from_str(include_str!("../data/carvers_26_1.json")).unwrap();
        let climates: Value =
            serde_json::from_str(include_str!("../data/surface_climates_26_1.json")).unwrap();
        assert_eq!(climates, fixture["surface_metadata"]["biome_climates"]);
        for sample in fixture["surface_metadata"]["palette"].as_array().unwrap() {
            let state = block_id(Some(&sample["spec"]));
            assert_eq!(
                u64::from(crate::registry_26_1::historical_state(state)),
                sample["state"].as_u64().unwrap(),
                "{}",
                sample["spec"]
            );
            assert_eq!(
                matches!(state, 86..=117),
                sample["has_fluid"].as_bool().unwrap()
            );
        }
        for sample in fixture["samples"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["op"] == "surface_bands")
        {
            assert_eq!(
                serde_json::json!(clay_bands(sample["seed"].as_i64().unwrap())
                    .iter()
                    .copied()
                    .map(crate::registry_26_1::historical_state)
                    .collect::<Vec<_>>()),
                sample["states"],
                "{}",
                sample["id"]
            );
        }
        SurfaceRule::parse(&fixture["surface_metadata"]["surface_rule"])
            .check_supported()
            .unwrap();
    }

    #[test]
    fn parses_real_tree() {
        let d = crate::assets::load("noise_settings/overworld.json").unwrap();
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

    #[test]
    fn unsupported_surface_branches_remain_explicit_under_conditions_and_inversion() {
        use crate::feature_world::FeatureError;
        use serde_json::json;

        let rule = SurfaceRule::parse(&json!({
            "type": "minecraft:condition",
            "if_true": {"type": "minecraft:biome", "biome_is": []},
            "then_run": {"type": "probe:surface_rule"}
        }));
        assert!(
            matches!(rule.check_supported(), Err(FeatureError::Unsupported(name))
            if name.contains("probe:surface_rule"))
        );
        let condition = parse_condition(&json!({
            "type": "minecraft:not",
            "invert": {"type": "probe:surface_condition"}
        }));
        assert!(
            matches!(condition.check_supported(), Err(FeatureError::Unsupported(name))
            if name.contains("probe:surface_condition"))
        );
    }
}
