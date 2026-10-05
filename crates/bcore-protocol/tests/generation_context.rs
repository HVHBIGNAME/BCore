use bcore_core::ChunkPos;
use bcore_protocol::{
    chunk::ChunkColumn,
    chunk_store::{decode_chunk, encode_chunk},
    world_state::World,
};
use bcore_worldgen::{generation::ChunkStatus, GenerationError, GenerationWorld};
use std::time::{Duration, Instant};

const SEED: i64 = 846_692_123_413_862_008;

#[test]
fn queued_adjacent_chunks_share_generation_history_across_world_clones() {
    let world = World::in_memory(SEED);
    let clone = world.clone();
    let independent = World::in_memory(SEED);
    let (first, first_coverage) = world.try_generate(0, 0).unwrap();
    assert_eq!(first_coverage.feature_sources.len(), 9);
    assert!(first_coverage.incoming_sources_finished);
    assert_eq!(
        world.generation_progress(0, 0).unwrap(),
        clone.generation_progress(0, 0).unwrap()
    );
    assert!(independent.generation_progress(0, 0).unwrap().is_none());

    clone.request_payload(1, 0);
    let deadline = Instant::now() + Duration::from_secs(90);
    let payload = loop {
        if let Some(payload) = world.cached_payload(1, 0) {
            break payload;
        }
        assert!(
            Instant::now() < deadline,
            "queued adjacent chunk did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let (second, second_coverage) = clone.try_generate(1, 0).unwrap();
    assert_eq!(payload, second.encode_payload(1, 0));
    let mut shared = 0;
    for source in &second_coverage.feature_sources {
        if let Some(previous) = first_coverage
            .feature_sources
            .iter()
            .find(|old| old.source == source.source)
        {
            assert_eq!(previous, source, "a shared FEATURES source was rerun");
            shared += 1;
        } else {
            assert!(source.sequence > 9);
        }
        assert_eq!(
            world
                .generation_progress(source.source.x, source.source.z)
                .unwrap()
                .unwrap()
                .stage(ChunkStatus::Features)
                .attempts,
            1
        );
    }
    assert_eq!(shared, 6);
    // The neighbour's LIGHT stage can brighten the first column. Blocks and
    // effects must remain identical; cache eviction must retain the newer light.
    let refreshed_first = clone.generate(0, 0);
    assert_ne!(refreshed_first.light(), first.light());
    let mut expected_first = first;
    assert!(expected_first.set_light(refreshed_first.light().unwrap().clone()));
    assert_eq!(refreshed_first, expected_first);
    world.clear_cache();
    assert_eq!(world.generate(0, 0), refreshed_first);
    assert_eq!(clone.generate(1, 0), second);
    assert!(independent.generation_progress(1, 0).unwrap().is_none());
}

#[test]
fn actual_generated_sculk_and_deferred_effects_survive_protocol_conversion() {
    let owner = ChunkPos::new(0, 0);
    let result = GenerationWorld::new(SEED).generate_chunk(owner).unwrap();
    assert!(
        !result.chunk.feature_block_entities().is_empty(),
        "exercise actual generated sculk entities"
    );
    assert!(
        !result.chunk.postprocessing_positions().is_empty(),
        "exercise actual deferred marks"
    );
    let column = ChunkColumn::from_generated(&result.chunk);
    assert_eq!(
        column.feature_block_entities(),
        result.chunk.feature_block_entities()
    );
    assert_eq!(column.block_entities(), result.chunk.block_entities());
    assert_eq!(column.tick_requests(), result.chunk.tick_requests());
    assert_eq!(
        column.postprocessing_positions(),
        result.chunk.postprocessing_positions()
    );
    let encoded = encode_chunk(owner.x, owner.z, &column);
    let loaded = decode_chunk(&encoded).unwrap();
    assert_eq!(loaded, column);
    assert_eq!(encode_chunk(owner.x, owner.z, &loaded), encoded);
    assert_eq!(
        loaded.encode_payload(owner.x, owner.z),
        column.encode_payload(owner.x, owner.z)
    );
}

#[test]
fn fallible_world_generation_rejects_coordinates_without_claiming_chunks() {
    let world = World::in_memory(1);
    let invalid = ChunkPos::new(i32::MAX, 0);
    assert_eq!(
        world.try_generate(invalid.x, invalid.z).unwrap_err(),
        GenerationError::CoordinateOutOfRange(invalid)
    );
    assert!(world
        .generation_progress(invalid.x, invalid.z)
        .unwrap()
        .is_none());
    assert!(world.generation_progress(0, 0).unwrap().is_none());
}
