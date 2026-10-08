use std::collections::BTreeMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{place_configured, place_named, GenerationEnvironment, Outcome};
use crate::dripstone::CaveRandomState;
use crate::generation::ChunkStatus;
use crate::region::FeatureRegion;
use crate::simplex::{WorldgenRandom, Xoroshiro128};
use crate::{ChunkPos, WorldGenerator, WorldgenHeightmaps};

fn expand<T>(value: &Value, parse: impl Fn(&Value) -> T) -> Vec<T>
where
    T: Clone,
{
    value
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|run| std::iter::repeat_n(parse(&run[1]), run[0].as_u64().unwrap() as usize))
        .collect()
}

fn position(value: &Value) -> ChunkPos {
    ChunkPos::new(
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
    )
}

fn random(rng: &Value) -> WorldgenRandom {
    let mut random = WorldgenRandom::new(0);
    random.source = Xoroshiro128::from_state(
        rng["seed_lo"].as_i64().unwrap() as u64,
        rng["seed_hi"].as_i64().unwrap() as u64,
    );
    random
}

#[test]
fn native_fossil_histories_match_region_blocks_writes_rng_and_retained_maps() {
    let data: Value =
        serde_json::from_str(include_str!("../../data/fossil_history_26_1.json")).unwrap();
    assert_eq!(
        data["jar_sha256"],
        crate::structure::template_pool::StructureAssets::JAR_SHA256
    );
    let seed = data["seed"].as_str().unwrap().parse().unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    let mut successes = 0;
    let mut total_writes = 0;
    for case in cases {
        let label = format!("{} at {}", case["feature"], case["source"]);
        let source = position(&case["source"]);
        let mut region = FeatureRegion::shared(WorldGenerator::new(seed));
        let mut available = BTreeMap::new();
        for key in case["before"].as_array().unwrap() {
            let expected = &data["snapshots"][key.as_str().unwrap()];
            let pos = position(&expected["pos"]);
            available.insert((pos.x, pos.z), ChunkStatus::Carvers);
            let chunk = region.owned_chunk_mut(pos);
            chunk.states = expand(&expected["states"], |v| v.as_u64().unwrap() as u32);
            assert_eq!(chunk.states.len(), 98304);
            chunk.noise_biomes = Some(expand(&expected["biomes"], |v| {
                crate::biome::id(v.as_str().unwrap()).unwrap()
            }));
            assert_eq!(chunk.noise_biomes.as_ref().unwrap().len(), 1536);
            chunk.worldgen_heightmaps = Some(
                serde_json::from_value::<WorldgenHeightmaps>(json!({
                    "world_surface": expected["world_surface_wg"],
                    "ocean_floor": expected["ocean_floor_wg"],
                    "frozen": true
                }))
                .unwrap(),
            );
            chunk.postprocessing =
                serde_json::from_value(expected["postprocessing"].clone()).unwrap();
        }
        assert_eq!(available.len(), 9);
        region.begin_source(source, ChunkStatus::Features, available);
        let entry_rng = &case["entry"]["rng"];
        assert_eq!(entry_rng["gaussian"]["has_next"], false);
        assert_eq!(
            random(entry_rng).next_long(),
            entry_rng["next_i64_from_copy"].as_i64().unwrap()
        );
        let mut rng = random(entry_rng);
        crate::region::begin_structure_write_trace();
        let result = place_named(
            case["feature"]
                .as_str()
                .unwrap()
                .trim_start_matches("minecraft:"),
            &mut region,
            &mut rng,
            source,
            &mut CaveRandomState::default(),
        )
        .unwrap();
        let writes = crate::region::take_structure_write_trace();
        let Outcome::Complete(placed) = result else {
            panic!("unavailable {label}")
        };
        assert_eq!(placed, case["exit"]["result"].as_bool().unwrap(), "{label}");
        successes += usize::from(placed);
        assert_eq!(
            rng.next_long(),
            case["exit"]["rng"]["next_i64_from_copy"].as_i64().unwrap(),
            "RNG {label}"
        );
        assert_eq!(
            serde_json::to_value(&writes).unwrap(),
            case["writes"],
            "ordered writes {label}"
        );
        total_writes += writes.len();
        let ticks: Vec<_> = region
            .tree_effects
            .tick_requests
            .iter()
            .map(|row| {
                assert_eq!(row[5], 1, "native fossil edge requests a fluid tick");
                assert_eq!(
                    row[3] as u32,
                    crate::structure::template_pool::StructureAssets::bundled()
                        .blocks
                        .water_fluid_id
                );
                json!({"pos": row[..3], "type": "minecraft:water", "delay": row[4]})
            })
            .collect();
        let expected_ticks: Vec<_> = case["raw_ticks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tick| {
                assert_eq!(tick["priority"], "NORMAL");
                // History schema 5 records Fluid.toString(), including a JVM
                // identity suffix. Retain the raw evidence and resolve only the
                // observed native source-water class, never an arbitrary fluid.
                assert_eq!(tick["type"].as_str().unwrap().split_once('@').unwrap().0,
                    "net.minecraft.world.level.material.WaterFluid$Source");
                json!({"pos": tick["pos"], "type": "minecraft:water", "delay": tick["trigger_tick"]})
            })
            .collect();
        assert_eq!(ticks, expected_ticks, "ordered raw ticks {label}");
        region.transfer_tree_effects().unwrap();
        region.end_source();
        for key in case["after"].as_array().unwrap() {
            let expected = &data["snapshots"][key.as_str().unwrap()];
            let pos = position(&expected["pos"]);
            let actual = region.owned_chunk(pos).unwrap();
            let mut digest = Sha256::new();
            for state in actual.states() {
                digest.update(state.to_le_bytes());
            }
            assert_eq!(
                format!("{:x}", digest.finalize()),
                expected["states_sha256"].as_str().unwrap(),
                "all blocks in {pos:?}: {label}"
            );
            let biomes: Vec<_> = expand(&expected["biomes"], |v| {
                crate::biome::id(v.as_str().unwrap()).unwrap()
            });
            assert_eq!(
                actual.noise_biomes.as_ref().unwrap(),
                &biomes,
                "quart cells {label}"
            );
            assert_eq!(
                serde_json::to_value(&actual.worldgen_heightmaps().unwrap().world_surface).unwrap(),
                expected["world_surface_wg"],
                "WG surface {label}"
            );
            assert_eq!(
                serde_json::to_value(&actual.worldgen_heightmaps().unwrap().ocean_floor).unwrap(),
                expected["ocean_floor_wg"],
                "WG floor {label}"
            );
            let marks: Vec<_> = (-4..20)
                .flat_map(|section| {
                    actual
                        .postprocessing_positions()
                        .iter()
                        .copied()
                        .filter(move |p| p.1 >> 4 == section)
                })
                .collect();
            assert_eq!(
                serde_json::to_value(marks).unwrap(),
                expected["postprocessing"],
                "ordered marks {label}"
            );
        }
    }
    assert_eq!(successes, 2);
    assert!(total_writes > 100);
    println!("{successes} successful / {} total native fossil streams; {total_writes} ordered writes; {} full chunk comparisons", cases.len(), cases.len() * 9);
}

#[test]
fn native_fossil_configured_templates_rotations_corners_and_ticks() {
    let data: Value =
        serde_json::from_str(include_str!("../../data/fossil_reference_26_1.json")).unwrap();
    let assets = crate::structure::template_pool::StructureAssets::bundled();
    assert_eq!(
        data["jar_sha256"],
        crate::structure::template_pool::StructureAssets::JAR_SHA256
    );
    assert_eq!(
        data["shape_order"],
        json!(["west", "east", "north", "south", "down", "up"])
    );
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 76);
    let mut total_ticks = 0;
    let mut total_writes = 0;
    let mut rejected = 0;
    for case in cases {
        let origin: [i32; 3] = serde_json::from_value(case["origin"].clone()).unwrap();
        let source = ChunkPos::new(origin[0] >> 4, origin[2] >> 4);
        let mode = case["mode"].as_str().unwrap();
        let feature = case["feature"].as_str().unwrap();
        let seed = case["seed"].as_i64().unwrap();
        let label = format!("{feature}, seed {seed}, {mode}, {origin:?}");
        let base = assets
            .blocks
            .default_state(if mode == "half_air" { "stone" } else { mode })
            .unwrap();
        let initial = |x: i32, y: i32| {
            if y >= 65 || mode == "half_air" && x & 1 == 0 {
                crate::block::AIR
            } else {
                base
            }
        };
        let mut region = FeatureRegion::shared(WorldGenerator::new(0));
        let mut available = BTreeMap::new();
        for cx in source.x - 1..=source.x + 1 {
            for cz in source.z - 1..=source.z + 1 {
                available.insert((cx, cz), ChunkStatus::Carvers);
                let chunk = region.owned_chunk_mut(ChunkPos::new(cx, cz));
                for y in crate::MIN_Y..=crate::MAX_Y {
                    for z in 0..16 {
                        for x in 0..16 {
                            chunk.states[((y + 64) * 256 + z * 16 + x) as usize] =
                                initial(cx * 16 + x, y);
                        }
                    }
                }
                chunk.worldgen_heightmaps = Some(
                    serde_json::from_value(json!({
                        "world_surface": vec![65; 256], "ocean_floor": vec![65; 256], "frozen": true
                    }))
                    .unwrap(),
                );
            }
        }
        region.begin_source(source, ChunkStatus::Features, available);
        let mut rng = WorldgenRandom::new(seed);
        crate::region::begin_structure_write_trace();
        let placed = place_configured(
            crate::block_predicate::catalog()
                .configured(feature)
                .unwrap(),
            Some(feature),
            &mut region,
            &mut rng,
            (origin[0], origin[1], origin[2]),
            &mut CaveRandomState::default(),
            &GenerationEnvironment,
        )
        .unwrap();
        let writes = crate::region::take_structure_write_trace();
        assert_eq!(placed, case["result"].as_bool().unwrap(), "{label}");
        rejected += usize::from(!placed);
        assert_eq!(
            rng.next_long(),
            case["next_long"].as_i64().unwrap(),
            "RNG {label}"
        );
        assert_eq!(
            serde_json::to_value(&writes).unwrap(),
            case["writes"],
            "writes {label}"
        );
        assert_eq!(
            serde_json::to_value(&region.tree_effects.tick_requests).unwrap(),
            case["ticks"],
            "ordered duplicate ticks {label}"
        );
        total_writes += writes.len();
        total_ticks += region.tree_effects.tick_requests.len();
        region.transfer_tree_effects().unwrap();
        region.end_source();
        let mut changes = Vec::new();
        let mut marks = Vec::new();
        for cx in source.x - 1..=source.x + 1 {
            for cz in source.z - 1..=source.z + 1 {
                let chunk = region.owned_chunk(ChunkPos::new(cx, cz)).unwrap();
                for (i, &state) in chunk.states().iter().enumerate() {
                    let x = cx * 16 + (i % 16) as i32;
                    let y = -64 + (i / 256) as i32;
                    let z = cz * 16 + (i / 16 % 16) as i32;
                    if state != initial(x, y) {
                        changes.push([x, y, z, state as i32]);
                    }
                }
                marks.extend(
                    chunk
                        .postprocessing_positions()
                        .iter()
                        .map(|&(x, y, z)| [cx * 16 + x as i32, y, cz * 16 + z as i32]),
                );
            }
        }
        changes.sort_unstable();
        assert_eq!(
            serde_json::to_value(changes).unwrap(),
            case["snapshot"]["changes"],
            "full 3x3 state delta {label}"
        );
        assert_eq!(
            serde_json::to_value(marks).unwrap(),
            case["snapshot"]["postprocessing"],
            "marks {label}"
        );
    }
    assert!(rejected > 0 && total_ticks > 0);
    println!("{} native configured fossils: {total_writes} writes, {total_ticks} ordered ticks, {rejected} rejected", cases.len());
}
