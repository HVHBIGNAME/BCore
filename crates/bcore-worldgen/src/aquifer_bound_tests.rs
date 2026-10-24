use super::*;
use crate::ChunkPos;
use serde_json::Value;

fn fixture() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("../data/aquifer_bound_26_1.json")).unwrap();
    assert_eq!(fixture["minecraft"], "26.1");
    assert_eq!(fixture["protocol"], 775);
    assert_eq!(
        fixture["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    fixture
}

fn int(value: &Value) -> i32 {
    i32::try_from(value.as_i64().unwrap()).unwrap()
}

fn graph_for(case: &Value) -> VanillaGraph {
    let mut graph = crate::reference_26_1::graph();
    if let Some(preliminary) = case.get("preliminary") {
        std::sync::Arc::get_mut(&mut graph.data)
            .unwrap()
            .preliminary_surface_level = Some(if let Some(value) = preliminary.as_f64() {
            density::DensityFunction::Constant(value)
        } else {
            density::parse_historical_26_1_json(
                &preliminary.to_string(),
                crate::reference_26_1::assets(),
            )
            .unwrap()
        });
    }
    graph
}

fn context_for(case: &Value) -> (ChunkPos, density::EvalContext) {
    let pos = ChunkPos::new(int(&case["chunk"][0]), int(&case["chunk"][1]));
    let cells = case.get("cell_count_xz").map_or(4, int);
    let ctx = density::EvalContext {
        seed: case["seed"].as_i64().unwrap(),
        ..Default::default()
    }
    .with_noise_bounds(pos.x * 16, pos.z * 16, cells);
    (pos, ctx)
}

fn assert_sample(aquifer: &mut Aquifer<'_>, sample: &Value, id: &Value) {
    let bits = u64::from_str_radix(sample["density_bits"].as_str().unwrap(), 16).unwrap();
    let state = aquifer.substance(
        int(&sample["pos"][0]),
        int(&sample["pos"][1]),
        int(&sample["pos"][2]),
        f64::from_bits(bits),
    );
    assert_eq!(
        state,
        sample["state"].as_u64().unwrap() as u32,
        "{id}: {sample}"
    );
    assert_eq!(
        aquifer.should_schedule_fluid_update(),
        sample["update"].as_bool().unwrap(),
        "{id}: {sample}"
    );
}

#[test]
fn native_chunk_bound_scan_and_strict_boundary() {
    let fixture = fixture();
    let mut outside_maximum = false;
    let mut ocean = false;
    let mut lowland = false;
    let mut mountain = false;
    let mut negative_coords = false;
    for case in fixture["cases"].as_array().unwrap() {
        density::clear_density_caches();
        let graph = graph_for(case);
        let (pos, ctx) = context_for(case);
        let mut aquifer = Aquifer::for_chunk(ctx.seed, &graph, ctx, pos);
        let id = &case["id"];
        assert_eq!(
            aquifer.skip_sampling_above_y,
            int(&case["threshold"]),
            "{id}"
        );
        assert_eq!(aquifer.preliminary_levels.len(), 121, "{id}");
        assert_eq!(case["constructor_cached_levels"], 121, "{id}");
        assert!(aquifer.centers.is_empty());
        assert!(aquifer.statuses.is_empty());
        assert_eq!(aquifer.ctx.flat_cache_bounds, ctx.flat_cache_bounds);
        assert_eq!(aquifer.ctx.mode, ctx.mode);
        let levels = case["levels"].as_array().unwrap();
        assert_eq!(levels.len(), 121);
        for (i, sample) in levels.iter().enumerate() {
            let (x, z, expected) = (int(&sample[0]), int(&sample[1]), int(&sample[2]));
            assert_eq!(x, pos.x * 16 - 16 + (i % 11) as i32 * 4);
            assert_eq!(z, pos.z * 16 - 16 + (i / 11) as i32 * 4);
            assert_eq!(
                aquifer.preliminary_levels.get(&(x, z)),
                Some(&expected),
                "{id}: {sample}"
            );
            assert_eq!(
                aquifer.preliminary_level(x + 3, z + 3),
                expected,
                "{id}: quantization"
            );
        }
        assert_eq!(aquifer.preliminary_levels.len(), 121);
        assert_eq!(
            aquifer.preliminary_levels.values().copied().max(),
            Some(int(&case["highest"]))
        );
        negative_coords |= pos.x < 0 || pos.z < 0;
        if case.get("preliminary").is_none() {
            let h = int(&case["highest"]);
            let biome = case["biome"].as_str().unwrap();
            ocean |= biome.contains("ocean");
            lowland |= biome == "minecraft:plains";
            mountain |= biome == "minecraft:frozen_peaks" && h > 128;
            outside_maximum |= int(&case["outside_max"]) > int(&case["inside_max"]);
        }
        let samples = case["samples"].as_array().unwrap();
        assert_eq!(samples.len(), 12);
        for sample in samples {
            let mut fresh = Aquifer::for_chunk(ctx.seed, &graph, ctx, pos);
            assert_sample(&mut fresh, sample, id);
            // Native snapshots count populated center-cache entries after one
            // call. This witnesses strict >T even when both paths return air.
            assert_eq!(
                fresh.centers.len(),
                sample["new_locations"].as_u64().unwrap() as usize,
                "{id}: {sample}"
            );
        }
        density::clear_density_caches();
    }
    assert!(outside_maximum && ocean && lowland && mountain && negative_coords);
}

#[test]
fn native_bound_resets_a_previously_scheduled_fluid_update() {
    let fixture = fixture();
    let mut histories = 0;
    for case in fixture["cases"].as_array().unwrap() {
        let Some(history) = case.get("history") else {
            continue;
        };
        density::clear_density_caches();
        let graph = graph_for(case);
        let (pos, ctx) = context_for(case);
        let mut aquifer = Aquifer::for_chunk(ctx.seed, &graph, ctx, pos);
        let history = history.as_array().unwrap();
        assert_eq!(history.len(), 6);
        for pair in history.chunks_exact(2) {
            assert_eq!(pair[0]["update"], true);
            assert_eq!(pair[1]["update"], false);
            assert!(int(&pair[1]["pos"][1]) > aquifer.skip_sampling_above_y);
            assert_sample(&mut aquifer, &pair[0], &case["id"]);
            let centers = aquifer.centers.len();
            assert_sample(&mut aquifer, &pair[1], &case["id"]);
            assert_eq!(aquifer.centers.len(), centers);
        }
        histories += 1;
        density::clear_density_caches();
    }
    assert!(histories >= 3);
}

#[test]
fn native_bound_java_floor_division_and_wrapping() {
    let fixture = fixture();
    let cases = fixture["arithmetic"].as_array().unwrap();
    assert_eq!(cases.len(), 23);
    for case in cases {
        density::clear_density_caches();
        let graph = graph_for(case);
        let (pos, ctx) = context_for(case);
        let aquifer = Aquifer::for_chunk(ctx.seed, &graph, ctx, pos);
        assert_eq!(
            aquifer.skip_sampling_above_y,
            int(&case["threshold"]),
            "{}",
            case["id"]
        );
        assert_eq!(aquifer.preliminary_levels.len(), 121);
        assert!(aquifer
            .preliminary_levels
            .values()
            .all(|&h| h == int(&case["preliminary"])));
        density::clear_density_caches();
    }
}

#[test]
fn native_prepass_retains_context_across_interleaved_flat_cache_bounds() {
    let fixture = fixture();
    let cases = fixture["cases"].as_array().unwrap();
    let full = cases
        .iter()
        .find(|c| c["id"] == "outside-flat-cache")
        .unwrap();
    let narrow = cases
        .iter()
        .find(|c| c["id"] == "explicit-geometry-narrow-flat-cache")
        .unwrap();
    let graph = graph_for(full);
    density::clear_density_caches();
    for case in [full, narrow, full] {
        let (pos, ctx) = context_for(case);
        let aquifer = Aquifer::for_chunk(ctx.seed, &graph, ctx, pos);
        assert_eq!(aquifer.skip_sampling_above_y, int(&case["threshold"]));
        assert_eq!(aquifer.preliminary_levels.len(), 121);
        for sample in case["levels"].as_array().unwrap() {
            assert_eq!(
                aquifer.preliminary_levels[&(int(&sample[0]), int(&sample[1]))],
                int(&sample[2]),
                "{}: {sample}",
                case["id"]
            );
        }
        let inside = (int(&case["cell_count_xz"]) + 1).pow(2) as usize;
        assert_eq!(
            aquifer
                .preliminary_levels
                .values()
                .filter(|&&y| y == -64)
                .count(),
            inside
        );
        assert_eq!(
            aquifer
                .preliminary_levels
                .values()
                .filter(|&&y| y == 16)
                .count(),
            121 - inside
        );
    }
    density::clear_density_caches();
}

#[test]
fn generic_aquifer_does_not_infer_chunk_geometry_from_narrow_bounds() {
    density::clear_density_caches();
    let graph = VanillaGraph::load().unwrap();
    let ctx = density::EvalContext::default().with_noise_bounds(-4, 12, 1);
    let aquifer = Aquifer::new(0, graph, ctx);
    assert_eq!(aquifer.skip_sampling_above_y, i32::MAX);
    assert!(aquifer.preliminary_levels.is_empty());
    assert_eq!(aquifer.ctx.flat_cache_bounds, Some([-1, 3, 0, 4]));
    density::clear_density_caches();
}
