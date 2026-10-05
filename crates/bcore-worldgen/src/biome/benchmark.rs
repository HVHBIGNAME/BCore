// SPDX-License-Identifier: MIT
//! Identical-input benchmark; run the ignored test explicitly in release mode.
use super::{
    reference_tests::{climate, fixture},
    *,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

fn old_linear(rows: &[(BiomeId, BiomeParameters)], climate: [f64; 6]) -> BiomeId {
    let target = climate.map(quantize);
    rows.iter()
        .min_by_key(|(_, p)| {
            [
                p.temperature,
                p.humidity,
                p.continentalness,
                p.erosion,
                p.depth,
                p.weirdness,
            ]
            .into_iter()
            .zip(target)
            .map(|(range, value)| (range.min - value).max(value - range.max).max(0).pow(2))
            .sum::<i64>()
                + p.offset * p.offset
        })
        .map(|(id, _)| *id)
        .expect("nonempty biome parameter list")
}

fn median(values: &[f64]) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    ordered[ordered.len() / 2]
}

fn measure<S>(
    inputs: &[Vec<[f64; 6]>],
    iterations: usize,
    mut begin_stream: impl FnMut() -> S,
    mut sample: impl FnMut(&mut S, [f64; 6]) -> u32,
) -> f64 {
    let start = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..iterations {
        for group in inputs {
            let mut stream = begin_stream();
            for &point in group {
                checksum = checksum.wrapping_add(u64::from(sample(&mut stream, black_box(point))));
            }
        }
    }
    black_box(checksum);
    let count: usize = inputs.iter().map(Vec::len).sum();
    start.elapsed().as_secs_f64() * 1e9 / (count * iterations) as f64
}

fn timings(
    inputs: &[Vec<[f64; 6]>],
    rows: &[(BiomeId, BiomeParameters)],
    tree: &ParameterList,
    iterations: usize,
    rounds: usize,
) -> serde_json::Value {
    let mut linear = Vec::new();
    let mut native = Vec::new();
    let mut explicit = Vec::new();
    let mut compatibility = Vec::new();
    measure(
        inputs,
        1,
        || tree.reset_thread_cache(),
        |(), p| tree.find(p),
    );
    for round in 0..rounds {
        let mut run = |kind| match kind {
            0 => linear.push(measure(
                inputs,
                iterations,
                || (),
                |(), p| old_linear(black_box(rows), p),
            )),
            1 => native.push(measure(
                inputs,
                iterations,
                || tree.reset_thread_cache(),
                |(), p| biome_at(black_box(tree), p[0], p[1], p[2], p[3], p[4], p[5]),
            )),
            2 => explicit.push(measure(
                inputs,
                iterations,
                || tree.sampler(),
                |sampler, p| sampler.find(p),
            )),
            3 => compatibility.push(measure(
                inputs,
                iterations,
                || (),
                |(), p| biome_at(black_box(rows), p[0], p[1], p[2], p[3], p[4], p[5]),
            )),
            _ => unreachable!(),
        };
        for offset in 0..4 {
            run((round + offset) % 4);
        }
    }
    json!({
        "linear_ns": linear, "native_tls_ns": native, "explicit_stream_ns": explicit, "cold_slice_ns": compatibility,
        "median_linear_ns": median(&linear), "median_native_tls_ns": median(&native),
        "median_explicit_stream_ns": median(&explicit), "median_cold_slice_ns": median(&compatibility),
        "native_tls_speedup": median(&linear) / median(&native),
        "explicit_stream_speedup": median(&linear) / median(&explicit),
        "cold_slice_speedup": median(&linear) / median(&compatibility)
    })
}

#[test]
#[ignore = "release benchmark; BCORE_BIOME_BENCH_ITERATIONS / BCORE_BIOME_BENCH_ROUNDS"]
fn benchmark_native_tree_against_original_linear_on_identical_router_inputs() {
    let fixture = fixture();
    let rows = canonical::rows();
    let start = Instant::now();
    let tree = ParameterList::new(rows.to_vec());
    let construction_us = start.elapsed().as_secs_f64() * 1e6;
    let iterations: usize = std::env::var("BCORE_BIOME_BENCH_ITERATIONS")
        .unwrap_or_else(|_| "4".into())
        .parse()
        .unwrap();
    let rounds: usize = std::env::var("BCORE_BIOME_BENCH_ROUNDS")
        .unwrap_or_else(|_| "7".into())
        .parse()
        .unwrap();
    assert!(iterations > 0 && rounds > 0);
    let reference = fixture
        .trees
        .iter()
        .find(|tree| tree.id == "overworld")
        .unwrap();
    let groups: Vec<_> = fixture
        .groups
        .iter()
        .filter(|group| group.settings.as_deref() == Some("overworld"))
        .collect();
    let mut inputs = Vec::new();
    let mut old_differences = BTreeMap::new();
    for order in ["forward", "reverse"] {
        let mut differences = 0;
        for group in &groups {
            tree.reset_thread_cache();
            for &index in &group.streams[order].order {
                let point = climate(&group.climate_f64_bits[index]);
                let old = old_linear(rows, point);
                let fast = tree.find(point);
                assert_eq!(
                    format!("minecraft:{}", name(old)),
                    reference.rows[group.linear[index] as usize].biome
                );
                assert_eq!(
                    format!("minecraft:{}", name(fast)),
                    reference.rows[group.streams[order].rows[index] as usize].biome
                );
                differences += usize::from(old != fast);
            }
        }
        old_differences.insert(order, differences);
    }
    for group in groups {
        inputs.push(
            group
                .climate_f64_bits
                .iter()
                .map(climate)
                .collect::<Vec<_>>(),
        );
    }
    let mut input_hash = Sha256::new();
    for point in inputs.iter().flatten() {
        for coordinate in point {
            input_hash.update(coordinate.to_bits().to_le_bytes());
        }
    }
    let input_hash = format!("{:x}", input_hash.finalize());
    let mut reports = Vec::new();
    for order in ["forward", "reverse"] {
        if order == "reverse" {
            for group in &mut inputs {
                group.reverse();
            }
        }
        let mut report = timings(&inputs, rows, &tree, iterations, rounds);
        report["order"] = json!(order);
        reports.push(report);
    }
    let result = json!({
        "benchmark": "biome/26.1/native-tree-vs-original-linear", "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "jar_sha256": canonical::JAR_SHA256, "parameter_rows": rows.len(), "tree_nodes": tree.node_count(),
        "tree_and_rows_bytes": tree.storage_bytes(), "construction_us": construction_us,
        "query_inputs": inputs.iter().map(Vec::len).sum::<usize>(), "seeds": 6, "iterations": iterations, "rounds": rounds,
        "inputs_sha256_le_f64": input_hash,
        "stream_policy": "reset once per seed group per iteration, matching native fixture streams",
        "native_vs_old_resource_key_differences": old_differences, "reports": reports
    });
    println!(
        "BIOME_BENCH={}",
        serde_json::to_string_pretty(&result).unwrap()
    );
    if let Ok(path) = std::env::var("BCORE_BIOME_BENCH_OUTPUT") {
        std::fs::write(path, serde_json::to_string_pretty(&result).unwrap() + "\n").unwrap();
    }
}
