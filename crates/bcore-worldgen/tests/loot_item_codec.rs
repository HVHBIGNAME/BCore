//! Strict container-input admission and native typed persistence boundaries.
use bcore_worldgen::loot::LootEngine;
use serde_json::{json, Value};

#[test]
fn persistent_input_rejects_invalid_counts_and_unknown_registry_references() {
    let engine = LootEngine::vanilla().unwrap();
    assert!(engine.item_stack_from_json(&Value::Null).unwrap().is_none());
    for count in [json!(-1), json!(0), json!(100), json!(1.5), Value::Null] {
        assert!(engine
            .item_stack_from_json(&json!({"id":"stone","count":count}))
            .is_err());
    }
    assert_eq!(
        engine
            .item_stack_from_json(&json!({"id":"stone"}))
            .unwrap()
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        engine
            .item_stack_from_json(&json!({"id":"stone","count":99}))
            .unwrap()
            .unwrap()
            .count(),
        99
    );
    for value in [
        json!({"id":"air"}),
        json!({"id":"test:unknown"}),
        json!({"id":"book","components":{"stored_enchantments":{"test:unknown":1}}}),
        json!({"id":"potion","components":{"potion_contents":{"potion":"test:unknown"}}}),
        json!({"id":"goat_horn","components":{"instrument":"test:unknown"}}),
        json!({"id":"suspicious_stew","components":{"suspicious_stew_effects":[{"id":"test:unknown"}]}}),
        json!({"id":"stone","components":{"damage":-1}}),
        json!({"id":"ominous_bottle","components":{"ominous_bottle_amplifier":5}}),
        json!({"id":"book","components":{"stored_enchantments":{"wind_burst":256}}}),
        json!({"id":"stone","components":{"enchantments":{},"minecraft:enchantments":{}}}),
        json!({"id":"book","components":{"stored_enchantments":{"wind_burst":1,"minecraft:wind_burst":2}}}),
    ] {
        assert!(
            engine.item_stack_from_json(&value).is_err(),
            "accepted {value}"
        );
    }
}

#[test]
fn persistent_input_normalizes_ids_and_elides_default_values() {
    let engine = LootEngine::vanilla().unwrap();
    let stack = engine
        .item_stack_from_json(&json!({
            "id":"enchanted_book","components":{"stored_enchantments":{"wind_burst":1}}
        }))
        .unwrap()
        .unwrap();
    assert_eq!(
        stack.to_json(),
        json!({"id":"minecraft:enchanted_book","count":1,
        "components":{"minecraft:stored_enchantments":{"minecraft:wind_burst":1}}})
    );
    assert_eq!(
        stack.to_nbt().unwrap().typed_json(),
        json!([10, {
            "id":[8,"minecraft:enchanted_book"], "count":[3,1],
            "components":[10,{"minecraft:stored_enchantments":[10,{"minecraft:wind_burst":[3,1]}]}]
        }])
    );
    let stew = engine
        .item_stack_from_json(&json!({"id":"suspicious_stew",
            "components":{"suspicious_stew_effects":[{"id":"night_vision","duration":160}]}
        }))
        .unwrap()
        .unwrap();
    assert_eq!(
        stew.component("minecraft:suspicious_stew_effects"),
        Some(&json!([{"id":"minecraft:night_vision"}]))
    );
    let bottle = engine
        .item_stack_from_json(&json!({"id":"ominous_bottle",
            "components":{"ominous_bottle_amplifier":0}
        }))
        .unwrap()
        .unwrap();
    assert!(bottle.persistent_patch().is_empty());
    assert_eq!(
        bottle.component("minecraft:ominous_bottle_amplifier"),
        Some(&json!(0))
    );
}

#[test]
fn catalog_validated_component_mutation_is_atomic_and_copy_on_write() {
    let engine = LootEngine::vanilla().unwrap();
    let mut potion = engine.item_stack("potion", 1).unwrap();
    let original = potion.clone();
    assert!(engine
        .set_component(
            &mut potion,
            "potion_contents",
            json!({"potion":"test:unknown"})
        )
        .is_err());
    assert_eq!(potion, original);
    engine
        .set_component(&mut potion, "potion_contents", json!({"potion":"healing"}))
        .unwrap();
    assert_eq!(
        potion.component("minecraft:potion_contents"),
        Some(&json!({"potion":"minecraft:healing"}))
    );
    assert_eq!(
        original.component("minecraft:potion_contents"),
        Some(&json!({}))
    );
    assert!(potion.to_nbt().is_ok());
    assert!(engine
        .set_component(&mut potion, "!minecraft:potion_contents", json!({}))
        .is_err());
}

#[test]
fn transient_counts_are_not_silently_persisted_outside_the_native_codec_range() {
    let engine = LootEngine::vanilla().unwrap();
    for count in [-1, 0, 100, i32::MAX] {
        assert!(engine.item_stack("stone", count).unwrap().to_nbt().is_err());
    }
    for count in [1, 64, 99] {
        assert!(engine.item_stack("stone", count).unwrap().to_nbt().is_ok());
    }
}
