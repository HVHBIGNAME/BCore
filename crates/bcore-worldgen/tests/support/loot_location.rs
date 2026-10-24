use bcore_worldgen::loot::{LootContext, LootWorld, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{cell::RefCell, sync::OnceLock};

pub fn reference() -> &'static Value {
    static REFERENCE: OnceLock<Value> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        let text = include_str!("../data/loot_26_2/loot_location_26_2.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            "c6b21d03e2837378ba361bf95fed6609d352090c3b09b0af7f61663d950bcfac"
        );
        let reference: Value = serde_json::from_str(text).unwrap();
        assert_eq!(reference["schema"], "bcore.native-loot-location.v1");
        assert_eq!(reference["minecraft"], bcore_registry::MC_VERSION);
        assert_eq!(reference["protocol"], bcore_registry::PROTOCOL_VERSION);
        assert_eq!(reference["data_version"], bcore_registry::DATA_VERSION);
        assert_eq!(reference["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
        assert_eq!(reference["gameplay_ticks"], 0);
        assert_eq!(reference["samples"].as_array().unwrap().len(), 20);
        for (name, source) in [
            ("scripts/loot-reference/LootAgent.java", &include_bytes!("../../../../scripts/loot-reference/LootAgent.java")[..]),
            ("scripts/loot-reference/LootProbe.java", &include_bytes!("../../../../scripts/loot-reference/LootProbe.java")[..]),
            ("scripts/native-generation-reference/NativeAccess.java", &include_bytes!("../../../../scripts/native-generation-reference/NativeAccess.java")[..]),
            ("scripts/migration-26.2/LootCatalogReference.java", &include_bytes!("../../../../scripts/migration-26.2/LootCatalogReference.java")[..]),
            ("scripts/migration-26.2/LootLocationAgent.java", &include_bytes!("../../../../scripts/migration-26.2/LootLocationAgent.java")[..]),
            ("scripts/migration-26.2/LootLocationReference.java", &include_bytes!("../../../../scripts/migration-26.2/LootLocationReference.java")[..]),
        ] {
            assert_eq!(format!("{:x}", Sha256::digest(source)), reference["source_sha256"][name], "native source: {name}");
        }
        reference
    })
}

pub struct ObservedWorld {
    query: Option<[i32; 3]>,
    biome: Option<u32>,
    pub reads: RefCell<Vec<[i32; 3]>>,
}

impl ObservedWorld {
    pub fn from_sample(sample: &Value) -> Self {
        Self {
            query: sample
                .get("query_pos")
                .map(|value| serde_json::from_value(value.clone()).unwrap()),
            biome: sample["biome"]
                .as_str()
                .map(|name| bcore_registry::registry_id("BIOME", name).unwrap()),
            reads: RefCell::default(),
        }
    }
}

impl LootWorld for ObservedWorld {
    fn biome_if_loaded(&self, pos: [i32; 3]) -> Result<Option<u32>> {
        assert_eq!(Some(pos), self.query, "location query differs from native");
        self.reads.borrow_mut().push(pos);
        Ok(self.biome)
    }
}

pub fn fill_world() -> ObservedWorld {
    let samples: Vec<_> = reference()["samples"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|sample| sample["name"] == "native_fill_context")
        .collect();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0]["loaded"], true);
    assert_eq!(samples[0]["origin"], reference()["fill_context"]["origin"]);
    ObservedWorld::from_sample(samples[0])
}

pub fn fill_context(world: &ObservedWorld) -> LootContext<'_> {
    LootContext::CHEST.with_location(
        Some(serde_json::from_value(reference()["fill_context"]["origin"].clone()).unwrap()),
        world,
    )
}
