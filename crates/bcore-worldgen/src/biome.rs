//! Quantized vanilla multi-noise biome parameters.
use serde_json::Value;
use std::sync::OnceLock;

pub type BiomeId = u32;

fn quantize(value: f64) -> i64 {
    // Climate.quantizeCoord receives a float, multiplies in float, then casts to long.
    (value as f32 * 10_000.0_f32) as i64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClimateRange {
    pub min: i64,
    pub max: i64,
}

impl ClimateRange {
    pub fn new(min: f64, max: f64) -> Self {
        Self {
            min: quantize(min),
            max: quantize(max),
        }
    }

    pub fn contains(self, value: f64) -> bool {
        (self.min..=self.max).contains(&quantize(value))
    }

    fn distance(self, value: i64) -> i64 {
        (self.min - value).max(value - self.max).max(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BiomeParameters {
    pub temperature: ClimateRange,
    pub humidity: ClimateRange,
    pub continentalness: ClimateRange,
    pub erosion: ClimateRange,
    pub weirdness: ClimateRange,
    pub depth: ClimateRange,
    /// Seventh climate dimension, measured against zero.
    pub offset: i64,
}

impl BiomeParameters {
    fn distance(&self, target: [i64; 6]) -> i64 {
        [
            self.temperature,
            self.humidity,
            self.continentalness,
            self.erosion,
            self.depth,
            self.weirdness,
        ]
        .into_iter()
        .zip(target)
        .map(|(range, value)| range.distance(value).pow(2))
        .sum::<i64>()
            + self.offset * self.offset
    }
}

/// Nearest quantized parameter row. Equal distances retain parameter-list order.
/// Vanilla's R-tree traversal/cache tie-breaking is not implemented here yet.
pub fn biome_at(
    parameters: &[(BiomeId, BiomeParameters)],
    temperature: f64,
    humidity: f64,
    continentalness: f64,
    erosion: f64,
    depth: f64,
    weirdness: f64,
) -> BiomeId {
    let target = [
        temperature,
        humidity,
        continentalness,
        erosion,
        depth,
        weirdness,
    ]
    .map(quantize);
    parameters
        .iter()
        .min_by_key(|(_, p)| p.distance(target))
        .map(|(id, _)| *id)
        .expect("nonempty biome parameter list")
}

pub const DEFAULT_BIOME: BiomeId = 40;

pub mod ids {
    pub const BADLANDS: u32 = 2;
    pub const BEACH: u32 = 3;
    pub const BIRCH_FOREST: u32 = 5;
    pub const DARK_FOREST: u32 = 13;
    pub const DESERT: u32 = 14;
    pub const FOREST: u32 = 21;
    pub const FROZEN_OCEAN: u32 = 22;
    pub const MUSHROOM_FIELDS: u32 = 34;
    pub const OCEAN: u32 = 35;
    pub const PLAINS: u32 = 40;
    pub const RIVER: u32 = 41;
    pub const SNOWY_PLAINS: u32 = 46;
    pub const SNOWY_SLOPES: u32 = 47;
    pub const WINDSWEPT_SAVANNA: u32 = 62;
}

fn registry() -> &'static Vec<String> {
    static REGISTRY: OnceLock<Vec<String>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        serde_json::from_value(
            crate::assets::load("biome_registry.json").expect("bundled biome registry"),
        )
        .expect("biome name array")
    })
}

pub fn name(id: BiomeId) -> &'static str {
    registry()[id as usize]
        .strip_prefix("minecraft:")
        .expect("vanilla biome namespace")
}

pub fn id(name: &str) -> Option<BiomeId> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    registry()
        .iter()
        .position(|entry| entry.strip_prefix("minecraft:") == Some(name))
        .map(|i| i as u32)
}

pub fn load_overworld_parameters(
    path: impl AsRef<std::path::Path>,
) -> Result<Vec<(BiomeId, BiomeParameters)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_parameters(&serde_json::from_str(&text).map_err(|e| e.to_string())?)
}

pub fn parse_parameters(value: &Value) -> Result<Vec<(BiomeId, BiomeParameters)>, String> {
    let rows = value
        .get("biomes")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or("missing or empty biome parameters")?;
    rows.iter()
        .map(|row| {
            let name = row
                .get("biome")
                .and_then(Value::as_str)
                .ok_or("missing biome name")?;
            let id = id(name).ok_or_else(|| format!("unknown biome: {name}"))?;
            let p = row.get("parameters").ok_or("missing parameters")?;
            let range = |key: &str| -> Result<ClimateRange, String> {
                let v = p
                    .get(key)
                    .ok_or_else(|| format!("missing {key} in {name}"))?;
                if let Some(point) = v.as_f64() {
                    return Ok(ClimateRange::new(point, point));
                }
                let a = v
                    .as_array()
                    .filter(|a| a.len() == 2)
                    .ok_or_else(|| format!("invalid {key} range"))?;
                let min = a[0].as_f64().ok_or("invalid range minimum")?;
                let max = a[1].as_f64().ok_or("invalid range maximum")?;
                if min > max {
                    return Err(format!("reversed {key} range"));
                }
                Ok(ClimateRange::new(min, max))
            };
            Ok((
                id,
                BiomeParameters {
                    temperature: range("temperature")?,
                    humidity: range("humidity")?,
                    continentalness: range("continentalness")?,
                    erosion: range("erosion")?,
                    depth: range("depth")?,
                    weirdness: range("weirdness")?,
                    offset: quantize(
                        p.get("offset")
                            .and_then(Value::as_f64)
                            .ok_or("missing offset")?,
                    ),
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(depth: f64) -> BiomeParameters {
        BiomeParameters {
            temperature: ClimateRange::new(-1., 1.),
            humidity: ClimateRange::new(-1., 1.),
            continentalness: ClimateRange::new(-1., 1.),
            erosion: ClimateRange::new(-1., 1.),
            weirdness: ClimateRange::new(-1., 1.),
            depth: ClimateRange::new(depth, depth),
            offset: 0,
        }
    }

    #[test]
    fn scalar_depth_does_not_read_the_following_erosion_array() {
        let mut value = crate::assets::load("biome_parameters/overworld.json").unwrap();
        let rows = value["biomes"].as_array_mut().unwrap();
        rows.truncate(2);
        let parsed = parse_parameters(&value).unwrap();
        assert_eq!(parsed[0].1.depth, ClimateRange::new(0., 0.));
        assert_eq!(parsed[1].1.depth, ClimateRange::new(1., 1.));
        assert_eq!(parsed[0].1.erosion, ClimateRange::new(-1., 1.));
    }

    #[test]
    fn nearest_row_includes_squared_offset_and_quantized_depth() {
        let mut offset = p(0.);
        offset.offset = quantize(0.1);
        assert_eq!(
            biome_at(&[(7, offset), (8, p(0.05))], 0., 0., 0., 0., 0., 0.),
            8
        );
        assert_eq!(
            biome_at(&[(7, p(0.)), (8, p(0.00009))], 0., 0., 0., 0., 0.00009, 0.),
            7
        );
    }

    #[test]
    fn all_extracted_biomes_keep_their_registry_identity() {
        let parsed =
            parse_parameters(&crate::assets::load("biome_parameters/overworld.json").unwrap())
                .unwrap();
        for expected in [
            "taiga",
            "jungle",
            "river",
            "deep_dark",
            "dripstone_caves",
            "lush_caves",
        ] {
            let expected_id = id(expected).unwrap();
            assert_ne!(expected_id, DEFAULT_BIOME);
            assert!(parsed.iter().any(|(id, _)| *id == expected_id));
            assert_eq!(name(expected_id), expected);
        }
    }
}
