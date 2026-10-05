use super::*;
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../data/feature_dependencies_26_1.json")).unwrap()
}

#[test]
fn both_pyramids_match_every_native_status_dependency_and_task_layer() {
    let native = fixture();
    assert_eq!(native["minecraft"], "26.1");
    assert_eq!(native["statuses"].as_array().unwrap().len(), 12);
    let mut layer_queries = 0;
    for status in ChunkStatus::ALL {
        let row = &native["statuses"][status.index()];
        assert_eq!(row["name"], status.name());
        assert_eq!(row["index"], status.index());
        assert_eq!(row["parent"], status.parent().name());
        assert_eq!(row["chunk_type"], status.chunk_type());
        assert_eq!(row["heightmaps_after"], json!(status.heightmaps_after()));
        for (name, pyramid) in [
            ("generation", ChunkPyramid::Generation),
            ("loading", ChunkPyramid::Loading),
        ] {
            let row = &native["pyramids"][name]["steps"][status.index()];
            let step = pyramid.step(status);
            assert_eq!(row["target_status"], step.target.name());
            assert_eq!(
                row["block_state_write_radius"],
                step.block_state_write_radius
            );
            for (field, deps) in [
                ("direct_dependencies", &step.direct),
                ("accumulated_dependencies", &step.accumulated),
            ] {
                assert_eq!(row[field]["radius"], deps.radius());
                assert_eq!(
                    row[field]["by_radius"],
                    json!(deps.by_radius.iter().map(|s| s.name()).collect::<Vec<_>>()),
                    "{name} {status} {field}"
                );
            }
            for layer in ChunkStatus::ALL {
                match row["task_layer_radii"].get(layer.name()) {
                    Some(radius) => {
                        assert_eq!(
                            step.layer_radius(layer),
                            Some(radius.as_u64().unwrap() as usize),
                            "{name} {status} {layer}"
                        );
                        layer_queries += 1;
                    }
                    None => assert_eq!(step.layer_radius(layer), None),
                }
            }
        }
    }
    assert_eq!(layer_queries, 156);
    assert_eq!(
        native["decoration_steps"].as_array().unwrap().len(),
        DECORATION_STEPS.len()
    );
    for (index, name) in DECORATION_STEPS.iter().enumerate() {
        assert_eq!(
            native["decoration_steps"][index],
            json!({"index":index,"name":name})
        );
    }
}

#[test]
fn feature_read_table_is_not_the_accumulated_planning_envelope() {
    let native = fixture();
    let mut neighbours = 0;
    for (name, pyramid) in [
        ("generation", ChunkPyramid::Generation),
        ("loading", ChunkPyramid::Loading),
    ] {
        let row = &native["feature_dependency_queries"][name];
        let pos =
            |v: &Value| ChunkPos::new(v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap() as i32);
        let center = pos(&row["center"]);
        let step = pyramid.step(ChunkStatus::Features);
        for sample in row["neighbour_samples"].as_array().unwrap() {
            let target = pos(&sample["chunk"]);
            assert_eq!(
                sample["native_chessboard_distance"],
                chessboard_distance(center, target)
            );
            for (field, deps) in [
                ("direct_dependency", &step.direct),
                ("accumulated_dependency", &step.accumulated),
            ] {
                assert_eq!(
                    sample[field],
                    deps.at(center, target)
                        .map_or("outside_dependency_table", ChunkStatus::name)
                );
            }
            neighbours += 1;
        }
        for (field, deps) in [
            ("direct_get_radius_of_features", &step.direct),
            ("accumulated_get_radius_of_features", &step.accumulated),
        ] {
            assert_eq!(row[field]["throws"], "java.lang.IllegalArgumentException");
            assert_eq!(deps.radius_of(ChunkStatus::Features), None);
        }
        assert_eq!(step.layer_radius(ChunkStatus::Features), Some(0));
    }
    assert_eq!(neighbours, 14);
}

#[test]
fn fresh_full_and_features_have_native_work_counts_and_local_layer_order() {
    use ChunkStatus::*;
    for (target, counts) in [
        (Features, [441, 441, 25, 25, 9, 9, 9, 1, 0, 0, 0, 0]),
        (Full, [529, 529, 49, 49, 25, 25, 25, 9, 9, 1, 1, 1]),
    ] {
        for (layer, expected) in ChunkStatus::ALL.into_iter().zip(counts) {
            let positions: Vec<_> = ChunkPyramid::Generation
                .step(target)
                .layer_radius(layer)
                .into_iter()
                .flat_map(|r| layer_positions(ChunkPos::new(-2, 3), r))
                .collect();
            assert_eq!(positions.len(), expected, "{target} {layer}");
            assert!(positions
                .windows(2)
                .all(|p| (p[0].x, p[0].z) < (p[1].x, p[1].z)));
        }
    }
}
