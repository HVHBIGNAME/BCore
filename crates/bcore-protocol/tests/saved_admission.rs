//! Saved columns and streaming/native reads share canonical data without generation.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use bcore_core::ChunkPos;
use bcore_protocol::chunk::{block_state, ChunkColumn, SECTION_BIOMES, SECTION_COUNT};
use bcore_protocol::chunk_store::{encode_chunk, ChunkStore, ChunkStoreError};
use bcore_protocol::world_state::{ChunkOrigin, World, WorldError};
use bcore_worldgen::block_entity::{BlockEntity, PendingBlockEntity};
use bcore_worldgen::generated_entity::GeneratedEntity;
use bcore_worldgen::generation::{ChunkAccessKind, ChunkStatus, FeatureBlockEntity, StageState};
use bcore_worldgen::lighting::{ChunkLight, GenerationLight, LightSection};
use bcore_worldgen::structure::template_pool::StructureAssets;
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use bcore_worldgen::{GenerationError, GenerationWorld};

const SEED: i64 = 846692123413862008;
const POS: ChunkPos = ChunkPos::new(-3, 2);

struct SavedWorld {
    root: PathBuf,
    store: ChunkStore,
    column: ChunkColumn,
}

impl SavedWorld {
    fn new(pos: ChunkPos) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir().join("opencode");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "bcore-saved-admission-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("new isolated world");
        let store = ChunkStore::at(&root);
        store.prepare_world(SEED).unwrap();
        let biomes = (0..SECTION_COUNT * SECTION_BIOMES)
            .map(|i| (i % bcore_registry::BIOME_COUNT) as u32)
            .collect();
        let mut column = ChunkColumn::from_parts(ChunkColumn::flat().states().to_vec(), biomes);
        assert!(column.set(15, 319, 0, block_state::DIAMOND_BLOCK));
        assert!(column.set(4, 32, 5, block_state::CHEST));
        assert!(column.set_block_entity(4, 32, 5, BlockEntity::DungeonChest { loot_seed: -37 }));
        assert!(column.set(8, 70, 9, block_state::CHEST));
        assert!(column.set_pending_block_entity(
            pos,
            8,
            70,
            9,
            PendingBlockEntity::dummy((pos.x * 16 + 8, 70, pos.z * 16 + 9))
        ));
        assert!(column.set(6, 71, 7, block_state::CHEST));
        let template = StructureAssets::bundled()
            .blocks
            .default_block_entity(block_state::CHEST, (pos.x * 16 + 6, 71, pos.z * 16 + 7))
            .unwrap()
            .unwrap();
        assert!(column.set_feature_block_entity(
            pos,
            6,
            71,
            7,
            FeatureBlockEntity::from_template(&template).unwrap()
        ));
        assert!(column.add_entity(
            pos,
            GeneratedEntity::ChestMinecart {
                block_pos: [pos.x * 16 + 8, 33, pos.z * 16 + 9],
                loot_seed: i64::MIN
            }
        ));
        let block_tick = TickRequest {
            block_pos: [pos.x * 16 + 4, 32, pos.z * 16 + 5],
            target: TickTarget::Block(block_state::STONE),
            delay: 9,
        };
        assert!(column.add_tick_request(pos, block_tick));
        assert!(column.add_tick_request(
            pos,
            TickRequest {
                delay: 17,
                ..block_tick
            }
        ));
        assert!(column.add_tick_request(
            pos,
            TickRequest {
                target: TickTarget::Fluid(
                    bcore_registry::registry_id("FLUID", "minecraft:water").unwrap()
                ),
                delay: -3,
                ..block_tick
            }
        ));
        assert!(column.mark_postprocessing(5, 12, 6));
        assert!(column.mark_postprocessing(5, 12, 6));
        let mut light = ChunkLight {
            min_section_y: -5,
            sections: vec![LightSection::default(); SECTION_COUNT + 2],
        };
        light.sections[1].sky = Some(vec![0; 2048]);
        light.sections[1].sky_empty = true;
        light.sections[2].sky = Some(vec![0xff; 2048]);
        assert!(column.set_light(light));
        store.save(pos.x, pos.z, &column).unwrap();
        Self {
            root,
            store,
            column,
        }
    }

    fn open(&self) -> World {
        World::open(SEED, self.store.clone()).unwrap()
    }
}

impl Drop for SavedWorld {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.root).expect("remove isolated saved world");
        }
    }
}

#[test]
fn loaded_data_and_generation_reads_preserve_the_entire_storage_snapshot() {
    let saved = SavedWorld::new(POS);
    let original = fs::read(saved.store.chunk_path(POS.x, POS.z)).unwrap();
    let world = saved.open();
    let (loaded, origin) = world.try_chunk(POS.x, POS.z).unwrap();
    assert_eq!(origin, ChunkOrigin::Loaded);
    assert_eq!(loaded, saved.column);
    let (retained, coverage) = world.try_generate(POS.x, POS.z).unwrap();
    assert_eq!(retained, saved.column);
    assert!(!coverage.is_complete());
    assert!(coverage.feature_sources.is_empty());
    assert_eq!(coverage.target.completed_status(), None);
    for status in ChunkStatus::ALL {
        assert_eq!(coverage.target.stage(status).state, StageState::Partial);
        assert_eq!(coverage.target.stage(status).attempts, 0);
    }
    assert_eq!(world.try_chunk(POS.x, POS.z).unwrap().0, saved.column);
    assert_eq!(
        fs::read(saved.store.chunk_path(POS.x, POS.z)).unwrap(),
        original
    );
    assert_eq!(saved.store.saved_chunks().unwrap(), [(POS.x, POS.z)]);
}

#[test]
fn admission_replaces_a_negative_request_cache_entry_without_generating() {
    let saved = SavedWorld::new(POS);
    let world = saved.open();
    assert!(world
        .request_native_chunk(POS.x, POS.z, ChunkStatus::Full, false)
        .unwrap()
        .is_none());
    world.try_chunk(POS.x, POS.z).unwrap();
    let level = world
        .request_native_chunk(POS.x, POS.z, ChunkStatus::Full, false)
        .unwrap()
        .unwrap();
    let repeated = world
        .request_native_chunk(POS.x, POS.z, ChunkStatus::Full, false)
        .unwrap()
        .unwrap();
    let lower = world
        .request_native_chunk(POS.x, POS.z, ChunkStatus::Noise, false)
        .unwrap()
        .unwrap();
    assert!(level.same_object(&repeated));
    assert_eq!(level.kind(), ChunkAccessKind::Level);
    assert_eq!(lower.kind(), ChunkAccessKind::ImposterProto);
    assert!(!level.same_object(&lower));
    for object in [&level, &lower] {
        let view = object.view().unwrap();
        assert_eq!(view.persisted_status, ChunkStatus::Full);
        assert!(!view.coverage.is_complete());
        assert_eq!(ChunkColumn::from_generated(&view.chunk), saved.column);
        assert!(view.chunk.worldgen_heightmaps().is_none());
    }
    let requests = world.native_requests().unwrap();
    let state = requests.snapshot();
    assert_eq!(state.pending_generation_tasks, 0);
    assert_eq!(state.queued_worker_tasks, 0);
    assert!(!state.worker_active);
    assert!(state.main_queue.is_empty());
    assert!(requests.holder(POS).unwrap().saved_data_admitted);
    requests.shutdown();
}

#[test]
fn admission_completes_issued_futures_and_retires_their_pending_generation() {
    let saved = SavedWorld::new(POS);
    let world = saved.open();
    let requests = world.native_requests().unwrap();
    let empty = requests
        .request_future(POS, ChunkStatus::Empty, true)
        .unwrap();
    let starts = requests
        .request_future(POS, ChunkStatus::StructureStarts, true)
        .unwrap();
    assert!(!empty.is_done());
    assert!(!starts.is_done());
    assert_eq!(requests.snapshot().pending_generation_tasks, 2);
    world.try_chunk(POS.x, POS.z).unwrap();
    let level = world
        .request_native_chunk(POS.x, POS.z, ChunkStatus::Full, false)
        .unwrap()
        .unwrap();
    for future in [&empty, &starts] {
        let object = future
            .get_now()
            .expect("issued future must resolve on admission")
            .unwrap()
            .unwrap();
        assert_eq!(object.kind(), ChunkAccessKind::ImposterProto);
        assert_eq!(
            ChunkColumn::from_generated(&object.view().unwrap().chunk),
            saved.column
        );
    }
    let holder = requests.holder(POS).unwrap();
    assert_eq!(holder.generation_target, None);
    assert!(!holder.full_body_queued);
    assert_eq!(requests.snapshot().pending_generation_tasks, 0);
    assert_eq!(requests.snapshot().queued_worker_tasks, 0);
    requests.shutdown();
    assert_eq!(
        empty.join().unwrap().unwrap().kind(),
        ChunkAccessKind::ImposterProto
    );
    assert_eq!(
        starts.join().unwrap().unwrap().kind(),
        ChunkAccessKind::ImposterProto
    );
    assert_eq!(level.kind(), ChunkAccessKind::Level);
}

#[test]
fn saved_only_neighborhood_has_no_finished_incoming_feature_history() {
    let world = GenerationWorld::new(SEED);
    let data = (-1..=1)
        .flat_map(|x| {
            (-1..=1).map(move |z| ChunkColumn::flat().into_saved_data(ChunkPos::new(x, z)))
        })
        .collect();
    world.admit_saved_chunks(data).unwrap();
    let result = world.generate_chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(!result.coverage.incoming_sources_finished);
    assert!(result.coverage.feature_sources.is_empty());
    assert!(!result.coverage.is_complete());
    for x in -1..=1 {
        for z in -1..=1 {
            let progress = world.progress(ChunkPos::new(x, z)).unwrap().unwrap();
            assert_eq!(progress.stage(ChunkStatus::Features).attempts, 0);
            assert_eq!(
                progress.stage(ChunkStatus::Features).state,
                StageState::Partial
            );
        }
    }
}

#[test]
fn saved_light_contributes_across_a_fresh_chunk_boundary_without_rewriting_disk() {
    let pos = ChunkPos::new(1, 0);
    let mut saved = SavedWorld::new(pos);
    let glowstone = bcore_worldgen::block_predicate::catalog()
        .default_state("glowstone")
        .unwrap();
    assert!(saved.column.set(0, 319, 8, glowstone));
    let mut light = GenerationLight::default();
    light.initialize_chunk(pos, saved.column.states()).unwrap();
    light.propagate_chunk(pos).unwrap();
    assert_eq!(light.block_brightness((16, 319, 8)), 15);
    assert!(saved.column.set_light(light.chunk_light(pos)));
    saved.store.save(pos.x, pos.z, &saved.column).unwrap();
    let original = fs::read(saved.store.chunk_path(pos.x, pos.z)).unwrap();
    let world = saved.open();
    let (fresh, origin) = world.try_chunk(0, 0).unwrap();
    assert_eq!(origin, ChunkOrigin::Generated);
    let snapshot = fresh.light().expect("fresh boundary lighting");
    let section = &snapshot.sections[((319 >> 4) - snapshot.min_section_y) as usize];
    let index = (319 & 15) as usize * 256 + 8 * 16 + 15;
    let byte = section.block.as_ref().expect("block light section")[index / 2];
    let level = if index & 1 == 0 { byte & 15 } else { byte >> 4 };
    assert_eq!(level, 14);
    assert_eq!(world.try_chunk(pos.x, pos.z).unwrap().0, saved.column);
    assert_eq!(
        fs::read(saved.store.chunk_path(pos.x, pos.z)).unwrap(),
        original
    );
}

#[test]
fn saved_neighbors_are_hydrated_before_a_fresh_chunk_can_decorate_them() {
    let pos = ChunkPos::new(1, 0);
    let saved = SavedWorld::new(pos);
    let original = fs::read(saved.store.chunk_path(pos.x, pos.z)).unwrap();
    let world = saved.open();
    assert_eq!(world.try_chunk(0, 0).unwrap().1, ChunkOrigin::Generated);
    let progress = world.generation_progress(pos.x, pos.z).unwrap().unwrap();
    assert!(progress
        .stages
        .iter()
        .all(|stage| stage.state == StageState::Partial && stage.attempts == 0));
    assert_eq!(world.try_generate(pos.x, pos.z).unwrap().0, saved.column);
    assert_eq!(world.try_chunk(pos.x, pos.z).unwrap().0, saved.column);
    assert_eq!(
        fs::read(saved.store.chunk_path(pos.x, pos.z)).unwrap(),
        original
    );
}

#[test]
fn a_corrupt_neighbor_stops_before_admission_generation_or_overwrite() {
    let saved = SavedWorld::new(ChunkPos::new(1, 0));
    let good = ChunkPos::new(-1, 0);
    saved
        .store
        .save(good.x, good.z, &ChunkColumn::flat())
        .unwrap();
    let path = saved.store.chunk_path(1, 0);
    let mut corrupt = fs::read(&path).unwrap();
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 1;
    fs::write(&path, &corrupt).unwrap();
    let world = World::with_store(SEED, saved.store.clone());
    assert!(
        matches!(world.try_chunk(0, 0), Err(WorldError::Store(error)) if matches!(error.as_ref(), ChunkStoreError::ChecksumMismatch { .. }))
    );
    assert!(world.generation_progress(0, 0).unwrap().is_none());
    assert!(world.generation_progress(good.x, good.z).unwrap().is_none());
    assert!(!saved.store.contains(0, 0));
    assert_eq!(fs::read(&path).unwrap(), corrupt);
}

#[test]
fn a_failed_first_native_disk_request_does_not_bind_the_owner_thread() {
    let saved = SavedWorld::new(POS);
    let path = saved.store.chunk_path(POS.x, POS.z);
    let mut corrupt = fs::read(&path).unwrap();
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 1;
    fs::write(&path, &corrupt).unwrap();
    let world = World::with_store(SEED, saved.store.clone());
    let other = world.clone();
    std::thread::spawn(move || {
        assert!(matches!(
            other.request_native_chunk(POS.x, POS.z, ChunkStatus::Full, true),
            Err(WorldError::Store(_))
        ));
    })
    .join()
    .unwrap();
    assert!(world
        .request_native_chunk(POS.x, POS.z, ChunkStatus::Full, false)
        .unwrap()
        .is_none());
    assert!(world.generation_progress(POS.x, POS.z).unwrap().is_none());
    assert_eq!(fs::read(&path).unwrap(), corrupt);
    world.native_requests().unwrap().shutdown();
}

#[test]
fn concurrent_disk_reads_share_admission_and_preserve_bytes() {
    let saved = SavedWorld::new(POS);
    let world = saved.open();
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let barrier = &barrier;
                let world = &world;
                scope.spawn(move || {
                    barrier.wait();
                    world.try_chunk(POS.x, POS.z).unwrap()
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(
                thread.join().unwrap(),
                (saved.column.clone(), ChunkOrigin::Loaded)
            );
        }
    });
    assert_eq!(world.try_generate(POS.x, POS.z).unwrap().0, saved.column);
    assert_eq!(
        saved.store.load(POS.x, POS.z).unwrap(),
        Some(saved.column.clone())
    );
}

#[test]
fn loaded_chest_inventory_changes_remain_canonical_and_survive_restart() {
    let saved = SavedWorld::new(POS);
    let world = saved.open();
    world.try_chunk(POS.x, POS.z).unwrap();
    let inventory = world
        .unpack_generated_chest(POS.x * 16 + 4, 32, POS.z * 16 + 5)
        .unwrap()
        .unwrap();
    assert!(inventory.loot_table().is_none());
    assert_eq!(
        world
            .unpack_generated_chest(POS.x * 16 + 4, 32, POS.z * 16 + 5)
            .unwrap(),
        Some(inventory.clone())
    );
    let changed = world.try_chunk(POS.x, POS.z).unwrap().0;
    assert_ne!(changed, saved.column);
    assert_eq!(
        saved.store.load(POS.x, POS.z).unwrap(),
        Some(saved.column.clone())
    );
    let column = world
        .prepare_generated_chunk_save(POS.x, POS.z, 100)
        .unwrap();
    saved.store.save(POS.x, POS.z, &column).unwrap();
    world.clear_cache();
    let reopened = saved.open();
    assert_eq!(reopened.try_chunk(POS.x, POS.z).unwrap().0, column);
    assert_eq!(
        reopened
            .unpack_generated_chest(POS.x * 16 + 4, 32, POS.z * 16 + 5)
            .unwrap(),
        Some(inventory)
    );
    assert_eq!(
        reopened
            .prepare_generated_chunk_save(POS.x, POS.z, 100)
            .unwrap(),
        column
    );
}

#[test]
fn loaded_tick_requests_keep_delays_duplicates_and_explicit_unpacking() {
    let saved = SavedWorld::new(POS);
    let generation = GenerationWorld::new(SEED);
    generation
        .admit_saved_chunks(vec![saved.column.clone().into_saved_data(POS)])
        .unwrap();
    let access = generation
        .chunk_access(POS, ChunkStatus::Full)
        .unwrap()
        .unwrap();
    access
        .read(|view| {
            let level = view.level.unwrap();
            assert_eq!(level.block_ticks().count(), 2);
            assert!(level.block_ticks().get_all().is_empty());
            assert_eq!(
                level
                    .block_ticks()
                    .pack(400)
                    .iter()
                    .map(|tick| tick.delay)
                    .collect::<Vec<_>>(),
                [9, 17]
            );
            assert_eq!(level.fluid_ticks().pack(400)[0].delay, -3);
        })
        .unwrap();
    generation.unpack_chunk_ticks(POS, 400).unwrap();
    access
        .read(|view| {
            let level = view.level.unwrap();
            assert_eq!(
                level
                    .block_ticks()
                    .get_all()
                    .iter()
                    .map(|tick| tick.trigger_tick)
                    .collect::<Vec<_>>(),
                [409, 417]
            );
            assert_eq!(level.fluid_ticks().get_all()[0].trigger_tick, 397);
        })
        .unwrap();
    assert_eq!(
        generation
            .chunk_snapshot(POS)
            .unwrap()
            .unwrap()
            .tick_requests(),
        saved.column.tick_requests()
    );
}

#[test]
fn validation_and_conflicts_never_publish_a_partial_admission_batch() {
    for case in 0..4 {
        let world = GenerationWorld::new(SEED);
        let good = ChunkColumn::flat().into_saved_data(ChunkPos::new(0, 0));
        let mut bad = ChunkColumn::flat().into_saved_data(ChunkPos::new(1, 0));
        match case {
            0 => {
                bad.biomes.pop();
            }
            1 => bad.states[0] = bcore_registry::BLOCK_STATE_COUNT as u32,
            2 => bad.pos = ChunkPos::new(i32::MAX, 0),
            _ => bad.postprocessing.push((16, 32, 0)),
        }
        assert!(world.admit_saved_chunks(vec![good, bad]).is_err());
        assert!(world.progress(ChunkPos::new(0, 0)).unwrap().is_none());
        assert!(world.chunk_snapshot(ChunkPos::new(0, 0)).unwrap().is_none());
    }
    let world = GenerationWorld::new(SEED);
    let initial = world
        .generate_to_status(ChunkPos::new(1, 0), ChunkStatus::Empty)
        .unwrap();
    assert_eq!(
        initial.coverage.target.stage(ChunkStatus::Empty).state,
        StageState::Complete
    );
    assert!(matches!(
        world.admit_saved_chunks(vec![
            ChunkColumn::flat().into_saved_data(ChunkPos::new(0, 0)),
            ChunkColumn::flat().into_saved_data(ChunkPos::new(1, 0))
        ]),
        Err(GenerationError::SavedChunkConflict(_))
    ));
    assert!(world.progress(ChunkPos::new(0, 0)).unwrap().is_none());
    assert_eq!(
        world
            .progress(ChunkPos::new(1, 0))
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::Empty)
            .attempts,
        1
    );
}

#[test]
fn a_late_disk_snapshot_does_not_replace_an_existing_proto_object() {
    let saved = SavedWorld::new(POS);
    let target = ChunkPos::new(25, 0);
    let world = saved.open();
    let proto = world
        .request_native_chunk(target.x, target.z, ChunkStatus::Empty, true)
        .unwrap()
        .unwrap();
    let before = proto.view().unwrap().chunk;
    let late = ChunkColumn::flat();
    saved.store.save(target.x, target.z, &late).unwrap();
    assert!(matches!(
        world.try_chunk(target.x, target.z),
        Err(WorldError::Generation(GenerationError::SavedChunkConflict(
            _
        )))
    ));
    assert_eq!(proto.view().unwrap().chunk, before);
    assert_eq!(
        fs::read(saved.store.chunk_path(target.x, target.z)).unwrap(),
        encode_chunk(target.x, target.z, &late)
    );
    world.native_requests().unwrap().shutdown();
}
