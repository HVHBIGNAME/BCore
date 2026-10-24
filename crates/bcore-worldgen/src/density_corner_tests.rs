use super::*;
use std::collections::BTreeSet;
use DensityFunction as D;

thread_local! {
    static BLEND_CALLS: RefCell<Vec<[i32; 3]>> = const { RefCell::new(Vec::new()) };
}

fn counted_blend(x: i32, y: i32, z: i32, value: f64) -> f64 {
    BLEND_CALLS.with(|calls| calls.borrow_mut().push([x, y, z]));
    value
}

fn reset_calls() {
    BLEND_CALLS.with(|calls| calls.borrow_mut().clear());
}

fn call_count() -> usize {
    BLEND_CALLS.with(|calls| calls.borrow().len())
}

fn noise() -> D {
    D::Noise {
        name: "ore_veininess".into(),
        xz: 1.5,
        y: 1.5,
    }
}

fn cold_bits(f: &D, ctx: &EvalContext, points: &[[f64; 3]]) -> Vec<u64> {
    let values = points
        .iter()
        .map(|&[x, y, z]| {
            clear_density_caches();
            f.evaluate(x, y, z, ctx).to_bits()
        })
        .collect();
    clear_density_caches();
    values
}

#[test]
fn adjacent_xyz_cells_share_four_corners_and_keep_cell_hits() {
    let f = D::Interpolated(Box::new(D::BlendDensity(Box::new(noise()))));
    for mode in [
        EvaluationMode::NoiseChunk,
        EvaluationMode::NoiseChunkMaterial,
    ] {
        let ctx = EvalContext {
            seed: -7,
            mode,
            blend_density: counted_blend,
            ..Default::default()
        };
        for origin in [[-8.0, -64.0, -12.0], [8.0, 312.0, -4.0]] {
            for axis in 0..3 {
                let first = [origin[0] + 1.0, origin[1] + 3.0, origin[2] + 2.0];
                let mut second = first;
                second[axis] += [4.0, 8.0, 4.0][axis];
                for points in [[first, second], [second, first]] {
                    let expected = cold_bits(&f, &ctx, &points);
                    reset_calls();
                    for (index, [x, y, z]) in points.into_iter().enumerate() {
                        assert_eq!(f.evaluate(x, y, z, &ctx).to_bits(), expected[index]);
                        assert_eq!(call_count(), [8, 12][index], "{mode:?}, axis {axis}");
                        assert_eq!(
                            density_cache_entries(),
                            [0, 0, 0, [8, 12][index], index + 1]
                        );
                        f.evaluate(x + 0.5, y + 0.5, z + 0.5, &ctx);
                        assert_eq!(call_count(), [8, 12][index], "cell hit reevaluated");
                    }
                    clear_density_caches();
                }
            }
        }
    }
}

#[test]
fn final_density_reuses_1225_vertices_for_6144_corner_requests() {
    let f = parse_router("overworld", "final_density").unwrap();
    let mut points = Vec::new();
    for x in 0..4 {
        for y in 0..48 {
            for z in 0..4 {
                points.push([
                    f64::from(-32 + x * 4 + 1),
                    f64::from(-64 + y * 8 + 3),
                    f64::from(48 + z * 4 + 2),
                ]);
            }
        }
    }
    let mut lattice = BTreeSet::new();
    for x in -8..=-4 {
        for y in -8..=40 {
            for z in 12..=16 {
                lattice.insert([x, y, z]);
            }
        }
    }
    for mode in [
        EvaluationMode::NoiseChunk,
        EvaluationMode::NoiseChunkMaterial,
    ] {
        let ctx = EvalContext {
            seed: 1234,
            mode,
            blend_density: counted_blend,
            ..Default::default()
        }
        .with_noise_bounds(-32, 48, 4);
        reset_calls();
        let expected = cold_bits(&f, &ctx, &points);
        assert_eq!(call_count(), 6144);
        let vertices: BTreeSet<_> =
            BLEND_CALLS.with(|calls| calls.borrow().iter().copied().collect());
        assert_eq!(vertices.len(), 1225);
        assert_eq!(vertices, lattice);
        for reverse in [false, true] {
            reset_calls();
            for visit in 0..points.len() {
                let index = if reverse {
                    points.len() - 1 - visit
                } else {
                    visit
                };
                let [x, y, z] = points[index];
                assert_eq!(f.evaluate(x, y, z, &ctx).to_bits(), expected[index]);
            }
            assert_eq!(call_count(), 1225, "{mode:?}, reverse={reverse}");
            let actual: BTreeSet<_> =
                BLEND_CALLS.with(|calls| calls.borrow().iter().copied().collect());
            assert_eq!(actual, vertices);
            for (&[x, y, z], &bits) in points.iter().zip(&expected).rev() {
                assert_eq!(f.evaluate(x, y, z, &ctx).to_bits(), bits);
            }
            assert_eq!(call_count(), 1225);
            clear_density_caches();
            assert_eq!(density_cache_entries(), [0; 5]);
        }
        println!("{mode:?}: 768 cells, 6144 cold corner requests, 1225 shared evaluations");
    }
}

#[test]
fn corner_point_keys_retain_coordinate_bits_seed_phase_and_node_identity() {
    clear_density_caches();
    reset_calls();
    let functions = [
        D::BlendDensity(Box::new(D::Y)),
        D::BlendDensity(Box::new(D::Constant(9.0))),
    ];
    let points = [
        [0.0, 0.0, 0.0],
        [-0.0, 0.0, 0.0],
        [0.0, -0.0, 0.0],
        [0.0, 0.0, -0.0],
        [0.0, 0.0f64.next_up(), 0.0],
        [1.0f64.next_up(), 0.0, 0.0],
    ];
    for seed in [0, i64::MIN] {
        for mode in [
            EvaluationMode::NoiseChunk,
            EvaluationMode::NoiseChunkMaterial,
        ] {
            let ctx = EvalContext {
                seed,
                mode,
                blend_density: counted_blend,
                ..Default::default()
            };
            for (index, f) in functions.iter().enumerate() {
                for &[x, y, z] in &points {
                    let expected = if index == 0 { y } else { 9.0 };
                    assert_eq!(cache_once(f, x, y, z, &ctx).to_bits(), expected.to_bits());
                    assert_eq!(cache_once(f, x, y, z, &ctx).to_bits(), expected.to_bits());
                }
            }
        }
    }
    assert_eq!(call_count(), 48);
    assert_eq!(density_cache_entries(), [0, 0, 0, 48, 0]);
    clear_density_caches();
}

#[test]
fn nested_shared_markers_preserve_seed_phase_and_raw_without_live_borrows() {
    let shared = Arc::new(D::CacheOnce(Box::new(D::Interpolated(Box::new(noise())))));
    let sum = D::Add(
        Box::new(D::Reference(Arc::clone(&shared))),
        Box::new(D::CacheAllInCell(Box::new(D::Reference(shared)))),
    );
    let f = D::CacheOnce(Box::new(D::Interpolated(Box::new(D::CacheOnce(Box::new(
        sum,
    ))))));
    let mut points: Vec<_> = (-17..=17).map(|x| [f64::from(x), 17.0, 3.0]).collect();
    points.extend([[-4.0, -64.0, -4.0], [0.0, 320.0, 0.0], [4.0, 319.0, 4.0]]);
    let mut cases = Vec::new();
    for seed in [0, -7, i64::MIN, i64::MAX] {
        for mode in [
            EvaluationMode::NoiseChunk,
            EvaluationMode::NoiseChunkMaterial,
            EvaluationMode::Raw,
        ] {
            let ctx = EvalContext {
                seed,
                mode,
                ..Default::default()
            };
            cases.push((ctx, cold_bits(&f, &ctx, &points)));
        }
    }
    assert!(cases
        .chunks_exact(3)
        .any(|phases| phases[0].1 != phases[1].1));
    for (ctx, expected) in cases.iter().chain(cases.iter().rev()) {
        let before = density_cache_entries();
        for (&[x, y, z], &bits) in points.iter().zip(expected).rev() {
            assert_eq!(
                f.evaluate(x, y, z, ctx).to_bits(),
                bits,
                "{ctx:?}, {x},{y},{z}"
            );
        }
        if ctx.mode == EvaluationMode::Raw {
            assert_eq!(density_cache_entries(), before);
        }
    }
    clear_density_caches();
    assert_eq!(density_cache_entries(), [0; 5]);
}

#[test]
fn corner_scopes_preserve_flat_bounds_geometry_and_blender_changes() {
    let f = D::Interpolated(Box::new(D::CacheOnce(Box::new(D::Add(
        Box::new(D::FlatCache(Box::new(D::Y))),
        Box::new(D::BlendDensity(Box::new(D::Constant(0.0)))),
    )))));
    let bounded = EvalContext::default().with_noise_bounds(0, 0, 1);
    clear_density_caches();
    for (x, z, expected) in [
        (1.0, 1.0, 0.0),
        (5.0, 1.0, 1.0),
        (-1.0, 1.0, 1.0),
        (9.0, 1.0, 4.0),
        (1.0, 5.0, 1.0),
    ] {
        assert_eq!(f.evaluate(x, 4.0, z, &bounded), expected);
    }
    let contexts = [
        bounded,
        EvalContext::default(),
        EvalContext {
            cell_width: 8,
            cell_height: 4,
            ..bounded
        },
        EvalContext {
            blend_density: |x, y, z, value| value + f64::from(x - y + z) * 0.25,
            ..bounded
        },
        bounded,
    ];
    let points = [
        [1.0, 3.0, 2.0],
        [5.0, 11.0, 2.0],
        [-1.0, -63.0, -2.0],
        [5.0, 319.0, 7.0],
    ];
    let mut first = None;
    for (index, ctx) in contexts.iter().enumerate() {
        let expected = cold_bits(&f, ctx, &points);
        for (&[x, y, z], &bits) in points.iter().zip(&expected).rev() {
            assert_eq!(f.evaluate(x, y, z, ctx).to_bits(), bits);
        }
        if index == 0 {
            first = Some(expected);
        } else if index == contexts.len() - 1 {
            assert_eq!(Some(expected), first);
        } else {
            assert_ne!(Some(&expected), first.as_ref());
        }
        clear_density_caches();
        assert_eq!(density_cache_entries(), [0; 5]);
    }
}

#[test]
fn native_interpolated_bits_survive_reverse_vertex_visitation() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../data/numeric_26_1.json")).unwrap();
    assert_eq!(fixture["minecraft"], "26.1");
    assert_eq!(
        fixture["jar_sha256"],
        crate::structure::template_pool::StructureAssets::HISTORICAL_JAR_SHA256
    );
    let mut cases = 0;
    for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
        case["id"]
            .as_str()
            .unwrap()
            .starts_with("noise_chunk/interpolated/")
    }) {
        let f = D::CacheAllInCell(Box::new(D::Add(
            Box::new(parse_json(&case["function"].to_string()).unwrap()),
            Box::new(D::Constant(0.0)),
        )));
        let ctx = EvalContext {
            seed: case["seed"].as_i64().unwrap(),
            ..Default::default()
        };
        let points = fixture["points"][case["points"].as_str().unwrap()]
            .as_array()
            .unwrap();
        let sample = fixture["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|sample| sample["id"] == case["id"])
            .unwrap();
        let bits = sample["bits"].as_array().unwrap();
        assert_eq!(points.len(), bits.len());
        for reverse in [false, true] {
            clear_density_caches();
            for visit in 0..points.len() {
                let index = if reverse {
                    points.len() - 1 - visit
                } else {
                    visit
                };
                let point = &points[index];
                let actual = f.evaluate(
                    point[0].as_f64().unwrap(),
                    point[1].as_f64().unwrap(),
                    point[2].as_f64().unwrap(),
                    &ctx,
                );
                assert_eq!(
                    actual.to_bits(),
                    u64::from_str_radix(bits[index].as_str().unwrap(), 16).unwrap(),
                    "{}, reverse={reverse}, {point}",
                    case["id"]
                );
            }
        }
        clear_density_caches();
        cases += 1;
    }
    assert_eq!(cases, 6);
}
