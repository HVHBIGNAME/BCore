//! Differential tests against the pinned engine, including null section layers.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use bcore_core::ChunkPos;
use bcore_worldgen::lighting::{self, GenerationLight};
use bcore_worldgen::{block, MAX_Y, MIN_Y, WORLD_HEIGHT};
use serde_json::Value;

fn reference() -> &'static Value {
    static REFERENCE: OnceLock<Value> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        serde_json::from_str(include_str!("../data/lighting_reference_26_1_v2.json")).unwrap()
    })
}

fn int(value: &Value) -> i32 {
    value.as_i64().unwrap() as i32
}

fn decode(value: &Value) -> Option<Vec<u8>> {
    value.as_str().map(|v| {
        v.as_bytes()
            .chunks_exact(2)
            .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
            .collect()
    })
}

fn verify_snapshot(
    engine: &GenerationLight,
    expected: &Value,
    label: &str,
    errors: &mut Vec<String>,
) {
    let pos = ChunkPos::new(int(&expected["pos"][0]), int(&expected["pos"][1]));
    let actual = engine.chunk_light(pos);
    assert_eq!(actual.min_section_y, int(&expected["min_section_y"]));
    if let Some(sources) = engine.sky_sources(pos) {
        let want: Vec<_> = expected["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(int)
            .collect();
        if sources.lowest_source_y != want {
            let i = sources
                .lowest_source_y
                .iter()
                .zip(&want)
                .position(|(a, b)| a != b)
                .unwrap();
            errors.push(format!(
                "{label}: source height ({},{}): {} expected {}",
                i & 15,
                i >> 4,
                sources.lowest_source_y[i],
                want[i]
            ));
        }
    }
    for (i, section) in actual.sections.iter().enumerate() {
        for (n, (name, got)) in [("sky", &section.sky), ("block", &section.block)]
            .into_iter()
            .enumerate()
        {
            let want = decode(&expected["sections"][i][n]);
            let empty = [section.sky_empty, section.block_empty][n];
            if empty != expected["sections"][i][n + 2].as_bool().unwrap() {
                errors.push(format!(
                    "{label}: {name} section {}, native lazy-zero flag differs",
                    actual.min_section_y + i as i32
                ));
            }
            if *got != want {
                let y = actual.min_section_y + i as i32;
                match (got, want) {
                    (Some(got), Some(want)) => {
                        let (first, _) = got
                            .iter()
                            .zip(&want)
                            .enumerate()
                            .find(|(_, (a, b))| a != b)
                            .unwrap();
                        let count = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                        errors.push(format!("{label}: {name} section {y}, {count} differing bytes; first {first}: got {:02x}, want {:02x}", got[first], want[first]));
                    }
                    _ => errors.push(format!(
                        "{label}: {name} section {y}, layer presence differs"
                    )),
                }
            }
        }
    }
    let mut count = 0;
    for sample in expected["samples"].as_array().unwrap() {
        let p = (
            pos.x * 16 + int(&sample[0]),
            int(&sample[1]),
            pos.z * 16 + int(&sample[2]),
        );
        let got = [engine.sky_brightness(p), engine.block_brightness(p)];
        let want = [int(&sample[3]) as u8, int(&sample[4]) as u8];
        if got != want {
            count += 1;
            if count <= 4 {
                errors.push(format!(
                    "{label}: at {p:?} sky/block {got:?}, want {want:?}"
                ));
            }
        }
    }
    if count != 0 {
        errors.push(format!("{label}: {count} light query mismatches"));
    }
}

fn inputs(fixture: &Value) -> (Vec<ChunkPos>, BTreeMap<(i32, i32), Vec<u32>>) {
    let positions: Vec<_> = fixture["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| ChunkPos::new(int(&p[0]), int(&p[1])))
        .collect();
    let mut blocks: BTreeMap<_, _> = positions
        .iter()
        .map(|p| ((p.x, p.z), vec![0; WORLD_HEIGHT as usize * 256]))
        .collect();
    for b in fixture["boxes"].as_array().unwrap() {
        for y in int(&b[1])..=int(&b[4]) {
            for z in int(&b[2])..=int(&b[5]) {
                for x in int(&b[0])..=int(&b[3]) {
                    if (MIN_Y..=MAX_Y).contains(&y) {
                        blocks.get_mut(&(x >> 4, z >> 4)).unwrap()
                            [((y - MIN_Y) * 256 + (z & 15) * 16 + (x & 15)) as usize] =
                            int(&b[6]) as u32;
                    }
                }
            }
        }
    }
    (positions, blocks)
}

fn verify_case(name: &str) {
    let fixture = reference()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap();
    let errors = verify_lighting_fixture(fixture);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

fn verify_lighting_fixture(fixture: &Value) -> Vec<String> {
    let name = fixture["name"].as_str().unwrap();
    let mut engine =
        GenerationLight::new(MIN_Y, WORLD_HEIGHT, fixture["has_sky"].as_bool().unwrap()).unwrap();
    let (positions, blocks) = inputs(fixture);
    for &p in &positions {
        engine.initialize_chunk(p, &blocks[&(p.x, p.z)]).unwrap();
    }
    let mut errors = Vec::new();
    verify_snapshot(
        &engine,
        &fixture["initialized"],
        &format!("{name}/initialized"),
        &mut errors,
    );
    engine.propagate_chunk(ChunkPos::new(0, 0)).unwrap();
    verify_snapshot(
        &engine,
        &fixture["center_only"],
        &format!("{name}/center_only"),
        &mut errors,
    );
    for &p in &positions {
        engine.propagate_chunk(p).unwrap();
    }
    verify_snapshot(
        &engine,
        &fixture["all_lit"],
        &format!("{name}/all_lit"),
        &mut errors,
    );
    verify_snapshot(
        &engine,
        &fixture["east"],
        &format!("{name}/east"),
        &mut errors,
    );
    errors
}

#[test]
fn native_light_empty_single() {
    verify_case("empty_single");
}
#[test]
fn native_light_empty_neighborhood() {
    verify_case("empty_neighborhood");
}
#[test]
fn native_light_single_border() {
    verify_case("single_border");
}
#[test]
fn native_light_floor_cave() {
    verify_case("floor_cave");
}
#[test]
fn native_light_no_sky() {
    verify_case("no_sky");
}
#[test]
fn native_light_suspended_roof() {
    verify_case("suspended_roof");
}
#[test]
fn native_light_vertical_limits() {
    verify_case("vertical_limits");
}
#[test]
fn native_light_attenuation() {
    verify_case("attenuation");
}
#[test]
fn native_light_shapes() {
    verify_case("shapes");
}
#[test]
fn native_light_all_emissions() {
    verify_case("all_emissions");
}
#[test]
fn native_light_complementary_faces() {
    verify_case("complementary_faces");
}

#[test]
fn native_light_partial_shapes_at_empty_section_boundaries() {
    let reference: Value =
        serde_json::from_str(include_str!("../data/lighting_boundaries_26_1_v2.json")).unwrap();
    let errors: Vec<_> = reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(verify_lighting_fixture)
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn native_light_inactive_columns_and_late_initialization_histories() {
    let reference: Value =
        serde_json::from_str(include_str!("../data/lighting_history_26_1_v2.json")).unwrap();
    let mut errors = Vec::new();
    for case in reference["cases"].as_array().unwrap() {
        let mut engine = GenerationLight::default();
        let (positions, blocks) = inputs(case);
        for &p in &positions {
            engine.register_chunk(p, &blocks[&(p.x, p.z)]).unwrap();
        }
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            if let Some(positions) = step["initialize"].as_array() {
                for p in positions {
                    let p = ChunkPos::new(int(&p[0]), int(&p[1]));
                    engine.initialize_chunk(p, &blocks[&(p.x, p.z)]).unwrap();
                }
            }
            if step["light"].is_array() {
                engine
                    .propagate_chunk(ChunkPos::new(
                        int(&step["light"][0]),
                        int(&step["light"][1]),
                    ))
                    .unwrap();
            }
            for expected in step["snapshots"].as_array().unwrap() {
                verify_snapshot(
                    &engine,
                    expected,
                    &format!("{}/step{index}/{}", case["name"], expected["pos"]),
                    &mut errors,
                );
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn native_region_brightness_before_and_after_initialization() {
    let fixture: Value =
        serde_json::from_str(include_str!("../data/generation_environment_26_1.json")).unwrap();
    let mut light = GenerationLight::default();
    for stage in ["FEATURES", "INITIALIZE_LIGHT", "LIGHT"] {
        if stage == "INITIALIZE_LIGHT" {
            for z in -1..=1 {
                for x in -1..=1 {
                    let mut states = vec![0; WORLD_HEIGHT as usize * 256];
                    if x == 0 && z == 0 {
                        for (x, y, z, name) in [
                            (8, 63, 8, "stone"),
                            (8, 80, 8, "stone"),
                            (9, 64, 8, "glowstone"),
                        ] {
                            states[((y - MIN_Y) * 256 + z * 16 + x) as usize] =
                                bcore_worldgen::block_predicate::catalog()
                                    .default_state(name)
                                    .unwrap();
                        }
                    }
                    light
                        .initialize_chunk(ChunkPos::new(x, z), &states)
                        .unwrap();
                }
            }
        }
        if stage == "LIGHT" {
            for z in -1..=1 {
                for x in -1..=1 {
                    light.propagate_chunk(ChunkPos::new(x, z)).unwrap();
                }
            }
        }
        for row in fixture["brightness"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["stage"] == stage)
        {
            let p = (
                int(&row["pos"][0]),
                int(&row["pos"][1]),
                int(&row["pos"][2]),
            );
            assert_eq!(
                light.sky_brightness(p),
                int(&row["sky"]) as u8,
                "{stage} {p:?} sky"
            );
            assert_eq!(
                light.block_brightness(p),
                int(&row["block"]) as u8,
                "{stage} {p:?} block"
            );
            assert_eq!(
                light.max_local_raw_brightness(p),
                int(&row["raw"]),
                "{stage} {p:?} raw"
            );
        }
    }
}

#[test]
fn unsupported_state_or_mutation_is_rejected_before_light_state_changes() {
    let mut engine = GenerationLight::default();
    let p = ChunkPos::new(-2, -3);
    assert!(engine.propagate_chunk(p).is_err());
    let mut states = vec![block::AIR; WORLD_HEIGHT as usize * 256];
    states[500] = u32::MAX;
    assert!(engine.initialize_chunk(p, &states).is_err());
    assert!(engine.sky_sources(p).is_none());
    states[500] = block::STONE;
    engine.initialize_chunk(p, &states).unwrap();
    let before = engine.chunk_light(p);
    states[500] = block::LAVA;
    assert!(engine.initialize_chunk(p, &states).is_err());
    assert_eq!(engine.chunk_light(p), before);
    assert!(lighting::biome_temperature(u32::MAX, (0, 0, 0), 63).is_err());
}

#[test]
fn native_wire_handoff_keeps_lazy_zero_and_materialized_zero_distinct() {
    let reference: Value = serde_json::from_str(include_str!(
        "../../bcore-protocol/data/chunk_light_wire_26_1.json"
    ))
    .unwrap();
    let (mut lazy, mut materialized_zero) = (0, 0);
    for sample in reference["samples"].as_array().unwrap() {
        let light = lighting::ChunkLight {
            min_section_y: int(&sample["min_section_y"]),
            sections: sample["sections"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| lighting::LightSection {
                    sky: decode(&s["sky"]),
                    block: decode(&s["block"]),
                    sky_empty: s["sky_empty"].as_bool().unwrap(),
                    block_empty: s["block_empty"].as_bool().unwrap(),
                })
                .collect(),
        };
        assert!(
            light.validate(MIN_Y, WORLD_HEIGHT, sample["has_sky"].as_bool().unwrap()),
            "{}",
            sample["name"]
        );
        for s in &light.sections {
            for (v, empty) in [(&s.sky, s.sky_empty), (&s.block, s.block_empty)] {
                if v.as_ref().is_some_and(|v| v.iter().all(|&b| b == 0)) {
                    if empty {
                        lazy += 1;
                    } else {
                        materialized_zero += 1;
                    }
                }
            }
        }
        let roundtrip: lighting::ChunkLight =
            serde_json::from_value(serde_json::to_value(&light).unwrap()).unwrap();
        assert_eq!(light, roundtrip);
    }
    assert!(lazy > 0 && materialized_zero > 0);
    let mut light = GenerationLight::default().chunk_light(ChunkPos::new(0, 0));
    light.sections[0].sky_empty = true;
    assert!(!light.validate(MIN_Y, WORLD_HEIGHT, true));
    light.sections[0].sky = Some(vec![1; 2048]);
    assert!(!light.validate(MIN_Y, WORLD_HEIGHT, true));
    light.sections[0].sky = Some(vec![0; 2048]);
    assert!(light.validate(MIN_Y, WORLD_HEIGHT, true));
}

#[test]
fn native_light_ordered_proto_mutations_repropagate_and_retain_allocation_flags() {
    let reference: Value =
        serde_json::from_str(include_str!("../data/lighting_updates_26_1.json")).unwrap();
    assert_eq!(
        reference["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    let mut errors = Vec::new();
    for case in reference["cases"].as_array().unwrap() {
        let mut engine =
            GenerationLight::new(MIN_Y, WORLD_HEIGHT, case["has_sky"].as_bool().unwrap()).unwrap();
        let (positions, blocks) = inputs(case);
        for p in positions {
            engine.initialize_chunk(p, &blocks[&(p.x, p.z)]).unwrap();
        }
        for p in case["enabled"].as_array().unwrap() {
            engine
                .propagate_chunk(ChunkPos::new(int(&p[0]), int(&p[1])))
                .unwrap();
        }
        for expected in case["before"].as_array().unwrap() {
            verify_snapshot(
                &engine,
                expected,
                &format!("{}/before/{}", case["name"], expected["pos"]),
                &mut errors,
            );
        }
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            if let Some(edits) = step["edits"].as_array() {
                for check in step["checks"].as_array().unwrap() {
                    assert_eq!(
                        lighting::has_different_light_properties(
                            int(&check["before"]) as u32,
                            int(&check["after"]) as u32
                        )
                        .unwrap(),
                        check["check"].as_bool().unwrap()
                    );
                }
                let edits: Vec<_> = edits
                    .iter()
                    .map(|p| ((int(&p[0]), int(&p[1]), int(&p[2])), int(&p[3]) as u32))
                    .collect();
                engine.apply_block_updates(&edits).unwrap();
            }
            if step["light"].is_array() {
                engine
                    .propagate_chunk(ChunkPos::new(
                        int(&step["light"][0]),
                        int(&step["light"][1]),
                    ))
                    .unwrap();
            }
            for expected in step["snapshots"].as_array().unwrap() {
                verify_snapshot(
                    &engine,
                    expected,
                    &format!("{}/step{index}/{}", case["name"], expected["pos"]),
                    &mut errors,
                );
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn invalid_update_batches_leave_blocks_sources_and_light_untouched() {
    let mut engine = GenerationLight::default();
    let p = ChunkPos::new(0, 0);
    let mut states = vec![block::AIR; WORLD_HEIGHT as usize * 256];
    states[0] = block::STONE;
    engine.initialize_chunk(p, &states).unwrap();
    engine.propagate_chunk(p).unwrap();
    let before = engine.chunk_light(p);
    let sources = engine.sky_sources(p).unwrap().clone();
    assert!(engine
        .apply_block_updates(&[((0, MIN_Y, 0), block::LAVA), ((1, MIN_Y, 0), u32::MAX)])
        .is_err());
    assert_eq!(engine.chunk_light(p), before);
    assert_eq!(engine.sky_sources(p), Some(&sources));
    // Re-registering the original input also proves the first write was not applied.
    engine.register_chunk(p, &states).unwrap();
    engine
        .apply_block_updates(&[
            ((0, i32::MIN, 0), block::LAVA),
            ((0, i32::MAX, 0), block::LAVA),
        ])
        .unwrap();
    assert_eq!(engine.chunk_light(p), before);
}
