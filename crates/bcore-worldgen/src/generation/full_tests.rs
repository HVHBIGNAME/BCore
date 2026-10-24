use super::*;
use crate::block_entity::PendingBlockEntity;
use crate::feature_world::FeatureWorld;
use crate::generation::{ChunkHolder, FeatureBlockEntity, StageState};
use crate::structure::template::TemplateBlockEntity;
use crate::structure::template_pool::StructureAssets;
use crate::tick_queue::TickPriority;
use crate::tick_request::{TickRequest, TickTarget};
use crate::{block, MIN_Y};

// Native authority: scripts/full-lifecycle-reference/LifecycleProbe.java:99-171.
// Fixture full-lifecycle-26_1-v1.zip SHA256:
// 499950d70f9422e04c45496dfb490b9be21496d1a5cdeb3cea3512e21c090637
// Observations SHA256: 07be9301756ca9fcee95012be20018f4fc2969ae1a66bba782dec1ee7b657be7.
const POS: ChunkPos = ChunkPos::new(0, 0);

fn requests() -> [TickRequest; 4] {
    [
        TickRequest {
            block_pos: [4, 80, 4],
            target: TickTarget::Block(block::STONE),
            delay: -5,
        },
        TickRequest {
            block_pos: [4, 80, 4],
            target: TickTarget::Block(block::STONE),
            delay: -9,
        },
        TickRequest {
            block_pos: [5, 80, 4],
            target: TickTarget::Block(block::STONE),
            delay: 7,
        },
        TickRequest {
            block_pos: [6, 80, 4],
            target: TickTarget::Fluid(2),
            delay: 11,
        },
    ]
}

fn sensor_tag(x: i32, frequency: i32) -> Nbt {
    Nbt::Compound(BTreeMap::from([
        ("id".into(), Nbt::String("minecraft:sculk_sensor".into())),
        ("x".into(), Nbt::Int(x)),
        ("y".into(), Nbt::Int(80)),
        ("z".into(), Nbt::Int(1)),
        ("components".into(), Nbt::empty_compound()),
        ("last_vibration_frequency".into(), Nbt::Int(frequency)),
        (
            "listener".into(),
            Nbt::Compound(BTreeMap::from([
                (
                    "selector".into(),
                    Nbt::Compound(BTreeMap::from([("tick".into(), Nbt::Long(-1))])),
                ),
                ("event_delay".into(), Nbt::Int(0)),
            ])),
        ),
    ]))
}

fn native_proto() -> GeneratedChunk {
    let mut chunk = GeneratedChunk::new(POS);
    let blocks = &StructureAssets::bundled().blocks;
    for x in 1..=3 {
        let name = if x == 2 {
            "sculk_shrieker"
        } else {
            "sculk_sensor"
        };
        let state = blocks.default_state(name).unwrap();
        assert!(chunk.set(x, 80, 1, state));
        if x == 3 {
            let entity =
                TemplateBlockEntity::from_load(blocks, state, (3, 80, 1), sensor_tag(3, 9))
                    .unwrap();
            chunk.feature_block_entities.insert(
                (3, 80, 1),
                FeatureBlockEntity::from_template(&entity).unwrap(),
            );
        } else {
            assert!(chunk.set_pending_block_entity(
                x,
                80,
                1,
                PendingBlockEntity::dummy((x as i32, 80, 1))
            ));
        }
    }
    chunk.capture_worldgen_heightmaps(true);
    chunk.postprocessing.extend([(8, 80, 8), (8, 80, 8)]);
    for request in requests() {
        assert!(chunk.add_tick_request(request));
    }
    chunk
}

fn spawn_world(chunk: GeneratedChunk) -> GenerationWorld {
    let world = GenerationWorld::new(846692123413862008);
    let mut state = world.state.lock().unwrap();
    let pos = chunk.pos;
    let mut holder = ChunkHolder::new(pos);
    for status in ChunkStatus::ALL.into_iter().take(ChunkStatus::Full.index()) {
        holder.progress.stages[status.index()].state = StageState::Complete;
        holder.progress.stages[status.index()].attempts = 1;
    }
    state.holders.insert((pos.x, pos.z), holder);
    *state.region.owned_chunk_mut(pos) = chunk;
    state.test_stage = Some(|_, _, _| panic!("FULL must execute the real converter"));
    drop(state);
    world
}

fn convert(world: &GenerationWorld) -> Result<(), GenerationError> {
    world
        .state
        .lock()
        .unwrap()
        .run_stage(world.generator, POS, ChunkStatus::Full)
}

fn identities(view: ChunkRead<'_>) -> (usize, usize) {
    (
        view.chunk.states().as_ptr() as usize,
        view.chunk.feature_block_entities()[&(3, 80, 1)].full_data["id"]
            .as_str()
            .unwrap()
            .as_ptr() as usize,
    )
}

#[test]
fn native_full_claim_preserves_live_data_pending_tags_marks_and_lower_view_identity() {
    let world = spawn_world(native_proto());
    let lower = world
        .chunk_access(POS, ChunkStatus::Light)
        .unwrap()
        .unwrap();
    let before = lower.snapshot().unwrap();
    let pointers = lower.read(identities).unwrap();
    assert!(world
        .chunk_access(POS, ChunkStatus::Full)
        .unwrap()
        .is_none());
    convert(&world).unwrap();
    let full = world.chunk_access(POS, ChunkStatus::Full).unwrap().unwrap();
    assert!(lower.same_holder(&full));
    assert_eq!(
        lower.read(|view| view.kind).unwrap(),
        ChunkAccessKind::ImposterProto
    );
    let level_identity = full
        .read(|view| {
            assert_eq!(view.kind, ChunkAccessKind::Level);
            assert_eq!(
                view.chunk.pending_block_entities(),
                before.pending_block_entities()
            );
            assert_eq!(
                view.chunk.feature_block_entities(),
                before.feature_block_entities()
            );
            assert_eq!(
                view.chunk.feature_block_entities()[&(3, 80, 1)]
                    .full_nbt()
                    .unwrap(),
                sensor_tag(3, 9)
            );
            assert!(view.chunk.worldgen_heightmaps().is_none());
            assert!(view.chunk.runtime_heightmaps().is_some());
            assert_eq!(
                view.chunk.postprocessing_positions(),
                &[(8, 80, 8), (8, 80, 8)]
            );
            assert_eq!(view.chunk.tick_requests(), &requests());
            let level = view.level.unwrap();
            assert_eq!(
                level.retired_proto_heightmaps(),
                before.worldgen_heightmaps()
            );
            assert_eq!(level.block_ticks().count(), 2);
            assert_eq!(level.fluid_ticks().count(), 1);
            assert!(level.block_ticks().peek().is_none());
            assert!(level.fluid_ticks().get_all().is_empty());
            assert!(level
                .block_ticks()
                .pack(99)
                .iter()
                .all(|tick| tick.delay == 0));
            level as *const LevelChunkState as usize
        })
        .unwrap();
    assert_eq!(lower.read(identities).unwrap(), pointers);
    assert_eq!(full.read(identities).unwrap(), pointers);
    assert_eq!(
        lower
            .read(|view| view.height(FeatureHeightmap::WorldSurfaceWg, 1, 1))
            .unwrap(),
        Some(80)
    );
    assert_eq!(
        full.read(|view| view.height(FeatureHeightmap::WorldSurface, 1, 1))
            .unwrap(),
        Some(80)
    );
    assert!(
        before.worldgen_heightmaps().is_some(),
        "detached proto snapshot keeps its own maps"
    );
    convert(&world).unwrap();
    assert_eq!(
        full.read(|view| view.level.unwrap() as *const LevelChunkState as usize)
            .unwrap(),
        level_identity
    );
    let progress = world.progress(POS).unwrap().unwrap();
    assert_eq!(progress.stage(ChunkStatus::Full).attempts, 1);
    assert_eq!(progress.stage(ChunkStatus::Full).state, StageState::Partial);
    assert_eq!(progress.completed_status(), Some(ChunkStatus::Spawn));
    assert!(progress
        .stage(ChunkStatus::Full)
        .missing
        .iter()
        .any(|reason| reason.contains("registration")));
}

#[test]
fn native_level_save_is_observable_and_repeat_full_does_not_reload_live_entities() {
    let world = spawn_world(native_proto());
    assert!(matches!(
        world.prepare_chunk_save(POS, 0),
        Err(GenerationError::NotConverted(POS))
    ));
    convert(&world).unwrap();
    let lower = world
        .chunk_access(POS, ChunkStatus::Light)
        .unwrap()
        .unwrap();
    let before = lower.snapshot().unwrap();
    let pointers = lower.read(identities).unwrap();
    let saved = world.prepare_chunk_save(POS, 0).unwrap();
    assert_eq!(before.pending_block_entities().len(), 2);
    assert!(saved.chunk.pending_block_entities().is_empty());
    assert_eq!(saved.chunk.feature_block_entities().len(), 3);
    assert_eq!(saved.block_entity_tags.len(), 3);
    let mut first_sensor = sensor_tag(1, 0);
    first_sensor
        .compound_mut()
        .unwrap()
        .insert("keepPacked".into(), Nbt::Byte(0));
    assert_eq!(saved.block_entity_tags[&(1, 80, 1)], first_sensor);
    let mut live_sensor = sensor_tag(3, 9);
    live_sensor
        .compound_mut()
        .unwrap()
        .insert("keepPacked".into(), Nbt::Byte(0));
    assert_eq!(saved.block_entity_tags[&(3, 80, 1)], live_sensor);
    let shrieker = &saved.block_entity_tags[&(2, 80, 1)];
    assert_eq!(
        shrieker.get("id").and_then(Nbt::string),
        Some("minecraft:sculk_shrieker")
    );
    assert_eq!(shrieker.get("warning_level"), Some(&Nbt::Int(0)));
    assert_eq!(shrieker.get("keepPacked"), Some(&Nbt::Byte(0)));
    assert_eq!(
        saved.chunk.feature_block_entities()[&(3, 80, 1)]
            .full_nbt()
            .unwrap(),
        sensor_tag(3, 9)
    );
    assert_eq!(lower.read(identities).unwrap(), pointers);
    assert_eq!(lower.snapshot().unwrap(), saved.chunk);
    assert_eq!(saved.chunk.tick_requests(), &requests());
    assert_eq!(
        saved.chunk.postprocessing_positions(),
        before.postprocessing_positions()
    );
    assert_eq!(saved.block_ticks.len(), 2);
    assert_eq!(saved.fluid_ticks.len(), 1);
    convert(&world).unwrap();
    let again = world.prepare_chunk_save(POS, 700).unwrap();
    assert_eq!(again.chunk, saved.chunk);
    assert_eq!(again.block_entity_tags, saved.block_entity_tags);
    assert_eq!(again.block_ticks, saved.block_ticks);
    assert_eq!(again.fluid_ticks, saved.fluid_ticks);
    assert!(lower
        .read(|view| view.level.unwrap().block_ticks().peek().is_none())
        .unwrap());
}

#[test]
fn lower_lookup_materializes_only_its_position_and_snapshots_stay_inert() {
    let world = spawn_world(native_proto());
    convert(&world).unwrap();
    let lower = world
        .chunk_access(POS, ChunkStatus::Light)
        .unwrap()
        .unwrap();
    let detached = lower.snapshot().unwrap();
    assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), detached);
    assert!(lower.lookup_block_entity(1, 80, 1).unwrap());
    assert!(lower.lookup_block_entity(1, 80, 1).unwrap());
    assert!(!lower.lookup_block_entity(8, 80, 8).unwrap());
    assert!(!lower.lookup_block_entity(16, 80, 1).unwrap());
    assert_eq!(lower.snapshot().unwrap().pending_block_entities().len(), 1);
    assert_eq!(detached.pending_block_entities().len(), 2);
    convert(&world).unwrap();
    assert_eq!(lower.snapshot().unwrap().pending_block_entities().len(), 1);
    assert_eq!(
        world
            .prepare_chunk_save(POS, 0)
            .unwrap()
            .block_entity_tags
            .len(),
        3
    );
}

#[test]
fn failed_save_keeps_pending_batch_and_does_not_fail_or_repeat_full() {
    let mut proto = native_proto();
    let mut unsupported = sensor_tag(2, 0);
    unsupported
        .compound_mut()
        .unwrap()
        .insert("id".into(), Nbt::String("minecraft:sculk_shrieker".into()));
    unsupported
        .compound_mut()
        .unwrap()
        .insert("unknown_runtime_field".into(), Nbt::Int(7));
    proto
        .pending_block_entities
        .insert((2, 80, 1), PendingBlockEntity::from_nbt(&unsupported));
    let world = spawn_world(proto);
    convert(&world).unwrap();
    let before = world.chunk_snapshot(POS).unwrap().unwrap();
    assert!(matches!(
        world.prepare_chunk_save(POS, 0),
        Err(GenerationError::BlockEntityFailed {
            boundary: "save",
            ..
        })
    ));
    assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
    convert(&world).unwrap();
    let progress = world.progress(POS).unwrap().unwrap();
    assert_eq!(progress.stage(ChunkStatus::Full).attempts, 1);
    assert_eq!(progress.stage(ChunkStatus::Full).state, StageState::Partial);
}

#[test]
fn full_review_failed_save_tag_encoding_does_not_commit_pending_entities() {
    let mut proto = native_proto();
    let live = proto.feature_block_entities.get_mut(&(3, 80, 1)).unwrap();
    let valid_tag = live.typed_data.clone();
    live.typed_data = serde_json::json!([10, {"invalid_byte": [1, 128]}]);
    let world = spawn_world(proto);
    convert(&world).unwrap();
    let access = world.chunk_access(POS, ChunkStatus::Full).unwrap().unwrap();
    let before = access.snapshot().unwrap();
    let pointers = access.read(identities).unwrap();
    for _ in 0..2 {
        assert!(matches!(
            world.prepare_chunk_save(POS, 0),
            Err(GenerationError::BlockEntityFailed {
                boundary: "save",
                ..
            })
        ));
        assert_eq!(access.snapshot().unwrap(), before);
        assert_eq!(access.read(identities).unwrap(), pointers);
    }
    world
        .state
        .lock()
        .unwrap()
        .region
        .owned_chunk_mut(POS)
        .feature_block_entities
        .get_mut(&(3, 80, 1))
        .unwrap()
        .typed_data = valid_tag;
    let saved = world.prepare_chunk_save(POS, 0).unwrap();
    assert!(saved.chunk.pending_block_entities().is_empty());
    assert_eq!(saved.block_entity_tags.len(), 3);
    assert_eq!(access.read(identities).unwrap(), pointers);
    assert_eq!(
        world
            .progress(POS)
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::Full)
            .attempts,
        1
    );
}

#[test]
fn failed_conversion_is_claimed_once_before_retiring_any_proto_data() {
    let mut proto = native_proto();
    proto.tick_requests.push(TickRequest {
        block_pos: [32, 80, 0],
        ..requests()[0]
    });
    let before = proto.clone();
    let world = spawn_world(proto);
    let error = convert(&world).unwrap_err();
    assert!(matches!(
        error,
        GenerationError::StageFailed {
            status: ChunkStatus::Full,
            ..
        }
    ));
    assert_eq!(convert(&world).unwrap_err(), error);
    assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
    assert!(world
        .chunk_access(POS, ChunkStatus::Full)
        .unwrap()
        .is_none());
    assert_eq!(
        world
            .progress(POS)
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::Full)
            .attempts,
        1
    );
}

#[test]
fn native_prepared_priorities_survive_the_constructor_until_explicit_unpack() {
    // The native witness directly schedules NORMAL/HIGH priorities. Feature
    // TickRequest history has a NORMAL-only contract; exercise the same converter
    // constructor with the witness's prepared queue as well as the real claim above.
    let mut proto = native_proto();
    let mut prepared = PreparedTickQueues::default();
    for (request, priority) in requests().into_iter().zip([
        TickPriority::Normal,
        TickPriority::High,
        TickPriority::High,
        TickPriority::Normal,
    ]) {
        prepared.schedule_request_with_priority(request, priority);
    }
    let mut level = LevelChunkState::from_proto(&mut proto, prepared);
    assert!(level.block_ticks.peek().is_none());
    assert_eq!(
        level
            .block_ticks
            .pack(0)
            .iter()
            .map(|tick| (tick.delay, tick.priority))
            .collect::<Vec<_>>(),
        vec![(0, TickPriority::Normal), (0, TickPriority::High)]
    );
    level.block_ticks.unpack(0);
    let head = level.block_ticks.peek().unwrap();
    assert_eq!(
        (head.trigger_tick, head.priority, head.sub_tick_order),
        (0, TickPriority::High, -1)
    );
    let active = level.block_ticks.get_all().to_vec();
    level.block_ticks.unpack(99);
    assert_eq!(level.block_ticks.get_all(), active);
    assert_eq!(level.block_ticks.count(), 2);
    assert_eq!(proto.postprocessing_positions(), &[(8, 80, 8), (8, 80, 8)]);
    assert_eq!(proto.pending_block_entities().len(), 2);
    assert_eq!(proto.tick_requests(), &requests());
}

#[test]
fn retained_queue_unpack_is_explicit_and_does_not_execute_or_postprocess() {
    let world = spawn_world(native_proto());
    convert(&world).unwrap();
    let before = world.chunk_snapshot(POS).unwrap().unwrap();
    world.unpack_chunk_ticks(POS, 123).unwrap();
    world.unpack_chunk_ticks(POS, 999).unwrap();
    let lower = world
        .chunk_access(POS, ChunkStatus::Light)
        .unwrap()
        .unwrap();
    lower
        .read(|view| {
            let level = view.level.unwrap();
            assert_eq!(level.block_ticks().count(), 2);
            assert_eq!(level.block_ticks().peek().unwrap().trigger_tick, 123);
            assert_eq!(level.block_ticks().peek().unwrap().sub_tick_order, -2);
            assert_eq!(level.fluid_ticks().peek().unwrap().sub_tick_order, -1);
        })
        .unwrap();
    assert_eq!(world.chunk_snapshot(POS).unwrap().unwrap(), before);
    let saved = world.prepare_chunk_save(POS, 130).unwrap();
    assert!(saved.block_ticks.iter().all(|tick| tick.delay == -7));
    assert_eq!(saved.chunk.tick_requests(), &requests());
    assert_eq!(
        saved.chunk.postprocessing_positions(),
        before.postprocessing_positions()
    );
}

#[test]
fn converted_lower_height_queries_redirect_stale_wg_and_keep_first_free_convention() {
    let mut proto = native_proto();
    proto.set(9, 40, 9, block::STONE);
    proto.capture_worldgen_heightmaps(true);
    proto.set(9, 80, 9, block::OAK_LEAVES);
    let world = spawn_world(proto);
    let lower = world
        .chunk_access(POS, ChunkStatus::Light)
        .unwrap()
        .unwrap();
    assert_eq!(
        lower
            .read(|view| view.height(FeatureHeightmap::WorldSurfaceWg, 9, 9))
            .unwrap(),
        Some(40)
    );
    convert(&world).unwrap();
    assert_eq!(
        lower
            .read(|view| view.height(FeatureHeightmap::WorldSurfaceWg, 9, 9))
            .unwrap(),
        Some(80)
    );
    assert_eq!(
        lower
            .read(|view| view.height(FeatureHeightmap::OceanFloorWg, 9, 9))
            .unwrap(),
        Some(80)
    );
    assert_eq!(
        lower
            .read(|view| view.height(FeatureHeightmap::MotionBlockingNoLeaves, 9, 9))
            .unwrap(),
        Some(40)
    );
    assert_eq!(
        lower
            .read(|view| view.height(FeatureHeightmap::WorldSurfaceWg, 15, 15))
            .unwrap(),
        Some(MIN_Y - 1)
    );
    let mut state = world.state.lock().unwrap();
    state.region.begin_source_with_levels(
        POS,
        ChunkStatus::Features,
        [((0, 0), ChunkStatus::Spawn)].into(),
        [(0, 0)].into(),
    );
    assert_eq!(
        state
            .region
            .feature_height(FeatureHeightmap::WorldSurfaceWg, 9, 9),
        81
    );
    assert_eq!(
        crate::ore::OreWorld::ocean_floor_wg(&state.region, 9, 9),
        81
    );
    assert!(!state.region.set_feature_block((8, 80, 8), block::STONE, 18));
    assert_eq!(
        state.region.owned_chunk_ref(POS).unwrap().get(8, 80, 8),
        Some(block::AIR)
    );
    state.region.end_source();
}

#[test]
fn explicit_level_save_preserves_core_block_entities_and_their_nbt_widths() {
    let mut proto = native_proto();
    let catalog = crate::block_predicate::catalog();
    for (x, name, data) in [
        (
            7,
            "chest",
            BlockEntity::DungeonChest {
                loot_seed: i64::MIN,
            },
        ),
        (
            8,
            "spawner",
            BlockEntity::Spawner {
                mob: crate::block_entity::SpawnerMob::CaveSpider,
            },
        ),
        (
            9,
            "bee_nest",
            BlockEntity::Beehive {
                ticks_in_hive: vec![i32::MIN, 97],
            },
        ),
    ] {
        proto.set(x, 70, 1, catalog.default_state(name).unwrap());
        assert!(proto.set_block_entity(x, 70, 1, data));
    }
    let core = proto.block_entities().clone();
    let world = spawn_world(proto);
    convert(&world).unwrap();
    let saved = world.prepare_chunk_save(POS, 0).unwrap();
    assert_eq!(saved.chunk.block_entities(), &core);
    for (&local @ (x, y, z), entity) in &core {
        let mut tag = saved.block_entity_tags[&local].clone();
        assert_eq!(
            tag.compound_mut().unwrap().remove("keepPacked"),
            Some(Nbt::Byte(0))
        );
        assert_eq!(tag.to_json(), entity.full_data((x as i32, y, z as i32)));
    }
    assert_eq!(
        saved.block_entity_tags[&(7, 70, 1)].get("LootTableSeed"),
        Some(&Nbt::Long(i64::MIN))
    );
    assert_eq!(
        saved.block_entity_tags[&(8, 70, 1)].get("Delay"),
        Some(&Nbt::Short(20))
    );
    assert_eq!(
        saved.block_entity_tags[&(9, 70, 1)]
            .get("bees")
            .unwrap()
            .list()
            .unwrap()[0]
            .get("ticks_in_hive"),
        Some(&Nbt::Int(i32::MIN))
    );
}

#[test]
fn full_review_core_save_tags_match_independent_native_payloads() {
    use crate::block_entity::SpawnerMob;

    let hives: serde_json::Value =
        serde_json::from_str(include_str!("../../data/beehives_26_1.json")).unwrap();
    let samples = hives["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 100);
    for sample in samples {
        let [x, y, z]: [i32; 3] = serde_json::from_value(sample["pos"].clone()).unwrap();
        let hive = BlockEntity::Beehive {
            ticks_in_hive: serde_json::from_value(sample["ticks_in_hive"].clone()).unwrap(),
        };
        let native = Nbt::from_logical_typed_json(&sample["typed_nbt"]).unwrap();
        assert_eq!(core_block_entity_tag(&hive, (x, y, z)), native);
    }
    let lifecycle: serde_json::Value = serde_json::from_str(include_str!(
        "../../data/deferred_block_entities_26_1_v1.json"
    ))
    .unwrap();
    for case in lifecycle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| {
            case["kind"] == "basic" && (case["block"] == "BEE_NEST" || case["block"] == "BEEHIVE")
        })
    {
        let saved = case["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["boundary"] == "level save")
            .unwrap();
        let [x, y, z]: [i32; 3] = serde_json::from_value(saved["after"]["pos"].clone()).unwrap();
        let expected: Nbt = serde_json::from_value(saved["saved"]["nbt"].clone()).unwrap();
        let mut actual = core_block_entity_tag(
            &BlockEntity::Beehive {
                ticks_in_hive: Vec::new(),
            },
            (x, y, z),
        );
        actual
            .compound_mut()
            .unwrap()
            .insert("keepPacked".into(), Nbt::Byte(0));
        assert_eq!(actual, expected, "native physical empty-list type");
    }

    let rooms: serde_json::Value =
        serde_json::from_str(include_str!("../../data/monster_rooms_26_1.json")).unwrap();
    let mut spawners = 0;
    for sample in rooms["samples"].as_array().unwrap() {
        for row in sample["block_entities"].as_array().unwrap() {
            let entity = match row["nbt"]["id"].as_str().unwrap() {
                "minecraft:chest" => BlockEntity::DungeonChest {
                    loot_seed: row["nbt"]["LootTableSeed"].as_i64().unwrap_or(0),
                },
                "minecraft:mob_spawner" => BlockEntity::Spawner {
                    mob: match row["nbt"]["SpawnData"]["entity"]["id"].as_str().unwrap() {
                        "minecraft:skeleton" => SpawnerMob::Skeleton,
                        "minecraft:zombie" => SpawnerMob::Zombie,
                        "minecraft:spider" => SpawnerMob::Spider,
                        other => panic!("native room mob {other}"),
                    },
                },
                other => panic!("native room entity {other}"),
            };
            let [x, y, z]: [i32; 3] = serde_json::from_value(row["pos"].clone()).unwrap();
            let tag = core_block_entity_tag(&entity, (x, y, z));
            assert_eq!(tag.to_json(), row["nbt"]);
            if matches!(entity, BlockEntity::Spawner { .. }) {
                spawners += 1;
                for name in [
                    "Delay",
                    "MaxNearbyEntities",
                    "MaxSpawnDelay",
                    "MinSpawnDelay",
                    "RequiredPlayerRange",
                    "SpawnCount",
                    "SpawnRange",
                ] {
                    let Some(Nbt::Short(value)) = tag.get(name) else {
                        panic!("native spawner {name} must be TAG_Short");
                    };
                    assert!(row["snbt"]
                        .as_str()
                        .unwrap()
                        .contains(&format!("{name}:{value}s")));
                }
                assert_eq!(
                    tag.get("SpawnPotentials"),
                    Some(&Nbt::List {
                        element_type: 0,
                        values: Vec::new(),
                    })
                );
            }
        }
    }
    assert_eq!(spawners, 54);
}

#[test]
fn full_review_runtime_maps_capture_carvers_and_follow_incoming_feature_writes() {
    fn stage(
        status: ChunkStatus,
        pos: ChunkPos,
        region: &mut crate::region::FeatureRegion,
    ) -> Result<(), String> {
        match status {
            ChunkStatus::Noise => {
                region.owned_chunk_mut(pos).set(2, 40, 2, block::STONE);
            }
            ChunkStatus::Carvers => {
                region.owned_chunk_mut(pos).set(2, 60, 2, block::WATER);
            }
            ChunkStatus::Features if pos == ChunkPos::new(-1, 0) => {
                assert!(region.set_feature_block((2, 80, 2), block::OAK_LEAVES, 18));
            }
            _ => {}
        }
        Ok(())
    }
    let world = GenerationWorld::new(0);
    world.state.lock().unwrap().test_stage = Some(stage);
    let noise = world
        .generate_to_status(POS, ChunkStatus::Noise)
        .unwrap()
        .chunk;
    assert!(noise.runtime_heightmaps().is_none());
    assert_eq!(noise.worldgen_heightmaps().unwrap().world_surface[34], 41);
    let carved = world
        .generate_to_status(POS, ChunkStatus::Carvers)
        .unwrap()
        .chunk;
    assert_eq!(carved.runtime_heightmaps().unwrap().world_surface[34], 61);
    assert_eq!(carved.runtime_heightmaps().unwrap().ocean_floor[34], 41);
    let _ = world
        .generate_to_status(ChunkPos::new(-1, 0), ChunkStatus::Features)
        .unwrap();
    let lower = world
        .chunk_access(POS, ChunkStatus::Carvers)
        .unwrap()
        .unwrap();
    lower
        .read(|view| {
            assert_eq!(view.kind, ChunkAccessKind::Proto);
            assert_eq!(
                view.height(FeatureHeightmap::WorldSurfaceWg, 2, 2),
                Some(60)
            );
            assert_eq!(view.height(FeatureHeightmap::WorldSurface, 2, 2), Some(80));
            assert_eq!(
                view.height(FeatureHeightmap::MotionBlockingNoLeaves, 2, 2),
                Some(60)
            );
        })
        .unwrap();
    let full = world.generate_chunk(POS).unwrap();
    assert_eq!(
        full.coverage.target.stage(ChunkStatus::Full).state,
        StageState::Partial
    );
    lower
        .read(|view| {
            assert_eq!(view.kind, ChunkAccessKind::ImposterProto);
            assert_eq!(
                view.height(FeatureHeightmap::WorldSurfaceWg, 2, 2),
                Some(80)
            );
            assert_eq!(
                view.height(FeatureHeightmap::MotionBlockingNoLeaves, 2, 2),
                Some(60)
            );
            assert_eq!(
                view.level
                    .unwrap()
                    .retired_proto_heightmaps()
                    .unwrap()
                    .world_surface[34],
                61
            );
        })
        .unwrap();
}
