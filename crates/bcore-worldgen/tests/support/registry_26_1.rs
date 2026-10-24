//! Explicit resource-identity translation for immutable 26.1 observations.
use bcore_worldgen::structure::template_pool::StructureAssets;
use std::{collections::BTreeMap, sync::OnceLock};

pub fn historical_assets() -> &'static StructureAssets {
    static ASSETS: OnceLock<StructureAssets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        StructureAssets::from_historical_json(include_str!(
            "../../data/pool_alias_trial_combined_assets_26_1_v2.json"
        ))
        .expect("authenticated historical structure registry")
    })
}

struct Mapping {
    target: Vec<u32>,
    historical: Vec<Option<u32>>,
}

fn mapping() -> &'static Mapping {
    static MAPPING: OnceLock<Mapping> = OnceLock::new();
    MAPPING.get_or_init(|| {
        let historical = &historical_assets().blocks;
        assert_eq!(historical.len(), 29873);
        let identities: BTreeMap<_, _> = bcore_registry::vanilla()
            .states
            .iter()
            .map(|state| ((state.name.as_str(), &state.properties), state.id))
            .collect();
        let mut target = Vec::with_capacity(historical.len());
        let mut reverse = vec![None; bcore_registry::BLOCK_STATE_COUNT];
        for old in 0..historical.len() as u32 {
            let state = historical.state(old).unwrap();
            let new = identities[&(state.name.as_str(), &state.properties)];
            assert!(reverse[new as usize].replace(old).is_none());
            target.push(new);
        }
        Mapping {
            target,
            historical: reverse,
        }
    })
}

pub fn target_state(historical: u32) -> u32 {
    mapping().target[historical as usize]
}

pub fn historical_state(target: u32) -> u32 {
    mapping().historical[target as usize].expect("target state has a historical resource identity")
}
