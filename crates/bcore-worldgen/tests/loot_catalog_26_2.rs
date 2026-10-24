//! Current native catalog identity, numeric IDs, defaults and ordered holders.
use bcore_worldgen::loot::{CatalogSources, ItemStack, LootContext, LootEngine, LootRng};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const PROMOTION: &str = include_str!("../data/loot_26_2/promotion.json");
const CATALOG: &str = include_str!("../data/loot_26_2/loot-catalog.json");
const LOADED: &str = include_str!("../data/loot_26_2/loaded-tables.json");
const INVENTORY: &str = include_str!("../data/loot_26_2/table-inventory.json");

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[test]
fn current_catalog_has_authenticated_26_2_identity_and_export_hashes() {
    let promotion = parsed(PROMOTION);
    let catalog = parsed(CATALOG);
    let native = bcore_registry::vanilla();
    assert_eq!(promotion["schema"], "bcore.loot-data-promotion.v2");
    assert_eq!(promotion["minecraft"], native.minecraft);
    assert_eq!(promotion["protocol"], native.protocol);
    assert_eq!(promotion["data_version"], native.data_version);
    assert_eq!(promotion["server_sha256"], native.jar_sha256);
    assert_eq!(catalog["schema"], "bcore.native-loot-catalog.v1");
    assert_eq!(catalog["minecraft"], "26.2");
    assert_eq!(catalog["protocol"], 776);
    assert_eq!(catalog["data_version"], 4903);
    assert_eq!(catalog["jar_sha256"], native.jar_sha256);
    assert_eq!(catalog["gameplay_ticks"], 0);
    assert_eq!(
        catalog["lookup_source"],
        "MinecraftServer.reloadableRegistries().lookup()"
    );
    assert_eq!(
        catalog["level_class"],
        "net.minecraft.server.level.ServerLevel"
    );
    assert_eq!(promotion["counts"], catalog["counts"]);
    let sources = CatalogSources::VANILLA;
    for (name, text) in [
        ("tables.json", sources.tables),
        ("items.json", sources.items),
        ("enchantment.json", sources.enchantments),
        ("instrument.json", sources.instruments),
        ("potions.json", sources.potions),
        ("registry-types.json", sources.registry_types),
        ("loot-catalog.json", CATALOG),
        ("loaded-tables.json", LOADED),
        ("table-inventory.json", INVENTORY),
    ] {
        let key = format!("crates/bcore-worldgen/data/loot_26_2/{name}");
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            promotion["files"][&key]["sha256"],
            "promoted export changed: {name}"
        );
    }
    assert_ne!(
        Sha256::digest(sources.items.as_bytes()),
        Sha256::digest(CatalogSources::VANILLA_26_1.items.as_bytes())
    );
}

#[test]
fn all_current_item_ids_and_default_components_match_native_exports() {
    let engine = LootEngine::vanilla().unwrap();
    let sources = CatalogSources::VANILLA;
    let items = parsed(sources.items);
    let rows = items["items_native_order"].as_array().unwrap();
    let native = bcore_registry::vanilla();
    let expected = &native.registries["ITEM"];
    assert_eq!(rows.len(), expected.len());
    assert_eq!(rows.len(), 1537);
    assert_eq!(json!(rows.len()), parsed(CATALOG)["counts"]["items"]);
    for (row, entry) in rows.iter().zip(expected) {
        assert_eq!(row["id"], entry.id);
        assert_eq!(row["key"], entry.key);
        if !row["components"]["unserialized_transient_types"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            assert!(engine.item_stack(&entry.key, 1).is_err());
            continue;
        }
        let stack = engine.item_stack(&entry.key, 1).unwrap();
        assert_eq!(stack.definition().numeric_id(), entry.id);
        assert_eq!(
            serde_json::to_value(stack.definition().defaults()).unwrap(),
            row["components"]["persistent"],
            "{} native defaults",
            entry.key
        );
        assert_eq!(json!(stack.max_stack_size()), row["max_stack_size"]);
        assert!(stack.persistent_patch().is_empty());
        if entry.key != "minecraft:air" {
            let effective: BTreeMap<_, _> = stack
                .effective_components()
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect();
            assert_eq!(
                serde_json::to_value(effective).unwrap(),
                row["components"]["persistent"]
            );
        }
    }
    for (key, id) in [
        ("minecraft:sulfur", bcore_registry::item::SULFUR),
        (
            "minecraft:potent_sulfur",
            bcore_registry::item::POTENT_SULFUR,
        ),
        ("minecraft:cinnabar", bcore_registry::item::CINNABAR),
        (
            "minecraft:sulfur_cube_bucket",
            bcore_registry::item::SULFUR_CUBE_BUCKET,
        ),
    ] {
        assert_eq!(engine.item_stack(key, 1).unwrap().numeric_id(), id);
    }
    let historical = LootEngine::from_sources(CatalogSources::VANILLA_26_1).unwrap();
    assert!(historical.item_stack("minecraft:sulfur", 1).is_err());
}

#[test]
fn potion_and_component_ids_cover_the_current_native_registry() {
    let engine = LootEngine::vanilla().unwrap();
    let native = bcore_registry::vanilla();
    let sources = CatalogSources::VANILLA;
    let potions = parsed(sources.potions);
    let rows = potions.as_array().unwrap();
    assert_eq!(rows.len(), native.registries["POTION"].len());
    assert_eq!(json!(rows.len()), parsed(CATALOG)["counts"]["potions"]);
    for (row, entry) in rows.iter().zip(&native.registries["POTION"]) {
        assert_eq!(row["key"], entry.key);
        assert_eq!(row["id"], entry.id);
        assert_eq!(engine.potion_numeric_id(&entry.key), Some(entry.id));
    }
    let types = parsed(sources.registry_types);
    let components = types["DATA_COMPONENT_TYPE"].as_array().unwrap();
    assert_eq!(
        components.len(),
        native.registries["DATA_COMPONENT_TYPE"].len()
    );
    assert_eq!(components.len(), 111);
    for (row, entry) in components
        .iter()
        .zip(&native.registries["DATA_COMPONENT_TYPE"])
    {
        assert_eq!(row["key"], entry.key);
        assert_eq!(row["id"], entry.id);
        assert!(row["persistent"].is_boolean());
        assert_eq!(engine.component_numeric_id(&entry.key), Some(entry.id));
    }
    assert_eq!(
        engine.component_numeric_id("minecraft:sulfur_cube_content"),
        Some(bcore_registry::component::SULFUR_CUBE_CONTENT)
    );
    for (name, rows) in types.as_object().unwrap() {
        assert_eq!(
            json!(rows.as_array().unwrap().len()),
            parsed(CATALOG)["counts"]["registry_types"][name]
        );
    }
}

#[test]
fn current_table_membership_and_chest_codecs_match_the_loaded_registry() {
    let engine = LootEngine::vanilla().unwrap();
    let tables = parsed(CatalogSources::VANILLA.tables);
    let tables = tables.as_object().unwrap();
    let loaded = parsed(LOADED);
    let inventory = parsed(INVENTORY);
    assert_eq!(inventory["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    assert_eq!(inventory["minecraft"], "26.2");
    let keys: BTreeSet<_> = tables.keys().map(String::as_str).collect();
    assert_eq!(engine.table_keys().collect::<BTreeSet<_>>(), keys);
    assert_eq!(
        loaded["all_loaded_keys"],
        json!(keys.iter().copied().collect::<Vec<_>>())
    );
    assert_eq!(inventory["all_jar_keys"], loaded["all_loaded_keys"]);
    assert_eq!(
        json!(keys.len()),
        parsed(CATALOG)["counts"]["loaded_tables"]
    );
    let candidates: Vec<_> = keys
        .iter()
        .copied()
        .filter(|key| key.starts_with("minecraft:chests/") || key.contains("trial_chambers/"))
        .collect();
    let rows = loaded["chest_and_trial_tables"].as_array().unwrap();
    assert_eq!(
        json!(candidates.len()),
        parsed(CATALOG)["counts"]["chest_and_trial_tables"]
    );
    assert_eq!(inventory["chest_and_trial_keys"], json!(candidates));
    assert_eq!(rows.len(), candidates.len());
    for (row, key) in rows.iter().zip(&candidates) {
        assert_eq!(row["key"], *key);
        assert_eq!(row["native_codec"], tables[*key]);
        if [
            "minecraft:chests/shipwreck_map",
            "minecraft:chests/underwater_ruin_big",
            "minecraft:chests/underwater_ruin_small",
        ]
        .contains(key)
        {
            assert!(engine.table(key).is_err());
        } else {
            assert!(!engine.table(key).unwrap().is_missing(), "{key}");
        }
    }
}

struct Pick {
    index: i32,
    maximum: bool,
    draws: Vec<i32>,
}

impl LootRng for Pick {
    fn next_int(&mut self, bound: i32) -> i32 {
        self.draws.push(bound);
        let value = if self.draws.len() == 1 {
            self.index
        } else if self.maximum {
            bound - 1
        } else {
            0
        };
        assert!((0..bound).contains(&value));
        value
    }
    fn next_float(&mut self) -> f32 {
        panic!("ordered-holder selection should not draw a float")
    }
    fn next_bool(&mut self) -> bool {
        panic!("ordered-holder selection should not draw a boolean")
    }
}

fn selection_engine(entry: Value) -> LootEngine {
    LootEngine::from_sources(CatalogSources {
        tables: &json!({"test:ordered": {"pools": [{"rolls": 1, "entries": [entry]}]}}).to_string(),
        ..CatalogSources::VANILLA
    })
    .unwrap()
}

fn select(engine: &LootEngine, rng: &mut Pick) -> ItemStack {
    let mut stacks = engine
        .table("test:ordered")
        .unwrap()
        .generate_with_rng(LootContext::CHEST, rng)
        .unwrap();
    assert_eq!(stacks.len(), 1);
    stacks.remove(0)
}

fn assert_enchantment_order(engine: &LootEngine, keys: &[Value], rows: &[Value]) {
    for (index, member) in keys.iter().enumerate() {
        let mut rng = Pick {
            index: index as i32,
            maximum: true,
            draws: Vec::new(),
        };
        let stack = select(engine, &mut rng);
        let key = member.as_str().unwrap();
        let definition = rows.iter().find(|row| row["key"] == *member).unwrap();
        let level = definition["value"]["max_level"].as_i64().unwrap();
        assert_eq!(stack.id(), "minecraft:enchanted_book");
        assert_eq!(
            stack.component("minecraft:stored_enchantments"),
            Some(&json!({key: level}))
        );
        assert_eq!(rng.draws[0], keys.len() as i32);
        assert_eq!(rng.draws.len(), if level > 1 { 2 } else { 1 });
    }
}

#[test]
fn current_enchantment_holder_tags_preserve_the_captured_member_order() {
    let exports = parsed(CatalogSources::VANILLA.enchantments);
    let rows = exports["values_native_order"].as_array().unwrap();
    assert_eq!(json!(rows.len()), parsed(CATALOG)["counts"]["enchantments"]);
    for tag in exports["tags"].as_array().unwrap() {
        let members = tag["members_native_order"].as_array().unwrap();
        let engine = selection_engine(
            json!({"type":"minecraft:item", "name":"minecraft:book", "functions":[{
                "function":"minecraft:enchant_randomly", "options":format!("#{}", tag["tag"].as_str().unwrap())
            }]}),
        );
        assert_enchantment_order(&engine, members, rows);
    }
    let engine = selection_engine(
        json!({"type":"minecraft:item", "name":"minecraft:book", "functions":[{
            "function":"minecraft:enchant_randomly"
        }]}),
    );
    let keys: Vec<_> = rows.iter().map(|row| row["key"].clone()).collect();
    assert_enchantment_order(&engine, &keys, rows);
}

#[test]
fn current_instrument_holder_tags_preserve_the_captured_member_order() {
    let exports = parsed(CatalogSources::VANILLA.instruments);
    assert_eq!(
        json!(exports["values_native_order"].as_array().unwrap().len()),
        parsed(CATALOG)["counts"]["instruments"]
    );
    for tag in exports["tags"].as_array().unwrap() {
        let members = tag["members_native_order"].as_array().unwrap();
        let engine = selection_engine(
            json!({"type":"minecraft:item", "name":"minecraft:goat_horn", "functions":[{
                "function":"minecraft:set_instrument", "options":format!("#{}", tag["tag"].as_str().unwrap())
            }]}),
        );
        for (index, member) in members.iter().enumerate() {
            let mut rng = Pick {
                index: index as i32,
                maximum: false,
                draws: Vec::new(),
            };
            let stack = select(&engine, &mut rng);
            assert_eq!(stack.component("minecraft:instrument"), Some(member));
            assert_eq!(rng.draws, vec![members.len() as i32]);
        }
    }
}
