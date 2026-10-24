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
        assert!(region.chunk(-1, 0).feature_block_entities().is_empty());
        assert_eq!(region.chunk(-1, 0).pending_block_entities().len(), 1);
        assert!(region.materialize_block_entity_at(pos).unwrap());
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

fn converted_neighbour() -> FeatureRegion {
    let source = ChunkPos::new(-1, 0);
    let mut region = FeatureRegion::shared(WorldGenerator::new(0));
    let chunk = region.owned_chunk_mut(ChunkPos::new(0, 0));
    chunk.set(1, 80, 1, 21774);
    assert!(chunk.set_block_entity(
        1,
        80,
        1,
        BlockEntity::Beehive {
            ticks_in_hive: vec![97]
        }
    ));
    let brushable = crate::block_predicate::catalog()
        .default_state("suspicious_sand")
        .unwrap();
    chunk.set(2, 80, 1, brushable);
    assert!(chunk.set_pending_block_entity(
        2,
        80,
        1,
        crate::block_entity::PendingBlockEntity::dummy((2, 80, 1))
    ));
    chunk.postprocessing.extend([(8, 80, 8), (8, 80, 8)]);
    let step = ChunkPyramid::Generation.step(ChunkStatus::Features);
    let available = layer_positions(source, step.direct.radius())
        .map(|pos| ((pos.x, pos.z), step.direct.at(source, pos).unwrap()))
        .collect();
    // Conversion admission is independent of available/requested coverage.
    region.begin_source_with_levels(source, ChunkStatus::Features, available, [(0, 0)].into());
    region
}

#[test]
fn converted_lower_admission_blocks_chunk_writes_marks_ticks_and_entity_insertion() {
    use crate::generated_entity::GeneratedEntity;
    use crate::structure::template::{Mirror, Nbt, Rotation, TemplateEffect, TemplateEntity};
    use crate::tree::standing::StandingTreeWorld;

    let mut region = converted_neighbour();
    let owner = ChunkPos::new(0, 0);
    let before = (*region.owned_chunk(owner).unwrap()).clone();
    assert!(!region.can_write_feature((8, 80, 8)));
    assert!(!region.set_feature_block((8, 80, 8), block::STONE, 3));
    assert!(!OreWorld::set_block(&mut region, (1, 80, 1), block::AIR));
    assert!(!crate::tree::fallen::FallenTreeWorld::set_block(
        &mut region,
        (8, 80, 8),
        block::STONE,
        18
    ));
    region.mark_feature_postprocessing((8, 80, 8));
    crate::tree::fallen::FallenTreeWorld::mark_for_postprocessing(&mut region, (8, 80, 8));
    crate::structure::mineshaft::blocks::MineshaftWorld::mark_for_postprocessing(
        &mut region,
        [8, 80, 8],
    );
    let tick = TickRequest {
        block_pos: [8, 80, 8],
        target: TickTarget::Fluid(2),
        delay: -7,
    };
    assert!(!region.schedule_feature_tick(tick));
    region.schedule_tree_tick([8, 80, 8, 2, -7, 1]);
    // Object-safe tree callbacks can buffer ticks before transfer as well.
    region.tree_effects.tick_requests.push([8, 80, 8, 2, -9, 1]);
    region.transfer_tree_effects().unwrap();
    let entity = GeneratedEntity::ChestMinecart {
        block_pos: [8, 80, 8],
        loot_seed: 12,
    };
    region.store_generated_entity(entity.clone()).unwrap();
    crate::structure::mineshaft::blocks::MineshaftWorld::add_entity(&mut region, entity);
    let position = [8.5, 80.0, 8.5];
    region
        .apply_template_effect(
            owner,
            TemplateEffect::Entity(TemplateEntity {
                pos: position,
                block_pos: (8, 80, 8),
                finalize: true,
                rotation: Rotation::None,
                mirror: Mirror::None,
                nbt: Nbt::Compound(BTreeMap::from([
                    ("id".into(), Nbt::String("minecraft:pig".into())),
                    (
                        "Pos".into(),
                        Nbt::List {
                            element_type: 6,
                            values: position.into_iter().map(Nbt::Double).collect(),
                        },
                    ),
                ])),
            }),
        )
        .unwrap();
    assert_eq!(*region.owned_chunk(owner).unwrap(), before);
    assert!(region.tree_effects.tick_requests.is_empty());
    assert!(region.light_updates.is_empty());
    assert!(region.set_feature_block((-1, 80, 1), block::STONE, 18));
    assert_eq!(region.chunk(-1, 0).get(15, 80, 1), Some(block::STONE));
}

#[test]
fn converted_lower_be_lookup_and_live_callbacks_are_distinct_from_chunk_insertion() {
    use crate::structure::template::{Nbt, TemplateBlockEntity, TemplateEffect};
    use crate::tree::standing::StandingTreeWorld;

    let mut region = converted_neighbour();
    let owner = ChunkPos::new(0, 0);
    assert!(region.materialize_block_entity_at((2, 80, 1)).unwrap());
    assert!(region.chunk(0, 0).pending_block_entities().is_empty());
    assert!(region
        .set_feature_brushable_loot((2, 80, 1), "minecraft:archaeology/desert_well", -7)
        .unwrap());
    assert_eq!(
        region.chunk(0, 0).feature_block_entities()[&(2, 80, 1)].full_data["LootTableSeed"],
        -7
    );
    region.store_bee((1, 80, 1), -5);
    region.transfer_tree_effects().unwrap();
    assert_eq!(
        region.chunk(0, 0).block_entities()[&(1, 80, 1)],
        BlockEntity::Beehive {
            ticks_in_hive: vec![97, -5]
        }
    );

    let blocks = &crate::structure::template_pool::StructureAssets::bundled().blocks;
    let state = region.chunk(0, 0).get(2, 80, 1).unwrap();
    let load = Nbt::Compound(BTreeMap::from([
        (
            "LootTable".into(),
            Nbt::String("minecraft:archaeology/desert_well".into()),
        ),
        ("LootTableSeed".into(), Nbt::Long(19)),
    ]));
    let entity = TemplateBlockEntity::from_load(blocks, state, (2, 80, 1), load).unwrap();
    region
        .apply_template_effect(owner, TemplateEffect::BlockEntity(entity))
        .unwrap();
    assert_eq!(
        region.chunk(0, 0).feature_block_entities()[&(2, 80, 1)].full_data["LootTableSeed"],
        19
    );
    // No BE lookup result means loadWithComponents cannot install a new entity.
    let absent =
        TemplateBlockEntity::from_load(blocks, state, (3, 80, 1), Nbt::empty_compound()).unwrap();
    region
        .apply_template_effect(owner, TemplateEffect::BlockEntity(absent))
        .unwrap();
    assert!(!region
        .chunk(0, 0)
        .feature_block_entities()
        .contains_key(&(3, 80, 1)));
    let before_clear = (*region.owned_chunk(owner).unwrap()).clone();
    assert!(matches!(
        region.apply_template_effect(owner, TemplateEffect::ClearBlockEntity((2, 80, 1))),
        Err(crate::feature_world::FeatureError::Unsupported(_))
    ));
    assert_eq!(*region.owned_chunk(owner).unwrap(), before_clear);
}

#[test]
fn full_review_brushable_callback_preserves_unrelated_live_storage() {
    let mut region = converted_neighbour();
    let owner = ChunkPos::new(0, 0);
    assert!(region.materialize_block_entity_at((2, 80, 1)).unwrap());
    let pointers = |region: &FeatureRegion| {
        let chunk = region.owned_chunk_ref(owner).unwrap();
        let BlockEntity::Beehive { ticks_in_hive } = &chunk.block_entities()[&(1, 80, 1)] else {
            panic!("retained hive");
        };
        (
            chunk.states().as_ptr() as usize,
            ticks_in_hive.as_ptr() as usize,
        )
    };
    let before = pointers(&region);
    assert!(region
        .set_feature_brushable_loot((2, 80, 1), "minecraft:archaeology/desert_well", -7)
        .unwrap());
    assert_eq!(
        pointers(&region),
        before,
        "a live BE callback must not fork canonical storage"
    );
    assert_eq!(
        region
            .owned_chunk_ref(owner)
            .unwrap()
            .feature_block_entities()[&(2, 80, 1)]
            .full_data["LootTableSeed"],
        -7
    );
}

#[test]
fn full_review_converted_dungeon_callback_reports_unverified_live_mutation() {
    use crate::block_entity::SpawnerMob;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let mut region = converted_neighbour();
    let owner = ChunkPos::new(0, 0);
    let chunk = region.owned_chunk_mut(owner);
    chunk.set(4, 80, 1, crate::dungeon::SPAWNER);
    assert!(chunk.set_block_entity(
        4,
        80,
        1,
        BlockEntity::Spawner {
            mob: SpawnerMob::Zombie
        }
    ));
    let before = region.owned_chunk_ref(owner).unwrap().clone();
    let error = catch_unwind(AssertUnwindSafe(|| {
        crate::dungeon::DungeonWorld::set_block_entity(
            &mut region,
            (4, 80, 1),
            BlockEntity::Spawner {
                mob: SpawnerMob::CaveSpider,
            },
        );
    }));
    assert!(
        error.is_err(),
        "native dungeon/spawner lookup callbacks are not read-only insertion no-ops"
    );
    assert_eq!(*region.owned_chunk_ref(owner).unwrap(), before);
}

#[test]
fn full_review_read_only_noops_still_enforce_direct_read_dependencies() {
    use crate::tree::standing::StandingTreeWorld;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let mut region = converted_neighbour();
    let owner = ChunkPos::new(0, 0);
    let far = ChunkPos::new(9, 0);
    region.owned_chunk_mut(far);
    let RegionAccess::Source { converted, .. } = &mut region.access else {
        panic!("source guard");
    };
    converted.insert((far.x, far.z));
    let before = region.owned_chunk_ref(owner).unwrap().clone();
    let far_before = region.owned_chunk_ref(far).unwrap().clone();
    let pos = (far.x * 16, 80, 1);
    assert!(catch_unwind(AssertUnwindSafe(|| {
        region.mark_feature_postprocessing(pos);
    }))
    .is_err());
    let tick = [pos.0, pos.1, pos.2, 2, 0, 1];
    assert!(catch_unwind(AssertUnwindSafe(|| region.schedule_tree_tick(tick))).is_err());
    assert!(region.tree_effects.tick_requests.is_empty());

    // The erased adapter can buffer a request before region admission. Its
    // failure must precede committing any otherwise valid staged hive mutation.
    region
        .tree_effects
        .beehives
        .insert((1, 80, 1), vec![97, -7]);
    region.tree_effects.tick_requests.push(tick);
    let effects = region.tree_effects.clone();
    assert!(catch_unwind(AssertUnwindSafe(|| region.transfer_tree_effects())).is_err());
    assert_eq!(region.tree_effects, effects);
    assert_eq!(*region.owned_chunk_ref(owner).unwrap(), before);
    assert_eq!(*region.owned_chunk_ref(far).unwrap(), far_before);
}
