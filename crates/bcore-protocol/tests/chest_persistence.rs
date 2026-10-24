//! BCC representation/persistence checks against captured native chest data.
//! Live feature values are constructed from saved tags and captured memory seeds.
//! These tests do not execute the canonical owner bridge, hydrate GenerationWorld
//! holders, or reproduce a native process restart. Fresh lookup uses the public
//! ChunkColumn pending loader, followed by detached ChestInventory no-reroll reads.
//! Native NBT comparisons preserve values and tag kinds; byte identity is checked
//! for BCC v6 encoding, not native compound wire ordering. The historical 26.1
//! inputs remain historical; these representation checks do not prove 26.2 loot parity.

use bcore_core::ChunkPos;
use bcore_protocol::chunk::ChunkColumn;
use bcore_protocol::chunk_store::{decode_chunk_at, encode_chunk};
use bcore_worldgen::block_entity::{ChestInventory, PendingBlockEntity};
use bcore_worldgen::generation::FeatureBlockEntity;
use bcore_worldgen::loot::{CatalogSources, ItemStack, LootContext, LootEngine, LootRng};
use bcore_worldgen::structure::template::{Nbt, TemplateBlockEntity};
use bcore_worldgen::structure::template_pool::StructureAssets;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[path = "../../bcore-worldgen/tests/support/loot_nbt.rs"]
mod loot_nbt;

const OWNER: ChunkPos = ChunkPos::new(0, 0);
const LOCAL: (usize, i32, usize) = (8, 80, 8);
const FIXTURE_SHA256: &str = "4b0bc865d02713dc62b50b2d5a9a6c28c742ee54af64e88461965e98ed1de8b0";

fn historical_loot_engine() -> &'static LootEngine {
    static ENGINE: OnceLock<LootEngine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        LootEngine::from_sources(CatalogSources {
            tables: include_str!("../../bcore-worldgen/data/loot_26_1/tables.json"),
            items: include_str!("../../bcore-worldgen/data/loot_26_1/items.json"),
            enchantments: include_str!("../../bcore-worldgen/data/loot_26_1/enchantment.json"),
            instruments: include_str!("../../bcore-worldgen/data/loot_26_1/instrument.json"),
            potions: include_str!("../../bcore-worldgen/data/loot_26_1/potions.json"),
            registry_types: include_str!("../../bcore-worldgen/data/loot_26_1/registry-types.json"),
        })
        .expect("historical 26.1 loot catalog")
    })
}

fn fixture() -> &'static Value {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let text =
            include_str!("../../bcore-worldgen/tests/data/loot_26_1/chest-lifecycle-26.1.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(text.as_bytes())),
            FIXTURE_SHA256
        );
        let fixture: Value = serde_json::from_str(text).unwrap();
        assert_eq!(fixture["schema"], "bcore.native-chest-lifecycle.v1");
        assert_eq!(fixture["minecraft"], "26.1");
        assert_eq!(fixture["protocol"], 775);
        assert_eq!(fixture["block_pos"], json!([8, 80, 8]));
        assert_eq!(fixture["player"], Value::Null);
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
            assert_eq!(raw, typed, "native raw/typed NBT differs: {digest}");
        }
        fixture
    })
}

fn cases() -> &'static [Value] {
    fixture()["cases"].as_array().unwrap()
}

fn native_tag(digest: &Value) -> Nbt {
    let blob = &fixture()["nbt_blobs"][digest.as_str().unwrap()];
    loot_nbt::decode(blob["base64"].as_str().unwrap())
}

fn memory_seed(snapshot: &Value) -> i64 {
    snapshot["loot_seed_i64"].as_str().unwrap().parse().unwrap()
}

fn chest_column() -> ChunkColumn {
    let mut column = ChunkColumn::flat();
    let state = StructureAssets::bundled()
        .blocks
        .default_state("chest")
        .unwrap();
    assert!(column.set(LOCAL.0, LOCAL.1, LOCAL.2, state));
    column
}

fn pending_column(tag: &Nbt) -> ChunkColumn {
    let mut column = chest_column();
    assert!(column.set_pending_block_entity(
        OWNER,
        LOCAL.0,
        LOCAL.1,
        LOCAL.2,
        PendingBlockEntity::from_nbt(tag),
    ));
    column
}

fn only_pending<'a>(column: &'a ChunkColumn, label: &str) -> &'a PendingBlockEntity {
    assert!(
        column.block_entities().is_empty(),
        "{label}: unexpected core entity"
    );
    assert!(
        column.feature_block_entities().is_empty(),
        "{label}: pending became live"
    );
    assert_eq!(column.pending_block_entities().len(), 1, "{label}");
    column
        .pending_block_entities()
        .get(&LOCAL)
        .expect("pending chest at its original slot")
}

fn only_feature<'a>(column: &'a ChunkColumn, label: &str) -> &'a FeatureBlockEntity {
    assert!(
        column.block_entities().is_empty(),
        "{label}: unexpected core entity"
    );
    assert!(
        column.pending_block_entities().is_empty(),
        "{label}: still pending"
    );
    assert_eq!(column.feature_block_entities().len(), 1, "{label}");
    column
        .feature_block_entities()
        .get(&LOCAL)
        .expect("live chest at its original slot")
}

fn runtime_seed(entity: &FeatureBlockEntity) -> i64 {
    let load = Nbt::from_typed_json(entity.template_load_data.as_ref().unwrap()).unwrap();
    match load.get("LootTableSeed") {
        None => 0,
        Some(Nbt::Long(seed)) => *seed,
        other => panic!("runtime seed lost its Long type: {other:?}"),
    }
}

fn assert_tag_types(tag: &Nbt, label: &str) {
    assert_eq!(
        tag.get("id"),
        Some(&Nbt::String("minecraft:chest".into())),
        "{label}"
    );
    for (name, value) in [("x", 8), ("y", 80), ("z", 8)] {
        assert_eq!(tag.get(name), Some(&Nbt::Int(value)), "{label}: {name}");
    }
    if let Some(table) = tag.get("LootTable") {
        assert!(matches!(table, Nbt::String(_)), "{label}: table type");
    }
    if let Some(seed) = tag.get("LootTableSeed") {
        assert!(matches!(seed, Nbt::Long(_)), "{label}: seed type");
    }
    if let Some(items) = tag.get("Items") {
        let Nbt::List {
            element_type,
            values,
        } = items
        else {
            panic!("{label}: Items must be a typed list");
        };
        assert_eq!(
            *element_type,
            if values.is_empty() { 0 } else { 10 },
            "{label}: Items type"
        );
        for item in values {
            assert!(matches!(item, Nbt::Compound(_)), "{label}: item type");
            assert!(
                matches!(item.get("Slot"), Some(Nbt::Byte(0..=26))),
                "{label}: Slot type/range"
            );
            assert!(
                matches!(item.get("count"), Some(Nbt::Int(1..=99))),
                "{label}: count type/range"
            );
            assert!(
                matches!(item.get("id"), Some(Nbt::String(_))),
                "{label}: item id"
            );
            if let Some(components) = item.get("components") {
                assert!(
                    matches!(components, Nbt::Compound(_)),
                    "{label}: component patch type"
                );
            }
        }
    }
}

fn assert_inventory(inventory: &ChestInventory, snapshot: &Value, label: &str) {
    assert_eq!(snapshot["slots"].as_array().unwrap().len(), 27, "{label}");
    assert_eq!(
        inventory.loot_table(),
        snapshot["loot_table"].as_str(),
        "{label}"
    );
    let slots: Vec<_> = inventory
        .slots()
        .iter()
        .map(|stack| stack.as_ref().map_or(Value::Null, ItemStack::to_json))
        .collect();
    assert_eq!(
        Value::Array(slots),
        snapshot["slots"],
        "{label}: slots/components"
    );
    for stack in inventory.slots().iter().flatten() {
        let serialized = stack.to_json();
        let native = fixture()["component_maps"]
            .as_object()
            .unwrap()
            .values()
            .find(|entry| entry["stack"] == serialized)
            .unwrap_or_else(|| {
                panic!("{label}: native component projection missing for {serialized}")
            });
        let effective: BTreeMap<_, _> = stack
            .effective_components()
            .map(|(key, value)| (key.to_owned(), value.clone()))
            .collect();
        assert_eq!(
            serde_json::to_value(effective).unwrap(),
            native["effective_components"],
            "{label}"
        );
    }
}

fn assert_feature_snapshot(column: &ChunkColumn, snapshot: &Value, label: &str) {
    let entity = only_feature(column, label);
    let expected = native_tag(&snapshot["saved_nbt_sha256"]);
    let full = entity.full_nbt().unwrap();
    // Nbt equality includes every nested component value and numeric/list tag kind.
    assert_eq!(full, expected, "{label}: full typed NBT");
    assert_eq!(entity.full_data, expected.to_json(), "{label}: full JSON");
    assert_tag_types(&full, label);
    assert_eq!(
        runtime_seed(entity),
        memory_seed(snapshot),
        "{label}: retained memory seed"
    );
    let inventory = ChestInventory::from_nbt(historical_loot_engine(), &full).unwrap();
    assert_inventory(&inventory, snapshot, label);
    if snapshot["loot_table"].is_null() {
        assert!(
            full.get("LootTableSeed").is_none(),
            "{label}: saved runtime seed"
        );
        assert_eq!(
            inventory.loot_seed(),
            0,
            "{label}: saved seed defaults to zero"
        );
    } else {
        assert_eq!(
            full.get("LootTableSeed"),
            Some(&Nbt::Long(memory_seed(snapshot))),
            "{label}"
        );
    }
}

fn round_trip(column: &ChunkColumn, label: &str) -> ChunkColumn {
    let encoded = encode_chunk(OWNER.x, OWNER.z, column);
    assert_eq!(u16::from_le_bytes(encoded[4..6].try_into().unwrap()), 6);
    assert_eq!(
        i32::from_le_bytes(encoded[24..28].try_into().unwrap()),
        4903
    );
    assert_eq!(i32::from_le_bytes(encoded[28..32].try_into().unwrap()), 776);
    assert_eq!(encoded[32..64], bcore_registry::REGISTRY_FINGERPRINT);
    let (x, z, loaded) = decode_chunk_at(&encoded)
        .unwrap_or_else(|error| panic!("{label}: BCC decode failed: {error}"));
    assert_eq!((x, z), (OWNER.x, OWNER.z), "{label}: owner coordinates");
    assert_eq!(
        loaded.block_entities(),
        column.block_entities(),
        "{label}: core map"
    );
    assert_eq!(
        loaded.pending_block_entities(),
        column.pending_block_entities(),
        "{label}: pending map"
    );
    assert_eq!(
        loaded.feature_block_entities(),
        column.feature_block_entities(),
        "{label}: complete live entities"
    );
    assert_eq!(&loaded, column, "{label}: complete column");
    assert_eq!(
        encode_chunk(x, z, &loaded),
        encoded,
        "{label}: BCC re-encoding changed bytes"
    );
    loaded
}

#[test]
fn bcc_preserves_all_five_raw_pending_chest_inputs() {
    for case in cases() {
        let label = case["id"].as_str().unwrap();
        let input = native_tag(&case["input_nbt_sha256"]);
        assert_tag_types(&input, label);
        let column = pending_column(&input);
        let pending = only_pending(&column, label);
        assert_eq!(
            pending.full_nbt().unwrap(),
            input,
            "{label}: raw pending input"
        );
        let loaded = round_trip(&column, label);
        assert_eq!(
            only_pending(&loaded, label),
            pending,
            "{label}: entire pending entity"
        );
        assert_eq!(
            only_pending(&loaded, label).full_nbt().unwrap(),
            input,
            "{label}: typed input changed"
        );
        if label == "table-and-items" {
            let saved = only_pending(&loaded, label).full_nbt().unwrap();
            assert!(saved.get("LootTable").is_some());
            assert_eq!(saved.get("Items").unwrap().list().unwrap().len(), 3);
        }
    }
    println!("BCC: all 5 original raw pending chest inputs round-trip without materialization");
}

#[test]
fn bcc_preserves_live_chest_snapshots_and_retained_memory_seeds() {
    let mut observed_seeds = BTreeSet::new();
    for case in cases() {
        for phase in ["loaded_before_unpack", "after_first_unpack"] {
            let label = format!("{} / {phase}", case["id"].as_str().unwrap());
            let snapshot = &case["snapshots"][phase];
            let seed = memory_seed(snapshot);
            let mut load = native_tag(&snapshot["saved_nbt_sha256"]);
            // BCore's runtime representation retains the captured seed separately
            // from the native saved tag, which omits it once its key is consumed.
            if seed != 0 {
                load.compound_mut()
                    .unwrap()
                    .insert("LootTableSeed".into(), Nbt::Long(seed));
            }
            let mut column = chest_column();
            let state = column.get(LOCAL.0, LOCAL.1, LOCAL.2).unwrap();
            let template = TemplateBlockEntity::from_load(
                &StructureAssets::bundled().blocks,
                state,
                (8, 80, 8),
                load,
            )
            .unwrap();
            let entity = FeatureBlockEntity::from_template(&template).unwrap();
            assert!(column.set_feature_block_entity(OWNER, LOCAL.0, LOCAL.1, LOCAL.2, entity));
            assert_feature_snapshot(&column, snapshot, &label);
            let loaded = round_trip(&column, &label);
            assert_feature_snapshot(&loaded, snapshot, &label);
            observed_seeds.insert(runtime_seed(only_feature(&loaded, &label)));
        }
    }
    assert_eq!(observed_seeds, BTreeSet::from([-36, 0, 1, 29]));
    println!("BCC: all 10 captured before/after live chest representations retain typed data and memory seeds");
}

struct NoDraw;
impl LootRng for NoDraw {
    fn next_int(&mut self, _: i32) -> i32 {
        panic!("fresh materialized BCC chest requested an RNG draw")
    }
    fn next_float(&mut self) -> f32 {
        panic!("fresh materialized BCC chest requested an RNG draw")
    }
    fn next_bool(&mut self) -> bool {
        panic!("fresh materialized BCC chest requested an RNG draw")
    }
}

#[test]
fn bcc_fresh_materialized_chests_have_zero_seed_and_cannot_reroll() {
    let engine = historical_loot_engine();
    for case in cases() {
        let label = case["id"].as_str().unwrap();
        let snapshots = &case["snapshots"];
        let saved = native_tag(&snapshots["after_first_unpack"]["saved_nbt_sha256"]);
        assert!(saved.get("LootTable").is_none(), "{label}");
        assert!(saved.get("LootTableSeed").is_none(), "{label}");
        let mut column = pending_column(&saved);
        assert_eq!(only_pending(&column, label).full_nbt().unwrap(), saved);
        assert!(column
            .materialize_block_entity(OWNER, LOCAL.0, LOCAL.1, LOCAL.2)
            .unwrap());
        assert_feature_snapshot(&column, &snapshots["fresh_loaded_materialized"], label);
        let mut loaded = round_trip(&column, label);
        assert_feature_snapshot(&loaded, &snapshots["fresh_loaded_materialized"], label);
        assert_eq!(
            runtime_seed(only_feature(&loaded, label)),
            0,
            "{label}: fresh runtime seed"
        );

        assert!(loaded
            .materialize_block_entity(OWNER, LOCAL.0, LOCAL.1, LOCAL.2)
            .unwrap());
        assert_eq!(
            loaded, column,
            "{label}: repeated live lookup changed the column"
        );
        let full = only_feature(&loaded, label).full_nbt().unwrap();
        let mut inventory = ChestInventory::from_nbt(engine, &full).unwrap();
        let before = inventory.clone();
        assert_eq!(inventory.loot_seed(), 0, "{label}");
        assert!(inventory.unpack(engine).unwrap().is_none());
        assert!(inventory
            .unpack_with_rng(engine, LootContext::CHEST, &mut NoDraw)
            .unwrap()
            .is_none());
        assert_eq!(
            inventory, before,
            "{label}: reopened unpack changed inventory"
        );
        assert_inventory(&inventory, &snapshots["after_reopened_unpack"], label);

        for slot in 0..27 {
            assert!(inventory
                .unpack_with_rng(engine, LootContext::CHEST, &mut NoDraw)
                .unwrap()
                .is_none());
            assert_eq!(
                inventory.slots()[slot]
                    .as_ref()
                    .map_or(Value::Null, ItemStack::to_json),
                snapshots["after_reopened_getItem"]["slots"][slot],
                "{label}: reopened slot {slot}"
            );
        }
        assert_eq!(inventory, before, "{label}: slot reads changed inventory");
        assert_inventory(&inventory, &snapshots["after_reopened_getItem"], label);
        let mut written = full.clone();
        inventory.write_nbt(&mut written).unwrap();
        assert_eq!(
            written,
            native_tag(&snapshots["after_reopened_getItem"]["saved_nbt_sha256"]),
            "{label}: no-reroll saved NBT"
        );
        assert_tag_types(&written, label);
    }
    println!("BCC: all 5 fresh materialized loads default to seed zero; 135 guarded slot reads do not reroll");
}
