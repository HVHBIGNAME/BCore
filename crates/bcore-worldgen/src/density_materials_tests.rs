//! Shared native-fixture decoding and integrated NOISE regression tests.
use crate::beardifier::{Beardifier, Rigid, TerrainAdjustment};
use crate::density::{self, EvalContext, EvaluationMode};
use crate::feature_world::Pos;
use crate::structure::jigsaw::{JigsawPiece, JigsawStart, Junction};
use crate::structure::template::{BoundingBox, Rotation};
use crate::structure::template_pool::{PoolElement, SingleElement};
use crate::{block, ChunkPos, GeneratedChunk, VanillaGraph, WorldGenerator, MAX_Y, MIN_Y};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn fixture(text: &str, section: &str) -> Value {
    let value: Value = serde_json::from_str(text).unwrap();
    assert_eq!(value["minecraft"], "26.1");
    assert_eq!(value["protocol"], 775);
    assert_eq!(value["section"], section);
    assert_eq!(
        value["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(value["probe_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(
        value["sources"].as_object().unwrap().len(),
        if section == "barriers" { 5 } else { 4 }
    );
    value
}

pub(super) fn bits(value: &Value) -> u64 {
    u64::from_str_radix(value.as_str().unwrap(), 16).unwrap()
}

pub(super) fn pos(value: &Value) -> Pos {
    (
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
        value[2].as_i64().unwrap() as i32,
    )
}

pub(super) fn chunk_pos(value: &Value) -> ChunkPos {
    ChunkPos::new(
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
    )
}

fn bounds(value: &Value) -> BoundingBox {
    BoundingBox::new(
        pos(value),
        (
            value[3].as_i64().unwrap() as i32,
            value[4].as_i64().unwrap() as i32,
            value[5].as_i64().unwrap() as i32,
        ),
    )
}

fn junctions(value: &Value) -> Vec<Junction> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|j| Junction {
            source: pos(&j["source"]),
            delta_y: j["delta_y"].as_i64().unwrap() as i32,
            destination_projection: serde_json::from_value(j["destination_projection"].clone())
                .unwrap(),
        })
        .collect()
}

pub(super) fn selected(value: &Value) -> Beardifier {
    let pieces = value["pieces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| Rigid {
            bounds: bounds(&p["bounds"]),
            terrain_adjustment: TerrainAdjustment::try_from(
                p["terrain_adaptation"].as_str().unwrap(),
            )
            .unwrap(),
            ground_level_delta: p["ground_level_delta"].as_i64().unwrap() as i32,
        })
        .collect();
    let result = Beardifier::from_parts(pieces, junctions(&value["junctions"]));
    let expected = value["affected_bounds"].as_array().unwrap();
    assert_eq!(
        result.affected_bounds(),
        (!expected.is_empty()).then(|| bounds(&value["affected_bounds"]))
    );
    result
}

pub(super) fn starts(value: &Value) -> Vec<JigsawStart> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let start = JigsawStart {
                generation_point: (0, 0, 0),
                pieces: s["pieces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        let bounds = bounds(&p["bounds"]);
                        JigsawPiece {
                            // Density consumes the captured native projection and box,
                            // not template block placement or the template's own size.
                            element: PoolElement::Single(SingleElement {
                                location: "minecraft:empty".into(),
                                processors: Vec::new(),
                                processor_config: serde_json::json!("minecraft:empty"),
                                projection: serde_json::from_value(p["projection"].clone())
                                    .unwrap(),
                                legacy: false,
                                override_waterlogging: None,
                            }),
                            origin: bounds.min,
                            bounds,
                            rotation: Rotation::None,
                            ground_level_delta: p["ground_level_delta"].as_i64().unwrap() as i32,
                            depth: 0,
                            junctions: junctions(&p["junctions"]),
                            waterlogging: true,
                        }
                    })
                    .collect(),
                terrain_adaptation: s["terrain_adaptation"].as_str().unwrap().into(),
                decoration_step: "underground_structures".into(),
                references: 0,
            };
            assert_eq!(
                start.reference_bounds().unwrap().as_array(),
                bounds(&s["reference_bounds"]).as_array()
            );
            start
        })
        .collect()
}

#[test]
fn native_noise_voxels_router_bits_heights_and_ordered_marks() {
    let fixture = fixture(include_str!("../data/noise_materials_26_1.json"), "noise");
    let barriers = self::fixture(
        include_str!("../data/noise_material_barriers_26_1.json"),
        "barriers",
    );
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 17);
    assert_eq!(barriers["cases"].as_array().unwrap().len(), 6);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let mut material_counts = BTreeMap::<u32, usize>::new();
    let mut pressure_barrier_veins = 0;
    let mut total_marks = 0;
    for case in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(barriers["cases"].as_array().unwrap())
    {
        let id = case["id"].as_str().unwrap();
        let seed = case["seed"].as_i64().unwrap();
        let cp = chunk_pos(&case["chunk"]);
        let starts = starts(&case["starts"]);
        let beard = Beardifier::for_chunk(cp, &starts).unwrap();
        let reference_beard = selected(&case["selected"]);
        assert_eq!(beard.pieces(), reference_beard.pieces(), "{id}");
        assert_eq!(beard.junctions(), reference_beard.junctions(), "{id}");
        let mut graph = VanillaGraph::load_uncached().unwrap();
        let functions = case.get("vein_functions").map_or_else(
            || graph.ore_veins.as_ref().unwrap().clone(),
            |functions| crate::ore_vein::VeinFunctions {
                toggle: density::parse_json(&functions["vein_toggle"].to_string()).unwrap(),
                ridged: density::parse_json(&functions["vein_ridged"].to_string()).unwrap(),
                gap: density::parse_json(&functions["vein_gap"].to_string()).unwrap(),
            },
        );
        std::sync::Arc::get_mut(&mut graph.data).unwrap().ore_veins = case["ore_veins_enabled"]
            .as_bool()
            .unwrap()
            .then(|| functions.clone());
        let mut chunk = GeneratedChunk::new(cp);
        // Existing effects must not be reordered by the parallel NOISE merge.
        chunk.postprocessing.push((15, 250, 15));
        pool.install(|| {
            WorldGenerator::new(seed).generate_noise_with_structures(&mut chunk, &graph, &beard)
        });
        // The retained runtime captures WG maps at this same successful stage
        // boundary. Keep those arrays tied to the native post-NOISE snapshot.
        chunk.capture_worldgen_heightmaps(false);
        let expected: Vec<u32> = case["runs"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|run| {
                std::iter::repeat_n(
                    run[0].as_u64().unwrap() as u32,
                    run[1].as_u64().unwrap() as usize,
                )
            })
            .collect();
        assert_eq!(expected.len(), 98_304);
        let mismatches: Vec<_> = expected
            .iter()
            .zip(chunk.states())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(index, (&a, &b))| {
                (
                    index % 16,
                    MIN_Y + (index / 256) as i32,
                    (index / 16) % 16,
                    a,
                    b,
                )
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "{id}: {} voxel mismatches, first {:?}",
            mismatches.len(),
            &mismatches[..mismatches.len().min(12)]
        );
        assert_eq!(chunk.postprocessing[0], (15, 250, 15));
        let marks: Vec<_> = case["marks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let (x, y, z) = pos(v);
                (x as usize, y, z as usize)
            })
            .collect();
        assert_eq!(
            &chunk.postprocessing[1..],
            marks,
            "{id}: fluid postprocessing order"
        );
        total_marks += marks.len();
        for state in &expected {
            *material_counts.entry(*state).or_default() += 1;
        }
        pressure_barrier_veins += case["nonpositive_density_veins"].as_u64().unwrap();
        for z in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    chunk.surface_y(x, z).map_or(MIN_Y, |y| y + 1),
                    case["world_surface_wg"][z * 16 + x].as_i64().unwrap() as i32,
                    "{id}: height {x},{z}"
                );
                assert_eq!(
                    chunk.worldgen_heightmaps().unwrap().world_surface[z * 16 + x],
                    case["world_surface_wg"][z * 16 + x].as_i64().unwrap() as i32,
                    "{id}: retained WG height {x},{z}"
                );
            }
        }

        density::clear_density_caches();
        let ctx = EvalContext {
            seed,
            ..Default::default()
        }
        .with_noise_bounds(cp.x * 16, cp.z * 16, 4);
        let live = EvalContext {
            mode: EvaluationMode::NoiseChunkMaterial,
            ..ctx
        };
        // Disabled ore-vein material still has all router fields in RandomState.
        let mut hashes: [md5::Context; 4] = std::array::from_fn(|_| md5::Context::new());
        for y in MIN_Y..=MAX_Y {
            for z in 0..16 {
                for x in 0..16 {
                    let (wx, wz) = (cp.x * 16 + x, cp.z * 16 + z);
                    let value = density::evaluate(
                        &graph.final_density,
                        wx as f64,
                        y as f64,
                        wz as f64,
                        &ctx,
                    ) + beard.compute(wx, y, wz);
                    hashes[0].consume(value.to_bits().to_le_bytes());
                    for (i, f) in [&functions.toggle, &functions.ridged, &functions.gap]
                        .into_iter()
                        .enumerate()
                    {
                        let value = density::evaluate(f, wx as f64, y as f64, wz as f64, &live);
                        hashes[i + 1].consume(value.to_bits().to_le_bytes());
                    }
                }
            }
        }
        for (i, hash) in hashes.into_iter().enumerate() {
            assert_eq!(
                format!("{:x}", hash.compute()),
                case["signal_md5"][i].as_str().unwrap(),
                "{id}: signal {}",
                fixture["signal_order"][i]
            );
        }
        for sample in case["samples"].as_array().unwrap() {
            let (x, y, z) = pos(&sample["pos"]);
            let mut values = vec![
                density::evaluate(&graph.final_density, x as f64, y as f64, z as f64, &ctx)
                    + beard.compute(x, y, z),
            ];
            values.extend(
                [&functions.toggle, &functions.ridged, &functions.gap]
                    .map(|f| density::evaluate(f, x as f64, y as f64, z as f64, &live)),
            );
            for (i, value) in values.into_iter().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    bits(&sample["bits"][i]),
                    "{id}: signal {i} at {x},{y},{z}"
                );
            }
        }
        density::clear_density_caches();
    }
    for state in [
        block::COPPER_ORE,
        block::RAW_COPPER_BLOCK,
        block::GRANITE,
        block::DEEPSLATE_IRON_ORE,
        block::RAW_IRON_BLOCK,
        block::TUFF,
        block::WATER,
        block::LAVA,
    ] {
        assert!(
            material_counts.get(&state).is_some_and(|&n| n > 0),
            "native NOISE corpus missing material {state}: {material_counts:?}"
        );
    }
    assert!(total_marks > 0);
    assert!(
        pressure_barrier_veins > 0,
        "controlled native corpus must exercise veins replacing aquifer pressure barriers"
    );
    println!("23 native NOISE chunks / 2,260,992 voxels / 9,043,968 exact signal values; {total_marks} ordered marks, {pressure_barrier_veins} pressure-barrier vein blocks; states {material_counts:?}");
}

#[test]
fn live_material_phase_does_not_leak_into_cell_cache() {
    use density::DensityFunction as D;
    let f = D::CacheOnce(Box::new(D::Interpolated(Box::new(D::Noise {
        name: "ore_veininess".into(),
        xz: 1.5,
        y: 1.5,
    }))));
    let cached = D::CacheAllInCell(Box::new(f.clone()));
    density::clear_density_caches();
    let ctx = EvalContext {
        seed: -7,
        ..Default::default()
    };
    let live = EvalContext {
        mode: EvaluationMode::NoiseChunkMaterial,
        ..ctx
    };
    let mut differing_roundoff = 0;
    for x in -17..=17 {
        for y in -12..=12 {
            let filling = f.evaluate(x as f64, y as f64, 3.0, &ctx);
            let material = f.evaluate(x as f64, y as f64, 3.0, &live);
            differing_roundoff += usize::from(filling.to_bits() != material.to_bits());
            assert_eq!(
                f.evaluate(x as f64, y as f64, 3.0, &ctx).to_bits(),
                filling.to_bits()
            );
            assert_eq!(
                cached.evaluate(x as f64, y as f64, 3.0, &live).to_bits(),
                filling.to_bits()
            );
        }
    }
    assert!(differing_roundoff > 0);
    density::clear_density_caches();
}
