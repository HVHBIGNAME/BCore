//! Native SPAWN requests through a real ServerChunkCache and NoiseBased generator.
//! Environment query answers are observed native inputs, not a terrain substitute.
use std::collections::BTreeMap;

use bcore_worldgen::biome;
use bcore_worldgen::feature_world::{FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::spawn::*;
use bcore_worldgen::tick_request::TickRequest;
use serde_json::Value;

struct NativeWorld<'a> {
    case: &'a Value,
}

fn pos_key(pos: Pos) -> String {
    format!("{:?}", [pos.0, pos.1, pos.2])
}

impl OreWorld for NativeWorld<'_> {
    fn get_block(&self, pos: Pos) -> Option<u32> {
        self.case["reads"][format!("block:{}", pos_key(pos))]
            .as_u64()
            .map(|v| v as u32)
    }
    fn ocean_floor_wg(&self, _: i32, _: i32) -> i32 {
        panic!("SPAWN uses final heightmaps")
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("no block mutation")
    }
}

impl FeatureWorld for NativeWorld<'_> {
    fn feature_biome(&self, pos: Pos) -> u32 {
        let sample: [i32; 3] =
            serde_json::from_value(self.case["input"]["entry_biome_pos"].clone()).unwrap();
        let name = if [pos.0, pos.1, pos.2] == sample {
            &self.case["input"]["biome"]
        } else {
            &self.case["reads"][format!("biome:{}", pos_key(pos))]
        };
        biome::id(
            name.as_str()
                .unwrap_or_else(|| panic!("unobserved native biome {pos:?}")),
        )
        .unwrap()
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        let kind = match kind {
            FeatureHeightmap::MotionBlocking => "MOTION_BLOCKING",
            FeatureHeightmap::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
            _ => panic!("native spawn heightmap"),
        };
        self.case["reads"][format!("height:{kind}:{x}:{z}")]
            .as_i64()
            .expect("observed native height") as i32
    }
    fn can_write_feature(&self, _: Pos) -> bool {
        panic!("no writes")
    }
    fn set_feature_block(&mut self, _: Pos, _: u32, _: i32) -> bool {
        panic!("no writes")
    }
    fn mark_feature_postprocessing(&mut self, _: Pos) {
        panic!("no postprocessing")
    }
    fn schedule_feature_tick(&mut self, _: TickRequest) -> bool {
        panic!("no gameplay ticks")
    }
}

struct NativeEnvironment<'a> {
    case: &'a Value,
    random: SpawnRandom,
    next_seed: i64,
    box_reads: BTreeMap<String, Vec<(SpawnBox, bool)>>,
}

impl<'a> NativeEnvironment<'a> {
    fn new(case: &'a Value) -> Self {
        let seed: i64 = case["input"]["seed"].as_str().unwrap().parse().unwrap();
        let chunk: [i32; 2] = serde_json::from_value(case["input"]["chunk"].clone()).unwrap();
        let mut box_reads = BTreeMap::<String, Vec<(SpawnBox, bool)>>::new();
        for (key, value) in case["reads"].as_object().unwrap() {
            for kind in ["collision", "liquid", "unobstructed"] {
                if let Some(bounds) = key.strip_prefix(&format!("{kind}:")) {
                    let b: [f64; 6] = serde_json::from_str(bounds).unwrap();
                    box_reads.entry(kind.into()).or_default().push((
                        SpawnBox {
                            min: [b[0], b[1], b[2]],
                            max: [b[3], b[4], b[5]],
                        },
                        value.as_bool().unwrap(),
                    ));
                }
            }
        }
        Self {
            case,
            random: SpawnRandom::for_region(seed, chunk).trace(),
            next_seed: case["input"]["entity_seed"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            box_reads,
        }
    }
    fn box_read(&self, name: &str, bounds: SpawnBox) -> SpawnResult<bool> {
        self.box_reads
            .get(name)
            .and_then(|rows| rows.iter().find(|(b, _)| *b == bounds))
            .map(|(_, v)| *v)
            .ok_or_else(|| {
                SpawnError::MissingData(format!("unobserved native {name} at {bounds:?}"))
            })
    }
}

impl SpawnEnvironment for NativeEnvironment<'_> {
    fn spawn_mobs(&self) -> bool {
        true
    }
    fn random(&mut self) -> &mut SpawnRandom {
        &mut self.random
    }
    fn entity_seed(&mut self, _: MobKind) -> SpawnResult<i64> {
        let seed = self.next_seed;
        self.next_seed = seed.wrapping_add(1);
        Ok(seed)
    }
    fn raw_brightness(&self, _: &dyn FeatureWorld, pos: Pos, sky_darken: i32) -> SpawnResult<i32> {
        self.case["reads"][format!("brightness:{}:{sky_darken}", pos_key(pos))]
            .as_i64()
            .map(|v| v as i32)
            .ok_or_else(|| SpawnError::MissingData(format!("unobserved native brightness {pos:?}")))
    }
    fn pathfinding_cost_from_light(&self, _: &dyn FeatureWorld, pos: Pos) -> SpawnResult<f32> {
        self.case["reads"][format!("path_cost:{}", pos_key(pos))]
            .as_f64()
            .map(|v| v as f32)
            .ok_or_else(|| SpawnError::MissingData(format!("unobserved native path cost {pos:?}")))
    }
    fn current_difficulty_at(&self, pos: Pos) -> SpawnResult<SpawnDifficulty> {
        let value = &self.case["reads"][format!("difficulty:{}", pos_key(pos))];
        let difficulty = value["difficulty"]
            .as_u64()
            .ok_or_else(|| SpawnError::MissingData("native difficulty callback".into()))?
            as u8;
        // These pre-first-tick captures have zero clock/inhabited time. The
        // supported finalizers do not read the DifficultyInstance payload.
        Ok(SpawnDifficulty {
            difficulty,
            overworld_time: 0,
            inhabited_time: 0,
            moon_brightness: 1.0,
        })
    }
    fn game_time(&self) -> SpawnResult<i64> {
        Ok(0) // Native entry fixtures are captured before the first gameplay tick.
    }
    fn no_collision(&self, _: &dyn FeatureWorld, bounds: SpawnBox) -> SpawnResult<bool> {
        self.box_read("collision", bounds)
    }
    fn contains_any_liquid(&self, _: &dyn FeatureWorld, bounds: SpawnBox) -> SpawnResult<bool> {
        self.box_read("liquid", bounds)
    }
    fn is_unobstructed(&self, _: MobKind, bounds: SpawnBox) -> SpawnResult<bool> {
        self.box_read("unobstructed", bounds)
    }
}

fn canonical(mut value: SpawnTag) -> SpawnTag {
    if let SpawnTag::Compound(fields) = &mut value {
        if let Some(SpawnTag::List(attributes)) = fields.get_mut("attributes") {
            attributes.sort_by_key(|v| v.data()["id"].as_str().unwrap().to_owned());
        }
    }
    value
}

fn compare_entry(data: &Value) -> usize {
    assert_eq!(data["gameplay_ticks"], 0);
    let mut spawned = 0;
    for case in data["cases"].as_array().unwrap() {
        let world = NativeWorld { case };
        let mut env = NativeEnvironment::new(case);
        let seed: i64 = case["input"]["seed"].as_str().unwrap().parse().unwrap();
        let chunk = serde_json::from_value(case["input"]["chunk"].clone()).unwrap();
        let report = spawn_original_mobs(
            &world,
            &mut env,
            seed,
            chunk,
            SpawnOptions {
                trace: true,
                ..SpawnOptions::default()
            },
        )
        .unwrap_or_else(|error| panic!("{}: {error}", case["input"]["name"]));
        let expected = case["entities"].as_array().unwrap();
        assert_eq!(
            report.mobs.len(),
            expected.len(),
            "native entry {}",
            case["input"]["name"]
        );
        spawned += report.mobs.len();
        for (mob, native) in report.mobs.iter().zip(expected) {
            assert_eq!(
                canonical(mob.metadata().clone()),
                canonical(SpawnTag::from_native_json(&native["typed_nbt"]).unwrap()),
                "{}",
                case["input"]["name"]
            );
            assert_eq!(
                mob.head_yaw.to_bits(),
                (native["head_yaw"].as_f64().unwrap() as f32).to_bits()
            );
            if let Some(sensors) = native.get("sensors") {
                assert_eq!(serde_json::json!(mob.sensors()), *sensors);
            }
        }
        for (draws, next, native) in [
            (
                report.placement_draws.as_slice(),
                report.placement_next_long.unwrap(),
                &case["rng"][0],
            ),
            (
                env.random.draws(),
                env.random.peek_next_long(),
                &case["rng"][1],
            ),
        ] {
            assert_eq!(
                serde_json::json!(draws
                    .iter()
                    .map(SpawnDraw::native_trace)
                    .collect::<Vec<_>>()),
                native["draws"],
                "stream {} at {}",
                native["stream"],
                case["input"]["name"]
            );
            assert_eq!(next.to_string(), native["next_i64"].as_str().unwrap());
        }
        let entity_rngs = case["rng"].as_array().unwrap().iter().skip(2);
        assert_eq!(
            report.mobs.len() + report.rejected_entity_draws.len(),
            entity_rngs.len()
        );
        let initial_seed: i64 = case["input"]["entity_seed"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        for (index, native) in entity_rngs.enumerate() {
            let seed = initial_seed.wrapping_add(index as i64);
            let (draws, next) =
                if let Some(mob) = report.mobs.iter().find(|m| m.entropy_seed == seed) {
                    (mob.random().draws(), mob.random().peek_next_long())
                } else {
                    let (_, draws, next) = report
                        .rejected_entity_draws
                        .iter()
                        .find(|(s, _, _)| *s == seed)
                        .expect("every native constructor is accounted for");
                    (draws.as_slice(), *next)
                };
            assert_eq!(
                serde_json::json!(draws
                    .iter()
                    .map(SpawnDraw::native_trace)
                    .collect::<Vec<_>>()),
                native["draws"],
                "native entity RNG {} at {}",
                native["stream"],
                case["input"]["name"]
            );
            assert_eq!(next.to_string(), native["next_i64"].as_str().unwrap());
        }
    }
    spawned
}

#[test]
fn real_native_spawn_stage_entry_matches_entities_and_both_world_seeded_streams() {
    let data: Value =
        serde_json::from_str(include_str!("../data/generation_spawn_entry_26_1_v1.json")).unwrap();
    let spawned = compare_entry(&data);
    assert!(
        spawned >= 5,
        "real native SPAWN must have actual successful entities, observed {spawned}"
    );
}

#[test]
fn real_native_seed42_spawn_stage_entry_matches_entities_and_entropy_continuation() {
    let data: Value = serde_json::from_str(include_str!(
        "../data/generation_spawn_entry_seed42_26_1_v1.json"
    ))
    .unwrap();
    assert_eq!(data["cases"].as_array().unwrap().len(), 6);
    assert!(data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .all(|case| case["input"]["seed"] == "42"));
    assert!(
        compare_entry(&data) > 0,
        "seed42 must exercise actual entity creation"
    );
}
