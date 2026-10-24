use bcore_protocol::chunk::ChunkColumn;
use bcore_protocol::world_state::World;
use bcore_worldgen::generation::{
    request::NativeRequestError, ChunkAccessKind, ChunkStatus, GenerationError, StageState,
};

#[test]
fn invalid_first_native_request_does_not_bind_the_world_to_the_rejected_thread() {
    let world = World::in_memory(37);
    let other = world.clone();
    std::thread::spawn(move || {
        for (x, z) in [(i32::MAX, 0), (0, i32::MIN)] {
            assert!(matches!(
                other.request_native_chunk(x, z, ChunkStatus::Light, true),
                Err(NativeRequestError::Generation(
                    GenerationError::CoordinateOutOfRange(_)
                ))
            ));
            assert!(other.generation_progress(x, z).unwrap().is_none());
        }
    })
    .join()
    .unwrap();
    let object = world
        .request_native_chunk(0, 0, ChunkStatus::Empty, true)
        .unwrap()
        .unwrap();
    assert_eq!(object.kind(), ChunkAccessKind::Proto);
    world.native_requests().unwrap().shutdown();
}

#[test]
fn native_request_public_api_drives_the_world_core_and_saves_the_selected_object() {
    let world = World::in_memory(7321);
    let pos = (-12, 19);
    assert!(world
        .request_native_chunk(pos.0, pos.1, ChunkStatus::Empty, false)
        .unwrap()
        .is_none());
    assert!(world.generation_progress(pos.0, pos.1).unwrap().is_none());
    let first = world
        .request_native_chunk(pos.0, pos.1, ChunkStatus::Empty, true)
        .unwrap()
        .unwrap();
    let clone = world.clone();
    assert!(std::ptr::eq(
        world.native_requests().unwrap(),
        clone.native_requests().unwrap()
    ));
    let repeated = clone
        .request_native_chunk(pos.0, pos.1, ChunkStatus::Empty, true)
        .unwrap()
        .unwrap();
    assert!(first.same_object(&repeated));
    let progress = world.generation_progress(pos.0, pos.1).unwrap().unwrap();
    assert_eq!(
        progress.stage(ChunkStatus::Empty).state,
        StageState::Complete
    );
    assert_eq!(progress.stage(ChunkStatus::Empty).attempts, 1);
    assert_eq!(
        progress.stage(ChunkStatus::StructureStarts).state,
        StageState::Pending
    );
    let snapshot = first.view().unwrap();
    let saved = first.prepare_save(0).unwrap();
    assert_eq!(snapshot.kind, ChunkAccessKind::Proto);
    assert_eq!(saved.view.persisted_status, ChunkStatus::Empty);
    assert_eq!(
        ChunkColumn::from_generated(&snapshot.chunk),
        ChunkColumn::from_generated(&saved.view.chunk)
    );
    world.native_requests().unwrap().shutdown();
}

#[test]
fn fresh_native_seven_request_history_runs_actual_generation_and_retains_early_proto() {
    // N1/N2 input history, not phase/timing expectations. Their watch snapshots
    // precede requested snapshots and can observe different background phases.
    let world = World::in_memory(846692123413862008);
    let mut first = None;
    for (x, z, status) in [
        (0, 0, ChunkStatus::Features),
        (0, 0, ChunkStatus::InitializeLight),
        (0, 0, ChunkStatus::Light),
        (1, 0, ChunkStatus::Features),
        (1, 0, ChunkStatus::InitializeLight),
        (1, 0, ChunkStatus::Light),
        (0, 0, ChunkStatus::Light),
    ] {
        let object = world
            .request_native_chunk(x, z, status, true)
            .unwrap()
            .unwrap();
        let view = object.view().unwrap();
        assert!(view.persisted_status >= status);
        if first.is_none() {
            assert_eq!(view.kind, ChunkAccessKind::Proto);
            first = Some(object);
        }
    }
    let level = world
        .request_native_chunk(0, 0, ChunkStatus::Full, true)
        .unwrap()
        .unwrap();
    let old = first.unwrap();
    let old_before_save = old.view().unwrap();
    assert_eq!(old_before_save.kind, ChunkAccessKind::Proto);
    assert_eq!(old_before_save.persisted_status, ChunkStatus::Spawn);
    assert_eq!(level.kind(), ChunkAccessKind::Level);
    assert!(!old.same_object(&level));
    let level_save = level.prepare_save(0).unwrap();
    let proto_save = old.prepare_save(0).unwrap();
    assert_eq!(
        proto_save.view.chunk.pending_block_entities(),
        old_before_save.chunk.pending_block_entities()
    );
    assert!(proto_save.view.chunk.worldgen_heightmaps().is_some());
    assert!(level_save.view.chunk.worldgen_heightmaps().is_none());
    assert!(level_save.view.chunk.pending_block_entities().is_empty());
    assert_eq!(
        proto_save.view.chunk.states(),
        level_save.view.chunk.states()
    );
    assert_eq!(
        proto_save.view.chunk.tick_requests(),
        level_save.view.chunk.tick_requests()
    );
    assert_eq!(
        proto_save.view.chunk.postprocessing_positions(),
        level_save.view.chunk.postprocessing_positions()
    );
    let progress = world.generation_progress(0, 0).unwrap().unwrap();
    assert_eq!(progress.stage(ChunkStatus::Full).attempts, 1);
    assert_eq!(progress.stage(ChunkStatus::Full).state, StageState::Partial);
    assert!(!level_save.view.coverage.is_complete());
    world.native_requests().unwrap().shutdown();
}
