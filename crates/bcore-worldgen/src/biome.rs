//! Vanilla multi-noise biome placement.

/// Numeric biome registry id.  The registry synchronisation layer owns the
/// authoritative mapping; keeping this type numeric makes this module usable
/// before that layer is wired into worldgen.
pub type BiomeId = u32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClimateRange {
    pub min: f64,
    pub max: f64,
}

impl ClimateRange {
    pub const fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }
    pub fn contains(self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BiomeParameters {
    pub temperature: ClimateRange,
    pub humidity: ClimateRange,
    pub continentalness: ClimateRange,
    pub erosion: ClimateRange,
    pub weirdness: ClimateRange,
    /// Vanilla's depth parameter is a range (usually a point, but not always).
    pub depth: ClimateRange,
    /// Present in the data report; selection does not use offset.
    pub offset: f64,
}

/// Selects the vanilla multi-noise row with the smallest squared distance.
/// A point inside a parameter range has zero distance; this is why exact
/// containment remains preferred, while out-of-range samples still select the
/// nearest biome instead of falling back to Plains.
pub fn biome_at(
    parameters: &[(BiomeId, BiomeParameters)],
    temperature: f64,
    humidity: f64,
    continentalness: f64,
    erosion: f64,
    depth: f64,
    weirdness: f64,
) -> BiomeId {
    parameters
        .iter()
        .min_by(|(_, a), (_, b)| {
            parameter_distance(
                a,
                temperature,
                humidity,
                continentalness,
                erosion,
                depth,
                weirdness,
            )
            .total_cmp(&parameter_distance(
                b,
                temperature,
                humidity,
                continentalness,
                erosion,
                depth,
                weirdness,
            ))
        })
        .map(|(id, _)| *id)
        .unwrap_or(DEFAULT_BIOME)
}

fn range_distance(range: ClimateRange, value: f64) -> f64 {
    if value < range.min {
        range.min - value
    } else if value > range.max {
        value - range.max
    } else {
        0.0
    }
}

fn parameter_distance(
    p: &BiomeParameters,
    temperature: f64,
    humidity: f64,
    continentalness: f64,
    erosion: f64,
    depth: f64,
    weirdness: f64,
) -> f64 {
    [
        range_distance(p.temperature, temperature),
        range_distance(p.humidity, humidity),
        range_distance(p.continentalness, continentalness),
        range_distance(p.erosion, erosion),
        range_distance(p.depth, depth),
        range_distance(p.weirdness, weirdness),
    ]
    .into_iter()
    .map(|distance| distance * distance)
    .sum::<f64>()
        + p.offset
}

/// Vanilla's normal fallback for an unmatched multi-noise point.
pub const DEFAULT_BIOME: BiomeId = 40; // minecraft:plains in BCore's registry

/// IDs currently used by the built-in surface rules.
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

fn number_after(text: &str, key: &str, from: usize) -> Option<f64> {
    let start = text[from..].find(key)? + from + key.len();
    let tail = text[start..].trim_start().strip_prefix(':')?.trim_start();
    let end = tail
        .find(|c: char| !matches!(c, '-' | '+' | '.' | '0'..='9' | 'e' | 'E'))
        .unwrap_or(tail.len());
    tail[..end].parse().ok()
}

fn range_after(text: &str, key: &str, from: usize) -> Option<ClimateRange> {
    let start = text[from..].find(key)? + from + key.len();
    let open = text[start..].find('[')? + start + 1;
    let close = text[open..].find(']')? + open;
    let values: Vec<f64> = text[open..close]
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    match values.as_slice() {
        [value] => Some(ClimateRange::new(*value, *value)),
        [min, max] => Some(ClimateRange::new(*min, *max)),
        _ => None,
    }
}

fn point_or_range_after(text: &str, key: &str, from: usize) -> Option<ClimateRange> {
    range_after(text, key, from)
        .or_else(|| number_after(text, key, from).map(|value| ClimateRange::new(value, value)))
}

/// Loads the generated `biome_parameters/.../overworld.json` report.
/// This small dependency-free reader intentionally accepts only that report's
/// stable schema, avoiding a runtime JSON dependency in worldgen.
pub fn load_overworld_parameters(
    path: impl AsRef<std::path::Path>,
) -> Result<Vec<(BiomeId, BiomeParameters)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find("\"biome\"") {
        let at = cursor + rel;
        let value_start = text[at..].find(':').ok_or("malformed biome entry")? + at + 1;
        let q1 = text[value_start..].find('"').ok_or("missing biome name")? + value_start + 1;
        let q2 = text[q1..].find('"').ok_or("unterminated biome name")? + q1;
        let name = &text[q1..q2];
        let id = biome_name_to_id(name);
        let end = text[q2..]
            .find("\"biome\"")
            .map(|x| q2 + x)
            .unwrap_or(text.len());
        let p = |k| range_after(&text, k, q2).ok_or_else(|| format!("missing {k}"));
        let depth = point_or_range_after(&text, "\"depth\"", q2).ok_or("missing depth")?;
        let offset = number_after(&text, "\"offset\"", q2).unwrap_or(0.0);
        out.push((
            id,
            BiomeParameters {
                temperature: p("\"temperature\"")?,
                humidity: p("\"humidity\"")?,
                continentalness: p("\"continentalness\"")?,
                erosion: p("\"erosion\"")?,
                weirdness: p("\"weirdness\"")?,
                depth,
                offset,
            },
        ));
        cursor = end;
    }
    Ok(out)
}

fn biome_name_to_id(name: &str) -> BiomeId {
    // Unknown names remain deterministic and can be replaced by registry ids later.
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "forest" => ids::FOREST,
        "birch_forest" => ids::BIRCH_FOREST,
        "dark_forest" => ids::DARK_FOREST,
        "beach" => ids::BEACH,
        "desert" => ids::DESERT,
        "ocean" => ids::OCEAN,
        "frozen_ocean" => ids::FROZEN_OCEAN,
        "river" => ids::RIVER,
        "plains" => ids::PLAINS,
        "snowy_plains" => ids::SNOWY_PLAINS,
        "snowy_slopes" => ids::SNOWY_SLOPES,
        "mushroom_fields" => ids::MUSHROOM_FIELDS,
        "badlands" | "wooded_badlands" | "eroded_badlands" => ids::BADLANDS,
        _ => DEFAULT_BIOME,
    }
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
            offset: 0.,
        }
    }
    #[test]
    fn first_matching_row_wins() {
        assert_eq!(
            biome_at(&[(7, p(0.)), (8, p(0.))], 0., 0., 0., 0., 0., 0.),
            7
        );
    }
    #[test]
    fn depth_outside_range_uses_nearest_row() {
        assert_eq!(biome_at(&[(7, p(1.))], 0., 0., 0., 0., 0., 0.), 7);
    }

    #[test]
    fn depth_range_matches_vanilla_parameter_span() {
        let mut parameter = p(0.);
        parameter.depth = ClimateRange::new(0.2, 0.9);
        assert_eq!(biome_at(&[(7, parameter)], 0., 0., 0., 0., 0., 0.5), 7);
    }
}
