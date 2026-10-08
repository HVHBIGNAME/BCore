use super::*;
use crate::generation::{ChunkHolder, ChunkStatus, StageState};
use crate::spawn::{BiomeSpawns, SpawnTag, SpawnerData};
use crate::structure::template::Nbt;
use crate::{biome, block};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

struct Inputs {
    next: AtomicI64,
    calls: AtomicU64,
    failure: Option<(u64, bool)>,
    enabled: bool,
    time: i64,
}

impl Inputs {
    fn new(seed: i64) -> Self {
        Self {
            next: AtomicI64::new(seed),
            calls: AtomicU64::new(0),
            failure: None,
            enabled: true,
            time: 0,
        }
    }
}

impl SpawnInputs for Inputs {
    fn spawn_mobs(&self) -> bool {
        self.enabled
    }
    fn entity_seed(&self, _: MobKind) -> SpawnResult<i64> {
        let index = self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some((fail_at, panic)) = self.failure {
            if index == fail_at {
                assert!(!panic, "injected entity factory panic");
                return Err(SpawnError::MissingData(
                    "injected entity entropy failure".into(),
                ));
            }
        }
        Ok(self.next.fetch_add(1, Ordering::Relaxed))
    }
    fn difficulty_at(&self, _: Pos) -> SpawnResult<SpawnDifficulty> {
        Ok(SpawnDifficulty {
            difficulty: 2,
            overworld_time: 0,
            moon_brightness: 1.0,
            inhabited_time: 0,
        })
    }
    fn game_time(&self) -> SpawnResult<i64> {
        Ok(self.time)
    }
}

/// Explicit flat FEATURES inputs. The real region, light initialization and
/// propagation, SPAWN adapter and subsequent claims all run normally.
fn flat(
    seed: i64,
    biome: u32,
    ground: u32,
    y: i32,
    inputs: Arc<dyn SpawnInputs>,
) -> GenerationState {
    let generator = WorldGenerator::new(seed);
    let mut state = GenerationState::new(generator);
    state.spawn_inputs = inputs;
    for x in -1..=2 {
        for z in -1..=1 {
            let pos = ChunkPos::new(x, z);
            let mut holder = ChunkHolder::new(pos);
            for stage in &mut holder.progress.stages[..=ChunkStatus::Features.index()] {
                stage.state = StageState::Complete;
            }
            state.holders.insert((x, z), holder);
            let chunk = state.region.owned_chunk_mut(pos);
            chunk.noise_biomes = Some(vec![biome; 1536]);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, y, z, ground);
                }
            }
        }
    }
    for x in -1..=2 {
        for z in -1..=1 {
            state
                .run_stage(generator, ChunkPos::new(x, z), ChunkStatus::InitializeLight)
                .unwrap();
        }
    }
    for x in 0..=1 {
        state
            .run_stage(generator, ChunkPos::new(x, 0), ChunkStatus::Light)
            .unwrap();
    }
    state
}

fn canonical(mut nbt: Nbt) -> Nbt {
    if let Some(Nbt::List { values, .. }) = nbt.compound_mut().unwrap().get_mut("attributes") {
        values.sort_by_key(|v| v.get("id").unwrap().string().unwrap().to_owned());
    }
    nbt
}

#[test]
fn retained_region_adapter_stores_native_saves_for_all_nineteen_factories() {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../data/generation_spawn_handoff_26_1.json"
    ))
    .unwrap();
    let mut kinds = std::collections::BTreeSet::new();
    let mut total = 0;
    for case in data["cases"].as_array().unwrap() {
        let input = &case["input"];
        let name = input["forced"].as_str().unwrap();
        if !kinds.insert(name.to_owned()) {
            continue;
        }
        let seed = input["seed"].as_str().unwrap().parse().unwrap();
        let mut inputs = Inputs::new(input["entity_seed"].as_str().unwrap().parse().unwrap());
        inputs.time = input["game_time"].as_str().unwrap().parse().unwrap();
        let mut state = flat(
            seed,
            biome::id(input["biome"].as_str().unwrap()).unwrap(),
            crate::block_predicate::catalog()
                .default_state(input["ground"].as_str().unwrap())
                .unwrap(),
            input["ground_y"].as_i64().unwrap() as i32,
            Arc::new(inputs),
        );
        let available = state
            .holders
            .iter()
            .map(|(&key, h)| (key, h.progress.available_status().unwrap()))
            .collect();
        state
            .region
            .begin_source(ChunkPos::new(0, 0), ChunkStatus::Spawn, available);
        let mut env = RegionSpawnEnvironment {
            region: &state.region,
            light: &state.light,
            light_missing: None,
            inputs: state.spawn_inputs.as_ref(),
            random: SpawnRandom::xoroshiro(
                input["environment_seed"].as_str().unwrap().parse().unwrap(),
            ),
        };
        let (_, mut placement) = SpawnRandom::for_chunk(seed, [0, 0]);
        let settings = BiomeSpawns {
            probability: input["probability"].as_f64().unwrap() as f32,
            groups: vec![SpawnerData {
                kind: MobKind::from_name(name).unwrap(),
                weight: 1,
                min_count: input["min"].as_i64().unwrap() as i32,
                max_count: input["max"].as_i64().unwrap() as i32,
            }],
        };
        let report = spawn::spawn_mobs_for_chunk_generation(
            &state.region,
            &mut env,
            &settings,
            [0, 0],
            &mut placement,
            false,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        state.region.end_source();
        let chunk = state.region.owned_chunk(ChunkPos::new(0, 0)).unwrap();
        assert_eq!(chunk.entities().len(), report.mobs.len());
        let expected = case["entities"].as_array().unwrap();
        assert_eq!(chunk.entities().len(), expected.len(), "{name}");
        for (entity, native) in chunk.entities().iter().zip(expected) {
            let GeneratedEntity::Mob(mob) = entity else {
                panic!("expected mob");
            };
            let nbt = Nbt::from_logical_typed_json(
                &SpawnTag::from_native_json(&native["typed_nbt"])
                    .unwrap()
                    .native_json(),
            )
            .unwrap();
            assert_eq!(canonical(mob.nbt()), canonical(nbt), "{name}");
            assert!(entity.valid_for(ChunkPos::new(0, 0)));
            total += 1;
        }
    }
    assert_eq!(kinds.len(), 19);
    assert_eq!(total, 76);
}

#[test]
fn real_spawn_claim_retains_mobs_once_across_repeated_and_adjacent_requests() {
    let inputs = Arc::new(Inputs::new(1000));
    let generator = WorldGenerator::new(4096);
    let mut state = flat(
        4096,
        biome::ids::PLAINS,
        block::GRASS_BLOCK,
        64,
        inputs.clone(),
    );
    let pos = ChunkPos::new(0, 0);
    state.run_stage(generator, pos, ChunkStatus::Spawn).unwrap();
    let chunk = state.region.owned_chunk(pos).unwrap();
    assert!(
        !chunk.entities().is_empty(),
        "the real biome list must create mobs"
    );
    let report = state.holders[&(0, 0)].spawn.as_ref().unwrap();
    assert_eq!(chunk.entities().len(), report.mobs.len());
    let calls = inputs.calls.load(Ordering::Relaxed);
    state.run_stage(generator, pos, ChunkStatus::Spawn).unwrap();
    assert_eq!(inputs.calls.load(Ordering::Relaxed), calls);
    state
        .run_stage(generator, ChunkPos::new(1, 0), ChunkStatus::Spawn)
        .unwrap();
    assert_eq!(
        state.region.owned_chunk(pos).unwrap().entities(),
        chunk.entities()
    );
    let progress = &state.holders[&(0, 0)].progress;
    assert_eq!(
        progress.stage(ChunkStatus::Spawn).state,
        StageState::Complete
    );
    assert_eq!(progress.stage(ChunkStatus::Spawn).attempts, 1);
    assert_eq!(progress.stage(ChunkStatus::Full).state, StageState::Pending);
}

#[test]
fn late_spawn_failure_or_panic_preserves_accepted_mobs_and_does_not_retry() {
    for panic in [false, true] {
        let mut inputs = Inputs::new(1000);
        inputs.failure = Some((1, panic));
        let inputs = Arc::new(inputs);
        let mut state = flat(
            4096,
            biome::ids::PLAINS,
            block::GRASS_BLOCK,
            64,
            inputs.clone(),
        );
        let pos = ChunkPos::new(0, 0);
        let result = state.run_stage(WorldGenerator::new(4096), pos, ChunkStatus::Spawn);
        assert_eq!(result.is_err(), panic);
        let saved = state.region.owned_chunk(pos).unwrap().entities().to_vec();
        assert_eq!(saved.len(), 1);
        let expected = if panic {
            StageState::Failed
        } else {
            StageState::Partial
        };
        assert_eq!(
            state.holders[&(0, 0)]
                .progress
                .stage(ChunkStatus::Spawn)
                .state,
            expected
        );
        if !panic {
            assert_eq!(state.holders[&(0, 0)].spawn.as_ref().unwrap().mobs.len(), 1);
        }
        let _ = state.run_stage(WorldGenerator::new(4096), pos, ChunkStatus::Spawn);
        assert_eq!(inputs.calls.load(Ordering::Relaxed), 2);
        assert_eq!(state.region.owned_chunk(pos).unwrap().entities(), saved);
        assert_eq!(
            state.holders[&(0, 0)]
                .progress
                .stage(ChunkStatus::Spawn)
                .attempts,
            1
        );
    }
}

#[test]
fn world_gamerule_and_noise_settings_skip_without_consuming_entity_entropy() {
    let mut input = Inputs::new(1000);
    input.enabled = false;
    let input = Arc::new(input);
    let mut state = flat(
        4096,
        biome::ids::PLAINS,
        block::GRASS_BLOCK,
        64,
        input.clone(),
    );
    state
        .run_stage(
            WorldGenerator::new(4096),
            ChunkPos::new(0, 0),
            ChunkStatus::Spawn,
        )
        .unwrap();
    assert_eq!(
        state.holders[&(0, 0)].spawn.as_ref().unwrap().skipped,
        Some(spawn::SpawnSkip::SpawnMobsGameRule)
    );
    assert!(state
        .region
        .owned_chunk(ChunkPos::new(0, 0))
        .unwrap()
        .entities()
        .is_empty());
    assert_eq!(input.calls.load(Ordering::Relaxed), 0);
    // This path must skip even before any biome or light input is accessed.
    let mut empty = GenerationState::new(WorldGenerator::new(0));
    empty
        .holders
        .insert((0, 0), ChunkHolder::new(ChunkPos::new(0, 0)));
    empty
        .spawn_source(WorldGenerator::new(0), ChunkPos::new(0, 0), true)
        .unwrap();
    assert_eq!(
        empty.holders[&(0, 0)].spawn.as_ref().unwrap().skipped,
        Some(spawn::SpawnSkip::MobGenerationDisabled)
    );
}
