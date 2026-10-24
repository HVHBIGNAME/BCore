//! Historical 26.1 saved chest records plus current strict-admission/error regressions.
//! Custom catalogs below exercise boundaries, not additional native authority.
use bcore_worldgen::block_entity::ChestInventory;
use bcore_worldgen::loot::{
    CatalogSources, FillOutcome, ItemStack, LegacyRandom, LootContext, LootEngine, LootError,
    LootRng,
};
use bcore_worldgen::structure::template::Nbt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, collections::BTreeSet, sync::OnceLock};

#[path = "support/loot_nbt.rs"]
mod loot_nbt;

#[path = "support/loot_location.rs"]
mod loot_location;

fn cases() -> &'static [Value] {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let text = include_str!("data/loot_26_1/loot-fill-26.1.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            "2158fd855f1bbc46d6b399620c9ced35822263c982151686f58e175c1ccf9db1"
        );
        let fixture: Value = serde_json::from_str(text).unwrap();
        assert_eq!(fixture["schema"], "bcore.native-loot-fill.v1");
        assert_eq!(fixture["minecraft"], "26.1");
        assert_eq!(fixture["protocol"], 775);
        assert_eq!(fixture["cases"].as_array().unwrap().len(), 137);
        fixture
    })["cases"]
        .as_array()
        .unwrap()
}

fn actual_chests() -> impl Iterator<Item = &'static Value> {
    cases().iter().filter(|case| {
        if case["authority"]["actual_chest_unpack_equal"] == true {
            assert_eq!(case["container_size"], 27);
            assert!(case["unpack"].is_object());
            true
        } else {
            false
        }
    })
}

fn historical_engine() -> &'static LootEngine {
    static ENGINE: OnceLock<LootEngine> = OnceLock::new();
    ENGINE.get_or_init(|| LootEngine::from_sources(CatalogSources::VANILLA_26_1).unwrap())
}

fn preset(case: &Value) -> bool {
    case["preexisting_slots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| !v.is_null())
}

fn saved(case: &Value, phase: &str) -> Nbt {
    loot_nbt::decode(case["unpack"][phase]["base64"].as_str().unwrap())
}

fn seed(case: &Value) -> i64 {
    case["seed_i64"].as_str().unwrap().parse().unwrap()
}

fn slots_json(inventory: &ChestInventory) -> Value {
    Value::Array(
        inventory
            .slots()
            .iter()
            .map(|stack| stack.as_ref().map_or(Value::Null, ItemStack::to_json))
            .collect(),
    )
}

struct NoDraw;
impl LootRng for NoDraw {
    fn next_int(&mut self, _: i32) -> i32 {
        panic!("no RNG draw allowed")
    }
    fn next_float(&mut self) -> f32 {
        panic!("no RNG draw allowed")
    }
    fn next_bool(&mut self) -> bool {
        panic!("no RNG draw allowed")
    }
}

fn assert_noop(engine: &LootEngine, inventory: &mut ChestInventory) {
    let before = inventory.clone();
    assert_eq!(inventory.unpack(engine).unwrap(), None);
    assert_eq!(
        inventory
            .unpack_with_rng(
                engine,
                LootContext {
                    luck: f32::NAN,
                    player_present: true,
                    ..LootContext::CHEST
                },
                &mut NoDraw,
            )
            .unwrap(),
        None
    );
    assert_eq!(*inventory, before);
}

fn assert_native_result(
    engine: &LootEngine,
    case: &Value,
    inventory: &mut ChestInventory,
    outcome: FillOutcome,
) {
    let id = case["id"].as_str().unwrap();
    assert_eq!(inventory.loot_table(), None, "{id}");
    assert_eq!(
        inventory.loot_seed().to_string(),
        case["unpack"]["loot_seed_after"],
        "{id}"
    );
    assert_eq!(case["unpack"]["loot_table_after"], Value::Null);
    assert_eq!(slots_json(inventory), case["expected_slots"], "{id}");
    let placed = case["placement_order"].as_array().unwrap().len();
    assert_eq!(outcome.placed_stacks, placed, "{id}");
    assert_eq!(
        outcome.discarded_stacks,
        case["shuffled_split_stacks"].as_array().unwrap().len() - placed,
        "{id}"
    );
    let mut tag = saved(case, "saved_before");
    inventory.write_nbt(&mut tag).unwrap();
    assert_eq!(tag, saved(case, "saved_after"), "{id}: native saved NBT");
    assert_eq!(case["unpack"]["second_unpack_noop"], true);
    assert_noop(engine, inventory);
    inventory.write_nbt(&mut tag).unwrap();
    assert_eq!(tag, saved(case, "saved_after"), "{id}: repeat save");
    let reloaded = ChestInventory::from_nbt(engine, &tag).unwrap();
    assert_eq!(reloaded.slots(), inventory.slots(), "{id}: reload slots");
    assert_eq!(reloaded.loot_seed(), 0, "{id}: saved seed is absent");
}

#[test]
fn native_deferred_chests_match_before_after_nbt_and_repeat_unpack() {
    let engine = historical_engine();
    let mut count = 0;
    let mut seeds = BTreeSet::new();
    for case in actual_chests().filter(|case| !preset(case)) {
        let before = saved(case, "saved_before");
        let mut inventory = ChestInventory::from_nbt(engine, &before).unwrap();
        assert!(inventory.slots().iter().all(Option::is_none));
        assert_eq!(inventory.loot_table(), case["table"].as_str());
        assert_eq!(inventory.loot_seed(), seed(case));
        let mut round_trip = before.clone();
        inventory.write_nbt(&mut round_trip).unwrap();
        assert_eq!(round_trip, before);
        let outcome = inventory.unpack(engine).unwrap().unwrap();
        assert_native_result(engine, case, &mut inventory, outcome);
        seeds.insert(seed(case));
        count += 1;
    }
    assert_eq!(count, 127);
    assert!(seeds.contains(&i64::MIN) && seeds.contains(&i64::MAX));
    println!("127 native deferred chests: before/after typed NBT, i64 seed endpoints, reload and repeat unpack match");
}

#[test]
fn native_preexisting_backing_slots_are_filled_without_merging_or_reloading() {
    let engine = historical_engine();
    let mut count = 0;
    for case in actual_chests().filter(|case| preset(case)) {
        let backing = std::array::from_fn(|slot| {
            engine
                .item_stack_from_json(&case["preexisting_slots"][slot])
                .unwrap()
        });
        let mut inventory =
            ChestInventory::with_slots(engine, backing.clone(), case["table"].as_str(), seed(case))
                .unwrap();
        let before = saved(case, "saved_before");
        let mut round_trip = before.clone();
        inventory.write_nbt(&mut round_trip).unwrap();
        assert_eq!(round_trip, before);
        // The native probe installed backing items with setters before setting
        // the loot key. Deferred saved NBT intentionally cannot encode them.
        let loaded = ChestInventory::from_nbt(engine, &before).unwrap();
        assert!(loaded.slots().iter().all(Option::is_none));
        let mut seeded = inventory.clone();
        let mut rng = LegacyRandom::new(seed(case));
        let outcome = inventory
            .unpack_with_rng(engine, LootContext::CHEST, &mut rng)
            .unwrap()
            .unwrap();
        assert_eq!(rng.state().to_string(), case["rng_after_fill"]["seed48"]);
        for (slot, previous) in backing.iter().enumerate() {
            if previous.is_some() {
                assert_eq!(&inventory.slots()[slot], previous);
            }
        }
        assert_native_result(engine, case, &mut inventory, outcome);
        let outcome = seeded.unpack(engine).unwrap().unwrap();
        assert_native_result(engine, case, &mut seeded, outcome);
        assert_eq!(inventory, seeded);
        count += 1;
    }
    assert_eq!(count, 7);
    println!("7 native preset-backing chests: occupied slots, full/one-empty capacity, both source APIs and typed saves match");
}

#[test]
fn native_saved_items_round_trip_all_supported_components_and_types() {
    let engine = historical_engine();
    let mut components = BTreeSet::new();
    let mut count = 0;
    let mut occupied = 0;
    for case in actual_chests() {
        let expected = saved(case, "saved_after");
        let mut inventory = ChestInventory::from_nbt(engine, &expected).unwrap();
        assert_eq!(inventory.loot_table(), None);
        assert_eq!(slots_json(&inventory), case["expected_slots"]);
        for stack in inventory.slots().iter().flatten() {
            components.extend(stack.persistent_patch().keys().cloned());
            occupied += 1;
        }
        let mut actual = expected.clone();
        inventory.write_nbt(&mut actual).unwrap();
        assert_eq!(actual, expected, "{}", case["id"]);
        assert_noop(engine, &mut inventory);
        count += 1;
    }
    assert_eq!(count, 134);
    assert_eq!(
        components,
        [
            "minecraft:damage",
            "minecraft:enchantments",
            "minecraft:stored_enchantments",
            "minecraft:potion_contents",
            "minecraft:instrument",
            "minecraft:suspicious_stew_effects",
            "minecraft:ominous_bottle_amplifier",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    println!("134 native materialized chest saves: {occupied} occupied slots and all seven supported component patch families round-trip with exact NBT types");
}

fn lifecycle_fixture() -> &'static Value {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let text = include_str!("data/loot_26_1/chest-lifecycle-26.1.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            "4b0bc865d02713dc62b50b2d5a9a6c28c742ee54af64e88461965e98ed1de8b0"
        );
        let fixture: Value = serde_json::from_str(text).unwrap();
        assert_eq!(fixture["schema"], "bcore.native-chest-lifecycle.v1");
        assert_eq!(fixture["minecraft"], "26.1");
        assert_eq!(fixture["protocol"], 775);
        assert_eq!(fixture["player"], Value::Null);
        assert_eq!(fixture["gameplay_ticks"], 0);
        assert_eq!(fixture["menu_creation_exercised"], false);
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 5);
        let ids: BTreeSet<_> = cases
            .iter()
            .map(|case| case["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            BTreeSet::from([
                "deferred-dungeon-reopen",
                "deferred-book-reopen",
                "preset-component-items",
                "table-and-items",
                "missing-table-reopen",
            ])
        );
        let blobs = fixture["nbt_blobs"].as_object().unwrap();
        assert_eq!(blobs.len(), 9);
        for (digest, blob) in blobs {
            let raw = loot_nbt::decode(blob["base64"].as_str().unwrap());
            let typed: Nbt = serde_json::from_value(blob["typed"].clone()).unwrap();
            assert_eq!(raw, typed, "native raw/typed NBT disagree: {digest}");
        }
        assert_eq!(fixture["component_maps"].as_object().unwrap().len(), 12);
        fixture
    })
}

fn lifecycle_nbt(fixture: &Value, digest: &Value) -> Nbt {
    let blob = &fixture["nbt_blobs"][digest.as_str().unwrap()];
    loot_nbt::decode(blob["base64"].as_str().unwrap())
}

fn lifecycle_operation<'a>(case: &'a Value, name: &str) -> &'a Value {
    case["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["operation"] == name)
        .unwrap()
}

fn native_calls(operation: &Value, prefix: &str) -> u64 {
    operation["method_entries"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(method, _)| method.starts_with(prefix))
        .map(|(_, count)| count.as_u64().unwrap())
        .sum()
}

fn assert_native_quiet_operation(operation: &Value) {
    let name = operation["operation"].as_str().unwrap();
    for field in ["fill_seeds_i64", "new_contexts", "table_resolutions"] {
        assert!(
            operation[field].as_array().unwrap().is_empty(),
            "{name}: {field}"
        );
    }
    assert_eq!(
        operation["loot_rng_states_before"], operation["loot_rng_states_after"],
        "{name}"
    );
    for prefix in [
        "util/RandomSource.",
        "world/level/levelgen/",
        "world/level/storage/loot/LootContext$Builder.create(",
        "world/level/storage/loot/LootTable.fill(",
    ] {
        assert_eq!(native_calls(operation, prefix), 0, "{name}: {prefix}");
    }
}

fn assert_lifecycle_snapshot(
    fixture: &Value,
    case: &Value,
    boundary: &str,
    inventory: &ChestInventory,
    save_target: &mut Nbt,
) {
    let snapshot = &case["snapshots"][boundary];
    let label = format!("{}/{boundary}", case["id"]);
    assert_eq!(
        snapshot["native_level_attached"], true,
        "{label}: native authority"
    );
    assert_eq!(snapshot["slots"].as_array().unwrap().len(), 27);
    assert_eq!(
        inventory.loot_table(),
        snapshot["loot_table"].as_str(),
        "{label}"
    );
    assert_eq!(
        inventory.loot_seed().to_string(),
        snapshot["loot_seed_i64"],
        "{label}"
    );
    assert_eq!(slots_json(inventory), snapshot["slots"], "{label}");
    for (slot, stack) in inventory.slots().iter().enumerate() {
        if let Some(stack) = stack {
            let serialized = stack.to_json();
            let native = fixture["component_maps"]
                .as_object()
                .unwrap()
                .values()
                .find(|entry| entry["stack"] == serialized)
                .unwrap_or_else(|| panic!("{label}: missing native component map for slot {slot}"));
            let effective: BTreeMap<_, _> = stack
                .effective_components()
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect();
            assert_eq!(
                serde_json::to_value(effective).unwrap(),
                native["effective_components"],
                "{label}: slot {slot} effective components"
            );
        }
    }
    inventory.write_nbt(save_target).unwrap();
    assert_eq!(
        *save_target,
        lifecycle_nbt(fixture, &snapshot["saved_nbt_sha256"]),
        "{label}"
    );
}

fn verify_lifecycle(case_id: &str) {
    let engine = historical_engine();
    let fixture = lifecycle_fixture();
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == case_id)
        .unwrap();
    assert_eq!(case["snapshots"].as_object().unwrap().len(), 6);
    let operations = case["operations"].as_array().unwrap();
    assert_eq!(
        operations
            .iter()
            .map(|op| op["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "load_initial",
            "save/loaded_before_unpack",
            "first_unpack_null",
            "save/after_first_unpack",
            "second_unpack_null",
            "save/after_second_unpack",
            "load_saved_materialized_into_fresh_chest",
            "save/fresh_loaded_materialized",
            "reopened_unpack_null",
            "save/after_reopened_unpack",
            "reopened_getItem_all_slots",
            "save/after_reopened_getItem",
        ]
    );
    let initial = &case["snapshots"]["loaded_before_unpack"];
    let pending = initial["loot_table"].is_string();
    for operation in operations {
        if operation["operation"] != "first_unpack_null" || !pending {
            assert_native_quiet_operation(operation);
        }
    }
    assert_eq!(
        native_calls(
            lifecycle_operation(case, "load_initial"),
            "world/ContainerHelper.loadAllItems("
        ),
        u64::from(!pending)
    );
    assert_eq!(
        native_calls(
            lifecycle_operation(case, "load_saved_materialized_into_fresh_chest"),
            "world/ContainerHelper.loadAllItems("
        ),
        1
    );

    let input = lifecycle_nbt(fixture, &case["input_nbt_sha256"]);
    let mut inventory = ChestInventory::from_nbt(engine, &input).unwrap();
    // The native owner adds base metadata (notably an empty components compound
    // for the preset SNBT input). Supply only that base frame; erase every owned
    // inventory field so write_nbt must regenerate all three from the value.
    let mut save_target = lifecycle_nbt(fixture, &initial["saved_nbt_sha256"]);
    for key in ["LootTable", "LootTableSeed", "Items"] {
        save_target.compound_mut().unwrap().remove(key);
    }
    for (key, value) in input.compound().unwrap() {
        if !["LootTable", "LootTableSeed", "Items"].contains(&key.as_str()) {
            assert_eq!(
                save_target.get(key),
                Some(value),
                "{case_id}: native base field {key}"
            );
        }
    }
    assert_lifecycle_snapshot(
        fixture,
        case,
        "loaded_before_unpack",
        &inventory,
        &mut save_target,
    );
    if case_id == "table-and-items" {
        assert_eq!(input.get("Items").unwrap().list().unwrap().len(), 3);
        assert!(inventory.slots().iter().all(Option::is_none));
    }

    let mut caller_inventory = inventory.clone();
    let mut rng = LegacyRandom::new(inventory.loot_seed());
    let initial_rng_state = rng.state();
    let outcome = inventory.unpack(engine).unwrap();
    let caller_outcome = caller_inventory
        .unpack_with_rng(engine, LootContext::CHEST, &mut rng)
        .unwrap();
    assert_eq!(outcome.is_some(), pending);
    assert_eq!(caller_outcome, outcome);
    assert_eq!(caller_inventory, inventory);
    let first = lifecycle_operation(case, "first_unpack_null");
    assert_eq!(
        native_calls(first, "world/level/storage/loot/LootTable.fill("),
        u64::from(pending)
    );
    if pending {
        assert_eq!(first["fill_seeds_i64"], json!([initial["loot_seed_i64"]]));
        let contexts = first["new_contexts"].as_array().unwrap();
        assert_eq!(contexts.len(), 1);
        let context = &contexts[0];
        assert_eq!(context["luck_f32_bits"], "0");
        assert_eq!(context["this_entity"], Value::Null);
        assert_eq!(context["same_server_level"], true);
        assert_eq!(initial_rng_state.to_string(), context["random"]["seed48"]);
        let index = context["rng_index"].as_u64().unwrap() as usize;
        assert_eq!(
            rng.state().to_string(),
            first["loot_rng_states_after"][index]["seed48"]
        );
        assert_eq!(
            first["table_resolutions"][0]["is_native_empty"],
            case_id == "missing-table-reopen"
        );
    } else {
        assert_eq!(rng.state(), initial_rng_state);
    }
    assert_lifecycle_snapshot(
        fixture,
        case,
        "after_first_unpack",
        &inventory,
        &mut save_target,
    );

    let final_rng_state = rng.state();
    assert_noop(engine, &mut inventory);
    assert_eq!(
        inventory
            .unpack_with_rng(engine, LootContext::CHEST, &mut rng)
            .unwrap(),
        None
    );
    assert_eq!(rng.state(), final_rng_state);
    assert_lifecycle_snapshot(
        fixture,
        case,
        "after_second_unpack",
        &inventory,
        &mut save_target,
    );

    // Reload the NBT actually written above, not an independently prefilled
    // expected inventory. The retained runtime seed is intentionally not restored.
    let mut fresh = ChestInventory::from_nbt(engine, &save_target).unwrap();
    assert_eq!(fresh.loot_seed(), 0);
    assert_lifecycle_snapshot(
        fixture,
        case,
        "fresh_loaded_materialized",
        &fresh,
        &mut save_target,
    );
    assert_noop(engine, &mut fresh);
    assert_eq!(
        fresh
            .unpack_with_rng(engine, LootContext::CHEST, &mut rng)
            .unwrap(),
        None
    );
    assert_eq!(rng.state(), final_rng_state);
    assert_lifecycle_snapshot(
        fixture,
        case,
        "after_reopened_unpack",
        &fresh,
        &mut save_target,
    );

    let reopened = fresh.clone();
    let expected = &case["snapshots"]["after_reopened_getItem"]["slots"];
    for slot in 0..27 {
        assert_eq!(
            fresh
                .unpack_with_rng(engine, LootContext::CHEST, &mut NoDraw)
                .unwrap(),
            None
        );
        assert_eq!(
            fresh.slots()[slot]
                .as_ref()
                .map_or(Value::Null, ItemStack::to_json),
            expected[slot]
        );
    }
    assert_eq!(fresh, reopened);
    assert_lifecycle_snapshot(
        fixture,
        case,
        "after_reopened_getItem",
        &fresh,
        &mut save_target,
    );
    assert_eq!(rng.state(), final_rng_state);
    println!("{case_id}: all six native lifecycle snapshots, effective components, typed/raw NBT and no-RNG reopen checks match");
}

#[test]
fn native_lifecycle_deferred_dungeon_reopens_without_loot() {
    verify_lifecycle("deferred-dungeon-reopen");
}

#[test]
fn native_lifecycle_negative_seed_book_retains_components() {
    verify_lifecycle("deferred-book-reopen");
}

#[test]
fn native_lifecycle_preset_component_items_never_unpack() {
    verify_lifecycle("preset-component-items");
}

#[test]
fn native_lifecycle_table_overrides_conflicting_items() {
    verify_lifecycle("table-and-items");
}

#[test]
fn native_lifecycle_missing_table_reopens_empty() {
    verify_lifecycle("missing-table-reopen");
}

fn compound(fields: impl IntoIterator<Item = (&'static str, Nbt)>) -> Nbt {
    Nbt::Compound(
        fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

fn pending(key: &str, seed: Option<Nbt>) -> Nbt {
    let mut tag = compound([
        ("id", Nbt::String("minecraft:chest".into())),
        ("LootTable", Nbt::String(key.into())),
    ]);
    if let Some(seed) = seed {
        tag.compound_mut()
            .unwrap()
            .insert("LootTableSeed".into(), seed);
    }
    tag
}

fn stone(slot: Nbt, count: Nbt) -> Nbt {
    compound([
        ("id", Nbt::String("minecraft:stone".into())),
        ("count", count),
        ("Slot", slot),
    ])
}

fn materialized(values: Vec<Nbt>) -> Nbt {
    compound([
        ("id", Nbt::String("minecraft:chest".into())),
        (
            "Items",
            Nbt::List {
                element_type: if values.is_empty() { 0 } else { 10 },
                values,
            },
        ),
    ])
}

#[test]
fn pending_load_skips_items_and_write_preserves_other_base_fields() {
    let engine = LootEngine::vanilla().unwrap();
    let mut tag = pending("chests/simple_dungeon", None);
    let fields = tag.compound_mut().unwrap();
    fields.insert(
        "Items".into(),
        Nbt::String("ignored only while a valid key exists".into()),
    );
    let base = BTreeMap::from([
        ("id".into(), Nbt::String("minecraft:chest".into())),
        ("x".into(), Nbt::Int(-31)),
        ("y".into(), Nbt::Int(80)),
        ("z".into(), Nbt::Int(5)),
        ("CustomName".into(), Nbt::String("Kept".into())),
        (
            "opaque".into(),
            compound([
                ("byte", Nbt::Byte(-1)),
                ("short", Nbt::Short(256)),
                ("long", Nbt::Long(i64::MAX)),
                ("float", Nbt::Float(0.25)),
                ("double", Nbt::Double(1.5)),
                ("bytes", Nbt::ByteArray(vec![-1, 2])),
            ]),
        ),
        ("components".into(), Nbt::empty_compound()),
    ]);
    fields.extend(base.clone());
    let inventory = ChestInventory::from_nbt(engine, &tag).unwrap();
    assert!(inventory.slots().iter().all(Option::is_none));
    assert_eq!(
        inventory.loot_table(),
        Some("minecraft:chests/simple_dungeon")
    );
    assert_eq!(inventory.loot_seed(), 0);
    inventory.write_nbt(&mut tag).unwrap();
    let mut expected = base.clone();
    expected.insert(
        "LootTable".into(),
        Nbt::String("minecraft:chests/simple_dungeon".into()),
    );
    assert_eq!(tag, Nbt::Compound(expected));
    let filled =
        ChestInventory::with_slots(engine, std::array::from_fn(|_| None), None, i64::MIN).unwrap();
    filled.write_nbt(&mut tag).unwrap();
    let mut expected = base;
    expected.insert(
        "Items".into(),
        Nbt::List {
            element_type: 0,
            values: Vec::new(),
        },
    );
    assert_eq!(tag, Nbt::Compound(expected));
}

#[test]
fn seed_load_runtime_retention_and_native_saved_representation_are_distinct() {
    let engine = LootEngine::vanilla().unwrap();
    for seed in [i64::MIN, -1, 0, 1, i64::MAX] {
        let mut tag = materialized(Vec::new());
        tag.compound_mut()
            .unwrap()
            .insert("LootTableSeed".into(), Nbt::Long(seed));
        // RandomizableContainer.tryLoadLootTable bytecode reads the seed even
        // without a key. Native materialized saving subsequently omits it.
        let mut loaded = ChestInventory::from_nbt(engine, &tag).unwrap();
        assert_eq!(loaded.loot_seed(), seed);
        assert_noop(engine, &mut loaded);
        loaded.write_nbt(&mut tag).unwrap();
        assert!(tag.get("LootTableSeed").is_none());
        assert_eq!(
            ChestInventory::from_nbt(engine, &tag).unwrap().loot_seed(),
            0
        );
        let runtime =
            ChestInventory::with_slots(engine, loaded.slots().clone(), None, seed).unwrap();
        assert_eq!(runtime.loot_seed(), seed);
        runtime.write_nbt(&mut tag).unwrap();
        assert!(tag.get("LootTableSeed").is_none());
    }
    let missing =
        ChestInventory::from_nbt(engine, &pending("chests/simple_dungeon", None)).unwrap();
    assert_eq!(missing.loot_seed(), 0);
    for value in [
        Nbt::Byte(1),
        Nbt::Int(1),
        Nbt::String("1".into()),
        Nbt::Double(1.0),
    ] {
        assert!(
            ChestInventory::from_nbt(engine, &pending("chests/simple_dungeon", Some(value)))
                .is_err()
        );
    }
}

#[test]
fn unsupported_table_context_and_zero_seed_preflight_preserve_backing_and_key() {
    let engine = LootEngine::vanilla().unwrap();
    for key in [
        "chests/shipwreck_map",
        "chests/underwater_ruin_big",
        "chests/underwater_ruin_small",
    ] {
        let tag = pending(key, Some(Nbt::Long(41)));
        let mut inventory = ChestInventory::from_nbt(engine, &tag).unwrap();
        let before = inventory.clone();
        assert!(matches!(
            inventory.unpack(engine),
            Err(LootError::Unsupported { .. })
        ));
        assert!(matches!(
            inventory.unpack_with_rng(engine, LootContext::CHEST, &mut NoDraw),
            Err(LootError::Unsupported { .. })
        ));
        assert_eq!(inventory, before);
    }
    let mut backing = std::array::from_fn(|_| None);
    backing[4] = Some(engine.item_stack("diamond", 7).unwrap());
    let mut inventory =
        ChestInventory::with_slots(engine, backing, Some("chests/simple_dungeon"), 0).unwrap();
    let before = inventory.clone();
    assert_eq!(
        inventory.unpack(engine),
        Err(LootError::ZeroSeedRequiresCallerRng)
    );
    for context in [
        LootContext {
            luck: 1.0,
            player_present: false,
            ..LootContext::CHEST
        },
        LootContext {
            luck: -0.0,
            player_present: false,
            ..LootContext::CHEST
        },
        LootContext {
            luck: f32::NAN,
            player_present: false,
            ..LootContext::CHEST
        },
        LootContext {
            luck: 0.0,
            player_present: true,
            ..LootContext::CHEST
        },
    ] {
        assert!(matches!(
            inventory.unpack_with_rng(engine, context, &mut NoDraw),
            Err(LootError::UnsupportedContext(_))
        ));
        assert_eq!(inventory, before);
    }
}

#[test]
fn explicit_rng_is_used_without_reseeding_even_when_retained_seed_is_zero() {
    let engine = historical_engine();
    let case = actual_chests()
        .find(|case| case["id"] == "table-00-positive")
        .unwrap();
    for retained in [0, -98765] {
        let mut tag = saved(case, "saved_before");
        tag.compound_mut()
            .unwrap()
            .insert("LootTableSeed".into(), Nbt::Long(retained));
        let mut inventory = ChestInventory::from_nbt(engine, &tag).unwrap();
        let mut rng = LegacyRandom::new(seed(case));
        inventory
            .unpack_with_rng(engine, LootContext::CHEST, &mut rng)
            .unwrap()
            .unwrap();
        assert_eq!(inventory.loot_seed(), retained);
        assert_eq!(slots_json(&inventory), case["expected_slots"]);
        assert_eq!(rng.state().to_string(), case["rng_after_fill"]["seed48"]);
        inventory.write_nbt(&mut tag).unwrap();
        assert_eq!(tag, saved(case, "saved_after"));
        assert_noop(engine, &mut inventory);
    }
}

#[test]
fn native_26_2_chest_unpacks_match_all_134_native_routes() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/loot_26_2");
    let promotion_path = directory.join("promotion.json");
    let promotion: Value = serde_json::from_slice(&std::fs::read(promotion_path).unwrap()).unwrap();
    assert_eq!(promotion["schema"], "bcore.loot-fixture-promotion.v1");
    assert_eq!(promotion["minecraft"], "26.2");
    assert_eq!(promotion["protocol"], 776);
    assert_eq!(
        promotion["server_sha256"],
        bcore_registry::SERVER_JAR_SHA256
    );
    let sources = CatalogSources::VANILLA;
    for (name, text) in [
        ("tables.json", sources.tables),
        ("items.json", sources.items),
        ("enchantment.json", sources.enchantments),
        ("instrument.json", sources.instruments),
        ("potions.json", sources.potions),
        ("registry-types.json", sources.registry_types),
    ] {
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            promotion["runtime_sha256"][name]
        );
    }
    let raw = std::fs::read(directory.join("loot-fill-26.2.json")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&raw)),
        promotion["fixture_sha256"]
    );
    let fixture: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(fixture["schema"], "bcore.native-loot-fill.v1");
    assert_eq!(fixture["minecraft"], "26.2");
    assert_eq!(fixture["protocol"], 776);
    assert_eq!(fixture["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    let engine = LootEngine::vanilla().unwrap();
    let mut count = 0;
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(json!(cases.len()), promotion["native_case_count"]);
    let world = loot_location::fill_world();
    let context = loot_location::fill_context(&world);
    for case in cases {
        if case["authority"]["actual_chest_unpack_equal"] != true {
            continue;
        }
        assert_eq!(case["container_size"], 27);
        assert_eq!(case["context"]["gameplay_ticks"], 0);
        assert_eq!(case["context"]["luck_f32_bits"], "0");
        assert_eq!(case["context"]["player"], Value::Null);
        assert_eq!(
            case["context"]["origin"],
            loot_location::reference()["fill_context"]["origin"]
        );
        let before = saved(case, "saved_before");
        let mut inventory = if preset(case) {
            ChestInventory::with_slots(
                engine,
                std::array::from_fn(|slot| {
                    engine
                        .item_stack_from_json(&case["preexisting_slots"][slot])
                        .unwrap()
                }),
                case["table"].as_str(),
                seed(case),
            )
            .unwrap()
        } else {
            ChestInventory::from_nbt(engine, &before).unwrap()
        };
        let mut round_trip = before.clone();
        inventory.write_nbt(&mut round_trip).unwrap();
        assert_eq!(round_trip, before);
        let mut rng = LegacyRandom::new(seed(case));
        let outcome = inventory
            .unpack_with_rng(engine, context, &mut rng)
            .unwrap()
            .unwrap();
        assert_eq!(rng.state().to_string(), case["rng_after_fill"]["seed48"]);
        assert_native_result(engine, case, &mut inventory, outcome);
        count += 1;
    }
    assert_eq!(count, 134);
    println!("{count} actual 26.2 native chest unpacks: typed saved NBT, backing slots, retained seed, cleared key and repeat no-op match");
}

#[test]
fn malformed_items_slots_and_counts_are_rejected_instead_of_dropped() {
    let engine = LootEngine::vanilla().unwrap();
    let valid = stone(Nbt::Byte(0), Nbt::Int(1));
    let mut invalid = vec![
        materialized(vec![valid.clone(), valid.clone()]),
        materialized(vec![Nbt::Int(1)]),
    ];
    for slot in [
        Nbt::Byte(-1),
        Nbt::Byte(27),
        Nbt::Byte(127),
        Nbt::Int(0),
        Nbt::Short(0),
        Nbt::Long(0),
    ] {
        invalid.push(materialized(vec![stone(slot, Nbt::Int(1))]));
    }
    for count in [
        Nbt::Int(-1),
        Nbt::Int(0),
        Nbt::Int(100),
        Nbt::Int(i32::MAX),
        Nbt::Byte(1),
        Nbt::Short(1),
        Nbt::Long(1),
        Nbt::Float(1.0),
        Nbt::Double(1.0),
    ] {
        invalid.push(materialized(vec![stone(Nbt::Byte(0), count)]));
    }
    for absent in ["Slot", "count", "id"] {
        let mut item = valid.clone();
        item.compound_mut().unwrap().remove(absent);
        invalid.push(materialized(vec![item]));
    }
    for id in ["minecraft:air", "test:unknown", "stone"] {
        let mut item = valid.clone();
        item.compound_mut()
            .unwrap()
            .insert("id".into(), Nbt::String(id.into()));
        invalid.push(materialized(vec![item]));
    }
    for items in [
        Nbt::empty_compound(),
        Nbt::String("wrong".into()),
        Nbt::List {
            element_type: 3,
            values: Vec::new(),
        },
        Nbt::List {
            element_type: 0,
            values: vec![valid.clone()],
        },
        Nbt::List {
            element_type: 3,
            values: vec![valid.clone()],
        },
        Nbt::List {
            element_type: 10,
            values: vec![valid.clone(); 28],
        },
    ] {
        let mut tag = materialized(Vec::new());
        tag.compound_mut().unwrap().insert("Items".into(), items);
        invalid.push(tag);
    }
    for tag in invalid {
        assert!(
            ChestInventory::from_nbt(engine, &tag).is_err(),
            "accepted {tag:?}"
        );
    }
    let valid = materialized(vec![stone(Nbt::Byte(26), Nbt::Int(99))]);
    let inventory = ChestInventory::from_nbt(engine, &valid).unwrap();
    assert_eq!(inventory.slots()[26].as_ref().unwrap().count(), 99);
    let mut output = valid.clone();
    inventory.write_nbt(&mut output).unwrap();
    assert_eq!(output, valid);
}

#[test]
fn unsupported_and_type_changing_item_components_fail_exact_nbt_admission() {
    let engine = LootEngine::vanilla().unwrap();
    for components in [
        compound([("test:unknown", Nbt::Int(1))]),
        compound([("!minecraft:damage", Nbt::empty_compound())]),
        compound([("minecraft:damage", Nbt::Short(3))]),
        compound([("minecraft:damage", Nbt::Long(3))]),
        compound([(
            "minecraft:enchantments",
            compound([("minecraft:unbreaking", Nbt::Byte(1))]),
        )]),
        compound([(
            "minecraft:suspicious_stew_effects",
            Nbt::List {
                element_type: 3,
                values: vec![compound([(
                    "id",
                    Nbt::String("minecraft:night_vision".into()),
                )])],
            },
        )]),
        compound([(
            "minecraft:suspicious_stew_effects",
            Nbt::List {
                element_type: 10,
                values: vec![compound([
                    ("id", Nbt::String("minecraft:night_vision".into())),
                    ("duration", Nbt::Long(100)),
                ])],
            },
        )]),
    ] {
        let mut item = stone(Nbt::Byte(0), Nbt::Int(1));
        item.compound_mut()
            .unwrap()
            .insert("components".into(), components);
        let tag = materialized(vec![item]);
        assert!(
            ChestInventory::from_nbt(engine, &tag).is_err(),
            "accepted {tag:?}"
        );
    }
}

fn custom_stone_limit(limit: i32, tables: Value) -> LootEngine {
    let mut items: Value = serde_json::from_str(CatalogSources::VANILLA.items).unwrap();
    let stone = items["items_native_order"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["key"] == "minecraft:stone")
        .unwrap();
    stone["max_stack_size"] = json!(limit);
    stone["components"]["persistent"]["minecraft:max_stack_size"] = json!(limit);
    LootEngine::from_sources(CatalogSources {
        tables: &tables.to_string(),
        items: &items.to_string(),
        ..CatalogSources::VANILLA
    })
    .unwrap()
}

#[test]
fn backing_constructor_validates_counts_catalog_identity_and_pending_key_syntax() {
    let engine = LootEngine::vanilla().unwrap();
    for (id, count) in [("stone", -1), ("stone", 0), ("stone", 100), ("air", 1)] {
        let mut slots = std::array::from_fn(|_| None);
        slots[3] = Some(engine.item_stack(id, count).unwrap());
        assert!(ChestInventory::with_slots(engine, slots, None, 0).is_err());
    }
    let foreign = custom_stone_limit(32, json!({}));
    let mut slots = std::array::from_fn(|_| None);
    slots[0] = Some(foreign.item_stack("stone", 1).unwrap());
    assert!(ChestInventory::with_slots(engine, slots, None, 0).is_err());
    for key in [
        "",
        ":empty",
        "minecraft:",
        "Upper:case",
        "test:bad key",
        "test:too:many",
    ] {
        assert!(
            ChestInventory::with_slots(engine, std::array::from_fn(|_| None), Some(key), 1)
                .is_err()
        );
        assert!(ChestInventory::from_nbt(engine, &pending(key, Some(Nbt::Long(1)))).is_err());
    }
}

#[test]
fn nonchest_metadata_and_noncompound_tags_are_rejected_atomically() {
    let engine = LootEngine::vanilla().unwrap();
    let inventory = ChestInventory::from_nbt(engine, &Nbt::empty_compound()).unwrap();
    for mut tag in [
        Nbt::Int(1),
        Nbt::List {
            element_type: 0,
            values: Vec::new(),
        },
        compound([("id", Nbt::Int(1))]),
        compound([("id", Nbt::String("minecraft:barrel".into()))]),
        compound([("id", Nbt::String("minecraft:trapped_chest".into()))]),
    ] {
        let before = tag.clone();
        assert!(ChestInventory::from_nbt(engine, &tag).is_err());
        assert!(inventory.write_nbt(&mut tag).is_err());
        assert_eq!(tag, before);
    }
    let mut tag = materialized(Vec::new());
    tag.compound_mut()
        .unwrap()
        .insert("LootTable".into(), Nbt::Int(1));
    assert!(ChestInventory::from_nbt(engine, &tag).is_err());
}

#[test]
fn late_evaluation_failure_consumes_key_without_restoring_the_advanced_source() {
    let tables = json!({"test:late": {"pools": [
        {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "stone", "functions": [
            {"function": "minecraft:set_count", "count": {"type": "minecraft:uniform", "min": 1, "max": 3}}
        ]}]},
        {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "stone", "functions": [
            {"function": "minecraft:set_count", "count": {"type": "minecraft:uniform", "min": i32::MIN, "max": i32::MAX}}
        ]}]}
    ]}});
    let engine = LootEngine::from_sources(CatalogSources {
        tables: &tables.to_string(),
        ..CatalogSources::VANILLA
    })
    .unwrap();
    let backing =
        std::array::from_fn(|slot| (slot == 8).then(|| engine.item_stack("diamond", 7).unwrap()));
    let mut inventory =
        ChestInventory::with_slots(&engine, backing.clone(), Some("test:late"), 53).unwrap();
    let mut seeded = inventory.clone();
    let mut rng = LegacyRandom::new(53);
    let mut expected_rng = rng.clone();
    expected_rng.next_int(3);
    let before_state = rng.state();
    assert!(matches!(
        inventory.unpack_with_rng(&engine, LootContext::CHEST, &mut rng),
        Err(LootError::LimitExceeded("integer provider interval"))
    ));
    assert_ne!(rng.state(), before_state);
    assert_eq!(rng.state(), expected_rng.state());
    assert_eq!(inventory.loot_table(), None);
    assert_eq!(inventory.loot_seed(), 53);
    assert_eq!(inventory.slots(), &backing);
    assert_noop(&engine, &mut inventory);
    assert_eq!(
        inventory
            .unpack_with_rng(&engine, LootContext::CHEST, &mut rng)
            .unwrap(),
        None
    );
    assert_eq!(rng.state(), expected_rng.state());
    assert!(matches!(
        seeded.unpack(&engine),
        Err(LootError::LimitExceeded(_))
    ));
    assert_eq!(seeded, inventory);
    let mut saved = pending("test:late", Some(Nbt::Long(53)));
    inventory.write_nbt(&mut saved).unwrap();
    assert!(saved.get("LootTable").is_none() && saved.get("LootTableSeed").is_none());
    assert_eq!(
        ChestInventory::from_nbt(&engine, &saved).unwrap().slots(),
        &backing
    );
}

#[test]
fn item_encoding_failure_does_not_partially_update_the_target_tag() {
    // A supplied catalog can produce a transient stack outside ItemStack's
    // persistent count range. Saving must report the error before removing keys.
    let engine = custom_stone_limit(
        200,
        json!({"test:oversize": {"pools": [{
            "rolls": 1, "entries": [{"type": "minecraft:item", "name": "stone", "functions": [
                {"function": "minecraft:set_count", "count": 200}
            ]}]
        }]}}),
    );
    let backing =
        std::array::from_fn(|slot| (slot != 26).then(|| engine.item_stack("diamond", 1).unwrap()));
    let mut inventory =
        ChestInventory::with_slots(&engine, backing, Some("test:oversize"), 1).unwrap();
    inventory.unpack(&engine).unwrap().unwrap();
    assert_eq!(inventory.slots()[26].as_ref().unwrap().count(), 200);
    let mut tag = pending("test:oversize", Some(Nbt::Long(1)));
    let before = tag.clone();
    assert!(inventory.write_nbt(&mut tag).is_err());
    assert_eq!(tag, before);
}
