use super::check_configured;
use crate::feature_world::FeatureError;
use serde_json::json;

#[test]
fn selector_preflight_visits_zero_weight_and_later_sequence_children() {
    let supported = json!({
        "feature": {"type": "minecraft:freeze_top_layer", "config": {}}, "placement": []
    });
    let unavailable = json!({
        "feature": {"type": "minecraft:template", "config": {"templates": [
            {"data": {"id": "minecraft:spring/missing_native_template"}, "weight": 1}
        ]}}, "placement": []
    });
    for document in [
        json!({"type": "minecraft:weighted_random_selector", "config": {"features": [
            {"weight": 1, "data": supported}, {"weight": 0, "data": unavailable}
        ]}}),
        json!({"type": "minecraft:sequence", "config": {"features": [supported, unavailable]}}),
    ] {
        assert_eq!(
            check_configured(&document, None, 0),
            Err(FeatureError::MissingData(
                "structure template minecraft:spring/missing_native_template".into()
            ))
        );
    }
}

#[test]
fn speleothem_families_keep_their_explicit_kernel_identity() {
    for kind in ["speleothem", "speleothem_cluster"] {
        assert!(matches!(
            check_configured(
                &json!({"type": format!("minecraft:{kind}"), "config": {}}),
                None,
                0
            ),
            Err(FeatureError::InvalidConfig(_))
        ));
    }
    let configured = crate::block_predicate::catalog();
    check_configured(configured.configured("dripstone_cluster").unwrap(), None, 0).unwrap();
    check_configured(configured.configured("pointed_dripstone").unwrap(), None, 0).unwrap();
}

#[test]
fn registered_tree_preflight_reads_the_target_definition_and_rejects_schema_overrides() {
    let document = crate::block_predicate::catalog().configured("oak").unwrap();
    check_configured(document, Some("minecraft:oak"), 0).unwrap();
    let mut changed = document.clone();
    changed["config"]["minimum_size"]["upper_size"] = json!(16);
    assert!(matches!(
        check_configured(&changed, Some("minecraft:oak"), 0),
        Err(FeatureError::InvalidConfig(_))
    ));
}
