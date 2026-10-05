//! Native admission and placement evidence exercised through the production adapter.
use super::*;
use crate::generation::{graph::layer_positions, ChunkHolder, GenerationWorld, StructureData};
use crate::structure::scattered::ScatteredStart;
use crate::WorldGenerator;
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn fixtures() -> [Value; 2] {
    [
        include_str!("../../data/scattered_structures_26_1.json"),
        include_str!("../../data/scattered_jungle_temple_26_1.json"),
    ]
    .map(|data| serde_json::from_str(data).unwrap())
}

fn int(value: &Value) -> i32 {
    value.as_i64().unwrap() as i32
}
fn pos(value: &Value) -> Pos {
    (int(&value[0]), int(&value[1]), int(&value[2]))
}
fn chunk(value: &Value) -> ChunkPos {
    ChunkPos::new(int(&value[0]), int(&value[1]))
}
fn nbt(value: &Value) -> Nbt {
    serde_json::from_value(value["nbt"].clone()).unwrap()
}

#[test]
fn native_scattered_admission_matches_actual_noise_heights_and_retained_starts() {
    let mut admitted = 0;
    for fixture in fixtures() {
        for row in fixture["admission"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["biome"] == "overworld")
        {
            let kind = ScatteredKind::from_name(row["kind"].as_str().unwrap()).unwrap();
            let seed = row["seed"].as_i64().unwrap();
            let source = chunk(&row["chunk"]);
            let generator = WorldGenerator::new(seed);
            assert_eq!(
                generator.base_height_for_vanilla(
                    kind.admission_heightmap(),
                    source.x * 16 + 8,
                    source.z * 16 + 8
                ),
                int(&row["first_free"]),
                "{kind:?} {source:?} native base height"
            );
            if let Some(corners) = row["corner_first_free"].as_array() {
                for corner in corners {
                    assert_eq!(
                        generator.base_height_vanilla(int(&corner[0]), int(&corner[1])),
                        int(&corner[2]),
                        "native temple corner {corner}"
                    );
                }
            }
            let world = GenerationWorld::new(seed);
            let first = world
                .generate_to_status(source, ChunkStatus::StructureStarts)
                .unwrap();
            let again = world
                .generate_to_status(source, ChunkStatus::StructureStarts)
                .unwrap();
            assert_eq!(first.chunk, again.chunk);
            assert_eq!(
                again
                    .coverage
                    .target
                    .stage(ChunkStatus::StructureStarts)
                    .attempts,
                1
            );
            let starts = row["starts"].as_array().unwrap();
            let actual = first.chunk.structures.scattered_starts.get(kind.name());
            assert_eq!(
                usize::from(actual.is_some()),
                starts.len(),
                "{kind:?} {source:?} admission"
            );
            if let Some(start) = actual {
                assert_eq!(start.to_nbt(), nbt(&starts[0]), "{kind:?} {source:?}");
                admitted += 1;
            }
            assert!(first.chunk.structures.valid_for(source));
            let restored: StructureData =
                serde_json::from_slice(&serde_json::to_vec(&first.chunk.structures).unwrap())
                    .unwrap();
            assert_eq!(restored, first.chunk.structures);
        }
    }
    assert_eq!(admitted, 6);
}

#[test]
fn native_scattered_reference_membership_and_cached_boxes_survive_storage() {
    let fixture = &fixtures()[1];
    let mut comparisons = 0;
    for row in fixture["references"].as_array().unwrap() {
        let start = ScatteredStart::from_nbt(&nbt(&row["start"])).unwrap();
        let mut state = GenerationState::new(WorldGenerator::new(0));
        for target in row["targets"].as_array().unwrap() {
            for pos in layer_positions(chunk(&target["chunk"]), 8) {
                state
                    .holders
                    .entry((pos.x, pos.z))
                    .or_insert_with(|| ChunkHolder::new(pos));
            }
        }
        let key = (start.source[0], start.source[1]);
        let kind = start.kind;
        state
            .holders
            .get_mut(&key)
            .unwrap()
            .structures
            .scattered_starts
            .insert(kind.name().into(), start);
        for target in row["targets"].as_array().unwrap() {
            let target_pos = chunk(&target["chunk"]);
            let actual = state.scattered_references(target_pos);
            assert_eq!(
                json!(actual.get(kind.name()).cloned().unwrap_or_default()),
                target["sources"]
            );
            state
                .holders
                .get_mut(&(target_pos.x, target_pos.z))
                .unwrap()
                .structures
                .scattered_references = actual;
            assert!(state.holders[&(target_pos.x, target_pos.z)]
                .structures
                .valid_for(target_pos));
            comparisons += 1;
        }
        let data = &state.holders[&key].structures;
        let mut restored: StructureData =
            serde_json::from_slice(&serde_json::to_vec(data).unwrap()).unwrap();
        assert_eq!(restored, *data);
        assert_eq!(
            json!(restored
                .scattered_starts
                .get_mut(kind.name())
                .unwrap()
                .reference_bounds()
                .as_array()),
            row["reference_bounds"]
        );
    }
    assert_eq!(comparisons, 216);
}

fn base(row: &Value, p: Pos, overrides: &BTreeMap<Pos, u32>) -> u32 {
    if let Some(&state) = overrides.get(&p) {
        return state;
    }
    let name = match row["terrain"].as_str().unwrap() {
        "flat" => {
            if p.1 < 64 {
                "stone"
            } else {
                "air"
            }
        }
        "beach" => {
            if p.1 < 53 {
                "stone"
            } else if p.1 < 63 {
                "sand"
            } else if p.1 < 65 {
                "water"
            } else {
                "air"
            }
        }
        "water" => {
            if p.1 < 58 {
                "stone"
            } else if p.1 < 64 {
                "water"
            } else {
                "air"
            }
        }
        "slope" => {
            if p.1 < 60 + (p.0 + 2 * p.2).rem_euclid(9) {
                "stone"
            } else {
                "air"
            }
        }
        "dirt" => {
            if p.1 < 64 {
                "dirt"
            } else {
                "air"
            }
        }
        "deepslate" => {
            if p.1 < 64 {
                "deepslate"
            } else {
                "air"
            }
        }
        "void" => "air",
        name => panic!("unexpected native fixture terrain {name}"),
    };
    StructureAssets::bundled()
        .blocks
        .default_state(name)
        .unwrap()
}

#[test]
fn native_scattered_placement_through_live_region_preserves_clips_loot_and_pending_entities() {
    let mut histories = 0;
    let mut passes = 0;
    let mut requests = 0;
    for fixture in fixtures() {
        for row in fixture["placements"].as_array().unwrap() {
            // The production adapter has overworld bounds and real factories;
            // artificial denied-write/null-factory profiles are component tests.
            if row["min_y"].as_i64().unwrap_or(i64::from(crate::MIN_Y)) != i64::from(crate::MIN_Y)
                || row["suppress_block_entities"] == true
                || row["denied"].as_array().is_some_and(|a| !a.is_empty())
            {
                continue;
            }
            let label = row["name"].as_str().unwrap();
            let seed = row["seed"].as_i64().unwrap();
            let mut start = ScatteredStart::from_nbt(&nbt(&row["initial"])).unwrap();
            start.reference_bounds();
            let mut region = FeatureRegion::shared(WorldGenerator::new(seed));
            let positions: BTreeSet<_> = row["passes"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|pass| layer_positions(chunk(&pass["source"]), 1).map(|p| (p.x, p.z)))
                .collect();
            let overrides: BTreeMap<_, _> = row["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| (pos(v), int(&v[3]) as u32))
                .collect();
            let mut baseline = BTreeMap::new();
            for &(cx, cz) in &positions {
                let target = region.owned_chunk_mut(ChunkPos::new(cx, cz));
                for (i, state) in target.states.iter_mut().enumerate() {
                    *state = base(
                        row,
                        (
                            cx * 16 + (i % 16) as i32,
                            crate::MIN_Y + (i / 256) as i32,
                            cz * 16 + ((i / 16) % 16) as i32,
                        ),
                        &overrides,
                    );
                }
                baseline.insert((cx, cz), target.states.clone());
            }
            for (pass_index, pass) in row["passes"].as_array().unwrap().iter().enumerate() {
                let source = chunk(&pass["source"]);
                let before_ticks: BTreeMap<_, _> = positions
                    .iter()
                    .map(|&(x, z)| {
                        (
                            (x, z),
                            region
                                .owned_chunk(ChunkPos::new(x, z))
                                .unwrap()
                                .tick_requests()
                                .len(),
                        )
                    })
                    .collect();
                let before_marks: BTreeMap<_, _> = positions
                    .iter()
                    .map(|&(x, z)| {
                        (
                            (x, z),
                            region
                                .owned_chunk(ChunkPos::new(x, z))
                                .unwrap()
                                .postprocessing_positions()
                                .len(),
                        )
                    })
                    .collect();
                region.begin_source(
                    source,
                    ChunkStatus::Features,
                    positions
                        .iter()
                        .map(|&p| (p, ChunkStatus::Carvers))
                        .collect(),
                );
                crate::region::begin_structure_write_trace();
                let mut random = WorldgenRandom::new(0);
                let decoration = random.set_decoration_seed(seed, source.x * 16, source.z * 16);
                let config = scattered::ScatteredCatalog::bundled().config(start.kind);
                random.set_feature_seed(decoration, config.structure_index, config.decoration_step);
                let b = &pass["clip"];
                let clip = BoundingBox {
                    min: pos(b),
                    max: (int(&b[3]), int(&b[4]), int(&b[5])),
                };
                let result = scattered::place_in_chunk(
                    &mut start,
                    &mut ScatteredRegion {
                        region: &mut region,
                        source,
                    },
                    &mut random,
                    clip,
                )
                .unwrap();
                assert_eq!(
                    random.next_long(),
                    pass["next_i64"].as_i64().unwrap(),
                    "{label} pass {pass_index} RNG"
                );
                region.end_source();
                let writes = crate::region::take_structure_write_trace();
                let expected_writes: Vec<[i32; 5]> = pass["effects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| e[0] == "block" && e[6] == true)
                    .map(|e| [int(&e[1]), int(&e[2]), int(&e[3]), int(&e[4]), int(&e[5])])
                    .collect();
                assert_eq!(
                    writes, expected_writes,
                    "{label} pass {pass_index} ordered writes"
                );
                assert_eq!(result.blocks_written, writes.len());
                requests += result.mob_requests;
                assert_eq!(
                    start.to_nbt(),
                    nbt(&pass["after"]),
                    "{label} retained start"
                );
                assert_eq!(
                    json!(start.reference_bounds().as_array()),
                    pass["cached_reference_bounds"]
                );
                let mut expected_states = baseline.clone();
                for entry in pass["states"].as_array().unwrap() {
                    let (x, y, z) = pos(entry);
                    expected_states.get_mut(&(x >> 4, z >> 4)).unwrap()
                        [((y - crate::MIN_Y) * 256 + (z & 15) * 16 + (x & 15)) as usize] =
                        int(&entry[3]) as u32;
                }
                let mut entities = BTreeMap::new();
                let mut actual_ticks = BTreeMap::<_, Vec<Value>>::new();
                let mut marks = BTreeMap::<Pos, usize>::new();
                for &(cx, cz) in &positions {
                    let target = region.owned_chunk(ChunkPos::new(cx, cz)).unwrap();
                    let first = target
                        .states
                        .iter()
                        .zip(&expected_states[&(cx, cz)])
                        .position(|(a, b)| a != b);
                    assert_eq!(
                        first, None,
                        "{label} pass {pass_index} chunk {cx},{cz} states"
                    );
                    for (&(x, y, z), data) in target.feature_block_entities() {
                        let p = (cx * 16 + x as i32, y, cz * 16 + z as i32);
                        assert!(data.valid_for(target.get(x, y, z).unwrap(), p));
                        entities.insert(p, (data.full_nbt().unwrap(), data.update_nbt().unwrap()));
                    }
                    for tick in &target.tick_requests()[before_ticks[&(cx, cz)]..] {
                        let crate::tick_request::TickTarget::Fluid(id) = tick.target else {
                            panic!("unexpected scattered tick");
                        };
                        actual_ticks.entry((cx, cz)).or_default().push(json!([
                            tick.block_pos,
                            "fluid",
                            id,
                            tick.delay
                        ]));
                    }
                    for &(x, y, z) in &target.postprocessing_positions()[before_marks[&(cx, cz)]..]
                    {
                        *marks
                            .entry((cx * 16 + x as i32, y, cz * 16 + z as i32))
                            .or_default() += 1;
                    }
                    for request in target.structure_entities() {
                        assert!(request.valid_for(target.pos));
                        assert!(request.finalize);
                    }
                }
                let expected_entities: BTreeMap<_, _> = pass["block_entities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| (pos(&e["pos"]), (nbt(&e["full"]), nbt(&e["update"]))))
                    .collect();
                assert_eq!(
                    entities, expected_entities,
                    "{label} pass {pass_index} full/update BE"
                );
                let mut expected_ticks = BTreeMap::<_, Vec<Value>>::new();
                for tick in pass["ticks"].as_array().unwrap() {
                    let (x, _, z) = pos(&tick[0]);
                    expected_ticks
                        .entry((x >> 4, z >> 4))
                        .or_default()
                        .push(tick.clone());
                }
                assert_eq!(actual_ticks, expected_ticks, "{label} ticks");
                let expected_marks: BTreeMap<_, _> = pass["marks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| (pos(e), e[3].as_u64().unwrap() as usize))
                    .collect();
                assert_eq!(marks, expected_marks, "{label} marks");
                passes += 1;
            }
            histories += 1;
        }
    }
    assert!(
        histories >= 30 && passes >= 60 && requests >= 20,
        "{histories} histories, {passes} passes, {requests} requests"
    );
}
