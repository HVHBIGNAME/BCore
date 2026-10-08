use super::*;
use crate::block_entity::BlockEntity;
use crate::feature_world::FeatureWorld;
use crate::ore::OreWorld;
use crate::tick_request::{TickRequest, TickTarget};
use crate::tree::standing::StandingTreeWorld;
use crate::{block, MIN_Y};
use std::sync::{Arc, Barrier};

fn fast_stage(
    status: ChunkStatus,
    pos: ChunkPos,
    region: &mut FeatureRegion,
) -> Result<(), String> {
    match status {
        ChunkStatus::Biomes => {
            region.owned_chunk_mut(pos).noise_biomes = Some(vec![crate::biome::ids::PLAINS; 1536])
        }
        ChunkStatus::Noise => {
            let chunk = region.owned_chunk_mut(pos);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 64, z, block::GRASS_BLOCK);
                }
            }
        }
        ChunkStatus::Features => {
            // A source's hive is owned by its eastern neighbour, whose own
            // FEATURES work can have run already or can run in a later request.
            let hive = (pos.x * 16 + 16, 90, pos.z * 16 + 4);
            assert!(region.set_feature_block(hive, 21774, 19));
            region.store_bee(hive, 97);
            let request = TickRequest {
                block_pos: [hive.0, hive.1, hive.2],
                target: TickTarget::Fluid(2),
                delay: -5,
            };
            assert!(region.schedule_feature_tick(request));
            assert!(region.schedule_feature_tick(request));
            region.mark_feature_postprocessing(hive);
            region.mark_feature_postprocessing(hive);
            assert!(region.set_block((pos.x * 16 + 15, 90, pos.z * 16 + 5), block::OAK_LOG));
        }
        _ => {}
    }
    Ok(())
}

fn fast_world(seed: i64) -> GenerationWorld {
    let world = GenerationWorld::new(seed);
    world.state.lock().unwrap().test_stage = Some(fast_stage);
    world
}

#[test]
fn requests_use_accumulated_layers_through_spawn_but_leave_full_pending() {
    let world = fast_world(42);
    let result = world.generate_chunk(ChunkPos::new(-2, 3)).unwrap();
    let state = world.state.lock().unwrap();
    assert_eq!(state.holders.len(), 529);
    for (status, expected) in ChunkStatus::ALL
        .into_iter()
        .zip([529, 529, 49, 49, 25, 25, 25, 9, 9, 1, 1, 0])
    {
        assert_eq!(
            state
                .holders
                .values()
                .filter(|h| h.progress.stage(status).state.has_data())
                .count(),
            expected,
            "{status}"
        );
    }
    assert_eq!(result.coverage.target.stage(ChunkStatus::Spawn).attempts, 1);
    for status in [ChunkStatus::Full] {
        assert_eq!(
            result.coverage.target.stage(status).state,
            StageState::Pending
        );
        assert_eq!(result.coverage.target.stage(status).attempts, 0);
    }
    assert!(result.coverage.incoming_sources_finished);
    assert!(!result.coverage.is_complete());
}

#[test]
fn deferred_full_request_materializes_target_only_and_keeps_conversion_pending() {
    fn stage(status: ChunkStatus, pos: ChunkPos, region: &mut FeatureRegion) -> Result<(), String> {
        if status == ChunkStatus::Features {
            let state = crate::structure::template_pool::StructureAssets::bundled()
                .blocks
                .default_state("sculk_sensor")
                .unwrap();
            region.set_feature_block((pos.x * 16, 80, pos.z * 16), state, 18);
        }
        Ok(())
    }
    let world = GenerationWorld::new(846692123413862008);
    world.state.lock().unwrap().test_stage = Some(stage);
    let target = ChunkPos::new(0, 0);
    let proto = world
        .generate_to_status(target, ChunkStatus::Features)
        .unwrap();
    assert_eq!(proto.chunk.pending_block_entities().len(), 1);
    assert!(proto.chunk.feature_block_entities().is_empty());
    let full = world.generate_chunk(target).unwrap();
    assert!(full.chunk.pending_block_entities().is_empty());
    assert_eq!(full.chunk.feature_block_entities().len(), 1);
    let conversion = full.coverage.target.stage(ChunkStatus::Full);
    assert_eq!(conversion.state, StageState::Pending);
    assert_eq!(conversion.attempts, 0);
    assert!(!full.coverage.is_complete());
    let neighbour = world.chunk_snapshot(ChunkPos::new(1, 0)).unwrap().unwrap();
    assert_eq!(neighbour.pending_block_entities().len(), 1);
    assert!(neighbour.feature_block_entities().is_empty());
    assert_eq!(
        world
            .generate_to_status(target, ChunkStatus::Features)
            .unwrap()
            .chunk,
        full.chunk,
        "a later lower-status read must preserve earlier materialization"
    );
}

#[test]
fn adjacent_requests_retain_once_only_feature_blocks_bees_ticks_and_marks() {
    let world = fast_world(1234);
    let first = world.generate_chunk(ChunkPos::new(0, 0)).unwrap();
    let pending_neighbour = world.chunk_snapshot(ChunkPos::new(1, 0)).unwrap().unwrap();
    assert_eq!(
        first.chunk.block_entities()[&(0, 90, 4)],
        BlockEntity::Beehive {
            ticks_in_hive: vec![97]
        }
    );
    let adjacent = world.generate_chunk(ChunkPos::new(1, 0)).unwrap();
    assert_eq!(adjacent.chunk, pending_neighbour);
    assert_eq!(adjacent.chunk.get(15, 90, 5), Some(block::OAK_LOG));
    assert_eq!(
        adjacent.chunk.block_entities()[&(0, 90, 4)],
        BlockEntity::Beehive {
            ticks_in_hive: vec![97]
        }
    );
    assert_eq!(adjacent.chunk.tick_requests().len(), 2);
    assert_eq!(
        adjacent.chunk.tick_requests()[0],
        adjacent.chunk.tick_requests()[1]
    );
    assert_eq!(
        adjacent.chunk.postprocessing_positions(),
        &[(0, 90, 4), (0, 90, 4)]
    );
    let repeated = world.generate_chunk(ChunkPos::new(0, 0)).unwrap();
    assert_eq!(first.chunk, repeated.chunk);
    let state = world.state.lock().unwrap();
    let executed: Vec<_> = state
        .holders
        .values()
        .filter(|h| h.progress.stage(ChunkStatus::Features).attempts > 0)
        .collect();
    assert_eq!(executed.len(), 12);
    assert!(executed
        .iter()
        .all(|h| h.progress.stage(ChunkStatus::Features).attempts == 1));
}

#[test]
fn concurrent_duplicate_requests_coalesce_all_status_claims() {
    let world = Arc::new(fast_world(7));
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let world = world.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            world.generate_chunk(ChunkPos::new(-3, -1)).unwrap().chunk
        }));
    }
    barrier.wait();
    assert_eq!(
        workers.remove(0).join().unwrap(),
        workers.remove(0).join().unwrap()
    );
    let state = world.state.lock().unwrap();
    assert!(state
        .holders
        .values()
        .flat_map(|h| &h.progress.stages)
        .all(|stage| stage.attempts <= 1));
}

#[test]
fn independent_worlds_with_equal_or_different_seeds_never_share_live_chunks() {
    let a = fast_world(1);
    let same_seed = fast_world(1);
    let different_seed = fast_world(2);
    let _ = a.generate_chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(same_seed.progress(ChunkPos::new(0, 0)).unwrap().is_none());
    assert!(different_seed
        .chunk_snapshot(ChunkPos::new(0, 0))
        .unwrap()
        .is_none());
    assert_eq!(
        (a.seed(), same_seed.seed(), different_seed.seed()),
        (1, 1, 2)
    );
    let _ = same_seed.generate_chunk(ChunkPos::new(0, 0)).unwrap();
    a.state
        .lock()
        .unwrap()
        .region
        .owned_chunk_mut(ChunkPos::new(0, 0))
        .set(1, MIN_Y, 1, block::DIAMOND_ORE);
    assert_eq!(
        same_seed
            .chunk_snapshot(ChunkPos::new(0, 0))
            .unwrap()
            .unwrap()
            .get(1, MIN_Y, 1),
        Some(block::AIR)
    );
    let p = ChunkPos::new(-2, 3);
    let real_a = GenerationWorld::new(a.seed())
        .generate_to_status(p, ChunkStatus::Noise)
        .unwrap();
    let real_b = GenerationWorld::new(different_seed.seed())
        .generate_to_status(p, ChunkStatus::Noise)
        .unwrap();
    assert_ne!(
        real_a.chunk.states(),
        real_b.chunk.states(),
        "terrain keeps the world's own seed"
    );
}

fn failing_stage(
    status: ChunkStatus,
    pos: ChunkPos,
    region: &mut FeatureRegion,
) -> Result<(), String> {
    fast_stage(status, pos, region)?;
    if status == ChunkStatus::Features && pos == ChunkPos::new(0, 0) {
        Err("injected feature failure".into())
    } else {
        Ok(())
    }
}

#[test]
fn generation_error_releases_claims_and_preserves_preceding_source_effects() {
    let world = fast_world(1);
    world.state.lock().unwrap().test_stage = Some(failing_stage);
    let error = world
        .generate_to_status(ChunkPos::new(0, 0), ChunkStatus::Features)
        .unwrap_err();
    assert!(error.to_string().contains("injected feature failure"));
    let progress = world.progress(ChunkPos::new(0, 0)).unwrap().unwrap();
    assert_eq!(
        progress.stage(ChunkStatus::Features).state,
        StageState::Failed
    );
    let retained = world.chunk_snapshot(ChunkPos::new(1, 0)).unwrap().unwrap();
    assert_eq!(retained.tick_requests().len(), 2);
    assert_eq!(
        retained.block_entities()[&(0, 90, 4)],
        BlockEntity::Beehive {
            ticks_in_hive: vec![97]
        }
    );
    assert_eq!(
        world
            .generate_to_status(ChunkPos::new(0, 0), ChunkStatus::Features)
            .unwrap_err(),
        error
    );
    assert_eq!(
        world.chunk_snapshot(ChunkPos::new(1, 0)).unwrap().unwrap(),
        retained
    );
    // FEATURES has no same-status neighbour dependency. The failed source must
    // not keep a write guard or poison another source's independent claim.
    let _ = world
        .generate_to_status(ChunkPos::new(1, 0), ChunkStatus::Features)
        .unwrap();
    assert_eq!(
        world
            .progress(ChunkPos::new(0, 0))
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::Features)
            .attempts,
        1
    );
    assert!(world.state.lock().unwrap().holders.values().all(|h| h
        .progress
        .stages
        .iter()
        .all(|s| s.state != StageState::Running)));
}

#[test]
fn panic_in_generation_does_not_poison_the_world_mutex_or_leave_a_source_guard() {
    fn panicking(
        status: ChunkStatus,
        pos: ChunkPos,
        region: &mut FeatureRegion,
    ) -> Result<(), String> {
        fast_stage(status, pos, region)?;
        if status == ChunkStatus::Features && pos == ChunkPos::new(0, 0) {
            panic!("injected panic");
        }
        Ok(())
    }
    let world = fast_world(1);
    world.state.lock().unwrap().test_stage = Some(panicking);
    assert!(matches!(
        world.generate_to_status(ChunkPos::new(0, 0), ChunkStatus::Features),
        Err(GenerationError::StageFailed { .. })
    ));
    let _ = world
        .generate_to_status(ChunkPos::new(1, 0), ChunkStatus::Features)
        .unwrap();
    assert_eq!(
        world
            .chunk_snapshot(ChunkPos::new(1, 0))
            .unwrap()
            .unwrap()
            .tick_requests()
            .len(),
        2
    );
}

#[test]
fn invalid_coordinates_leave_no_claims() {
    let world = fast_world(0);
    for pos in [ChunkPos::new(i32::MAX, 0), ChunkPos::new(0, i32::MIN)] {
        assert_eq!(
            world.generate_chunk(pos).unwrap_err(),
            GenerationError::CoordinateOutOfRange(pos)
        );
    }
    assert!(world.state.lock().unwrap().holders.is_empty());
    let _ = world.generate_chunk(ChunkPos::new(0, 0)).unwrap();
}

#[test]
fn real_source_driver_reuses_neighbours_and_reports_every_candidate_feature() {
    let world = fast_world(1234);
    {
        let mut state = world.state.lock().unwrap();
        // Flat CARVERS fixtures exercise the real driver and its owning region
        // without making this ordering test depend on a terrain/biome location.
        for target in [ChunkPos::new(0, 0), ChunkPos::new(1, 0)] {
            for status in ChunkStatus::ALL
                .into_iter()
                .take(ChunkStatus::Features.index())
            {
                let radius = ChunkPyramid::Generation
                    .step(ChunkStatus::Full)
                    .layer_radius(status)
                    .unwrap();
                for pos in layer_positions(target, radius) {
                    state.run_stage(world.generator, pos, status).unwrap();
                }
            }
        }
        let owners: Vec<_> = state
            .holders
            .iter()
            .filter(|(_, h)| h.progress.stage(ChunkStatus::Biomes).state.has_data())
            .map(|(&p, _)| ChunkPos::new(p.0, p.1))
            .collect();
        let dripstone = crate::biome::id("dripstone_caves").unwrap();
        for pos in owners {
            let carved = state.holders[&(pos.x, pos.z)]
                .progress
                .stage(ChunkStatus::Carvers)
                .state
                .has_data();
            let chunk = state.region.owned_chunk_mut(pos);
            chunk
                .noise_biomes
                .as_mut()
                .unwrap()
                .fill(crate::biome::ids::FOREST);
            chunk.noise_biomes.as_mut().unwrap()[..32 * 16].fill(dripstone);
            if carved {
                for y in MIN_Y..64 {
                    for x in 0..16 {
                        for z in 0..16 {
                            chunk.set(x, y, z, block::STONE);
                        }
                    }
                }
            }
        }
        state.test_stage = None;
    }
    let first = world.generate_chunk(ChunkPos::new(0, 0)).unwrap();
    assert_eq!(first.coverage.feature_sources.len(), 9);
    assert!(first.coverage.incoming_sources_finished);
    assert!(!first.coverage.is_complete());
    let expected: BTreeSet<_> = ["forest", "dripstone_caves"]
        .into_iter()
        .flat_map(|biome| {
            (0..11).flat_map(move |step| {
                crate::feature_sorter::sorter()
                    .indices_for_biome(step, biome)
                    .iter()
                    .map(move |&index| {
                        (
                            step,
                            index,
                            crate::feature_sorter::sorter()
                                .feature_name(step, index)
                                .unwrap()
                                .to_owned(),
                        )
                    })
            })
        })
        .chain(
            crate::structure::template_pool::StructureAssets::bundled()
                .structure_metadata
                .iter()
                .map(|(name, metadata)| {
                    (
                        metadata.step as usize,
                        metadata.feature_index as usize,
                        name.clone(),
                    )
                }),
        )
        .chain(
            crate::structure::scattered::ScatteredKind::ALL
                .into_iter()
                .map(|kind| {
                    let config =
                        crate::structure::scattered::ScatteredCatalog::bundled().config(kind);
                    (
                        config.decoration_step as usize,
                        config.structure_index as usize,
                        kind.name().into(),
                    )
                }),
        )
        .collect();
    for source in &first.coverage.feature_sources {
        assert!(source.mineshafts_processed);
        assert!(
            source
                .completed
                .iter()
                .any(|work| work.feature == "pointed_dripstone"),
            "nested placement must share the cave Gaussian cache: {:?}",
            source.missing
        );
        let actual: BTreeSet<_> = source
            .completed
            .iter()
            .map(|work| (work.step, work.index, work.feature.clone()))
            .chain(
                source
                    .missing
                    .iter()
                    .map(|work| (work.step, work.index, work.feature.clone())),
            )
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(
            source.completed.len() + source.missing.len(),
            expected.len()
        );
        assert!(source.completed.windows(2).all(|p| {
            let key = |work: &FeatureWork| {
                (
                    work.step,
                    !(crate::structure::template_pool::StructureAssets::bundled()
                        .structure_metadata
                        .contains_key(&work.feature)
                        || crate::structure::scattered::ScatteredKind::ALL
                            .iter()
                            .any(|kind| kind.name() == work.feature)),
                    work.index,
                )
            };
            key(&p[0]) < key(&p[1])
        }));
    }
    assert!(first.coverage.feature_sources.iter().any(|source| source
        .completed
        .iter()
        .any(|work| work.feature.starts_with("ore_") && work.placed)));
    let second = world.generate_chunk(ChunkPos::new(1, 0)).unwrap();
    // Lighting can legitimately reach this column when the neighbour's LIGHT
    // stage runs; its already-owned feature blocks and effects are stable.
    let refreshed = world.chunk_snapshot(ChunkPos::new(0, 0)).unwrap().unwrap();
    assert_eq!(refreshed.states(), first.chunk.states());
    assert_eq!(refreshed.block_entities(), first.chunk.block_entities());
    assert_eq!(
        refreshed.feature_block_entities(),
        first.chunk.feature_block_entities()
    );
    assert_eq!(refreshed.tick_requests(), first.chunk.tick_requests());
    assert_eq!(
        refreshed.postprocessing_positions(),
        first.chunk.postprocessing_positions()
    );
    assert_eq!(
        world.generate_chunk(ChunkPos::new(1, 0)).unwrap().chunk,
        second.chunk
    );
    let state = world.state.lock().unwrap();
    assert_eq!(state.source_sequence, 12);
    assert!(state
        .holders
        .values()
        .all(|h| h.progress.stage(ChunkStatus::Features).attempts <= 1));
}

#[test]
fn invalid_tree_effect_batch_is_retained_on_the_failed_source_and_does_not_leak() {
    fn invalid(
        status: ChunkStatus,
        pos: ChunkPos,
        region: &mut FeatureRegion,
    ) -> Result<(), String> {
        fast_stage(status, pos, region)?;
        if status == ChunkStatus::Features && pos == ChunkPos::new(0, 0) {
            region.schedule_tree_tick([0, 90, 0, -1, 1, 0]);
        }
        Ok(())
    }
    let world = fast_world(1);
    world.state.lock().unwrap().test_stage = Some(invalid);
    assert!(world
        .generate_to_status(ChunkPos::new(0, 0), ChunkStatus::Features)
        .is_err());
    {
        let state = world.state.lock().unwrap();
        let failed = &state.holders[&(0, 0)];
        assert_eq!(
            failed.pending_tree_effects.tick_requests,
            vec![[0, 90, 0, -1, 1, 0]]
        );
        assert_eq!(failed.pending_tree_effects.beehives[&(16, 90, 4)], vec![97]);
        assert_eq!(state.region.tree_effects, Default::default());
    }
    let _ = world
        .generate_to_status(ChunkPos::new(1, 0), ChunkStatus::Features)
        .unwrap();
    assert_eq!(
        world
            .chunk_snapshot(ChunkPos::new(2, 0))
            .unwrap()
            .unwrap()
            .block_entities()[&(0, 90, 4)],
        BlockEntity::Beehive {
            ticks_in_hive: vec![97]
        }
    );
}

#[test]
fn real_full_target_reports_actual_feature_coverage_and_is_reused() {
    let world = GenerationWorld::new(1234);
    let pos = ChunkPos::new(0, 0);
    let first = world.generate_chunk(pos).unwrap();
    assert_eq!(
        first.chunk.states().len(),
        16 * 16 * crate::WORLD_HEIGHT as usize
    );
    assert_eq!(first.coverage.feature_sources.len(), 9);
    assert!(first.coverage.incoming_sources_finished);
    assert_eq!(
        first.coverage.target.stage(ChunkStatus::Full).state,
        StageState::Pending
    );
    let repeated = world.generate_chunk(pos).unwrap();
    assert_eq!(first.chunk, repeated.chunk);
    assert_eq!(first.coverage, repeated.coverage);
    let missing: BTreeSet<_> = first
        .coverage
        .missing_features()
        .map(|m| (&m.feature, &m.reason))
        .collect();
    let completed = first
        .coverage
        .feature_sources
        .iter()
        .map(|s| s.completed.len())
        .sum::<usize>();
    println!("GenerationWorld::new(1234).generate_chunk({pos:?}): {} source tasks, {completed} completed placed streams, {} unique missing feature/reason pairs, {} deferred marks, {} ticks", first.coverage.feature_sources.len(), missing.len(), first.chunk.postprocessing_positions().len(), first.chunk.tick_requests().len());
    for (feature, reason) in missing {
        println!("missing {feature}: {reason}");
    }
    for stage in &first.coverage.missing_stages {
        println!("stage {}: {}", stage.status, stage.reason);
    }
}
