// SPDX-License-Identifier: MIT
use super::{id, BiomeId, BiomeParameters, ClimateRange};
use serde::Deserialize;
use std::sync::OnceLock;

pub(super) const JAR_SHA256: &str = bcore_registry::SERVER_JAR_SHA256;

#[derive(Deserialize)]
pub(super) struct CapturedRow {
    pub(super) biome: String,
    pub(super) ranges: [[i64; 2]; 6],
    pub(super) offset: i64,
}

impl CapturedRow {
    pub(super) fn parameters(&self) -> BiomeParameters {
        let [temperature, humidity, continentalness, erosion, depth, weirdness] =
            self.ranges.map(|[min, max]| ClimateRange { min, max });
        BiomeParameters {
            temperature,
            humidity,
            continentalness,
            erosion,
            depth,
            weirdness,
            offset: self.offset,
        }
    }
}

pub(super) fn rows() -> &'static [(BiomeId, BiomeParameters)] {
    static ROWS: OnceLock<Vec<(BiomeId, BiomeParameters)>> = OnceLock::new();
    ROWS.get_or_init(|| {
        #[derive(Deserialize)]
        struct Capture {
            minecraft: String,
            jar_sha256: String,
            rows: Vec<CapturedRow>,
        }
        let capture: Capture =
            serde_json::from_str(include_str!("../../data/biome_parameters_26_2.json"))
                .expect("bundled native climate parameters");
        assert_eq!(capture.minecraft, bcore_registry::MC_VERSION);
        assert_eq!(capture.jar_sha256, JAR_SHA256);
        capture
            .rows
            .iter()
            .map(|row| {
                (
                    id(&row.biome).expect("native biome resource key in BCore registry"),
                    row.parameters(),
                )
            })
            .collect()
    })
}
