use super::*;
use crate::block_entity::PendingBlockEntity;
use crate::generation::{ChunkAccessKind, ChunkHolder, ChunkStatus, StageState};
use crate::loot::{CatalogSources, ItemStack, LegacyRandom};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};

const POS: ChunkPos = ChunkPos::new(0, 0);
const LOCAL: LocalPos = (8, 80, 8);

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/data/loot_26_1/chest-lifecycle-26.1.json"
    ))
    .unwrap()
}

fn nbt(fixture: &Value, digest: &Value) -> Nbt {
    serde_json::from_value(fixture["nbt_blobs"][digest.as_str().unwrap()]["typed"].clone()).unwrap()
}

fn world(tag: &Nbt, pending: bool) -> Arc<GenerationWorld> {
    let world = Arc::new(GenerationWorld::new(846692123413862008));
    let mut state = world.state.lock().unwrap();
    let mut holder = ChunkHolder::new(POS);
    for status in ChunkStatus::ALL.into_iter().take(ChunkStatus::Full.index()) {
        holder.progress.stages[status.index()].state = StageState::Complete;
        holder.progress.stages[status.index()].attempts = 1;
    }
    state.holders.insert((POS.x, POS.z), holder);
    let chunk = state.region.owned_chunk_mut(POS);
    let blocks = &StructureAssets::bundled().blocks;
    let chest = blocks.default_state("chest").unwrap();
    chunk.set(LOCAL.0, LOCAL.1, LOCAL.2, chest);
    if pending {
        assert!(chunk.set_pending_block_entity(
            LOCAL.0,
            LOCAL.1,
            LOCAL.2,
            PendingBlockEntity::from_nbt(tag)
        ));
    } else {
        let template =
            TemplateBlockEntity::from_load(blocks, chest, (8, 80, 8), tag.clone()).unwrap();
        chunk
            .feature_block_entities
            .insert(LOCAL, FeatureBlockEntity::from_template(&template).unwrap());
    }
    drop(state);
    world
}

fn convert(world: &GenerationWorld) {
    world
        .state
        .lock()
        .unwrap()
        .run_stage(world.generator, POS, ChunkStatus::Full)
        .unwrap();
}

fn saved_tag(world: &GenerationWorld) -> Nbt {
    let saved = world.prepare_chunk_save(POS, 0).unwrap();
    let mut tag = saved.block_entity_tags[&LOCAL].clone();
    assert_eq!(
        tag.compound_mut().unwrap().remove("keepPacked"),
        Some(Nbt::Byte(0))
    );
    tag
}

fn slots(inventory: &ChestInventory) -> Value {
    Value::Array(
        inventory
            .slots()
            .iter()
            .map(|stack| stack.as_ref().map_or(Value::Null, ItemStack::to_json))
            .collect(),
    )
}

fn assert_snapshot(
    world: &GenerationWorld,
    actual: &ChestInventory,
    native: &Value,
    fixture: &Value,
) {
    assert_eq!(slots(actual), native["slots"]);
    assert_eq!(
        actual.loot_table().map(Value::from).unwrap_or(Value::Null),
        native["loot_table"]
    );
    assert_eq!(actual.loot_seed().to_string(), native["loot_seed_i64"]);
    assert_eq!(saved_tag(world), nbt(fixture, &native["saved_nbt_sha256"]));
}

struct NoDraw;
impl LootRng for NoDraw {
    fn next_int(&mut self, _: i32) -> i32 {
        panic!("unexpected RNG draw")
    }
    fn next_float(&mut self) -> f32 {
        panic!("unexpected RNG draw")
    }
    fn next_bool(&mut self) -> bool {
        panic!("unexpected RNG draw")
    }
}

fn read_again(world: &GenerationWorld) -> ChestInventory {
    world
        .unpack_chest_with_rng(POS, LOCAL, LootContext::CHEST, &mut NoDraw)
        .unwrap()
        .unwrap()
}

#[test]
fn canonical_chests_follow_all_five_native_save_reload_histories() {
    let fixture = fixture();
    let mut compared = 0;
    for case in fixture["cases"].as_array().unwrap() {
        for pending in [false, true] {
            let input = nbt(&fixture, &case["input_nbt_sha256"]);
            let initial_save = world(&input, pending);
            convert(&initial_save);
            assert_eq!(
                saved_tag(&initial_save),
                nbt(
                    &fixture,
                    &case["snapshots"]["loaded_before_unpack"]["saved_nbt_sha256"]
                )
            );

            // Saving performs a lookup; use a distinct instance to exercise
            // direct pending-to-unpack without materializing the tag first.
            let world = world(&input, pending);
            convert(&world);
            let before = world.chunk_snapshot(POS).unwrap().unwrap();
            assert_eq!(
                before.pending_block_entities().contains_key(&LOCAL),
                pending
            );
            assert_eq!(
                before.feature_block_entities().contains_key(&LOCAL),
                !pending
            );
            let first = world.unpack_chest(POS, LOCAL).unwrap().unwrap();
            assert_snapshot(
                &world,
                &first,
                &case["snapshots"]["after_first_unpack"],
                &fixture,
            );
            let second = read_again(&world);
            assert_eq!(first, second);
            assert_snapshot(
                &world,
                &second,
                &case["snapshots"]["after_second_unpack"],
                &fixture,
            );
            let after = world.chunk_snapshot(POS).unwrap().unwrap();
            assert_eq!(before.states(), after.states());
            assert_eq!(before.tick_requests(), after.tick_requests());
            assert_eq!(
                before.postprocessing_positions(),
                after.postprocessing_positions()
            );
            assert!(after.pending_block_entities().is_empty());
            let saved = saved_tag(&world);
            let fresh = self::world(&saved, true);
            convert(&fresh);
            let reopened = read_again(&fresh);
            assert_snapshot(
                &fresh,
                &reopened,
                &case["snapshots"]["after_reopened_unpack"],
                &fixture,
            );
            assert_eq!(reopened.loot_seed(), 0);
            assert_eq!(reopened.slots(), first.slots());
            compared += 1;
        }
    }
    assert_eq!(compared, 10);
}

fn deferred(table: &str, seed: i64) -> Nbt {
    Nbt::Compound(BTreeMap::from([
        ("id".into(), Nbt::String("minecraft:chest".into())),
        ("x".into(), Nbt::Int(8)),
        ("y".into(), Nbt::Int(80)),
        ("z".into(), Nbt::Int(8)),
        ("components".into(), Nbt::empty_compound()),
        ("LootTable".into(), Nbt::String(table.into())),
        ("LootTableSeed".into(), Nbt::Long(seed)),
    ]))
}

fn block_storage(world: &GenerationWorld) -> usize {
    world
        .chunk_access(POS, ChunkStatus::Full)
        .unwrap()
        .unwrap()
        .read(|view| view.chunk.states().as_ptr() as usize)
        .unwrap()
}

#[test]
fn live_core_and_feature_members_remain_shared_by_retained_proto() {
    for core in [false, true] {
        let tag = deferred("minecraft:chests/simple_dungeon", i64::MIN);
        let world = world(&tag, false);
        if core {
            assert!(world
                .state
                .lock()
                .unwrap()
                .region
                .owned_chunk_mut(POS)
                .set_block_entity(
                    8,
                    80,
                    8,
                    BlockEntity::DungeonChest {
                        loot_seed: i64::MIN
                    }
                ));
        }
        let old = world
            .returned_chunk(POS, ChunkStatus::Light)
            .unwrap()
            .unwrap();
        let snapshot = old.prepare_save(0).unwrap();
        convert(&world);
        let level = world
            .returned_chunk(POS, ChunkStatus::Full)
            .unwrap()
            .unwrap();
        assert!(!old.same_object(&level));
        let pointer = block_storage(&world);
        let inventory = world.unpack_chest(POS, LOCAL).unwrap().unwrap();
        assert_eq!(inventory.loot_seed(), i64::MIN);
        let live = saved_tag(&world);
        let old_saved = old.prepare_save(0).unwrap();
        assert_eq!(old.kind(), ChunkAccessKind::Proto);
        assert_eq!(old_saved.block_entity_tags[&LOCAL], live);
        assert_eq!(old_saved.view.persisted_status, ChunkStatus::Spawn);
        assert!(old_saved.block_entity_tags[&LOCAL]
            .get("LootTable")
            .is_none());
        assert!(snapshot.block_entity_tags[&LOCAL]
            .get("LootTable")
            .is_some());
        assert!(!old.same_object(
            &world
                .returned_chunk(POS, ChunkStatus::Light)
                .unwrap()
                .unwrap()
        ));
        assert_eq!(block_storage(&world), pointer);
    }
}

#[test]
fn level_only_pending_promotions_do_not_retarget_the_old_protos_saved_tag() {
    let tag = deferred("minecraft:chests/simple_dungeon", 1);
    let world = world(&tag, true);
    let old = world
        .returned_chunk(POS, ChunkStatus::Light)
        .unwrap()
        .unwrap();
    convert(&world);
    let inventory = world.unpack_chest(POS, LOCAL).unwrap().unwrap();
    assert!(inventory.slots().iter().any(Option::is_some));
    assert_eq!(old.prepare_save(0).unwrap().block_entity_tags[&LOCAL], tag);
    assert!(old
        .view()
        .unwrap()
        .chunk
        .feature_block_entities()
        .is_empty());
    assert!(saved_tag(&world).get("LootTable").is_none());
}

#[test]
fn unsupported_preflight_preserves_pending_and_live_state_and_source() {
    for pending in [false, true] {
        for (table, seed) in [
            ("minecraft:chests/shipwreck_map", 7),
            ("minecraft:chests/simple_dungeon", 0),
        ] {
            let world = world(&deferred(table, seed), pending);
            convert(&world);
            let before = world.chunk_snapshot(POS).unwrap().unwrap();
            assert!(world.unpack_chest(POS, LOCAL).is_err());
            assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
        }
        let world = world(&deferred("minecraft:chests/simple_dungeon", 7), pending);
        convert(&world);
        let before = world.chunk_snapshot(POS).unwrap().unwrap();
        assert!(world
            .unpack_chest_with_rng(
                POS,
                LOCAL,
                LootContext {
                    luck: 0.0,
                    player_present: true,
                    ..LootContext::CHEST
                },
                &mut NoDraw
            )
            .is_err());
        assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
    }
}

#[test]
fn concurrent_reads_share_one_fill_and_preserve_rng_continuation() {
    let seed = -36;
    let world = world(
        &deferred("minecraft:chests/trial_chambers/reward_ominous_rare", seed),
        true,
    );
    convert(&world);
    let barrier = Barrier::new(8);
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let world = &world;
                let barrier = &barrier;
                scope.spawn(move || {
                    let mut rng = LegacyRandom::new(seed);
                    let before = rng.state();
                    barrier.wait();
                    let inventory = world
                        .unpack_chest_with_rng(POS, LOCAL, LootContext::CHEST, &mut rng)
                        .unwrap()
                        .unwrap();
                    (inventory, before, rng.state())
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results
            .iter()
            .filter(|(_, before, after)| before != after)
            .count(),
        1
    );
    for (inventory, _, _) in &results {
        assert_eq!(inventory, &results[0].0);
    }
    assert!(saved_tag(&world).get("LootTable").is_none());
}

#[test]
fn late_evaluation_error_commits_consumed_key_before_returning_error() {
    let tables = json!({"example:failure": {
        "type": "minecraft:chest",
        "pools": [{"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:stone",
            "functions": [{"function": "minecraft:set_count", "count": {"type": "minecraft:uniform", "min": -2147483648_i64, "max": 2147483647_i64}}]}],
            "conditions": [{"condition": "minecraft:random_chance", "chance": 1.0}]}]
    }}).to_string();
    let engine = LootEngine::from_sources(CatalogSources {
        tables: &tables,
        ..CatalogSources::VANILLA
    })
    .unwrap();
    let world = world(&deferred("example:failure", 19), true);
    convert(&world);
    let mut rng = LegacyRandom::new(19);
    let initial = rng.state();
    let result = world.access_chest(POS, LOCAL, &engine, |inventory, _, _| {
        inventory.unpack_with_rng(&engine, LootContext::CHEST, &mut rng)
    });
    assert!(result.is_err());
    assert_ne!(initial, rng.state());
    assert!(saved_tag(&world).get("LootTable").is_none());
    let inventory = read_again(&world);
    assert_eq!(inventory.loot_seed(), 19);
    assert!(inventory.slots().iter().all(Option::is_none));
}

#[test]
fn access_checks_full_geometry_and_single_chest_state_without_synthesizing_data() {
    let world = world(&deferred("minecraft:chests/simple_dungeon", 1), true);
    let before = world.chunk_snapshot(POS).unwrap().unwrap();
    assert!(matches!(
        world.unpack_chest(POS, LOCAL),
        Err(GenerationError::NotConverted(POS))
    ));
    assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
    convert(&world);
    for local in [(16, 80, 8), (8, crate::MAX_Y + 1, 8), (7, 80, 8)] {
        assert!(world.unpack_chest(POS, local).unwrap().is_none());
    }
    let blocks = &StructureAssets::bundled().blocks;
    let double = blocks
        .with_property(blocks.default_state("chest").unwrap(), "type", "left")
        .unwrap();
    world
        .state
        .lock()
        .unwrap()
        .region
        .owned_chunk_mut(POS)
        .set(8, 80, 8, double);
    let before = world.chunk_snapshot(POS).unwrap().unwrap();
    assert!(world.unpack_chest(POS, LOCAL).is_err());
    assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
}

#[test]
fn mineshaft_loot_uses_the_loaded_chest_biome_and_preserves_the_supplied_source() {
    let engine = LootEngine::vanilla().unwrap();
    let table = engine
        .table("minecraft:chests/abandoned_mineshaft")
        .unwrap();
    for biome in [
        bcore_registry::biome::PLAINS,
        bcore_registry::biome::SULFUR_CAVES,
    ] {
        for seed in [17, -17] {
            let world = world(&deferred(table.key(), seed), true);
            world
                .state
                .lock()
                .unwrap()
                .region
                .owned_chunk_mut(POS)
                .noise_biomes = Some(vec![biome; 1536]);
            convert(&world);
            let mut rng = LegacyRandom::new(seed);
            let mut expected_rng = rng.clone();
            let expected = {
                let state = world.state.lock().unwrap();
                let lookup = ChestLootWorld {
                    state: &state,
                    zoom: crate::biome_zoom::BiomeZoom::new(world.generator.seed()),
                };
                assert_eq!(lookup.biome_if_loaded([8, 80, 8]).unwrap(), Some(biome));
                assert_eq!(lookup.biome_if_loaded([1000, 80, 1000]).unwrap(), None);
                let mut slots = std::array::from_fn(|_| None);
                table
                    .fill_with_rng(
                        &mut slots,
                        LootContext::CHEST.with_location(Some([8.5, 80.5, 8.5]), &lookup),
                        &mut expected_rng,
                    )
                    .unwrap();
                slots
            };
            let inventory = world
                .unpack_chest_with_rng(POS, LOCAL, LootContext::CHEST, &mut rng)
                .unwrap()
                .unwrap();
            assert_eq!(inventory.slots(), &expected);
            assert_eq!(rng.state(), expected_rng.state());
            assert_eq!(inventory.loot_table(), None);
            assert_eq!(read_again(&world), inventory);
            assert!(saved_tag(&world).get("LootTable").is_none());
        }
    }
}

#[test]
fn loaded_chest_lookup_reports_absent_quart_biomes_without_requesting_generation() {
    let world = world(&deferred("minecraft:chests/simple_dungeon", 17), true);
    convert(&world);
    let state = world.state.lock().unwrap();
    let lookup = ChestLootWorld {
        state: &state,
        zoom: crate::biome_zoom::BiomeZoom::new(world.generator.seed()),
    };
    assert_eq!(
        lookup.biome_if_loaded([8, 80, 8]),
        Err(loot::LootError::UnsupportedContext(
            "loaded-world quart biomes"
        ))
    );
    assert_eq!(state.holders.len(), 1);
}

#[test]
fn no_key_access_preserves_the_complete_live_entity_and_load_payload() {
    let fixture = fixture();
    let preset = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "preset-component-items")
        .unwrap();
    let world = world(&nbt(&fixture, &preset["input_nbt_sha256"]), false);
    convert(&world);
    let before = world.chunk_snapshot(POS).unwrap().unwrap();
    let first = world.unpack_chest(POS, LOCAL).unwrap().unwrap();
    let second = world
        .unpack_chest_with_rng(
            POS,
            LOCAL,
            LootContext {
                luck: f32::NAN,
                player_present: true,
                ..LootContext::CHEST
            },
            &mut NoDraw,
        )
        .unwrap()
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
}

#[test]
fn saved_load_normalizes_accepted_inventory_forms_before_any_unpack() {
    // Admission variants of captured data, not additional native captures.
    let fixture = fixture();
    let preset = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "preset-component-items")
        .unwrap();
    let expected = nbt(
        &fixture,
        &preset["snapshots"]["loaded_before_unpack"]["saved_nbt_sha256"],
    );
    let mut unsorted = nbt(&fixture, &preset["input_nbt_sha256"]);
    let Nbt::List { values, .. } = unsorted.compound_mut().unwrap().get_mut("Items").unwrap()
    else {
        panic!("preset item list")
    };
    values.reverse();
    unsorted
        .compound_mut()
        .unwrap()
        .insert("LootTableSeed".into(), Nbt::Long(-88));
    let world = world(&unsorted, true);
    convert(&world);
    assert_eq!(saved_tag(&world), expected);
    assert_eq!(read_again(&world).loot_seed(), -88);

    let mut empty = expected.clone();
    empty.compound_mut().unwrap().insert(
        "Items".into(),
        Nbt::List {
            element_type: 10,
            values: Vec::new(),
        },
    );
    let world = self::world(&empty, true);
    convert(&world);
    assert_eq!(
        saved_tag(&world).get("Items"),
        Some(&Nbt::List {
            element_type: 0,
            values: Vec::new(),
        })
    );

    let world = self::world(&deferred("chests/simple_dungeon", -1), true);
    convert(&world);
    assert_eq!(
        saved_tag(&world),
        deferred("minecraft:chests/simple_dungeon", -1)
    );
}
