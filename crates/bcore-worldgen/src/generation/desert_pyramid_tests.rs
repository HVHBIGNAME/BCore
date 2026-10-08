//! Native pyramid cases replayed through the retained scheduler and live adapter.
use super::*;
use crate::generation::{graph::layer_positions, ChunkHolder, GenerationWorld, StructureData};
use crate::structure::scattered::{
    desert_pyramid, ScatteredCatalog, ScatteredPieceData, ScatteredStart,
};
use crate::WorldGenerator;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::OnceLock;

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/desert_pyramid_26_1.json")).unwrap()
    })
}
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn pos(v: &Value) -> Pos {
    (n(&v[0]), n(&v[1]), n(&v[2]))
}
fn chunk(v: &Value) -> ChunkPos {
    ChunkPos::new(n(&v[0]), n(&v[1]))
}
fn nbt(v: &Value) -> Nbt {
    serde_json::from_value(v["nbt"].clone()).unwrap()
}

#[test]
fn desert_pyramid_native_overworld_anchors_use_real_heights_and_retained_admission() {
    let mut admitted = 0;
    for row in fixture()["admission"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["biome"] == "overworld")
    {
        let seed = row["seed"].as_i64().unwrap();
        let source = chunk(&row["chunk"]);
        let generator = WorldGenerator::new(seed);
        assert_eq!(
            generator.base_height_vanilla(source.x * 16 + 8, source.z * 16 + 8),
            n(&row["first_free"])
        );
        for corner in row["corner_first_free"].as_array().unwrap() {
            assert_eq!(
                generator.base_height_vanilla(n(&corner[0]), n(&corner[1])),
                n(&corner[2]),
                "native pyramid corner {corner}"
            );
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
        let actual = first
            .chunk
            .structures
            .scattered_starts
            .get(ScatteredKind::DesertPyramid.name());
        let starts = row["starts"].as_array().unwrap();
        assert_eq!(
            usize::from(actual.is_some()),
            starts.len(),
            "native pyramid {source:?}"
        );
        if let Some(start) = actual {
            assert_eq!(start.to_nbt(), nbt(&starts[0]));
            admitted += 1;
        }
        assert!(first.chunk.structures.valid_for(source));
    }
    assert_eq!(admitted, 2);
}

#[test]
fn desert_pyramid_native_references_follow_cached_start_boxes() {
    let mut comparisons = 0;
    for row in fixture()["references"].as_array().unwrap() {
        let start = ScatteredStart::from_nbt(&nbt(&row["start"])).unwrap();
        let key = (start.source[0], start.source[1]);
        let mut state = GenerationState::new(WorldGenerator::new(42));
        for target in row["targets"].as_array().unwrap() {
            for p in layer_positions(chunk(&target["chunk"]), 8) {
                state
                    .holders
                    .entry((p.x, p.z))
                    .or_insert_with(|| ChunkHolder::new(p));
            }
        }
        state
            .holders
            .get_mut(&key)
            .unwrap()
            .structures
            .scattered_starts
            .insert(start.kind.name().into(), start);
        for target in row["targets"].as_array().unwrap() {
            let actual = state.scattered_references(chunk(&target["chunk"]));
            assert_eq!(
                json!(actual
                    .get(ScatteredKind::DesertPyramid.name())
                    .cloned()
                    .unwrap_or_default()),
                target["sources"]
            );
            comparisons += 1;
        }
        let data = &state.holders[&key].structures;
        let restored: StructureData = serde_json::from_value(json!(data)).unwrap();
        assert_eq!(restored, *data);
    }
    assert_eq!(comparisons, 98);
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
        "slope" => {
            if p.1 < 60 + (p.0 + 2 * p.2).rem_euclid(9) {
                "stone"
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
        "void" => "air",
        other => panic!("unexpected pyramid terrain {other}"),
    };
    StructureAssets::bundled()
        .blocks
        .default_state(name)
        .unwrap()
}

#[test]
fn desert_pyramid_native_placement_through_live_region_keeps_archaeology_and_ordered_handoff() {
    let mut histories = 0;
    let mut passes = 0;
    for row in fixture()["placements"].as_array().unwrap() {
        if row["suppress_block_entities"] == true || !row["denied"].as_array().unwrap().is_empty() {
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
            .flat_map(|p| layer_positions(chunk(&p["source"]), 1).map(|p| (p.x, p.z)))
            .collect();
        let overrides: BTreeMap<_, _> = row["overrides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (pos(p), n(&p[3]) as u32))
            .collect();
        let mut baseline = BTreeMap::new();
        for &(cx, cz) in &positions {
            let target = region.owned_chunk_mut(ChunkPos::new(cx, cz));
            for (index, state) in target.states.iter_mut().enumerate() {
                *state = base(
                    row,
                    (
                        cx * 16 + (index % 16) as i32,
                        crate::MIN_Y + (index / 256) as i32,
                        cz * 16 + ((index / 16) % 16) as i32,
                    ),
                    &overrides,
                );
            }
            baseline.insert((cx, cz), target.states.clone());
        }
        for (index, pass) in row["passes"].as_array().unwrap().iter().enumerate() {
            if row["special"] == "reload" && index > 0 {
                start = ScatteredStart::from_nbt(&start.to_nbt()).unwrap();
            }
            let source = chunk(&pass["source"]);
            let mut before = BTreeMap::new();
            for &(x, z) in &positions {
                let c = region.owned_chunk(ChunkPos::new(x, z)).unwrap();
                before.insert(
                    (x, z),
                    (c.tick_requests().len(), c.postprocessing_positions().len()),
                );
            }
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
            let d = random.set_decoration_seed(seed, source.x * 16, source.z * 16);
            let config = ScatteredCatalog::bundled().config(ScatteredKind::DesertPyramid);
            random.set_feature_seed(d, config.structure_index, config.decoration_step);
            let mut region_random = desert_pyramid::region_random(seed, source);
            for _ in 0..n(&pass["region_advance"]) {
                region_random.next_long();
            }
            let b = &pass["clip"];
            let clip = BoundingBox {
                min: pos(b),
                max: (n(&b[3]), n(&b[4]), n(&b[5])),
            };
            let result = desert_pyramid::place_in_chunk(
                &mut start,
                &mut ScatteredRegion {
                    region: &mut region,
                    source,
                },
                &mut random,
                &mut region_random,
                seed,
                clip,
            )
            .unwrap();
            assert_eq!(
                json!(random.next_long()),
                pass["next_i64"],
                "{label} {index} feature RNG"
            );
            assert_eq!(
                json!(region_random.next_long() as i64),
                pass["region_next_i64"],
                "{label} {index} region RNG"
            );
            region.end_source();
            let writes = crate::region::take_structure_write_trace();
            let expected: Vec<[i32; 5]> = pass["effects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[0] == "block" && e[6] == true)
                .map(|e| [n(&e[1]), n(&e[2]), n(&e[3]), n(&e[4]), n(&e[5])])
                .collect();
            for (ordinal, (a, b)) in writes.iter().zip(&expected).enumerate() {
                assert_eq!(a, b, "{label} {index} live write {ordinal}");
            }
            assert_eq!(writes.len(), expected.len(), "{label} {index} write count");
            assert_eq!(result.blocks_written, writes.len());
            assert_eq!(start.to_nbt(), nbt(&pass["after"]));
            assert_eq!(
                json!(start.reference_bounds().as_array()),
                pass["cached_reference_bounds"]
            );
            let ScatteredPieceData::DesertPyramid { archaeology, .. } = &start.piece.data else {
                unreachable!()
            };
            assert_eq!(json!(archaeology), pass["archaeology"]);
            let mut expected_states = baseline.clone();
            for entry in pass["states"].as_array().unwrap() {
                let (x, y, z) = pos(entry);
                expected_states.get_mut(&(x >> 4, z >> 4)).unwrap()
                    [((y - crate::MIN_Y) * 256 + (z & 15) * 16 + (x & 15)) as usize] =
                    n(&entry[3]) as u32;
            }
            let mut entities = BTreeMap::new();
            let mut ticks = BTreeMap::<_, Vec<Value>>::new();
            let mut marks = BTreeMap::<_, Vec<Pos>>::new();
            for &(cx, cz) in &positions {
                let c = region.owned_chunk(ChunkPos::new(cx, cz)).unwrap();
                let first = c
                    .states
                    .iter()
                    .zip(&expected_states[&(cx, cz)])
                    .position(|(a, b)| a != b);
                assert_eq!(first, None, "{label} {index} states {cx},{cz}");
                for (&(x, y, z), data) in c.feature_block_entities() {
                    let p = (cx * 16 + x as i32, y, cz * 16 + z as i32);
                    assert!(data.valid_for(c.get(x, y, z).unwrap(), p));
                    entities.insert(p, (data.full_nbt().unwrap(), data.update_nbt().unwrap()));
                }
                for tick in &c.tick_requests()[before[&(cx, cz)].0..] {
                    let crate::tick_request::TickTarget::Fluid(id) = tick.target else {
                        panic!("unexpected pyramid block tick")
                    };
                    ticks.entry((cx, cz)).or_default().push(json!([
                        tick.block_pos,
                        "fluid",
                        id,
                        tick.delay
                    ]));
                }
                for &(x, y, z) in &c.postprocessing_positions()[before[&(cx, cz)].1..] {
                    marks.entry((cx, cz)).or_default().push((
                        cx * 16 + x as i32,
                        y,
                        cz * 16 + z as i32,
                    ));
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
                "{label} {index} typed full/update BE"
            );
            let mut expected_ticks = BTreeMap::<_, Vec<Value>>::new();
            for tick in pass["ticks"].as_array().unwrap() {
                let p = pos(&tick[0]);
                expected_ticks
                    .entry((p.0 >> 4, p.2 >> 4))
                    .or_default()
                    .push(tick.clone());
            }
            assert_eq!(ticks, expected_ticks, "{label} {index} ticks");
            let mut expected_marks = BTreeMap::<_, Vec<Pos>>::new();
            for e in pass["effects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[0] == "mark")
            {
                let p = (n(&e[1]), n(&e[2]), n(&e[3]));
                expected_marks
                    .entry((p.0 >> 4, p.2 >> 4))
                    .or_default()
                    .push(p);
            }
            assert_eq!(marks, expected_marks, "{label} {index} ordered marks");
            passes += 1;
        }
        histories += 1;
    }
    assert_eq!((histories, passes), (22, 127));
}
