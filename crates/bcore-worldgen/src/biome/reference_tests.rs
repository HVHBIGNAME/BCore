// SPDX-License-Identifier: MIT
use super::{canonical::CapturedRow, *};
use crate::density::{self, EvalContext};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Barrier, OnceLock};

#[derive(Deserialize)]
pub(super) struct TreeReference {
    pub(super) id: String,
    pub(super) rows: Vec<CapturedRow>,
    nodes: usize,
    preorder_sha256_le: String,
}

impl TreeReference {
    pub(super) fn build(&self) -> ParameterList {
        ParameterList::new(
            self.rows
                .iter()
                .enumerate()
                .map(|(index, row)| (index as u32, row.parameters()))
                .collect(),
        )
    }
}

#[derive(Deserialize)]
pub(super) struct Stream {
    pub(super) order: Vec<usize>,
    pub(super) rows: Vec<u32>,
}

#[derive(Deserialize)]
pub(super) struct Group {
    pub(super) id: String,
    pub(super) tree: String,
    pub(super) targets: Vec<[i64; 6]>,
    pub(super) linear: Vec<u32>,
    pub(super) cold: Vec<u32>,
    pub(super) streams: BTreeMap<String, Stream>,
    seed: Option<i64>,
    pub(super) settings: Option<String>,
    #[serde(default)]
    quart_positions: Vec<[i32; 3]>,
    #[serde(default)]
    pub(super) climate_f64_bits: Vec<[String; 6]>,
}

#[derive(Deserialize)]
struct Concurrent {
    tree: String,
    targets: Vec<[i64; 6]>,
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Step {
    tree: String,
    target: [i64; 6],
    row: u32,
}

#[derive(Deserialize)]
struct Contexts {
    trees: Vec<TreeReference>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Quantization {
    f64_bits: String,
    quantized: i64,
}

#[derive(Deserialize)]
struct RangeDistance {
    range: [i64; 2],
    point: i64,
    distance: i64,
}

#[derive(Deserialize)]
struct NodeDistance {
    target: [i64; 7],
    distance: i64,
}

#[derive(Deserialize)]
struct Arithmetic {
    root_tree: String,
    ranges: Vec<RangeDistance>,
    nodes: Vec<NodeDistance>,
}

#[derive(Deserialize)]
pub(super) struct Fixture {
    minecraft: String,
    jar_sha256: String,
    probe_sha256: String,
    source_hash_order: Vec<String>,
    source_hash_encoding: String,
    dimensions: Vec<String>,
    registry: Vec<String>,
    pub(super) trees: Vec<TreeReference>,
    pub(super) groups: Vec<Group>,
    concurrent: Vec<Concurrent>,
    contexts: Contexts,
    quantization: Vec<Quantization>,
    arithmetic: Arithmetic,
    summary: serde_json::Value,
}

pub(super) fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/biome_tree_26_1.json"))
            .expect("native biome tree capture")
    })
}

pub(super) fn climate(bits: &[String; 6]) -> [f64; 6] {
    std::array::from_fn(|d| f64::from_bits(u64::from_str_radix(&bits[d], 16).unwrap()))
}

fn topology(tree: &tree::Tree, index: usize, hash: &mut Sha256) {
    let node = tree.nodes[index];
    hash.update([u8::from(node.count != 0)]);
    for range in node.space {
        hash.update(range.min.to_le_bytes());
        hash.update(range.max.to_le_bytes());
    }
    hash.update(
        if node.count == 0 {
            node.first
        } else {
            node.count
        }
        .to_le_bytes(),
    );
    for child in node.first..node.first + node.count {
        topology(tree, child as usize, hash);
    }
}

#[test]
fn native_tree_topologies_and_source_provenance_match() {
    let fixture = fixture();
    assert_eq!(fixture.minecraft, "26.1");
    assert_eq!(fixture.jar_sha256, canonical::JAR_SHA256);
    assert_eq!(
        fixture.source_hash_order,
        ["scripts/BiomeTreeReference.java"]
    );
    assert_eq!(
        fixture.source_hash_encoding,
        "UTF-8, CRLF normalized to LF, concatenated in source_hash_order"
    );
    assert_eq!(
        fixture.probe_sha256,
        format!(
            "{:x}",
            Sha256::digest(
                include_str!("../../../../scripts/BiomeTreeReference.java")
                    .replace("\r\n", "\n")
                    .as_bytes()
            )
        )
    );
    assert_eq!(
        fixture.dimensions,
        [
            "temperature",
            "humidity",
            "continentalness",
            "erosion",
            "depth",
            "weirdness",
            "offset"
        ]
    );
    assert_eq!(fixture.trees.len(), 17);
    assert_eq!(fixture.groups.len(), 36);
    assert_eq!(
        fixture
            .groups
            .iter()
            .map(|group| group.targets.len())
            .sum::<usize>(),
        106893
    );
    for reference in fixture.trees.iter().chain(&fixture.contexts.trees) {
        let list = reference.build();
        assert_eq!(list.node_count(), reference.nodes, "{}", reference.id);
        let mut hash = Sha256::new();
        topology(&list.tree, 0, &mut hash);
        assert_eq!(
            format!("{:x}", hash.finalize()),
            reference.preorder_sha256_le,
            "{}",
            reference.id
        );
    }
}

#[test]
fn canonical_rows_and_wire_ids_match_native_resource_keys() {
    let fixture = fixture();
    let native = fixture
        .trees
        .iter()
        .find(|tree| tree.id == "overworld")
        .unwrap();
    let list = ParameterList::overworld();
    assert_eq!(native.rows.len(), 7593);
    assert_eq!(list.values().len(), native.rows.len());
    assert_eq!(fixture.registry.len(), 65);
    assert_eq!(registry().len(), 66);
    let mut shifted_ids = 0;
    for (native_id, key) in fixture.registry.iter().enumerate() {
        let wire_id = id(key).unwrap();
        assert_eq!(format!("minecraft:{}", name(wire_id)), *key);
        shifted_ids += usize::from(native_id as u32 != wire_id);
    }
    assert!(
        shifted_ids > 0,
        "the native and wire registries must not be conflated"
    );
    for (index, (row, (wire_id, parameters))) in native.rows.iter().zip(list.values()).enumerate() {
        assert_eq!(
            format!("minecraft:{}", name(*wire_id)),
            row.biome,
            "row {index}"
        );
        assert_eq!(*parameters, row.parameters(), "row {index}");
    }
    let old =
        parse_parameters(&crate::assets::bundled()["biome_parameters/overworld.json"]).unwrap();
    let differences = old
        .iter()
        .zip(list.values())
        .filter(|(old, native)| old != native)
        .count();
    println!("canonical rows={}, bundled rows={}, differing rows={differences}, remapped registry ids={shifted_ids}", list.values().len(), old.len());
    assert_eq!(
        old,
        list.values(),
        "bundled parameters differ from the pinned JAR; use ParameterList::overworld()"
    );
    let canonical: serde_json::Value =
        serde_json::from_str(include_str!("../../data/biome_parameters_26_1.json")).unwrap();
    assert_eq!(canonical["probe_sha256"], fixture.probe_sha256);
    assert_eq!(
        canonical["source_hash_order"],
        serde_json::json!(fixture.source_hash_order)
    );
}

#[test]
fn all_native_cold_and_history_dependent_leaf_choices_match() {
    let fixture = fixture();
    let mut queries = 0;
    let mut linear_differences = 0;
    let mut cache_differences = 0;
    let mut biome_differences = 0;
    for reference in &fixture.trees {
        let list = reference.build();
        for group in fixture
            .groups
            .iter()
            .filter(|group| group.tree == reference.id)
        {
            assert_eq!(group.cold.len(), group.targets.len());
            assert_eq!(group.linear.len(), group.targets.len());
            for (stream_name, stream) in &group.streams {
                assert_eq!(stream.rows.len(), group.targets.len());
                assert_eq!(stream.order.len(), group.targets.len());
                list.reset_thread_cache();
                let mut sampler = list.sampler();
                for &index in &stream.order {
                    let target = group.targets[index];
                    let expected = stream.rows[index];
                    assert_eq!(
                        list.find_biome(target),
                        expected,
                        "{} {stream_name} #{index} TLS",
                        group.id
                    );
                    assert_eq!(
                        sampler.find_quantized(target),
                        expected,
                        "{} {stream_name} #{index} sampler",
                        group.id
                    );
                    queries += 2;
                }
            }
            let mut sampler = list.sampler();
            for (index, (&target, &expected)) in group.targets.iter().zip(&group.cold).enumerate() {
                sampler.reset();
                assert_eq!(
                    sampler.find_quantized(target),
                    expected,
                    "{} cold #{index}",
                    group.id
                );
                // Exercise the source-compatible API as well, including mutated allocations.
                if index < 32 {
                    assert_eq!(
                        list.values().find_biome(target),
                        expected,
                        "{} slice #{index}",
                        group.id
                    );
                }
                let forward = group.streams["forward"].rows[index];
                linear_differences += usize::from(group.linear[index] != forward);
                cache_differences += usize::from(expected != forward);
                biome_differences += usize::from(
                    reference.rows[group.linear[index] as usize].biome
                        != reference.rows[forward as usize].biome,
                );
                queries += 1;
            }
        }
    }
    assert_eq!(
        linear_differences,
        fixture.summary["linear_vs_forward_row_differences"]
            .as_u64()
            .unwrap() as usize
    );
    assert_eq!(
        cache_differences,
        fixture.summary["cold_vs_forward_row_differences"]
            .as_u64()
            .unwrap() as usize
    );
    assert!(linear_differences > 0 && cache_differences > 0);
    println!("{queries} exact native searches; linear/forward row differences={linear_differences}, resource-key differences={biome_differences}, cold/forward differences={cache_differences}");
}

#[test]
fn native_concurrent_threads_and_identical_list_contexts_stay_independent() {
    let fixture = fixture();
    for case in &fixture.concurrent {
        let reference = fixture
            .trees
            .iter()
            .find(|tree| tree.id == case.tree)
            .unwrap();
        let list = reference.build();
        let barrier = Barrier::new(case.streams.len());
        std::thread::scope(|scope| {
            for stream in &case.streams {
                let list = &list;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for &index in &stream.order {
                        assert_eq!(
                            list.find_biome(case.targets[index]),
                            stream.rows[index],
                            "{} thread #{index}",
                            case.tree
                        );
                    }
                });
            }
        });
    }
    let lists: BTreeMap<_, _> = fixture
        .contexts
        .trees
        .iter()
        .map(|tree| (tree.id.as_str(), tree.build()))
        .collect();
    for step in &fixture.contexts.steps {
        assert_eq!(
            lists[step.tree.as_str()].find_biome(step.target),
            step.row,
            "{} {:?}",
            step.tree,
            step.target
        );
    }
}

#[test]
fn native_quantization_range_branches_and_wrapping_long_distances_match() {
    let fixture = fixture();
    for case in &fixture.quantization {
        let value = f64::from_bits(u64::from_str_radix(&case.f64_bits, 16).unwrap());
        assert_eq!(quantize(value), case.quantized, "{}", case.f64_bits);
    }
    for case in &fixture.arithmetic.ranges {
        let [min, max] = case.range;
        assert_eq!(
            ClimateRange { min, max }.distance(case.point),
            case.distance,
            "{:?} {}",
            case.range,
            case.point
        );
    }
    let tree = fixture
        .trees
        .iter()
        .find(|tree| tree.id == fixture.arithmetic.root_tree)
        .unwrap()
        .build();
    for case in &fixture.arithmetic.nodes {
        assert_eq!(
            tree.tree.nodes[0].distance(&case.target),
            case.distance,
            "{:?}",
            case.target
        );
    }
}

#[test]
fn six_seeds_three_router_configs_match_native_climates_and_resource_keys() {
    let fixture = fixture();
    let reference = fixture
        .trees
        .iter()
        .find(|tree| tree.id == "overworld")
        .unwrap();
    let list = ParameterList::overworld();
    let fields = [
        "temperature",
        "vegetation",
        "continents",
        "erosion",
        "depth",
        "ridges",
    ];
    let mut values = 0;
    let mut biomes = 0;
    for group in fixture.groups.iter().filter(|group| group.seed.is_some()) {
        let settings = &crate::assets::bundled()
            [&format!("noise_settings/{}.json", group.settings.as_ref().unwrap())];
        let router: Vec<_> = fields
            .iter()
            .map(|field| density::parse_json(&settings["noise_router"][field].to_string()).unwrap())
            .collect();
        density::clear_density_caches();
        let context = EvalContext {
            seed: group.seed.unwrap(),
            ..Default::default()
        };
        list.reset_thread_cache();
        assert_eq!(group.targets.len(), group.quart_positions.len());
        assert_eq!(group.targets.len(), group.climate_f64_bits.len());
        for (index, position) in group.quart_positions.iter().enumerate() {
            let [x, y, z] = position.map(|p| p.wrapping_mul(4) as f64);
            let actual: [f64; 6] =
                std::array::from_fn(|d| density::evaluate(&router[d], x, y, z, &context));
            let expected = climate(&group.climate_f64_bits[index]);
            assert_eq!(
                actual.map(f64::to_bits),
                expected.map(f64::to_bits),
                "{} #{index} climate",
                group.id
            );
            assert_eq!(
                actual.map(quantize),
                group.targets[index],
                "{} #{index} quantization",
                group.id
            );
            let result = biome_at(
                &list, actual[0], actual[1], actual[2], actual[3], actual[4], actual[5],
            );
            let row = group.streams["forward"].rows[index] as usize;
            assert_eq!(
                format!("minecraft:{}", name(result)),
                reference.rows[row].biome,
                "{} #{index} biome",
                group.id
            );
            values += 6;
            biomes += 1;
        }
    }
    assert_eq!(biomes, 29376);
    println!("{values} exact native router doubles and {biomes} biome resource keys");
}

#[test]
fn captured_4608_pipeline_biomes_keep_native_resource_key_names() {
    let capture: serde_json::Value =
        serde_json::from_str(include_str!("../../data/parity_26_1.json")).unwrap();
    let graph = crate::VanillaGraph::load().unwrap().fork();
    let context = EvalContext {
        seed: capture["seed"].as_i64().unwrap(),
        ..Default::default()
    };
    let mut checked = 0;
    for sample in capture["samples"].as_array().unwrap() {
        for row in sample["biomes"].as_array().unwrap() {
            let x = row[0].as_i64().unwrap() as i32;
            let y = row[1].as_i64().unwrap() as i32;
            let z = row[2].as_i64().unwrap() as i32;
            let actual = graph.noise_biome_at(x >> 2, y >> 2, z >> 2, &context);
            assert_eq!(
                format!("minecraft:{}", name(actual)),
                row[3].as_str().unwrap(),
                "captured biome at ({x},{y},{z})"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 4608);
}
