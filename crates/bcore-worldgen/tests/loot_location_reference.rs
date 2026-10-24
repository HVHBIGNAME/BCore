use bcore_worldgen::{
    block_entity::ChestInventory,
    loot::{CatalogSources, LegacyRandom, LootContext, LootEngine, LootError, LootWorld},
};
use serde_json::{json, Value};

#[path = "support/loot_location.rs"]
mod loot_location;

fn engine(condition: Value) -> LootEngine {
    LootEngine::from_sources(CatalogSources {
        tables: &json!({"test:location": {"pools": [{"rolls": 1, "entries": [{
            "type": "minecraft:item", "name": "minecraft:stone", "conditions": [condition]
        }]}]}})
        .to_string(),
        ..CatalogSources::VANILLA
    })
    .unwrap()
}

#[test]
fn all_twenty_native_location_checks_match_queries_results_and_rng_continuation() {
    let reference = loot_location::reference();
    for sample in reference["samples"].as_array().unwrap() {
        let mut condition = sample["condition"].clone();
        condition["condition"] = json!("minecraft:location_check");
        let engine = engine(condition.clone());
        let world = loot_location::ObservedWorld::from_sample(sample);
        let origin = serde_json::from_value(sample["origin"].clone()).unwrap();
        let context = LootContext::CHEST.with_location(origin, &world);
        let mut rng = LegacyRandom::new(17);
        assert_eq!(rng.state().to_string(), sample["rng_before"]["seed48"]);
        let result = engine
            .table("test:location")
            .unwrap()
            .generate_with_rng(context, &mut rng)
            .unwrap();
        assert_eq!(
            !result.is_empty(),
            sample["matched"].as_bool().unwrap(),
            "{sample}"
        );
        assert_eq!(rng.state().to_string(), sample["rng_after"]["seed48"]);
        assert_eq!(rng.next_long().to_string(), sample["next_i64"]);
        let expected: Vec<[i32; 3]> =
            if origin.is_some() && condition["predicate"].get("biomes").is_some() {
                vec![serde_json::from_value(sample["query_pos"].clone()).unwrap()]
            } else {
                Vec::new()
            };
        assert_eq!(*world.reads.borrow(), expected, "{sample}");
    }
}

#[test]
fn missing_world_support_is_preflighted_through_nested_tables_and_functions() {
    let condition = json!({"condition": "minecraft:location_check", "predicate": {"biomes": "minecraft:sulfur_caves"}});
    let tables = json!({
        "test:entry": {"pools": [{"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:stone", "conditions": [condition]}]}]},
        "test:parent": {"pools": [{"rolls": 1, "conditions": [{"condition": "minecraft:random_chance", "chance": 0}], "entries": [{"type": "minecraft:loot_table", "value": "test:entry"}]}]},
        "test:function": {"functions": [{"function": "minecraft:set_count", "count": 2, "conditions": [condition]}], "pools": [{"rolls": 0, "entries": []}]},
        "test:inline": {"pools": [{"rolls": 0, "entries": [{"type": "minecraft:loot_table", "value": {"pools": [{"rolls": 1, "conditions": [condition], "entries": []}]}}]}]}
    }).to_string();
    let engine = LootEngine::from_sources(CatalogSources {
        tables: &tables,
        ..CatalogSources::VANILLA
    })
    .unwrap();
    for name in ["test:entry", "test:parent", "test:function", "test:inline"] {
        let table = engine.table(name).unwrap();
        let mut rng = LegacyRandom::new(17);
        let before_rng = rng.state();
        let mut slots = vec![Some(engine.item_stack("stone", 3).unwrap()), None];
        let before_slots = slots.clone();
        assert_eq!(
            table.fill_with_rng(&mut slots, LootContext::CHEST, &mut rng),
            Err(LootError::UnsupportedContext("loot location lookup"))
        );
        assert_eq!(slots, before_slots);
        assert_eq!(rng.state(), before_rng);
        let mut chest =
            ChestInventory::with_slots(&engine, std::array::from_fn(|_| None), Some(name), 17)
                .unwrap();
        let before = chest.clone();
        assert_eq!(
            chest.unpack(&engine),
            Err(LootError::UnsupportedContext("loot location lookup"))
        );
        assert_eq!(chest, before);
    }
}

#[test]
fn unsupported_location_fields_invalid_holders_and_offsets_are_never_skipped() {
    for condition in [
        json!({"condition": "minecraft:location_check", "predicate": {"block": {}}}),
        json!({"condition": "minecraft:location_check", "predicate": {"biomes": "probe:missing"}}),
        json!({"condition": "minecraft:location_check", "offsetX": 0.5}),
        json!({"condition": "minecraft:location_check", "offsetY": "1"}),
        json!({"condition": "minecraft:location_check", "predicate": null}),
    ] {
        assert!(engine(condition).table("test:location").is_err());
    }
}

struct MissingWorld;
impl LootWorld for MissingWorld {
    fn biome_if_loaded(&self, _pos: [i32; 3]) -> bcore_worldgen::loot::Result<Option<u32>> {
        Err(LootError::UnsupportedContext("test biome unavailable"))
    }
}

#[test]
fn late_lookup_errors_keep_slots_and_commit_consumed_chest_keys() {
    let engine = engine(json!({"condition": "minecraft:all_of", "terms": [
        {"condition": "minecraft:random_chance", "chance": 1},
        {"condition": "minecraft:location_check", "predicate": {"biomes": "minecraft:sulfur_caves"}}
    ]}));
    let context = LootContext::CHEST.with_location(Some([8.5, 80.5, 8.5]), &MissingWorld);
    let mut chest = ChestInventory::with_slots(
        &engine,
        std::array::from_fn(|_| None),
        Some("test:location"),
        17,
    )
    .unwrap();
    let before_slots = chest.slots().clone();
    let mut rng = LegacyRandom::new(17);
    let before_rng = rng.state();
    assert_eq!(
        chest.unpack_with_rng(&engine, context, &mut rng),
        Err(LootError::UnsupportedContext("test biome unavailable"))
    );
    assert_eq!(chest.slots(), &before_slots);
    assert_eq!(chest.loot_table(), None);
    assert_eq!(chest.loot_seed(), 17);
    assert_ne!(rng.state(), before_rng);
    let after_rng = rng.state();
    assert_eq!(
        chest.unpack_with_rng(&engine, context, &mut rng).unwrap(),
        None
    );
    assert_eq!(rng.state(), after_rng);
}

#[test]
fn captured_fill_location_rejects_sulfur_membership_without_drawing() {
    let world = loot_location::fill_world();
    let context = loot_location::fill_context(&world);
    let engine = engine(
        json!({"condition": "minecraft:location_check", "predicate": {"biomes": "minecraft:sulfur_caves"}}),
    );
    let mut rng = LegacyRandom::new(17);
    let before = rng.state();
    assert!(engine
        .table("test:location")
        .unwrap()
        .generate_with_rng(context, &mut rng)
        .unwrap()
        .is_empty());
    assert_eq!(rng.state(), before);
}
