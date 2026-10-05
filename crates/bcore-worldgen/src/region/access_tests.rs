use super::*;
use crate::generation::graph::layer_positions;

fn shared(source: ChunkPos, status: ChunkStatus) -> FeatureRegion {
    let mut region = FeatureRegion::shared(WorldGenerator::new(1234));
    let step = ChunkPyramid::Generation.step(status);
    let available = layer_positions(source, step.direct.radius())
        .map(|pos| ((pos.x, pos.z), step.direct.at(source, pos).unwrap()))
        .collect();
    region.begin_source(source, status, available);
    region
}

#[test]
fn direct_status_reads_do_not_load_terrain_or_expand_to_the_accumulated_radius() {
    let source = ChunkPos::new(-2, 3);
    let region = shared(source, ChunkStatus::Features);
    let outer = ChunkPos::new(source.x - 8, source.z + 8);
    assert_eq!(
        region
            .chunk_at_status(outer, ChunkStatus::StructureStarts)
            .unwrap()
            .states()
            .iter()
            .copied()
            .max(),
        Some(block::AIR)
    );
    assert!(region.chunk_at_status(outer, ChunkStatus::Biomes).is_err());
    assert!(region
        .chunk_at_status(source, ChunkStatus::Features)
        .is_err());
    for radius in [9, 10, 11] {
        assert!(region
            .chunk_at_status(
                ChunkPos::new(source.x - radius, source.z + radius),
                ChunkStatus::Empty
            )
            .is_err());
    }
    assert_eq!(region.chunks.borrow().len(), 1);
}

#[test]
fn reads_see_live_writes_without_promoting_the_requested_status() {
    let source = ChunkPos::new(-2, 3);
    let mut region = shared(source, ChunkStatus::Features);
    let point = (source.x * 16 - 1, 90, source.z * 16 + 16);
    assert!(region.set_feature_block(point, block::OAK_LOG, 19));
    let owner = ChunkPos::new(point.0 >> 4, point.2 >> 4);
    assert_eq!(
        region
            .chunk_at_status(owner, ChunkStatus::Carvers)
            .unwrap()
            .get(15, 90, 0),
        Some(block::OAK_LOG)
    );
    assert!(region
        .chunk_at_status(owner, ChunkStatus::Features)
        .is_err());
}

#[test]
fn every_feature_adapter_obeys_source_relative_write_radius_and_vertical_limits() {
    for (status, radius) in [
        (ChunkStatus::Features, 1),
        (ChunkStatus::Surface, 0),
        (ChunkStatus::StructureReferences, -1),
    ] {
        let source = ChunkPos::new(-2, 3);
        let mut region = shared(source, status);
        for dx in -2_i32..=2 {
            for dz in -2_i32..=2 {
                let pos = ((source.x + dx) * 16, 64, (source.z + dz) * 16);
                let expected = dx.abs().max(dz.abs()) <= radius;
                assert_eq!(region.can_write_feature(pos), expected);
                assert_eq!(region.set_block(pos, block::DIRT), expected);
                assert_eq!(
                    crate::tree::fallen::FallenTreeWorld::set_block(
                        &mut region,
                        pos,
                        block::DIRT,
                        19
                    ),
                    expected
                );
                assert_eq!(
                    crate::decoration::TreeFeatureWorld::can_place_tree(&region, pos),
                    expected
                );
            }
        }
        for y in [MIN_Y - 1, MAX_Y + 1] {
            assert!(!region.set_feature_block((source.x * 16, y, source.z * 16), block::DIRT, 3));
        }
        region.end_source();
        assert!(!region.can_write_feature((source.x * 16, 64, source.z * 16)));
    }
}

#[test]
fn marks_and_tick_ownership_use_direct_access_not_the_block_write_radius() {
    let source = ChunkPos::new(0, 0);
    let mut region = shared(source, ChunkStatus::Features);
    let point = (-32, 80, 32);
    assert!(!region.can_write_feature(point));
    region.mark_feature_postprocessing(point);
    let request = TickRequest {
        block_pos: [point.0, point.1, point.2],
        target: TickTarget::Fluid(2),
        delay: 5,
    };
    assert!(region.schedule_feature_tick(request));
    assert!(region.schedule_feature_tick(request));
    assert!(!region.schedule_feature_tick(TickRequest {
        block_pos: [-144, 80, 0],
        ..request
    }));
    let chunk = region.chunk(-2, 2);
    assert_eq!(chunk.postprocessing_positions(), &[(0, 80, 0)]);
    assert_eq!(chunk.tick_requests(), &[request, request]);
    assert!(chunk.states().iter().all(|&state| state == block::AIR));
}

#[test]
fn shared_feature_heightmaps_track_trees_water_and_removal() {
    let mut region = shared(ChunkPos::new(0, 0), ChunkStatus::Features);
    assert!(region.set_feature_block((0, 63, 0), block::DIRT, 19));
    assert!(region.set_feature_block((0, 64, 0), block::WATER, 19));
    assert!(region.set_feature_block((0, 80, 0), block::OAK_LEAVES, 19));
    assert_eq!(
        region.feature_height(FeatureHeightmap::WorldSurface, 0, 0),
        81
    );
    assert_eq!(
        region.feature_height(FeatureHeightmap::OceanFloor, 0, 0),
        81
    );
    assert_eq!(
        region.feature_height(FeatureHeightmap::MotionBlocking, 0, 0),
        81
    );
    assert_eq!(
        region.feature_height(FeatureHeightmap::MotionBlockingNoLeaves, 0, 0),
        65
    );
    assert!(region.set_feature_block((0, 80, 0), block::AIR, 19));
    assert_eq!(
        region.feature_height(FeatureHeightmap::WorldSurfaceWg, 0, 0),
        65
    );
    assert_eq!(
        region.feature_height(FeatureHeightmap::OceanFloorWg, 0, 0),
        64
    );
    assert_eq!(
        region.feature_height(FeatureHeightmap::MotionBlocking, 0, 0),
        65
    );
}

#[test]
fn sculk_entity_defaults_keep_typed_nbt_and_survive_compatible_writes() {
    let mut region = shared(ChunkPos::new(0, 0), ChunkStatus::Features);
    for (z, name) in ["sculk_sensor", "sculk_catalyst", "sculk_shrieker"]
        .into_iter()
        .enumerate()
    {
        let definition = crate::block_predicate::catalog().definition(name).unwrap();
        let pos = (-1, 80, z as i32);
        assert!(region.set_feature_block(pos, definition.default_state, 3));
        let expected = crate::sculk::generated_block_entity(definition.default_state, pos)
            .unwrap()
            .unwrap();
        let entity = region.chunk(-1, 0).feature_block_entities()[&(15, 80, z)].clone();
        assert_eq!(entity.type_id, expected.type_id);
        assert_eq!(entity.full_data, expected.full_data);
        assert_eq!(entity.typed_data, expected.typed_data);
        assert!(region.set_feature_block(pos, definition.first, 19));
        assert_eq!(
            region.chunk(-1, 0).feature_block_entities()[&(15, 80, z)],
            entity
        );
        assert!(region.set_feature_block(pos, block::AIR, 19));
        assert!(!region
            .chunk(-1, 0)
            .feature_block_entities()
            .contains_key(&(15, 80, z)));
    }
}
