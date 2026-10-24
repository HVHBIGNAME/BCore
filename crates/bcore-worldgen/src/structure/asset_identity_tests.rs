use super::StructureAssets;
use crate::feature_world::FeatureError;
use crate::structure::template::{Nbt, TemplateBlockEntity};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn historical() -> &'static StructureAssets {
    static ASSETS: OnceLock<StructureAssets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        StructureAssets::from_historical_json(include_str!(
            "../../data/pool_alias_trial_combined_assets_26_1_v2.json"
        ))
        .unwrap()
    })
}

#[test]
fn historical_templates_keep_complete_states_defaults_and_factory_type_ids() {
    let assets = historical();
    assert_eq!(assets.blocks.len(), 29873);
    assert_eq!(assets.blocks.default_state("copper_ore").unwrap(), 25313);
    assert_eq!(assets.blocks.default_state("deepslate").unwrap(), 27924);
    assert_eq!(assets.blocks.state(27924).unwrap().properties["axis"], "y");
    assert_eq!(
        assets.blocks.block_entities["minecraft:bee_nest"].type_id,
        34
    );
    assert_eq!(
        assets.blocks.block_entities["minecraft:comparator"].type_id,
        19
    );
    let target = StructureAssets::bundled();
    assert_eq!(target.blocks.default_state("copper_ore").unwrap(), 27790);
    assert_eq!(target.blocks.default_state("deepslate").unwrap(), 30417);
    assert_eq!(
        target.blocks.block_entities["minecraft:bee_nest"].type_id,
        33
    );
}

#[test]
fn historical_template_item_decode_does_not_admit_target_only_items() {
    fn load(assets: &StructureAssets) -> TemplateBlockEntity {
        let item = Nbt::Compound(BTreeMap::from([
            ("id".into(), Nbt::String("minecraft:sulfur_spike".into())),
            ("count".into(), Nbt::Int(1)),
        ]));
        TemplateBlockEntity::from_load(
            &assets.blocks,
            assets.blocks.default_state("decorated_pot").unwrap(),
            (8, 65, 8),
            Nbt::Compound(BTreeMap::from([("item".into(), item)])),
        )
        .unwrap()
    }
    assert!(load(historical()).full_data().get("item").is_none());
    assert_eq!(
        load(StructureAssets::bundled())
            .full_data()
            .get("item")
            .unwrap()
            .get("id")
            .and_then(Nbt::string),
        Some("minecraft:sulfur_spike")
    );
}

#[test]
fn target_and_historical_loaders_reject_each_others_provenance() {
    let historical =
        json!({"minecraft": "26.1", "jar_sha256": StructureAssets::HISTORICAL_JAR_SHA256});
    assert!(matches!(
        StructureAssets::from_json(&historical.to_string()),
        Err(FeatureError::InvalidConfig(_))
    ));
    let target = json!({"minecraft": bcore_registry::MC_VERSION,
        "protocol": bcore_registry::PROTOCOL_VERSION, "jar_sha256": StructureAssets::JAR_SHA256});
    assert!(matches!(
        StructureAssets::from_historical_json(&target.to_string()),
        Err(FeatureError::InvalidConfig(_))
    ));
}
