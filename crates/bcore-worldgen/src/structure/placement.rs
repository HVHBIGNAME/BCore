//! Random-spread candidates and frequency gates, before biome/terrain admission.

use crate::simplex::JavaRandom;
use bcore_core::ChunkPos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadType {
    Linear,
    Triangular,
}

impl SpreadType {
    fn sample(self, random: &mut JavaRandom, bound: usize) -> i32 {
        let first = random.next_int(bound) as i32;
        match self {
            Self::Linear => first,
            Self::Triangular => (first + random.next_int(bound) as i32) / 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyReduction {
    Default,
    LegacyType1,
    LegacyType2,
    LegacyType3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RandomSpreadPlacement {
    spacing: i32,
    separation: i32,
    salt: i32,
    spread: SpreadType,
    frequency: f32,
    reduction: FrequencyReduction,
}

impl RandomSpreadPlacement {
    pub const VILLAGES: Self = Self {
        spacing: super::VILLAGE_SPACING,
        separation: super::VILLAGE_SEPARATION,
        salt: super::VILLAGE_SALT as i32,
        spread: SpreadType::Linear,
        frequency: 1.0,
        reduction: FrequencyReduction::Default,
    };

    pub const MINESHAFTS: Self = Self {
        spacing: 1,
        separation: 0,
        salt: 0,
        spread: SpreadType::Linear,
        frequency: 0.004,
        reduction: FrequencyReduction::LegacyType3,
    };

    /// Applies the 26.1 codec's range constraints.
    pub fn new(
        spacing: i32,
        separation: i32,
        salt: i32,
        spread: SpreadType,
        frequency: f32,
        reduction: FrequencyReduction,
    ) -> Option<Self> {
        if !(1..=4096).contains(&spacing)
            || !(0..spacing).contains(&separation)
            || salt < 0
            || !(0.0..=1.0).contains(&frequency)
        {
            return None;
        }
        Some(Self {
            spacing,
            separation,
            salt,
            spread,
            frequency,
            reduction,
        })
    }

    pub fn potential_chunk(self, seed: i64, chunk: ChunkPos) -> ChunkPos {
        let rx = chunk.x.div_euclid(self.spacing);
        let rz = chunk.z.div_euclid(self.spacing);
        let mut random = salted_random(seed, rx, rz, self.salt);
        let bound = (self.spacing - self.separation) as usize;
        ChunkPos::new(
            rx.wrapping_mul(self.spacing)
                .wrapping_add(self.spread.sample(&mut random, bound)),
            rz.wrapping_mul(self.spacing)
                .wrapping_add(self.spread.sample(&mut random, bound)),
        )
    }

    pub fn passes_frequency(self, seed: i64, chunk: ChunkPos) -> bool {
        if self.frequency >= 1.0 {
            return true;
        }
        match self.reduction {
            FrequencyReduction::Default => {
                // The 26.1 probabilityReducer forwards (salt, x, z) in this order
                // to setLargeFeatureWithSalt; do not normalize it to (x, z, salt).
                salted_random(seed, self.salt, chunk.x, chunk.z).next_float() < self.frequency
            }
            FrequencyReduction::LegacyType1 => {
                let region = (chunk.x >> 4) ^ (chunk.z >> 4).wrapping_shl(4);
                let mut random = JavaRandom::new(i64::from(region) ^ seed);
                random.next_int_unbounded();
                random.next_int((1.0_f32 / self.frequency) as i32 as usize) == 0
            }
            FrequencyReduction::LegacyType2 => {
                salted_random(seed, chunk.x, chunk.z, 10_387_320).next_float() < self.frequency
            }
            FrequencyReduction::LegacyType3 => {
                large_feature_random(seed, chunk).next_double() < f64::from(self.frequency)
            }
        }
    }

    /// A candidate still needs exclusion-zone, biome and structure-specific checks.
    pub fn is_candidate(self, seed: i64, chunk: ChunkPos) -> bool {
        self.potential_chunk(seed, chunk) == chunk && self.passes_frequency(seed, chunk)
    }
}

fn salted_random(seed: i64, x: i32, z: i32, salt: i32) -> JavaRandom {
    JavaRandom::new(
        i64::from(x)
            .wrapping_mul(341_873_128_712)
            .wrapping_add(i64::from(z).wrapping_mul(132_897_987_541))
            .wrapping_add(seed)
            .wrapping_add(i64::from(salt)),
    )
}

/// `WorldgenRandom.setLargeFeatureSeed` with a LegacyRandomSource.
pub fn large_feature_random(seed: i64, chunk: ChunkPos) -> JavaRandom {
    let mut random = JavaRandom::new(seed);
    let x_scale = random.next_long();
    let z_scale = random.next_long();
    random.set_seed(
        i64::from(chunk.x).wrapping_mul(x_scale) ^ i64::from(chunk.z).wrapping_mul(z_scale) ^ seed,
    );
    random
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_and_frequency_match_vanilla_26_1() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../data/structures_26_1.json")).unwrap();
        let low = fixture["grid_min"].as_i64().unwrap() as i32;
        let high = fixture["grid_max"].as_i64().unwrap() as i32;
        for feature in fixture["samples"].as_array().unwrap() {
            let config = &feature["placement"];
            let spread = match config["spread_type"].as_str().unwrap_or("linear") {
                "linear" => SpreadType::Linear,
                "triangular" => SpreadType::Triangular,
                other => panic!("unknown spread: {other}"),
            };
            let reduction = match config["frequency_reduction_method"]
                .as_str()
                .unwrap_or("default")
            {
                "default" => FrequencyReduction::Default,
                "legacy_type_1" => FrequencyReduction::LegacyType1,
                "legacy_type_2" => FrequencyReduction::LegacyType2,
                "legacy_type_3" => FrequencyReduction::LegacyType3,
                other => panic!("unknown frequency reduction: {other}"),
            };
            let placement = RandomSpreadPlacement::new(
                config["spacing"].as_f64().unwrap() as i32,
                config["separation"].as_f64().unwrap() as i32,
                config["salt"].as_f64().unwrap() as i32,
                spread,
                config["frequency"].as_f64().unwrap_or(1.0) as f32,
                reduction,
            )
            .unwrap();
            match feature["name"].as_str().unwrap() {
                "villages" => assert_eq!(placement, RandomSpreadPlacement::VILLAGES),
                "mineshafts" => assert_eq!(placement, RandomSpreadPlacement::MINESHAFTS),
                _ => {}
            }
            for sample in feature["samples"].as_array().unwrap() {
                let seed = sample["seed"].as_i64().unwrap();
                let mut digest = md5::Context::new();
                let mut accepted = 0;
                for z in low..=high {
                    for x in low..=high {
                        let chunk = ChunkPos::new(x, z);
                        let candidate = placement.potential_chunk(seed, chunk);
                        let allowed = placement.passes_frequency(seed, chunk);
                        digest.consume(candidate.x.to_le_bytes());
                        digest.consume(candidate.z.to_le_bytes());
                        digest.consume([u8::from(allowed)]);
                        if placement.is_candidate(seed, chunk) {
                            accepted += 1;
                        }
                    }
                }
                assert_eq!(
                    format!("{:x}", digest.compute()),
                    sample["candidates_and_frequency_md5"].as_str().unwrap(),
                    "{} seed {seed}",
                    feature["name"]
                );
                assert_eq!(accepted, sample["accepted_candidates"].as_u64().unwrap());
            }
        }
    }

    #[test]
    fn invalid_placement_parameters_are_rejected() {
        for (spacing, separation, salt, frequency) in [
            (0, 0, 0, 1.0),
            (4097, 0, 0, 1.0),
            (10, 10, 0, 1.0),
            (10, -1, 0, 1.0),
            (10, 1, -1, 1.0),
            (10, 1, 0, -0.1),
            (10, 1, 0, 1.1),
            (10, 1, 0, f32::NAN),
        ] {
            assert!(RandomSpreadPlacement::new(
                spacing,
                separation,
                salt,
                SpreadType::Linear,
                frequency,
                FrequencyReduction::Default,
            )
            .is_none());
        }
    }
}
