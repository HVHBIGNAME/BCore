//! Explicit data inputs for retained historical component replays.
use crate::{biome, density, VanillaGraph};
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

pub(crate) fn assets() -> &'static BTreeMap<String, Value> {
    static ASSETS: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    ASSETS.get_or_init(|| {
        serde_json::from_str(include_str!("../data/vanilla_worldgen.json"))
            .expect("preserved 26.1 worldgen documents")
    })
}

pub(crate) fn feature_documents() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let value: Value =
            serde_json::from_str(include_str!("../data/base_feature_catalog_26_1.json")).unwrap();
        assert_eq!(value["minecraft"], "26.1");
        assert_eq!(value["state_count"], 29873);
        assert_eq!(
            value["jar_sha256"],
            crate::structure::template_pool::StructureAssets::HISTORICAL_JAR_SHA256
        );
        value
    })
}

pub(crate) fn parameters() -> biome::ParameterList {
    let capture: Value =
        serde_json::from_str(include_str!("../data/biome_parameters_26_1.json")).unwrap();
    assert_eq!(capture["minecraft"], "26.1");
    assert_eq!(
        capture["jar_sha256"],
        crate::structure::template_pool::StructureAssets::HISTORICAL_JAR_SHA256
    );
    let rows = capture["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 7593);
    biome::ParameterList::new(
        rows.iter()
            .map(|row| {
                let ranges: [[i64; 2]; 6] = serde_json::from_value(row["ranges"].clone()).unwrap();
                let [temperature, humidity, continentalness, erosion, depth, weirdness] =
                    ranges.map(|[min, max]| biome::ClimateRange { min, max });
                (
                    biome::id(row["biome"].as_str().unwrap()).unwrap(),
                    biome::BiomeParameters {
                        temperature,
                        humidity,
                        continentalness,
                        erosion,
                        depth,
                        weirdness,
                        offset: row["offset"].as_i64().unwrap(),
                    },
                )
            })
            .collect(),
    )
}

pub(crate) fn graph() -> VanillaGraph {
    let assets = assets();
    VanillaGraph::from_sources(
        &mut |path| {
            assets
                .get(path)
                .cloned()
                .ok_or_else(|| format!("missing historical worldgen document {path}"))
        },
        &mut |input| density::parse_historical_26_1_json(input, assets),
        parameters(),
    )
    .expect("complete historical 26.1 density and climate inputs")
}
