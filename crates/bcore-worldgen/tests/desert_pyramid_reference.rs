#[path = "support/desert_pyramid.rs"]
mod support;
use bcore_worldgen::feature_world::FeatureHeightmap;
use bcore_worldgen::generation::FeatureBlockEntity;
use bcore_worldgen::structure::jigsaw::HeightContext;
use bcore_worldgen::structure::placement::large_feature_random;
use bcore_worldgen::structure::scattered::{
    self, desert_pyramid, ScatteredCatalog, ScatteredKind, ScatteredPieceData, ScatteredStart,
};
use bcore_worldgen::structure::template::TemplateRandom;
use bcore_worldgen::structure::template_pool::StructureAssets;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use support::*;

#[test]
fn desert_pyramid_native_admission_corners_registry_slots_and_rng() {
    let data = fixture();
    assert_eq!(data["jar_sha256"], StructureAssets::JAR_SHA256);
    let kind = ScatteredKind::DesertPyramid;
    let config = ScatteredCatalog::bundled().config(kind);
    assert_eq!(
        (
            config.structure_id,
            config.decoration_step,
            config.structure_index
        ),
        (3, 4, 1)
    );
    assert_eq!(config.structure_set, "minecraft:desert_pyramids");
    let mut real = 0;
    let mut height_rejects = 0;
    for row in data["admission"].as_array().unwrap() {
        let source = chunk(&row["chunk"]);
        let seed = row["seed"].as_i64().unwrap();
        let potential = config.placement.potential_chunk(seed, source);
        assert_eq!(json!([potential.x, potential.z]), row["potential"]);
        assert_eq!(
            json!(config.placement.is_candidate(seed, source)),
            row["candidate"]
        );
        let mut heights: BTreeMap<_, _> = row["corner_first_free"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| ((n(&p[0]), n(&p[1])), n(&p[2])))
            .collect();
        let lowest = heights.values().min().unwrap() - 1;
        assert_eq!(json!(lowest), row["native_lowest_y"]);
        let center = (source.x * 16 + 8, source.z * 16 + 8);
        heights.insert(center, n(&row["first_free"]));
        let queries = RefCell::new(Vec::new());
        let first_free = |kind, x, z| {
            assert_eq!(kind, FeatureHeightmap::WorldSurfaceWg);
            queries.borrow_mut().push((x, z));
            heights[&(x, z)]
        };
        let heights = HeightContext {
            min_y: -64,
            max_y: 319,
            first_free: &first_free,
        };
        let biome = n(&row["noise_biome"]) as u32;
        let sea = n(&row["sea_level"]);
        let mut random = large_feature_random(seed, source);
        let built =
            scattered::assemble_with_sea_level(kind, source, &heights, sea, &mut random, |p| {
                assert_eq!(p, (center.0, n(&row["first_free"]) - 1, center.1));
                config.biomes.contains(&biome)
            })
            .unwrap();
        assert_eq!(
            json!(random.next_long()),
            row["next_i64"],
            "native admission RNG {row}"
        );
        assert_eq!(json!(built.is_some()), row["biome_admitted"]);
        let mut expected = vec![
            (source.x * 16, source.z * 16),
            (source.x * 16, source.z * 16 + 21),
            (source.x * 16 + 21, source.z * 16),
            (source.x * 16 + 21, source.z * 16 + 21),
        ];
        if lowest >= sea {
            expected.push(center);
        } else {
            height_rejects += 1;
        }
        assert_eq!(*queries.borrow(), expected);
        if let Some(start) = built {
            assert_eq!(start.to_nbt(), nbt(&row["assembled"]));
            assert_eq!(
                json!(start.generation_point.unwrap()),
                row["generation_point"]
            );
        }
        let admitted =
            scattered::for_chunk_with_sea_level(kind, seed, source, &heights, sea, |_| biome)
                .unwrap();
        let starts = row["starts"].as_array().unwrap();
        assert_eq!(
            usize::from(admitted.is_some()),
            starts.len(),
            "createStructures {row}"
        );
        if let Some(mut start) = admitted {
            assert_eq!(start.to_nbt(), nbt(&starts[0]));
            assert_eq!(
                json!(start.reference_bounds().as_array()),
                starts[0]["reference_bounds"]
            );
            assert!(start.valid_for(kind.name(), source));
            if row["biome"] == "overworld" {
                real += 1;
            }
        }
        assert_eq!(row["retained"], true);
    }
    assert_eq!(data["admission"].as_array().unwrap().len(), 62);
    assert_eq!(real, 2);
    assert!(height_rejects >= 2);
}

fn feature_random(
    seed: i64,
    source: bcore_core::ChunkPos,
    backend: usize,
) -> (i64, Box<dyn TemplateRandom>) {
    let config = ScatteredCatalog::bundled().config(ScatteredKind::DesertPyramid);
    if backend == 0 {
        let mut random = bcore_worldgen::simplex::WorldgenRandom::new(0);
        let d = random.set_decoration_seed(seed, source.x * 16, source.z * 16);
        random.set_feature_seed(d, config.structure_index, config.decoration_step);
        (d, Box::new(random))
    } else {
        let mut random = bcore_worldgen::random::WorldgenRandom::from_seed(0);
        let d = random.set_decoration_seed(seed, source.x * 16, source.z * 16);
        random.set_feature_seed(d, config.structure_index, config.decoration_step);
        (d, Box::new(random))
    }
}

#[test]
fn desert_pyramid_native_geometry_archaeology_effect_order_clips_and_two_rng_backends() {
    let data = fixture();
    let mut orientations = BTreeSet::new();
    let mut total_passes = 0;
    let mut writes = 0;
    for backend in 0..2 {
        for row in data["placements"].as_array().unwrap() {
            let label = row["name"].as_str().unwrap();
            let seed = row["seed"].as_i64().unwrap();
            let mut start = ScatteredStart::from_nbt(&nbt(&row["initial"])).unwrap();
            assert_eq!(
                json!(start.reference_bounds().as_array()),
                row["reference_bounds"]
            );
            orientations.insert(start.piece.orientation.unwrap().name());
            let mut world = World::new(row);
            for (index, pass) in row["passes"].as_array().unwrap().iter().enumerate() {
                if row["special"] == "reload" && index > 0 {
                    start = ScatteredStart::from_nbt(&start.to_nbt()).unwrap();
                }
                let ScatteredPieceData::DesertPyramid { archaeology, .. } = &start.piece.data
                else {
                    unreachable!()
                };
                assert_eq!(json!(archaeology), pass["before_archaeology"]);
                let source = chunk(&pass["source"]);
                world.reset(source);
                let (decoration, mut random) = feature_random(seed, source, backend);
                assert_eq!(json!(decoration), pass["decoration_seed"]);
                let mut region = desert_pyramid::region_random(seed, source);
                for _ in 0..n(&pass["region_advance"]) {
                    region.next_long();
                }
                let report = desert_pyramid::place_in_chunk(
                    &mut start,
                    &mut world,
                    &mut *random,
                    &mut region,
                    seed,
                    bounds(&pass["clip"]),
                )
                .unwrap();
                assert_eq!(
                    json!(random.next_long()),
                    pass["next_i64"],
                    "{label} pass {index} feature RNG backend {backend}"
                );
                assert_eq!(
                    json!(region.next_long() as i64),
                    pass["region_next_i64"],
                    "{label} pass {index} region RNG"
                );
                assert_eq!(
                    *world.heights.borrow(),
                    *pass["height_queries"].as_array().unwrap(),
                    "{label} pass {index} heights"
                );
                let expected = pass["effects"].as_array().unwrap();
                for (event, (a, b)) in world.effects.iter().zip(expected).enumerate() {
                    assert_eq!(a, b, "{label} pass {index} ordered effect {event}");
                }
                assert_eq!(
                    world.effects.len(),
                    expected.len(),
                    "{label} pass {index} effect count"
                );
                assert_eq!(json!(report.blocks_written), pass["write_count"]);
                assert_eq!(
                    report.block_attempts,
                    world.effects.iter().filter(|e| e[0] == "block").count()
                );
                assert_eq!(report.mob_requests, 0);
                let actual: Vec<_> = world
                    .states
                    .iter()
                    .map(|(p, s)| json!([p.0, p.1, p.2, s]))
                    .collect();
                assert_eq!(
                    actual.len(),
                    pass["states"].as_array().unwrap().len(),
                    "{label} {index} states length"
                );
                for (a, b) in actual.iter().zip(pass["states"].as_array().unwrap()) {
                    assert_eq!(a, b, "{label} pass {index} states");
                }
                assert_eq!(json!(world.ticks), pass["ticks"], "{label} ticks");
                assert_eq!(
                    json!(world
                        .marks
                        .iter()
                        .map(|(p, c)| json!([p.0, p.1, p.2, c]))
                        .collect::<Vec<_>>()),
                    pass["marks"],
                    "{label} marks"
                );
                assert_eq!(
                    world.entities.len(),
                    pass["block_entities"].as_array().unwrap().len(),
                    "{label} pass {index} BE count"
                );
                for entry in pass["block_entities"].as_array().unwrap() {
                    let entity = &world.entities[&pos(&entry["pos"])];
                    assert_eq!(
                        entity.full_data(),
                        nbt(&entry["full"]),
                        "{label} pass {index} saved BE"
                    );
                    let handoff = FeatureBlockEntity::from_template(entity).unwrap();
                    assert_eq!(handoff.full_nbt().unwrap(), nbt(&entry["full"]));
                    assert_eq!(
                        handoff.update_nbt().unwrap(),
                        nbt(&entry["update"]),
                        "{label} pass {index} update BE"
                    );
                }
                assert_eq!(
                    start.to_nbt(),
                    nbt(&pass["after"]),
                    "{label} pass {index} piece flags"
                );
                assert_eq!(
                    json!(start.reference_bounds().as_array()),
                    pass["cached_reference_bounds"]
                );
                let ScatteredPieceData::DesertPyramid { archaeology, .. } = &start.piece.data
                else {
                    unreachable!()
                };
                assert_eq!(
                    json!(archaeology),
                    pass["archaeology"],
                    "{label} pass {index} transient archaeology"
                );
                let retained: ScatteredStart = serde_json::from_value(json!(start)).unwrap();
                assert_eq!(retained, start);
                let mut loaded = ScatteredStart::from_nbt(&start.to_nbt()).unwrap();
                assert_eq!(
                    json!(loaded.reference_bounds().as_array()),
                    pass["reloaded_reference_bounds"]
                );
                let ScatteredPieceData::DesertPyramid { archaeology, .. } = &loaded.piece.data
                else {
                    unreachable!()
                };
                assert_eq!(json!(archaeology), pass["reloaded_archaeology"]);
                total_passes += 1;
                writes += report.blocks_written;
            }
        }
    }
    assert_eq!(orientations.len(), 4);
    assert_eq!(total_passes, 274);
    assert_eq!(writes, 706704);
}

#[test]
fn desert_pyramid_native_reference_membership() {
    let mut comparisons = 0;
    for row in fixture()["references"].as_array().unwrap() {
        let mut start = ScatteredStart::from_nbt(&nbt(&row["start"])).unwrap();
        assert_eq!(
            json!(start.reference_bounds().as_array()),
            row["reference_bounds"]
        );
        for target in row["targets"].as_array().unwrap() {
            let admitted = start.references_chunk(chunk(&target["chunk"]));
            assert_eq!(admitted, !target["sources"].as_array().unwrap().is_empty());
            if admitted {
                assert_eq!(json!([start.source]), target["sources"]);
            }
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 98);
}
