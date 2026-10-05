use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::OnceLock;

use bcore_worldgen::biome;
use bcore_worldgen::block_predicate::catalog;
use bcore_worldgen::feature_world::{FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::spawn::*;
use bcore_worldgen::tick_request::TickRequest;
use bcore_worldgen::{MAX_Y, MIN_Y};
use serde_json::{json, Value};

fn fixtures() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../data/generation_spawn_reference_26_1_v1.json"
        ))
        .unwrap()
    })
}

fn expanded_fixtures() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../data/generation_spawn_reference_26_1_v2.json"
        ))
        .unwrap()
    })
}

fn callback_fixtures() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../data/generation_spawn_callbacks_reference_26_1_v1.json"
        ))
        .unwrap()
    })
}

struct World {
    input: Value,
    biome_reads: RefCell<Vec<Pos>>,
    missing: Cell<bool>,
}

impl World {
    fn new(input: &Value) -> Self {
        Self {
            input: input.clone(),
            biome_reads: RefCell::new(Vec::new()),
            missing: Cell::new(false),
        }
    }

    fn state(&self, (x, y, _z): Pos) -> u32 {
        let terrain = self.input["terrain"].as_str().unwrap();
        let mut ground = self.input["ground_y"].as_i64().unwrap() as i32;
        if terrain == "steps" {
            ground += x.rem_euclid(4);
        }
        let name = if !(MIN_Y..=MAX_Y).contains(&y) || terrain == "void" {
            "air"
        } else if y <= ground {
            self.input["ground"].as_str().unwrap()
        } else if terrain == "low_ceiling" && y == ground + 2
            || terrain == "lateral_collision" && x.rem_euclid(2) == 0 && y <= ground + 3
        {
            "stone"
        } else if y == ground + 1 {
            match terrain {
                "water" => "water",
                "snow" => "snow",
                "fence" => "oak_fence",
                _ => "air",
            }
        } else {
            "air"
        };
        catalog().default_state(name).unwrap()
    }
}

impl OreWorld for World {
    fn get_block(&self, pos: Pos) -> Option<u32> {
        (!self.missing.get()).then(|| self.state(pos))
    }
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.feature_height(FeatureHeightmap::OceanFloorWg, x, z)
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("spawning must not write terrain")
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.biome_reads.borrow_mut().push(pos);
        biome::id(self.input["biome"].as_str().unwrap()).unwrap()
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        for y in (MIN_Y..=MAX_Y).rev() {
            let state = self.state((x, y, z));
            let info = catalog().info(state).unwrap();
            let opaque = match kind {
                FeatureHeightmap::WorldSurface | FeatureHeightmap::WorldSurfaceWg => !info.is_air(),
                FeatureHeightmap::OceanFloor | FeatureHeightmap::OceanFloorWg => {
                    info.blocks_motion()
                }
                FeatureHeightmap::MotionBlocking => info.blocks_motion() || info.fluid_amount > 0,
                FeatureHeightmap::MotionBlockingNoLeaves => {
                    (info.blocks_motion() || info.fluid_amount > 0)
                        && !catalog().in_block_tag(state, "leaves").unwrap()
                }
            };
            if opaque {
                return y + 1;
            }
        }
        MIN_Y
    }
    fn can_write_feature(&self, _: Pos) -> bool {
        panic!("no block writes")
    }
    fn set_feature_block(&mut self, _: Pos, _: u32, _: i32) -> bool {
        panic!("no block writes")
    }
    fn mark_feature_postprocessing(&mut self, _: Pos) {
        panic!("no postprocessing")
    }
    fn schedule_feature_tick(&mut self, _: TickRequest) -> bool {
        panic!("no gameplay ticks")
    }
}

struct Environment {
    input: Value,
    random: SpawnRandom,
    next_entity: i64,
    enabled: bool,
    difficulty_calls: Cell<usize>,
    entropy_missing: bool,
}

impl Environment {
    fn new(input: &Value) -> Self {
        Self {
            input: input.clone(),
            random: SpawnRandom::xoroshiro(
                input["environment_seed"].as_str().unwrap().parse().unwrap(),
            )
            .trace(),
            next_entity: input["entity_seed"].as_str().unwrap().parse().unwrap(),
            enabled: true,
            difficulty_calls: Cell::new(0),
            entropy_missing: false,
        }
    }
}

impl SpawnEnvironment for Environment {
    fn spawn_mobs(&self) -> bool {
        self.enabled
    }
    fn random(&mut self) -> &mut SpawnRandom {
        &mut self.random
    }
    fn entity_seed(&mut self, _: MobKind) -> SpawnResult<i64> {
        if self.entropy_missing {
            return Err(SpawnError::MissingData("entity entropy".into()));
        }
        let value = self.next_entity;
        self.next_entity = self.next_entity.wrapping_add(1);
        Ok(value)
    }
    fn raw_brightness(&self, _: &dyn FeatureWorld, _: Pos, _: i32) -> SpawnResult<i32> {
        Ok(self.input["brightness"].as_i64().unwrap() as i32)
    }
    fn current_difficulty_at(&self, _: Pos) -> SpawnResult<SpawnDifficulty> {
        self.difficulty_calls.set(self.difficulty_calls.get() + 1);
        Ok(SpawnDifficulty {
            difficulty: 2,
            overworld_time: 0,
            moon_brightness: 0.0,
            inhabited_time: 0,
        })
    }
    fn game_time(&self) -> SpawnResult<i64> {
        // v1 captures preceded the clock input field and ran before the first tick.
        Ok(self.input["game_time"]
            .as_str()
            .map(|time| time.parse().unwrap())
            .unwrap_or(0))
    }
    fn border(&self) -> SpawnBorder {
        if let Some(size) = self.input["border_size"].as_f64() {
            SpawnBorder {
                min_x: -size / 2.0,
                min_z: -size / 2.0,
                max_x: size / 2.0,
                max_z: size / 2.0,
            }
        } else {
            SpawnBorder::default()
        }
    }
    fn no_collision(&self, world: &dyn FeatureWorld, bounds: SpawnBox) -> SpawnResult<bool> {
        if self.input["collision"] == true {
            Ok(false)
        } else {
            no_block_collision(world, self, bounds)
        }
    }
    fn is_unobstructed(&self, _: MobKind, _: SpawnBox) -> SpawnResult<bool> {
        Ok(self.input["obstructed"] != true)
    }
}

fn settings(input: &Value) -> BiomeSpawns {
    if let Some(forced) = input["forced"].as_str() {
        BiomeSpawns {
            probability: input["probability"].as_f64().unwrap() as f32,
            groups: if forced == "empty" {
                vec![]
            } else {
                vec![SpawnerData {
                    kind: MobKind::from_name(forced).unwrap(),
                    weight: 1,
                    min_count: input["min"].as_i64().unwrap() as i32,
                    max_count: input["max"].as_i64().unwrap() as i32,
                }]
            },
        }
    } else {
        biome_spawns(biome::id(input["biome"].as_str().unwrap()).unwrap())
            .unwrap()
            .clone()
    }
}

fn source(input: &Value) -> SpawnRandom {
    let seed: i64 = input["seed"].as_str().unwrap().parse().unwrap();
    let chunk: [i32; 2] = serde_json::from_value(input["chunk"].clone()).unwrap();
    let (decoration, random) = SpawnRandom::for_chunk(seed, chunk);
    assert_eq!(
        decoration.to_string(),
        input["decoration_seed"].as_str().unwrap()
    );
    random.trace()
}

fn canonical(mut tag: SpawnTag) -> SpawnTag {
    if let SpawnTag::Compound(fields) = &mut tag {
        if let Some(SpawnTag::List(entries)) = fields.get_mut("attributes") {
            entries.sort_by_key(|entry| entry.data()["id"].as_str().unwrap().to_owned());
        }
    }
    tag
}

fn rng_matches(random: &SpawnRandom, expected: &Value, label: &str) {
    let actual: Vec<_> = random.draws().iter().map(SpawnDraw::native_trace).collect();
    let wanted: Vec<String> = serde_json::from_value(expected["draws"].clone()).unwrap();
    assert_eq!(actual, wanted, "RNG draws {label}: {}", expected["stream"]);
    assert_eq!(
        random.peek_next_long().to_string(),
        expected["next_i64"].as_str().unwrap(),
        "continuation {label}: {}",
        expected["stream"]
    );
}

fn compare_case(case: &Value) -> Option<SpawnReport> {
    let input = &case["input"];
    let label = input["name"].as_str().unwrap();
    if input["forced"] == "ocelot" {
        return None;
    } // monster category, not vanilla generation CREATURE
    let world = World::new(input);
    let mut env = Environment::new(input);
    let mut random = source(input);
    let chunk = serde_json::from_value(input["chunk"].clone()).unwrap();
    let report = spawn_mobs_for_chunk_generation(
        &world,
        &mut env,
        &settings(input),
        chunk,
        &mut random,
        true,
    )
    .unwrap_or_else(|failure| panic!("{label}: {failure}"));
    let expected = case["entities"].as_array().unwrap();
    assert_eq!(report.mobs.len(), expected.len(), "spawn count {label}");
    for (i, (actual, expected)) in report.mobs.iter().zip(expected).enumerate() {
        let wanted = canonical(SpawnTag::from_native_json(&expected["typed_nbt"]).unwrap());
        assert_eq!(
            canonical(actual.metadata().clone()),
            wanted,
            "typed entity {label} #{i}"
        );
        assert_eq!(
            actual.head_yaw.to_bits(),
            (expected["head_yaw"].as_f64().unwrap() as f32).to_bits(),
            "head yaw {label} #{i}"
        );
        assert_eq!(
            actual.position,
            serde_json::from_value::<[f64; 3]>(expected["position"].clone()).unwrap()
        );
        if let Some(sensors) = expected.get("sensors") {
            assert_eq!(
                json!(actual.sensors()),
                *sensors,
                "constructor sensors {label} #{i}"
            );
        }
        if let Some(baby) = expected["is_baby"].as_bool() {
            assert_eq!(actual.is_baby(), baby, "native isBaby {label} #{i}");
        }
    }
    let rngs = case["rng"].as_array().unwrap();
    assert_eq!(
        report.mobs.len() + report.rejected_entity_draws.len(),
        rngs.len() - 2,
        "every actual native constructor is accounted for in {label}"
    );
    rng_matches(&random, &rngs[0], label);
    rng_matches(&env.random, &rngs[1], label);
    let by_seed: BTreeMap<_, _> = report
        .mobs
        .iter()
        .map(|m| (m.entropy_seed, m.random()))
        .collect();
    let initial_seed = input["entity_seed"]
        .as_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    for (i, native) in rngs[2..].iter().enumerate() {
        let seed = initial_seed.wrapping_add(i as i64);
        if let Some(random) = by_seed.get(&seed) {
            rng_matches(random, native, label);
        } else {
            let (_, draws, next) = report
                .rejected_entity_draws
                .iter()
                .find(|(s, _, _)| *s == seed)
                .expect("created and rejected entropy stream");
            assert_eq!(
                json!(draws
                    .iter()
                    .map(SpawnDraw::native_trace)
                    .collect::<Vec<_>>()),
                native["draws"],
                "rejected entropy {label}"
            );
            assert_eq!(next.to_string(), native["next_i64"].as_str().unwrap());
        }
    }
    let native_positions: Vec<Pos> = case["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| {
            e.get("top").map(|p| {
                let [x, y, z]: [i32; 3] = serde_json::from_value(p.clone()).unwrap();
                (x, y, z)
            })
        })
        .collect();
    assert_eq!(
        report.attempts.iter().map(|a| a.top).collect::<Vec<_>>(),
        native_positions,
        "individual/group retries {label}"
    );
    assert_eq!(
        env.difficulty_calls.get(),
        report.mobs.len(),
        "difficulty callback {label}"
    );
    let finals: Vec<_> = case["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event.get("finalize_exit").is_some())
        .collect();
    assert_eq!(
        finals.len(),
        report.mobs.len(),
        "actual native finalizers {label}"
    );
    let mut finalized = 0;
    for (group_index, group) in report.groups.iter().enumerate() {
        let spawned = report
            .attempts
            .iter()
            .filter(|attempt| {
                attempt.group == group_index
                    && matches!(attempt.outcome, SpawnAttemptOutcome::Spawned { .. })
            })
            .count();
        if let Some(data) = &group.final_data {
            assert!(spawned > 0);
            let native = &finals[finalized + spawned - 1]["group"];
            assert_eq!(json!(data.size), native["size"], "group size {label}");
            assert_eq!(
                json!(data.spawn_babies),
                native["babies"],
                "group babies {label}"
            );
            assert_eq!(
                data.baby_chance.to_bits(),
                (native["baby_chance"].as_f64().unwrap() as f32).to_bits(),
                "group chance {label}"
            );
        } else {
            assert_eq!(spawned, 0);
        }
        finalized += spawned;
    }
    Some(report)
}

#[test]
fn native_generation_spawn_typed_entities_positions_groups_and_rng() {
    let mut complete = 0;
    let mut mobs = 0;
    for case in fixtures()["cases"].as_array().unwrap() {
        if let Some(report) = compare_case(case) {
            complete += 1;
            mobs += report.mobs.len();
        }
    }
    assert!(
        complete == fixtures()["cases"].as_array().unwrap().len() - 1,
        "all native CREATURE executions must complete: {complete}"
    );
    assert!(mobs >= 300, "actual finalized native comparisons: {mobs}");
}

#[test]
fn remaining_native_finalizers_execute_with_clock_memory_sensor_and_group_evidence() {
    let mut counts = BTreeMap::<MobKind, usize>::new();
    let mut baby_goats = 0;
    let mut screaming_goats = 0;
    let mut negative_age_frogs = 0;
    let mut missing_left_horns = 0;
    let mut missing_right_horns = 0;
    let cases = [expanded_fixtures(), callback_fixtures()];
    for case in cases
        .iter()
        .flat_map(|data| data["cases"].as_array().unwrap())
    {
        let Some(report) = compare_case(case) else {
            continue;
        };
        for mob in report.mobs {
            *counts.entry(mob.kind).or_default() += 1;
            if mob.kind == MobKind::Goat && mob.is_baby() {
                baby_goats += 1;
                assert!(mob.data()["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| { a["id"] == "minecraft:attack_damage" && a["base"] == 1.0 }));
            }
            if let MobProperties::Goat {
                screaming,
                has_left_horn,
                has_right_horn,
                ..
            } = mob.properties
            {
                screaming_goats += usize::from(screaming);
                missing_left_horns += usize::from(!has_left_horn);
                missing_right_horns += usize::from(!has_right_horn);
            }
            if mob.kind == MobKind::Frog && mob.age < 0 {
                negative_age_frogs += 1;
                assert!(!mob.is_baby(), "native Frog.isBaby is always false");
            }
        }
    }
    for kind in [
        MobKind::Armadillo,
        MobKind::Camel,
        MobKind::Frog,
        MobKind::Goat,
    ] {
        assert!(
            counts[&kind] >= 15,
            "executed finalizers for {kind:?}: {}",
            counts[&kind]
        );
    }
    assert!(baby_goats > 0, "native baby goat coverage");
    assert!(screaming_goats > 0, "native screaming goat coverage");
    assert!(
        negative_age_frogs > 0,
        "native negative-Age adult frog coverage"
    );
    assert!(
        missing_left_horns > 0 && missing_right_horns > 0,
        "both native horn choices"
    );
}

fn named(name: &str) -> &'static Value {
    fixtures()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(expanded_fixtures()["cases"].as_array().unwrap().iter())
        .find(|case| case["input"]["name"] == name)
        .unwrap()
}

#[test]
fn native_rejections_do_not_consume_success_jitter_or_entity_entropy() {
    for name in [
        "reject-collision",
        "light-0",
        "light-8",
        "terrain-low_ceiling",
        "terrain-lateral_collision",
    ] {
        let report = compare_case(named(name)).unwrap();
        assert!(report.mobs.is_empty() && report.rejected_entity_draws.is_empty());
        for attempts in report.attempts.chunks(4) {
            assert_eq!(attempts.len(), 4);
            assert!(attempts.iter().all(|a| a.top == attempts[0].top));
        }
    }
    let obstructed = compare_case(named("reject-obstructed")).unwrap();
    assert!(obstructed.mobs.is_empty());
    assert_eq!(
        obstructed.rejected_entity_draws.len(),
        obstructed.attempts.len()
    );
    assert!(obstructed.attempts.windows(2).any(|a| a[0].top != a[1].top));
}

#[test]
fn native_callback_rejections_preserve_constructor_rng_and_never_finalize() {
    let data: Value = serde_json::from_str(include_str!(
        "../data/generation_spawn_callback_boundaries_26_1_v1.json"
    ))
    .unwrap();
    assert_eq!(data["cases"].as_array().unwrap().len(), 10);
    for case in data["cases"].as_array().unwrap() {
        let label = case["input"]["name"].as_str().unwrap();
        let report = compare_case(case).unwrap();
        assert!(
            report.mobs.is_empty() && !report.attempts.is_empty(),
            "executed rejection {label}"
        );
        assert!(report.groups.iter().all(|group| group.final_data.is_none()));
        let expected = if label.ends_with("-collision") {
            assert!(report.rejected_entity_draws.is_empty());
            SpawnAttemptOutcome::CollisionRejected
        } else {
            assert_eq!(report.rejected_entity_draws.len(), report.attempts.len());
            // Every rejected brain constructor must still consume its sensor draws.
            assert!(report
                .rejected_entity_draws
                .iter()
                .all(|(_, draws, _)| draws.len() >= 7));
            if label.contains("-mob-rule-") {
                SpawnAttemptOutcome::MobRuleRejected
            } else {
                SpawnAttemptOutcome::Obstructed
            }
        };
        assert!(
            report
                .attempts
                .iter()
                .all(|attempt| attempt.outcome == expected),
            "callback phase {label}"
        );
        assert!(
            case["rng"][1]["draws"].as_array().unwrap().is_empty(),
            "finalization RNG stays untouched {label}"
        );
    }
}

#[test]
fn native_height_border_and_partial_block_cases_execute() {
    for name in [
        "terrain-steps",
        "terrain-water",
        "terrain-snow",
        "terrain-fence",
        "terrain-void",
        "reject-border",
        "height--64",
        "height-65",
        "height-66",
        "height-67",
        "height-318",
        "height-319",
    ] {
        compare_case(named(name)).unwrap();
    }
}

#[test]
fn constructor_entropy_is_required_and_never_derived_from_world_seed() {
    let input = &named("forced-chicken")["input"];
    let world = World::new(input);
    let mut env = Environment::new(input);
    env.entropy_missing = true;
    let failure = spawn_mobs_for_chunk_generation(
        &world,
        &mut env,
        &settings(input),
        [0, 0],
        &mut source(input),
        true,
    )
    .unwrap_err();
    assert!(matches!(failure.error, SpawnError::MissingData(_)));
    assert!(failure.partial.mobs.is_empty());
    assert_eq!(env.difficulty_calls.get(), 0);
}

#[test]
fn camel_requires_the_actual_game_clock_before_it_can_become_a_spawned_entity() {
    struct Clockless(Environment);
    impl SpawnEnvironment for Clockless {
        fn spawn_mobs(&self) -> bool {
            self.0.spawn_mobs()
        }
        fn random(&mut self) -> &mut SpawnRandom {
            self.0.random()
        }
        fn entity_seed(&mut self, kind: MobKind) -> SpawnResult<i64> {
            self.0.entity_seed(kind)
        }
        fn raw_brightness(
            &self,
            world: &dyn FeatureWorld,
            pos: Pos,
            darken: i32,
        ) -> SpawnResult<i32> {
            self.0.raw_brightness(world, pos, darken)
        }
        fn current_difficulty_at(&self, pos: Pos) -> SpawnResult<SpawnDifficulty> {
            self.0.current_difficulty_at(pos)
        }
    }
    let input = &named("forced-camel")["input"];
    let mut env = Clockless(Environment::new(input));
    let failure = spawn_mobs_for_chunk_generation(
        &World::new(input),
        &mut env,
        &settings(input),
        [0, 0],
        &mut source(input),
        true,
    )
    .unwrap_err();
    assert_eq!(
        failure.error,
        SpawnError::MissingData("ServerLevel game time".into())
    );
    assert!(failure.partial.mobs.is_empty());
    assert!(failure
        .partial
        .groups
        .iter()
        .all(|group| group.final_data.is_none()));
    assert_eq!(env.0.difficulty_calls.get(), 1);
    assert_eq!(
        env.0.next_entity, 1001,
        "factory ran but is not a spawned entity"
    );
}

#[test]
fn missing_generation_reads_are_not_air_or_rejected_mobs() {
    let input = &named("forced-sheep")["input"];
    let world = World::new(input);
    world.missing.set(true);
    let failure = spawn_mobs_for_chunk_generation(
        &world,
        &mut Environment::new(input),
        &settings(input),
        [0, 0],
        &mut source(input),
        true,
    )
    .unwrap_err();
    assert!(matches!(failure.error, SpawnError::MissingData(_)));
    assert!(failure.partial.mobs.is_empty());
}

#[test]
fn entry_gates_and_biome_query_follow_native_call_order() {
    let input = &named("plains-0")["input"];
    let world = World::new(input);
    let mut env = Environment::new(input);
    for options in [
        SpawnOptions {
            upgrading_chunk: true,
            ..SpawnOptions::default()
        },
        SpawnOptions {
            disable_mob_generation: true,
            ..SpawnOptions::default()
        },
    ] {
        let report = spawn_original_mobs(&world, &mut env, 0, [-4, 7], options).unwrap();
        assert!(report.skipped.is_some() && report.placement_next_long.is_none());
        assert!(world.biome_reads.borrow().is_empty());
    }
    env.enabled = false;
    let report = spawn_original_mobs(
        &world,
        &mut env,
        0,
        [-4, 7],
        SpawnOptions {
            trace: true,
            ..SpawnOptions::default()
        },
    )
    .unwrap();
    assert_eq!(world.biome_reads.borrow().as_slice(), &[(-64, 319, 112)]);
    assert_eq!(report.skipped, Some(SpawnSkip::SpawnMobsGameRule));
    assert!(report.placement_draws.is_empty() && report.mobs.is_empty());
}

#[test]
fn unsupported_entity_callbacks_are_explicit_incomplete_executions() {
    let mut input = named("forced-sheep")["input"].clone();
    input["forced"] = json!("strider");
    assert!(!MobKind::Strider.supports_finalization());
    let failure = spawn_mobs_for_chunk_generation(
        &World::new(&input),
        &mut Environment::new(&input),
        &settings(&input),
        [0, 0],
        &mut source(&input),
        true,
    )
    .unwrap_err();
    assert!(matches!(failure.error, SpawnError::Unsupported(_)));
    assert!(failure.partial.mobs.is_empty());
}

#[test]
fn generation_catalog_retains_native_biome_order_and_duplicate_groups() {
    for native in fixtures()["catalog"]["biomes"].as_array().unwrap() {
        let id = biome::id(native["name"].as_str().unwrap()).unwrap();
        let actual = biome_spawns(id).unwrap();
        assert_eq!(
            actual.probability.to_bits(),
            (native["probability"].as_f64().unwrap() as f32).to_bits()
        );
        let groups: Vec<_> = actual.groups.iter().map(|g| json!({"type": g.kind.name(), "weight": g.weight, "min": g.min_count, "max": g.max_count})).collect();
        assert_eq!(json!(groups), native["spawns"]);
    }
    let bamboo = biome_spawns(biome::id("bamboo_jungle").unwrap()).unwrap();
    assert!(
        bamboo
            .groups
            .iter()
            .filter(|g| g.kind == MobKind::Chicken)
            .count()
            > 1
    );
}

#[test]
fn native_position_dependent_collision_shapes_match_at_world_boundaries() {
    let assets: Value =
        serde_json::from_str(include_str!("../data/generation_spawn_assets_26_1_v2.json")).unwrap();
    let input = &named("plains-0")["input"];
    let world = World::new(input);
    let env = Environment::new(input);
    let samples = assets["blocks"]["shape_samples"].as_array().unwrap();
    assert!(!samples.is_empty());
    for sample in samples {
        let state = sample["state"].as_u64().unwrap() as u32;
        let [x, y, z]: [i32; 3] = serde_json::from_value(sample["pos"].clone()).unwrap();
        let actual = env.collision_boxes(&world, (x, y, z), state).unwrap();
        let expected: Vec<[f64; 6]> = serde_json::from_value(sample["boxes"].clone()).unwrap();
        let actual: Vec<_> = actual
            .iter()
            .map(|b| [b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]])
            .collect();
        assert_eq!(actual, expected, "collision state {state} at {x},{y},{z}");
    }
}
