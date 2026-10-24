//! Preserved 26.1 oracle and promoted 26.2 item/setter/RNG observations.
use bcore_worldgen::loot::{
    CatalogSources, Components, ItemStack, LegacyRandom, LootContext, LootEngine, LootObserver,
    LootRng,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[path = "support/loot_nbt.rs"]
mod loot_nbt;

#[path = "support/loot_location.rs"]
mod loot_location;

const FIXTURE: &str = include_str!("data/loot_26_1/loot-fill-26.1.json");

#[derive(Default)]
struct Trace {
    generated: Vec<ItemStack>,
    slots: Vec<usize>,
    split: Vec<ItemStack>,
    placements: Vec<Value>,
    states: Vec<u64>,
}
impl LootObserver for Trace {
    fn generated(&mut self, items: &[ItemStack], rng: &dyn LootRng) {
        self.generated = items.to_vec();
        self.states.push(rng.legacy_state().unwrap());
    }
    fn slots_shuffled(&mut self, slots: &[usize], rng: &dyn LootRng) {
        self.slots = slots.to_vec();
        self.states.push(rng.legacy_state().unwrap());
    }
    fn items_shuffled(&mut self, items: &[ItemStack], rng: &dyn LootRng) {
        self.split = items.to_vec();
        self.states.push(rng.legacy_state().unwrap());
    }
    fn placed(&mut self, slot: usize, stack: &ItemStack) {
        self.placements
            .push(json!({"slot": slot, "stack": stack.to_json()}));
    }
}

fn equal(actual: Value, expected: &Value, label: &str) -> Result<(), String> {
    if actual == *expected {
        Ok(())
    } else {
        Err(format!("{label}\nactual: {actual}\nexpected: {expected}"))
    }
}
fn state(value: &Value) -> u64 {
    value["seed48"].as_str().unwrap().parse().unwrap()
}
fn items(items: &[ItemStack]) -> Value {
    Value::Array(items.iter().map(ItemStack::to_json).collect())
}
fn slots(items: &[Option<ItemStack>]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|v| v.as_ref().map_or(Value::Null, ItemStack::to_json))
            .collect(),
    )
}

fn verify_case(
    engine: &LootEngine,
    case: &Value,
    context: LootContext<'_>,
) -> Result<usize, String> {
    let table = engine
        .table(case["table"].as_str().unwrap())
        .map_err(|e| e.to_string())?;
    let seed: i64 = case["seed_i64"].as_str().unwrap().parse().unwrap();
    let preexisting = case["preexisting_slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| {
            engine
                .item_stack_from_json(value)
                .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut filled = preexisting.clone();
    let mut rng = LegacyRandom::new(seed);
    let mut trace = Trace::default();
    let outcome = table
        .fill_observed(&mut filled, context, &mut rng, &mut trace)
        .map_err(|e| e.to_string())?;
    equal(
        items(&trace.generated),
        &case["generated_stacks"],
        "generated order / patch",
    )?;
    for (index, key) in [
        "rng_after_generation",
        "rng_after_slot_shuffle",
        "rng_after_split_shuffle",
    ]
    .iter()
    .enumerate()
    {
        if trace.states[index] != state(&case[*key]) {
            return Err(format!(
                "{key}: actual {} expected {}",
                trace.states[index],
                state(&case[*key])
            ));
        }
        assert_eq!(case[*key]["gaussian_cached"], false);
        assert_eq!(case[*key]["gaussian_bits"], "0");
    }
    equal(
        json!(trace.slots),
        &case["available_slots_shuffled"],
        "empty-slot shuffle",
    )?;
    equal(
        items(&trace.split),
        &case["shuffled_split_stacks"],
        "split / item shuffle",
    )?;
    equal(
        json!(trace.placements),
        &case["placement_order"],
        "native setter order",
    )?;
    equal(slots(&filled), &case["expected_slots"], "final slots")?;
    assert_eq!(rng.state(), state(&case["rng_after_fill"]));
    assert_eq!(outcome.placed_stacks, trace.placements.len());
    assert_eq!(
        outcome.discarded_stacks,
        trace.split.len() - trace.placements.len()
    );
    let mut occupied = 0;
    for (slot, stack) in filled.iter().enumerate() {
        let Some(stack) = stack else {
            continue;
        };
        occupied += 1;
        let detail = &case["expected_slot_details"][slot];
        let effective: Components = stack
            .effective_components()
            .map(|(k, v)| (k.into(), v.clone()))
            .collect();
        equal(
            serde_json::to_value(effective).unwrap(),
            &detail["effective_components"]["persistent"],
            &format!("slot {slot} effective components"),
        )?;
        equal(
            json!(stack.max_stack_size()),
            &detail["max_stack_size"],
            &format!("slot {slot} max stack size"),
        )?;
        let expected = loot_nbt::decode(detail["nbt"]["base64"].as_str().unwrap());
        let actual = stack.to_nbt().map_err(|e| e.to_string())?;
        if actual != expected {
            return Err(format!(
                "slot {slot} typed persistent NBT\nactual: {actual:?}\nexpected: {expected:?}"
            ));
        }
        let imported = engine
            .item_stack_from_json(&detail["stack"])
            .map_err(|e| format!("slot {slot} native JSON import: {e}"))?
            .ok_or_else(|| format!("slot {slot} native item imported as empty"))?;
        equal(
            imported.to_json(),
            &detail["stack"],
            "native JSON round trip",
        )?;
        if imported.to_nbt().map_err(|e| e.to_string())? != expected {
            return Err(format!(
                "slot {slot} imported native JSON changed typed NBT"
            ));
        }
    }
    let expected = &case["rng_continuation"];
    let mut continuation = rng.clone();
    equal(
        json!(continuation.next_int(1000)),
        &expected["next_int_1000"],
        "continuation int",
    )?;
    equal(
        json!(continuation.next_long().to_string()),
        &expected["next_long"]["i64"],
        "continuation long",
    )?;
    equal(
        json!(continuation.next_bool()),
        &expected["next_boolean"],
        "continuation boolean",
    )?;
    equal(
        json!(continuation.next_float().to_bits().to_string()),
        &expected["next_float"]["bits"],
        "continuation float bits",
    )?;
    equal(
        json!(continuation.next_double().to_bits().to_string()),
        &expected["next_double"]["bits"],
        "continuation double bits",
    )?;
    assert_eq!(continuation.state(), state(&expected["after"]));
    // The convenience API must take the same production path as the observed one.
    let mut seeded = preexisting;
    table
        .fill_with_context(&mut seeded, context, seed)
        .map_err(|e| e.to_string())?;
    equal(slots(&seeded), &case["expected_slots"], "optional-seed API")?;
    Ok(occupied)
}

#[test]
fn native_26_1_all_137_cases() {
    assert_eq!(
        format!("{:x}", Sha256::digest(FIXTURE.as_bytes())),
        "2158fd855f1bbc46d6b399620c9ced35822263c982151686f58e175c1ccf9db1"
    );
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture["schema"], "bcore.native-loot-fill.v1");
    assert_eq!(fixture["minecraft"], "26.1");
    assert_eq!(fixture["protocol"], 775);
    let engine = LootEngine::from_sources(CatalogSources::VANILLA_26_1).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 137);
    let mut failures = Vec::new();
    let mut tables = BTreeSet::new();
    let mut occupied = 0;
    for case in cases {
        if case["table_resolution"] == "registered" {
            tables.insert(case["table"].as_str().unwrap());
        }
        match verify_case(&engine, case, LootContext::CHEST) {
            Ok(count) => occupied += count,
            Err(error) => failures.push(format!(
                "{} ({}, seed {}): {error}",
                case["id"], case["table"], case["seed_i64"]
            )),
        }
    }
    assert_eq!(tables.len(), 57);
    assert!(
        failures.is_empty(),
        "{} / {} native cases diverged:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n\n")
    );
    assert_eq!(occupied, 1292);
    println!("137/137 cases; 57 registered tables; 1292 typed-NBT/effective-component slots and native JSON round trips; all generated lists, shuffles, setters, checkpoints and continuations match");
}

#[test]
fn historical_26_1_catalog_hashes_are_pinned() {
    let manifest: Value =
        serde_json::from_str(include_str!("../data/loot_26_1/promotion.json")).unwrap();
    assert_eq!(manifest["minecraft"], "26.1");
    let sources = CatalogSources::VANILLA_26_1;
    for (file, text) in [
        ("tables.json", sources.tables),
        ("items.json", sources.items),
        ("enchantment.json", sources.enchantments),
        ("instrument.json", sources.instruments),
        ("potions.json", sources.potions),
        ("registry-types.json", sources.registry_types),
    ] {
        let key = format!("crates/bcore-worldgen/data/loot_26_1/{file}");
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            manifest["files"][&key]["sha256"]
        );
    }
}

#[test]
fn native_26_2_all_137_cases() {
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
            promotion["runtime_sha256"][name],
            "26.2 native fill/runtime context differs: {name}"
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
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 137);
    assert_eq!(json!(cases.len()), promotion["native_case_count"]);
    let mut failures = Vec::new();
    let mut occupied = 0;
    let world = loot_location::fill_world();
    let context = loot_location::fill_context(&world);
    for case in cases {
        assert_eq!(case["context"]["gameplay_ticks"], 0);
        assert_eq!(case["context"]["luck_f32_bits"], "0");
        assert_eq!(case["context"]["player"], Value::Null);
        assert_eq!(
            case["context"]["origin"],
            loot_location::reference()["fill_context"]["origin"]
        );
        assert_eq!(
            case["context"]["world_seed_i64"],
            loot_location::reference()["world_seed_i64"]
        );
        match verify_case(engine, case, context) {
            Ok(count) => occupied += count,
            Err(error) => failures.push(format!("{}: {error}", case["id"])),
        }
    }
    assert!(
        failures.is_empty(),
        "26.2 native fills diverged:\n{}",
        failures.join("\n\n")
    );
    println!("{}/{} actual 26.2 native fill cases, {occupied} typed-NBT/effective-component slots, RNG checkpoints and continuations match", cases.len(), cases.len());
}
