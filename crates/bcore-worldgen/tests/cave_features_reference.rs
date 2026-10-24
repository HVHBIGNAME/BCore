//! Standalone integration before the scheduler exports the new cave modules.
pub use bcore_worldgen::{
    block, block_predicate, feature_world, mth, sculk, simplex, tick_request, MAX_Y, MIN_Y,
};
#[path = "../src/dripstone.rs"]
mod dripstone;
#[path = "../src/lush_caves.rs"]
mod lush_caves;
#[path = "support/registry_26_1.rs"]
mod registry_26_1;

use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::tick_request::TickRequest;
use feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use serde_json::Value;
use sha2::{Digest, Sha256};
use simplex::WorldgenRandom;
use std::collections::BTreeMap;

struct World {
    historical: bool,
    floor: i32,
    ceiling: i32,
    rock: u32,
    fill: u32,
    fill_top: i32,
    reject: bool,
    deny_origin: bool,
    missing: Option<Pos>,
    initial: BTreeMap<Pos, u32>,
    writes: BTreeMap<Pos, u32>,
    calls: Vec<[i32; 6]>,
    marks: Vec<[i32; 3]>,
    ticks: Vec<TickRequest>,
}

fn pos(v: &Value) -> Pos {
    (
        v[0].as_i64().unwrap() as i32,
        v[1].as_i64().unwrap() as i32,
        v[2].as_i64().unwrap() as i32,
    )
}

impl World {
    fn from_sample(v: &Value, historical: bool) -> Self {
        let input = |state: u32| {
            if historical {
                registry_26_1::target_state(state)
            } else {
                state
            }
        };
        Self {
            historical,
            floor: v["floor"].as_i64().unwrap() as i32,
            ceiling: v["ceiling"].as_i64().unwrap() as i32,
            rock: input(v["rock"].as_u64().unwrap() as u32),
            fill: input(v["fill"].as_u64().unwrap() as u32),
            fill_top: v["fill_top"].as_i64().unwrap() as i32,
            reject: v["reject"].as_bool().unwrap(),
            deny_origin: v["deny_origin"].as_bool().unwrap(),
            missing: None,
            initial: v["initial"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| (pos(row), input(row[3].as_u64().unwrap() as u32)))
                .collect(),
            writes: BTreeMap::new(),
            calls: Vec::new(),
            marks: Vec::new(),
            ticks: Vec::new(),
        }
    }

    fn observed_state(&self, state: u32) -> u32 {
        if self.historical {
            registry_26_1::historical_state(state)
        } else {
            state
        }
    }

    fn assert_snapshot(&self, v: &Value, label: &str) {
        let expected: Vec<[i32; 6]> = serde_json::from_value(v["write_calls"].clone()).unwrap();
        let calls: Vec<_> = self
            .calls
            .iter()
            .copied()
            .map(|mut row| {
                row[3] = self.observed_state(row[3] as u32) as i32;
                row
            })
            .collect();
        if calls != expected {
            let index = calls
                .iter()
                .zip(&expected)
                .position(|(a, b)| a != b)
                .unwrap_or(calls.len().min(expected.len()));
            panic!(
                "{label}: write {index}: actual {:?}, expected {:?}; lengths {} != {}",
                calls.get(index),
                expected.get(index),
                calls.len(),
                expected.len()
            );
        }
        let actual: Vec<[i32; 4]> = self
            .writes
            .iter()
            .map(|(&(x, y, z), &s)| [x, y, z, self.observed_state(s) as i32])
            .collect();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            v["writes"],
            "{label}: final blocks"
        );
        assert_eq!(
            serde_json::to_value(&self.marks).unwrap(),
            v["marks"],
            "{label}: explicit postprocessing"
        );
        assert_eq!(
            self.marks.len() as u64,
            v["native_mark_count"].as_u64().unwrap(),
            "{label}: native ProtoChunk mark count"
        );
        let ticks: Vec<[i32; 6]> = self
            .ticks
            .iter()
            .map(|request| {
                let (target, fluid) = match request.target {
                    tick_request::TickTarget::Block(id) => (self.observed_state(id), 0),
                    tick_request::TickTarget::Fluid(id) => (id, 1),
                };
                let [x, y, z] = request.block_pos;
                [x, y, z, target as i32, request.delay, fluid]
            })
            .collect();
        assert_eq!(
            serde_json::to_value(ticks).unwrap(),
            v["ticks"],
            "{label}: ticks"
        );
        assert!(
            v["implicit_marks"].as_array().unwrap().is_empty(),
            "{label}: implicit marks require a setter adapter"
        );
    }
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.feature_height(FeatureHeightmap::OceanFloorWg, x, z)
    }
    fn get_block(&self, p: Pos) -> Option<u32> {
        if self.missing == Some(p) {
            return None;
        }
        if !(MIN_Y..=MAX_Y).contains(&p.1) {
            return Some(0);
        }
        let background = if p.1 <= self.floor || p.1 >= self.ceiling {
            self.rock
        } else if p.1 <= self.fill_top {
            self.fill
        } else {
            0
        };
        Some(
            *self
                .writes
                .get(&p)
                .or_else(|| self.initial.get(&p))
                .unwrap_or(&background),
        )
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("cave feature must use explicit write flags")
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, _: Pos) -> u32 {
        panic!("configured cave bodies do not query biome")
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        assert!(matches!(
            kind,
            FeatureHeightmap::WorldSurfaceWg | FeatureHeightmap::OceanFloorWg
        ));
        for y in (MIN_Y..=MAX_Y).rev() {
            let state = self.get_block((x, y, z)).unwrap();
            if dripstone::state_info(state).unwrap().flags & 1 == 0 {
                return y + 1;
            }
        }
        MIN_Y
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        !self.deny_origin && (MIN_Y..=MAX_Y).contains(&pos.1)
    }
    fn set_feature_block(&mut self, p: Pos, state: u32, flags: i32) -> bool {
        let accepted = !self.reject && (MIN_Y..=MAX_Y).contains(&p.1);
        self.calls
            .push([p.0, p.1, p.2, state as i32, flags, i32::from(accepted)]);
        if accepted {
            self.writes.insert(p, state);
        }
        accepted
    }
    fn mark_feature_postprocessing(&mut self, p: Pos) {
        self.marks.push([p.0, p.1, p.2]);
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        self.ticks.push(request);
        true
    }
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("../data/cave_features_26_1.json")).unwrap()
}

fn check_sample(sample: &Value, historical: bool) {
    let kind = sample["document"]["type"].as_str().unwrap();
    let label = format!(
        "{} / {} / {}",
        sample["name"], sample["scenario"], sample["seed"]
    );
    let mut world = World::from_sample(sample, historical);
    let seed = sample["seed"].as_i64().unwrap();
    let mut rng = WorldgenRandom::new(seed);
    let mut native_source = simplex::Xoroshiro128::new(seed);
    let mut native_draws = 0;
    let mut state = dripstone::CaveRandomState::default();
    if sample["scenario"].as_str().unwrap().contains("advanced") {
        rng.next_int(17);
        rng.next_float();
        rng.next_int(1073741825);
    }
    for (index, result) in sample["results"].as_array().unwrap().iter().enumerate() {
        if sample["scenario"] == "reseed_repeat" && index > 0 {
            rng.set_seed(seed + index as i64);
            native_source = simplex::Xoroshiro128::new(seed + index as i64);
        }
        let actual = if matches!(
            kind,
            "minecraft:speleothem"
                | "minecraft:speleothem_cluster"
                | "minecraft:pointed_dripstone"
                | "minecraft:large_dripstone"
                | "minecraft:dripstone_cluster"
        ) {
            dripstone::place_configured(
                kind,
                &sample["document"]["config"],
                &mut world,
                &mut rng,
                pos(&sample["origin"]),
                &mut state,
            )
        } else {
            lush_caves::place_configured(
                kind,
                &sample["document"]["config"],
                &mut world,
                &mut rng,
                pos(&sample["origin"]),
                &mut state,
            )
        }
        .unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(actual, result.as_bool().unwrap(), "{label}: return {index}");
        let count = sample["draw_counts"][index].as_u64().unwrap();
        for _ in native_draws..count {
            native_source.next_long();
        }
        native_draws = count;
        let mut expected = native_source.clone();
        let mut observed = rng.source.clone();
        for _ in 0..2 {
            assert_eq!(
                observed.next_long(),
                expected.next_long(),
                "{label}: exact native draw count after attempt {index}"
            );
        }
    }
    assert_eq!(
        native_draws,
        sample["rng_count"].as_u64().unwrap(),
        "{label}: total draw count"
    );
    world.assert_snapshot(sample, &label);
    assert_eq!(
        rng.next_long(),
        sample["next_i64"].as_i64().unwrap(),
        "{label}: RNG continuation"
    );
}

#[test]
fn native_dripstone_features() {
    let data = fixture();
    let mut cases = 0;
    for sample in data["samples"].as_array().unwrap() {
        let kind = sample["document"]["type"].as_str().unwrap();
        if !matches!(
            kind,
            "minecraft:pointed_dripstone"
                | "minecraft:large_dripstone"
                | "minecraft:dripstone_cluster"
        ) {
            continue;
        }
        cases += 1;
        check_sample(sample, true);
    }
    assert_eq!(cases, 77);
}

fn target_speleothems() -> Value {
    let source = include_bytes!("../data/speleothem_26_2.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(source)),
        "1e3e0873f05f865ed4a069798eafb9b3eefc25d22fafb7a424dd5734ca126ffb"
    );
    let data: Value = serde_json::from_slice(source).unwrap();
    assert_eq!(data["minecraft"], "26.2");
    assert_eq!(data["protocol"], 776);
    assert_eq!(data["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    data
}

#[test]
fn native_26_2_speleothems_preserve_materials_growth_order_flags_and_rng() {
    let data = target_speleothems();
    let samples = data["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 102);
    assert_eq!(data["configs"].as_object().unwrap().len(), 4);
    for sample in samples {
        check_sample(sample, false);
    }
}

#[test]
fn native_26_2_speleothem_codec_admission_preserves_numeric_coercion_and_bounds() {
    let data = target_speleothems();
    let rows = data["admission"].as_array().unwrap();
    assert_eq!(rows.len(), 18);
    for row in rows {
        let document = &row["document"];
        let kind = document["type"]
            .as_str()
            .unwrap()
            .strip_prefix("minecraft:")
            .unwrap();
        let admitted = dripstone::check_speleothem(kind, &document["config"]).is_ok();
        assert_eq!(
            admitted,
            row["admitted"].as_bool().unwrap(),
            "{}",
            row["name"]
        );
        if admitted {
            let canonical = &row["canonical_document"];
            dripstone::check_speleothem(kind, &canonical["config"]).unwrap();
        }
    }
}

#[test]
fn unavailable_reads_and_unknown_configs_fail_explicitly() {
    let data = fixture();
    let sample = &data["samples"][0];
    let mut world = World::from_sample(sample, true);
    let origin = pos(&sample["origin"]);
    world.missing = Some((origin.0, origin.1 + 1, origin.2));
    let mut rng = WorldgenRandom::new(0);
    let mut state = dripstone::CaveRandomState::default();
    assert!(matches!(
        dripstone::place_configured(
            "pointed_dripstone",
            &sample["document"]["config"],
            &mut world,
            &mut rng,
            origin,
            &mut state
        ),
        Err(FeatureError::MissingData(_))
    ));
    assert!(world.calls.is_empty());
    assert!(matches!(
        dripstone::place_configured(
            "unknown",
            &serde_json::json!({}),
            &mut world,
            &mut rng,
            origin,
            &mut state
        ),
        Err(FeatureError::Unsupported(_))
    ));
}

#[test]
fn native_lush_cave_features() {
    let data = fixture();
    let mut cases = 0;
    for sample in data["samples"].as_array().unwrap() {
        let kind = sample["document"]["type"].as_str().unwrap();
        if matches!(
            kind,
            "minecraft:pointed_dripstone"
                | "minecraft:large_dripstone"
                | "minecraft:dripstone_cluster"
        ) {
            continue;
        }
        cases += 1;
        check_sample(sample, true);
    }
    assert_eq!(cases, 167);
}

#[test]
fn external_child_errors_are_not_successful_root_systems() {
    let fixture = fixture();
    let sample = fixture["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["scenario"] == "rooted_stone")
        .unwrap();
    let mut world = World::from_sample(sample, true);
    let mut random = WorldgenRandom::new(17);
    let mut random_state = dripstone::CaveRandomState::default();
    let origin = pos(&sample["origin"]);
    let mut calls = Vec::new();
    let mut child = |placed: &Value,
                     _: &mut World,
                     _: &mut WorldgenRandom,
                     child_origin: Pos,
                     _: &mut dripstone::CaveRandomState| {
        calls.push((placed.clone(), child_origin));
        Err(FeatureError::Unsupported("external child kernel".into()))
    };
    let result = lush_caves::place_configured_with(
        "root_system",
        &sample["document"]["config"],
        &mut world,
        &mut random,
        origin,
        &mut random_state,
        &mut child,
    );
    assert_eq!(
        result,
        Err(FeatureError::Unsupported("external child kernel".into()))
    );
    assert_eq!(
        calls,
        vec![(
            sample["document"]["config"]["feature"].clone(),
            (origin.0, origin.1 + 8, origin.2)
        )]
    );
    assert!(
        world.calls.is_empty(),
        "root replacement must follow child success"
    );
    assert_eq!(random.next_long(), WorldgenRandom::new(17).next_long());
}

#[test]
fn unsupported_lush_provider_and_predicate_do_not_consume_rng() {
    let fixture = fixture();
    let sample = &fixture["samples"][0];
    let mut world = World::from_sample(sample, true);
    let mut random = WorldgenRandom::new(42);
    let mut random_state = dripstone::CaveRandomState::default();
    let origin = pos(&sample["origin"]);
    let mut config = dripstone::configured_feature("cave_vine").unwrap()["config"].clone();
    config["allowed_placement"] = serde_json::json!({"type": "minecraft:unknown_predicate"});
    assert!(matches!(
        lush_caves::place_configured(
            "block_column",
            &config,
            &mut world,
            &mut random,
            origin,
            &mut random_state
        ),
        Err(FeatureError::Unsupported(_))
    ));
    let config = serde_json::json!({"to_place": {"type": "minecraft:unknown_provider"}});
    assert!(matches!(
        lush_caves::place_configured(
            "simple_block",
            &config,
            &mut world,
            &mut random,
            origin,
            &mut random_state
        ),
        Err(FeatureError::Unsupported(_))
    ));
    assert!(world.calls.is_empty());
    assert_eq!(random.next_long(), WorldgenRandom::new(42).next_long());
}

#[test]
fn native_catalog_and_feature_fixtures_have_matching_provenance() {
    let fixture = fixture();
    let catalog: Value =
        serde_json::from_str(include_str!("../data/cave_feature_data_26_1.json")).unwrap();
    for field in ["minecraft", "jar_sha256", "probe_sha256", "configs"] {
        assert_eq!(catalog[field], fixture[field], "native capture {field}");
    }
    assert_eq!(catalog["state_count"], 29_873);
    assert_eq!(fixture["configs"].as_object().unwrap().len(), 17);
    let target: Value =
        serde_json::from_str(include_str!("../data/cave_feature_data_26_2.json")).unwrap();
    assert_eq!(target["minecraft"], "26.2");
    assert_eq!(target["protocol"], 776);
    assert_eq!(target["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    assert_eq!(target["state_count"], bcore_registry::BLOCK_STATE_COUNT);
    assert_eq!(target["configs"].as_object().unwrap().len(), 17);
    for (name, document) in target["configs"].as_object().unwrap() {
        assert_eq!(
            dripstone::configured_feature(name).unwrap(),
            document,
            "runtime catalog {name}"
        );
    }
}

fn native_sculk_shapes() -> Value {
    serde_json::from_str(include_str!("../data/sculk_states_26_1.json")).unwrap()
}

fn shape_block_name(catalog: &Value, state: u32) -> &str {
    catalog["blocks"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, range)| {
            (range[0].as_u64().unwrap()..range[1].as_u64().unwrap()).contains(&u64::from(state))
        })
        .unwrap()
        .0
}

fn lichen_shape_world(support: Pos, state: u32) -> World {
    World {
        historical: true,
        floor: MIN_Y - 1,
        ceiling: MAX_Y + 1,
        rock: 1,
        fill: 0,
        fill_top: MIN_Y - 1,
        reject: false,
        deny_origin: false,
        missing: None,
        initial: BTreeMap::from([(support, registry_26_1::target_state(state))]),
        writes: BTreeMap::new(),
        calls: Vec::new(),
        marks: Vec::new(),
        ticks: Vec::new(),
    }
}

fn lichen_shape_config(direction: usize, supports: &[&str]) -> Value {
    serde_json::json!({
        "block": "minecraft:glow_lichen",
        "can_be_placed_on": supports,
        "can_place_on_floor": direction == 0,
        "can_place_on_ceiling": direction == 1,
        "can_place_on_wall": direction >= 2,
        "chance_of_spreading": 0.0,
        "search_range": 1,
    })
}

#[test]
fn native_multiface_masks_preserve_neighbour_direction_conventions() {
    let native = native_sculk_shapes();
    let cave: Value =
        serde_json::from_str(include_str!("../data/cave_feature_data_26_1.json")).unwrap();
    assert_eq!(native["jar_sha256"], cave["jar_sha256"]);
    assert_eq!(
        native["directions"],
        serde_json::json!(["DOWN", "UP", "NORTH", "SOUTH", "WEST", "EAST"])
    );
    let mut cached = 0;
    for state in 0..native["state_count"].as_u64().unwrap() as u32 {
        let mapped = registry_26_1::target_state(state);
        let cave = dripstone::state_info(mapped).unwrap();
        if cave.attach < 0 {
            continue;
        }
        cached += 1;
        for direction in 0..6 {
            assert_eq!(
                sculk::multiface_can_attach_to(mapped, direction).unwrap(),
                cave.attach & (1 << (direction ^ 1)) != 0,
                "cached state {state}, toward-neighbour direction {direction}"
            );
        }
    }
    assert_eq!(cached, 29_694);

    let observations = native["uncached_shape_cases"].as_array().unwrap();
    let mut states = BTreeMap::new();
    for observation in observations {
        let state = observation[0].as_u64().unwrap() as u32;
        let attachment_mask = observation[2][2].as_u64().unwrap() as u8;
        let mapped = registry_26_1::target_state(state);
        assert_eq!(dripstone::state_info(mapped).unwrap().attach, -1);
        if let Some(previous) = states.insert(state, attachment_mask) {
            assert_eq!(
                previous, attachment_mask,
                "position-dependent state {state}"
            );
        }
        for direction in 0..6 {
            assert_eq!(
                sculk::multiface_can_attach_to(mapped, direction).unwrap(),
                attachment_mask & (1 << direction) != 0,
                "native uncached state {state}, position {}, direction {direction}",
                observation[1]
            );
        }
    }
    assert_eq!(observations.len(), 195);
    assert_eq!(states.len(), 65);
}

#[test]
fn lichen_handles_native_uncached_shapes_at_reported_failure_positions() {
    let native = native_sculk_shapes();
    let observed: BTreeMap<u32, u8> = native["uncached_shape_cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row[0].as_u64().unwrap() as u32,
                row[2][2].as_u64().unwrap() as u8,
            )
        })
        .collect();
    // Independent native cached controls with the same attachment masks. These
    // test return values, writes, marks and current-RNG continuation, not just
    // whether the formerly failing metadata lookup stops throwing.
    let controls: BTreeMap<u8, u32> = [0, 1]
        .into_iter()
        .map(|mask| {
            let state = (0..native["state_count"].as_u64().unwrap() as u32)
                .find(|&state| {
                    let info = dripstone::state_info(registry_26_1::target_state(state)).unwrap();
                    info.attach >= 0
                        && info.flags & 31 == 0
                        && (0..6).all(|direction| {
                            (info.attach & (1 << (direction ^ 1)) != 0)
                                == (mask & (1 << direction) != 0)
                        })
                })
                .unwrap();
            (mask, state)
        })
        .collect();
    let seed = 846692123413862008;
    let mut cases = 0;
    let mut spread_cases = 0;
    for support in [(-22, 78, 28), (-13, 69, 24)] {
        for (&state, &mask) in &observed {
            let control_state = controls[&mask];
            let supports = [
                shape_block_name(&native, state),
                shape_block_name(&native, control_state),
            ];
            for (direction, delta) in dripstone::DIRECTIONS.into_iter().enumerate() {
                let origin = (
                    support.0 - delta.0,
                    support.1 - delta.1,
                    support.2 - delta.2,
                );
                let config = lichen_shape_config(direction, &supports);
                let mut world = lichen_shape_world(support, state);
                let mut control = lichen_shape_world(support, control_state);
                let mut random = WorldgenRandom::new(seed);
                let mut control_random = WorldgenRandom::new(seed);
                for rng in [&mut random, &mut control_random] {
                    rng.next_int(17);
                    rng.next_float();
                }
                let actual = lush_caves::place_glow_lichen(
                    &config,
                    &mut world,
                    &mut random,
                    origin,
                )
                .unwrap_or_else(|error| {
                    panic!("state {state}, support {support:?}, direction {direction}: {error}")
                });
                let expected = lush_caves::place_glow_lichen(
                    &config,
                    &mut control,
                    &mut control_random,
                    origin,
                )
                .unwrap();
                assert_eq!(actual, mask & (1 << direction) != 0);
                assert_eq!(actual, expected);
                assert_eq!(world.calls, control.calls);
                assert_eq!(world.writes, control.writes);
                assert_eq!(world.marks, control.marks);
                assert_eq!(world.ticks, control.ticks);
                assert_eq!(world.calls.len(), usize::from(actual));
                if actual {
                    assert_eq!(world.calls[0][4], 3);
                    assert_eq!(world.marks, [[origin.0, origin.1, origin.2]]);
                }
                assert_eq!(random.next_long(), control_random.next_long());
                cases += 1;
            }

            // The production whitelist excludes these uncached blocks. Its
            // spreader can nevertheless encounter them around an existing face.
            // Block every perpendicular neighbour so the first spread attempt
            // must evaluate an uncached support before wrapping around the stone.
            let origin = (support.0, support.1 - 1, support.2);
            let mut config =
                dripstone::configured_feature("glow_lichen").unwrap()["config"].clone();
            config["chance_of_spreading"] = serde_json::json!(1.0);
            let mut world = lichen_shape_world(support, state);
            let mut control = lichen_shape_world(support, control_state);
            for direction in [0, 1, 2, 3] {
                let p = dripstone::offset(origin, direction);
                world.initial.insert(p, registry_26_1::target_state(state));
                control
                    .initial
                    .insert(p, registry_26_1::target_state(control_state));
            }
            let stone = dripstone::block_id("stone").unwrap();
            world.initial.insert(dripstone::offset(origin, 5), stone);
            control.initial.insert(dripstone::offset(origin, 5), stone);
            let mut random = WorldgenRandom::new(seed);
            let mut control_random = WorldgenRandom::new(seed);
            for rng in [&mut random, &mut control_random] {
                rng.next_int(17);
                rng.next_float();
            }
            assert!(
                lush_caves::place_glow_lichen(&config, &mut world, &mut random, origin)
                    .unwrap_or_else(|error| panic!(
                        "spreading near state {state} at {support:?}: {error}"
                    ))
            );
            assert!(lush_caves::place_glow_lichen(
                &config,
                &mut control,
                &mut control_random,
                origin
            )
            .unwrap());
            assert_eq!(world.calls, control.calls);
            assert_eq!(world.writes, control.writes);
            assert_eq!(world.marks, control.marks);
            assert_eq!(
                world.calls.iter().map(|call| call[4]).collect::<Vec<_>>(),
                [3, 2]
            );
            assert_eq!(world.marks.len(), 2);
            assert!(world.ticks.is_empty() && control.ticks.is_empty());
            assert_eq!(random.next_long(), control_random.next_long());
            spread_cases += 1;
        }
    }
    assert_eq!(cases, 780);
    assert_eq!(spread_cases, 130);
}

#[test]
fn lichen_retains_errors_for_unproven_context_dependent_shapes() {
    let native = native_sculk_shapes();
    let mut checked_states = 0;
    for (name, count) in native["unsupported_shape_blocks"].as_object().unwrap() {
        let range = &native["blocks"][name];
        let start = range[0].as_u64().unwrap() as u32;
        let end = range[1].as_u64().unwrap() as u32;
        assert_eq!(u64::from(end - start), count.as_u64().unwrap());
        for state in start..end {
            for direction in 0..6 {
                assert!(matches!(
                    sculk::multiface_can_attach_to(registry_26_1::target_state(state), direction),
                    Err(FeatureError::Unsupported(_))
                ));
            }
            let mut world = lichen_shape_world((0, 40, 0), state);
            let mut random = WorldgenRandom::new(42);
            assert!(matches!(
                lush_caves::place_glow_lichen(
                    &lichen_shape_config(0, &[name]),
                    &mut world,
                    &mut random,
                    (0, 41, 0)
                ),
                Err(FeatureError::Unsupported(_))
            ));
            assert!(world.calls.is_empty() && world.marks.is_empty() && world.ticks.is_empty());
            assert_eq!(random.next_long(), WorldgenRandom::new(42).next_long());
            checked_states += 1;
        }
    }
    assert_eq!(checked_states, 114);
    assert!(matches!(
        sculk::multiface_can_attach_to(1, 6),
        Err(FeatureError::InvalidConfig(_))
    ));
    assert!(matches!(
        sculk::multiface_can_attach_to(bcore_registry::BLOCK_STATE_COUNT as u32, 0),
        Err(FeatureError::MissingData(_))
    ));
}
