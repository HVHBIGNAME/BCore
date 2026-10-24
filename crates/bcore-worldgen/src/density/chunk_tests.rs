use super::*;
use crate::density::{clear_density_caches, density_cache_entries};
use std::cell::Cell;

fn sampled_cells(root: &RootDensity<'_>) -> usize {
    root.interpolators
        .iter()
        .flat_map(|node| node.valid)
        .map(|word| word.count_ones() as usize)
        .sum()
}

#[test]
fn bundled_plan_matches_all_root_bits_at_every_block_and_reverse_order() {
    let graph = VanillaGraph::load().unwrap();
    for (seed, pos) in [
        (846_692_123_413_862_008, ChunkPos::new(0, 0)),
        (-7, ChunkPos::new(-64, -32)),
    ] {
        let geometry = Geometry::new(pos).unwrap();
        let veins = graph.ore_veins.as_ref().unwrap();
        for (original, mode, cuts) in [
            (&graph.final_density, EvaluationMode::NoiseChunk, 5),
            (&veins.toggle, EvaluationMode::NoiseChunkMaterial, 1),
            (&veins.ridged, EvaluationMode::NoiseChunkMaterial, 2),
            (&veins.gap, EvaluationMode::NoiseChunkMaterial, 0),
        ] {
            clear_density_caches();
            let ctx = geometry.context(seed, mode);
            let mut points = Vec::new();
            let mut expected = Vec::new();
            for z in 0..16 {
                for x in 0..16 {
                    for y in MIN_Y..=MAX_Y {
                        let point = (geometry.base_x + x, y, geometry.base_z + z);
                        points.push(point);
                        expected.push(
                            original
                                .evaluate(point.0 as f64, y as f64, point.2 as f64, &ctx)
                                .to_bits(),
                        );
                    }
                }
            }
            clear_density_caches();
            let mut remaining = MAX_INTERPOLATORS;
            let mut root = RootDensity::new(original, ctx, geometry, &mut remaining).unwrap();
            assert_eq!(root.interpolators.len(), cuts);
            for (&point, &bits) in points.iter().zip(&expected) {
                assert_eq!(
                    root.sample(point).to_bits(),
                    bits,
                    "{seed}/{mode:?}/{point:?}"
                );
            }
            let after = density_cache_entries();
            assert_eq!(
                after[4], 0,
                "indexed cells entered the old interpolation map"
            );
            for (&point, &bits) in points.iter().zip(&expected).rev() {
                assert_eq!(root.sample(point).to_bits(), bits, "warm reverse {point:?}");
            }
            assert_eq!(density_cache_entries(), after);
            clear_density_caches();
            let mut remaining = MAX_INTERPOLATORS;
            let mut root = RootDensity::new(original, ctx, geometry, &mut remaining).unwrap();
            for (&point, &bits) in points.iter().zip(&expected).rev() {
                assert_eq!(root.sample(point).to_bits(), bits, "cold reverse {point:?}");
            }
            assert_eq!(density_cache_entries()[4], 0);
            clear_density_caches();
        }
    }
}

#[test]
fn integer_geometry_matches_generic_fractions_and_all_boundaries() {
    for pos in [
        ChunkPos::new(-125, 187),
        ChunkPos::new(0, 0),
        ChunkPos::new(62, -1),
    ] {
        let geometry = Geometry::new(pos).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                for y in MIN_Y..=MAX_Y {
                    let p = geometry
                        .point((geometry.base_x + x, y, geometry.base_z + z))
                        .unwrap();
                    let values = [p.pos.0 as f64, y as f64, p.pos.2 as f64];
                    for axis in 0..3 {
                        let size = [4.0, 8.0, 4.0][axis];
                        let origin = (values[axis] / size).floor() * size;
                        let fraction = ((values[axis] - origin) / size).clamp(0.0, 1.0);
                        assert_eq!(p.origin[axis].to_bits(), origin.to_bits());
                        assert_eq!(p.fractions[axis].to_bits(), fraction.to_bits());
                    }
                    assert!(p.cell < CELL_COUNT);
                }
            }
        }
        for p in [
            (geometry.base_x - 1, 0, geometry.base_z),
            (geometry.base_x + 16, 0, geometry.base_z),
            (geometry.base_x, 0, geometry.base_z + 16),
            (geometry.base_x, MIN_Y - 1, geometry.base_z),
            (geometry.base_x, MAX_Y + 1, geometry.base_z),
        ] {
            assert!(geometry.point(p).is_none());
        }
    }
    assert!(Geometry::new(ChunkPos::new(i32::MAX, 0)).is_none());
    assert!(Geometry::new(ChunkPos::new(i32::MIN, 0)).is_none());
}

#[test]
fn range_is_lazy_and_out_of_job_samples_delegate_without_populating_cells() {
    let function = D::Range {
        input: Box::new(D::Interpolated(Box::new(D::Y))),
        min: -1_000_000.0,
        max: 0.0,
        when_in: Box::new(D::Constant(64.0)),
        when_out: Box::new(D::Interpolated(Box::new(D::Y))),
    };
    let geometry = Geometry::new(ChunkPos::new(-2, 3)).unwrap();
    let ctx = geometry.context(0, EvaluationMode::NoiseChunk);
    clear_density_caches();
    let mut remaining = MAX_INTERPOLATORS;
    let mut root = RootDensity::new(&function, ctx, geometry, &mut remaining).unwrap();
    assert_eq!(root.sample((-31, -63, 50)), 64.0);
    assert_eq!(sampled_cells(&root), 1);
    assert!(root.interpolators[1].valid.iter().all(|&word| word == 0));
    assert_eq!(root.sample((-31, 0, 50)), 0.0);
    assert_eq!(sampled_cells(&root), 3);
    assert_eq!(density_cache_entries()[4], 0);
    for point in [(-33, 7, 50), (-16, 7, 50), (-31, 320, 50)] {
        let before = sampled_cells(&root);
        let expected = function.evaluate(point.0 as f64, point.1 as f64, point.2 as f64, &ctx);
        assert_eq!(root.sample(point).to_bits(), expected.to_bits());
        assert_eq!(sampled_cells(&root), before);
    }
    clear_density_caches();
}

#[test]
fn compile_budget_and_independent_jobs_preserve_generic_nodes_and_scope() {
    let graph = VanillaGraph::load().unwrap();
    let pos = ChunkPos::new(0, 0);
    let geometry = Geometry::new(pos).unwrap();
    let function = D::CacheAllInCell(Box::new(D::Interpolated(Box::new(D::Y))));
    let mut remaining = 0;
    let generic = RootDensity::new(
        &function,
        geometry.context(0, EvaluationMode::NoiseChunkMaterial),
        geometry,
        &mut remaining,
    )
    .unwrap();
    assert!(generic.interpolators.is_empty());
    let interpolated = D::Interpolated(Box::new(D::Y));
    assert!(RootDensity::new(&interpolated, generic.ctx, geometry, &mut remaining).is_none());
    for seed in [0, i64::MIN, i64::MAX, 0] {
        clear_density_caches();
        let mut job = ChunkDensity::new(graph, seed, pos).unwrap();
        assert_eq!(sampled_cells(&job.final_density), 0);
        let p = (1, 3, 2);
        let expected = graph.final_density.evaluate(
            1.0,
            3.0,
            2.0,
            &geometry.context(seed, EvaluationMode::NoiseChunk),
        );
        assert_eq!(job.final_density.sample(p).to_bits(), expected.to_bits());
    }
    clear_density_caches();
}

#[test]
fn incomplete_corner_fill_remains_invalid_after_unwind() {
    thread_local! {
        static FAIL: Cell<bool> = const { Cell::new(true) };
    }
    fn hook(_: i32, y: i32, _: i32, value: f64) -> f64 {
        if y == 1 && FAIL.with(Cell::get) {
            panic!("corner witness");
        }
        value
    }
    let function = D::Interpolated(Box::new(D::BlendDensity(Box::new(D::Y))));
    let geometry = Geometry::new(ChunkPos::new(0, 0)).unwrap();
    let ctx = EvalContext {
        blend_density: hook,
        ..geometry.context(0, EvaluationMode::NoiseChunk)
    };
    clear_density_caches();
    FAIL.with(|fail| fail.set(true));
    let mut remaining = MAX_INTERPOLATORS;
    let mut root = RootDensity::new(&function, ctx, geometry, &mut remaining).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.sample((1, 3, 2))));
    assert!(result.is_err());
    assert_eq!(sampled_cells(&root), 0);
    FAIL.with(|fail| fail.set(false));
    assert_eq!(root.sample((1, 3, 2)), 3.0);
    assert_eq!(sampled_cells(&root), 1);
    clear_density_caches();
}

#[test]
fn compiler_declines_deep_unselected_and_over_budget_graphs() {
    fn chain(count: usize, mut inner: D) -> D {
        for _ in 0..count {
            inner = D::Abs(Box::new(inner));
        }
        inner
    }
    fn balanced(depth: usize) -> D {
        if depth == 0 {
            D::Constant(1.0)
        } else {
            D::Add(Box::new(balanced(depth - 1)), Box::new(balanced(depth - 1)))
        }
    }
    fn dispose(function: D) {
        let mut pending = vec![function];
        while let Some(node) = pending.pop() {
            match node {
                D::Abs(a) => pending.push(*a),
                D::Reference(a) => {
                    if let Ok(inner) = std::sync::Arc::try_unwrap(a) {
                        pending.push(inner);
                    }
                }
                D::Add(a, b) => pending.extend([*a, *b]),
                D::Range {
                    input,
                    when_in,
                    when_out,
                    ..
                } => pending.extend([*input, *when_in, *when_out]),
                _ => {}
            }
        }
    }
    let geometry = Geometry::new(ChunkPos::new(0, 0)).unwrap();
    let ctx = geometry.context(0, EvaluationMode::NoiseChunk);
    let unselected = D::Range {
        input: Box::new(D::Constant(0.0)),
        min: 0.0,
        max: 1.0,
        when_in: Box::new(D::Constant(7.0)),
        when_out: Box::new(chain(1024, D::Constant(1.0))),
    };
    assert!(RootDensity::new(&unselected, ctx, geometry, &mut 0).is_none());
    assert_eq!(unselected.evaluate(1.0, 2.0, 3.0, &ctx), 7.0);
    dispose(unselected);

    let admitted = chain(1, balanced(11));
    let root = RootDensity::new(&admitted, ctx, geometry, &mut 0).unwrap();
    assert_eq!(root.nodes.len(), MAX_SCALAR_NODES);
    drop(root);
    let declined = D::Abs(Box::new(admitted));
    assert!(RootDensity::new(&declined, ctx, geometry, &mut 0).is_none());
    dispose(declined);

    let shared = std::sync::Arc::new(chain(110, D::Constant(1.0)));
    let nested = D::Add(
        Box::new(D::Reference(std::sync::Arc::clone(&shared))),
        Box::new(chain(30, D::Reference(shared))),
    );
    assert!(RootDensity::new(&nested, ctx, geometry, &mut 0).is_none());
    dispose(nested);
}

#[test]
fn mixed_generic_markers_keep_material_coercion_and_cold_reverse_bits() {
    let noise = || D::Noise {
        name: "ore_vein_a".into(),
        xz: 4.0,
        y: 4.0,
    };
    let phase = D::Interpolated(Box::new(noise()));
    let geometry = Geometry::new(ChunkPos::new(-2, 3)).unwrap();
    let ctx = geometry.context(-7, EvaluationMode::NoiseChunkMaterial);
    let points = [(-31, 3, 50), (-29, 11, 51), (-33, -63, 50), (-18, 319, 63)];
    let mut phase_differs = false;
    for &(x, y, z) in &points {
        clear_density_caches();
        let material = phase.evaluate(x as f64, y as f64, z as f64, &ctx);
        clear_density_caches();
        let fill = phase.evaluate(
            x as f64,
            y as f64,
            z as f64,
            &EvalContext {
                mode: EvaluationMode::NoiseChunk,
                ..ctx
            },
        );
        phase_differs |= material.to_bits() != fill.to_bits();
    }
    assert!(
        phase_differs,
        "phase coercion witness does not distinguish lerp order"
    );
    let function = D::Add(
        Box::new(phase),
        Box::new(D::Add(
            Box::new(D::CacheAllInCell(Box::new(D::Interpolated(Box::new(
                noise(),
            ))))),
            Box::new(D::FlatCache(Box::new(D::Y))),
        )),
    );
    let expected: Vec<_> = points
        .iter()
        .map(|&(x, y, z)| {
            clear_density_caches();
            function
                .evaluate(x as f64, y as f64, z as f64, &ctx)
                .to_bits()
        })
        .collect();
    for reverse in [false, true] {
        clear_density_caches();
        let mut remaining = MAX_INTERPOLATORS;
        let mut root = RootDensity::new(&function, ctx, geometry, &mut remaining).unwrap();
        assert_eq!(root.interpolators.len(), 1);
        for visit in 0..points.len() {
            let i = if reverse {
                points.len() - 1 - visit
            } else {
                visit
            };
            assert_eq!(root.sample(points[i]).to_bits(), expected[i]);
            assert_eq!(root.sample(points[i]).to_bits(), expected[i]);
        }
    }
    clear_density_caches();
}

#[test]
fn indexed_scalar_operations_preserve_exceptional_bits_and_range_boundaries() {
    let geometry = Geometry::new(ChunkPos::new(0, 0)).unwrap();
    let ctx = geometry.context(0, EvaluationMode::NoiseChunk);
    let values = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(0x7ff8_0000_0000_1234),
    ];
    for &a in &values {
        for &b in &values {
            for op in [Binary::Add, Binary::Mul, Binary::Min, Binary::Max] {
                let a = Box::new(D::Constant(a));
                let b = Box::new(D::Constant(b));
                let binary = match op {
                    Binary::Add => D::Add(a, b),
                    Binary::Mul => D::Mul(a, b),
                    Binary::Min => D::Min(a, b),
                    Binary::Max => D::Max(a, b),
                };
                for squeeze in [false, true] {
                    let value = if squeeze {
                        D::Squeeze(Box::new(binary.clone()))
                    } else {
                        binary.clone()
                    };
                    let function = D::Range {
                        input: Box::new(D::Interpolated(Box::new(D::Y))),
                        min: 0.0,
                        max: 4.0,
                        when_in: Box::new(value),
                        when_out: Box::new(D::Constant(7.0)),
                    };
                    clear_density_caches();
                    let expected = function.evaluate(1.0, 3.0, 2.0, &ctx);
                    clear_density_caches();
                    let mut remaining = MAX_INTERPOLATORS;
                    let mut root =
                        RootDensity::new(&function, ctx, geometry, &mut remaining).unwrap();
                    assert_eq!(root.sample((1, 3, 2)).to_bits(), expected.to_bits());
                }
            }
        }
    }
    for (input, min, max) in [
        (-0.0, 0.0, 1.0),
        (0.0, 0.0, 1.0),
        (1.0, 0.0, 1.0),
        (1.0f64.next_down(), 0.0, 1.0),
        (f64::NAN, 0.0, 1.0),
        (0.0, f64::NAN, 1.0),
        (0.0, -1.0, f64::NAN),
    ] {
        let function = D::Range {
            input: Box::new(D::Constant(input)),
            min,
            max,
            when_in: Box::new(D::Interpolated(Box::new(D::Constant(-0.0)))),
            when_out: Box::new(D::Interpolated(Box::new(D::Constant(7.0)))),
        };
        clear_density_caches();
        let expected = function.evaluate(1.0, 3.0, 2.0, &ctx);
        clear_density_caches();
        let mut remaining = MAX_INTERPOLATORS;
        let mut root = RootDensity::new(&function, ctx, geometry, &mut remaining).unwrap();
        assert_eq!(root.sample((1, 3, 2)).to_bits(), expected.to_bits());
        assert_eq!(sampled_cells(&root), 1);
    }
    clear_density_caches();
}

#[test]
fn indexed_vein_bridge_preserves_generic_reader_demand_order() {
    thread_local! {
        static READS: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
        static SIGNALS: Cell<[f64; 3]> = const { Cell::new([0.8, -1.0, 1.0]) };
    }
    fn hook(_: i32, _: i32, _: i32, id: f64) -> f64 {
        let index = id as usize;
        READS.with(|reads| reads.borrow_mut().push(index as u8));
        SIGNALS.with(|signals| signals.get()[index])
    }
    let reader = |index: usize| {
        D::Add(
            Box::new(D::BlendDensity(Box::new(D::Constant(index as f64)))),
            Box::new(D::Interpolated(Box::new(D::Constant(0.0)))),
        )
    };
    let functions = VeinFunctions {
        toggle: reader(0),
        ridged: reader(1),
        gap: reader(2),
    };
    let geometry = Geometry::new(ChunkPos::new(0, 0)).unwrap();
    let ctx = EvalContext {
        blend_density: hook,
        ..geometry.context(1234, EvaluationMode::NoiseChunkMaterial)
    };
    let ore = OreVeinifier::new(1234);
    let mut patterns = std::collections::BTreeSet::new();
    for signals in [[0.1, -1.0, 1.0], [0.8, 1.0, 1.0], [0.8, -1.0, 1.0]] {
        SIGNALS.with(|values| values.set(signals));
        clear_density_caches();
        let mut remaining = MAX_INTERPOLATORS;
        let mut veins = ChunkVeins {
            toggle: RootDensity::new(&functions.toggle, ctx, geometry, &mut remaining).unwrap(),
            ridged: RootDensity::new(&functions.ridged, ctx, geometry, &mut remaining).unwrap(),
            gap: RootDensity::new(&functions.gap, ctx, geometry, &mut remaining).unwrap(),
        };
        for x in 0..16 {
            for z in 0..16 {
                READS.with(|reads| reads.borrow_mut().clear());
                let expected = functions.calculate(&ore, (x, 20, z), &ctx);
                let expected_reads = READS.with(|reads| reads.borrow().clone());
                patterns.insert(expected_reads.clone());
                READS.with(|reads| reads.borrow_mut().clear());
                assert_eq!(veins.calculate(&ore, (x, 20, z)), expected);
                assert_eq!(READS.with(|reads| reads.borrow().clone()), expected_reads);
            }
        }
    }
    assert!(patterns.contains(&vec![0]));
    assert!(patterns.contains(&vec![0, 1]));
    assert!(patterns.contains(&vec![0, 1, 2]));
    clear_density_caches();
}
