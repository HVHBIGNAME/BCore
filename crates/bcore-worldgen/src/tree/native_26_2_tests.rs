use super::{
    assert_trace, replay, FallenTreeWorld, Pos, Random, ReplayRandom, Sample, StandingTreeWorld,
};
use crate::tree::extra_data::Provider;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::BTreeSet, sync::OnceLock};

const FIXTURE: &str = include_str!("../../data/tree_semantics_26_2.json");
const TREES: [&str; 6] = [
    "oak",
    "oak_leaf_litter",
    "birch_leaf_litter",
    "fancy_oak_leaf_litter",
    "dark_oak_leaf_litter",
    "azalea_tree",
];
const SCENARIOS: [&str; 6] = [
    "flat",
    "vine_cover",
    "reject_writes",
    "soil_no_rule",
    "soil_nested_no_rule",
    "soil_weighted",
];

#[derive(Deserialize)]
struct Reference {
    schema: String,
    minecraft: String,
    protocol: i32,
    data_version: i32,
    jar_sha256: String,
    probe_sha256: String,
    samples: Vec<Sample>,
    providers: Vec<ProviderSample>,
}

#[derive(Deserialize)]
struct ProviderSample {
    name: String,
    provider: serde_json::Value,
    optional: bool,
    pos: [i32; 3],
    state: Option<u32>,
    read_count: usize,
    draws: Vec<[i32; 3]>,
    next_i64: i64,
}

fn reference() -> &'static Reference {
    static DATA: OnceLock<Reference> = OnceLock::new();
    DATA.get_or_init(|| {
        assert_eq!(
            format!("{:x}", Sha256::digest(FIXTURE.as_bytes())),
            "db5a24307c6e71a18c01e382d4b557f156e0897f6e9014ef6e648ba9da69597b"
        );
        let reference: Reference = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(reference.schema, "bcore.native-tree-semantics.v1");
        assert_eq!(reference.minecraft, bcore_registry::MC_VERSION);
        assert_eq!(reference.protocol, bcore_registry::PROTOCOL_VERSION);
        assert_eq!(reference.data_version, bcore_registry::DATA_VERSION);
        assert_eq!(reference.jar_sha256, bcore_registry::SERVER_JAR_SHA256);
        let mut hash = Sha256::new();
        for source in [
            &include_bytes!("../../../../scripts/TreeReference.java")[..],
            &include_bytes!("../../../../scripts/NativeWorldgenRegistries.java")[..],
            &include_bytes!("../../../../scripts/OreReference.java")[..],
            &include_bytes!("../../../../scripts/VegetationReference.java")[..],
            &include_bytes!("../../../../scripts/ExtraTreeReference.java")[..],
            &include_bytes!("../../../../scripts/migration-26.2/TreeSemanticsReference.java")[..],
        ] {
            hash.update(source);
        }
        assert_eq!(format!("{:x}", hash.finalize()), reference.probe_sha256);
        reference
    })
}

#[test]
fn native_26_2_tree_and_provider_matrix_is_complete_and_authenticated() {
    let reference = reference();
    let mut matrix = BTreeSet::new();
    for sample in &reference.samples {
        assert!(matrix.insert((sample.kind.as_str(), sample.seed, sample.scenario.as_str())));
        assert!(sample.config.is_some());
        assert_eq!(sample.biome.as_deref(), Some("minecraft:forest"));
        assert_eq!(sample.origin, [-1, 65, 16]);
        assert_eq!(sample.floor_y, 64);
        assert_eq!(sample.soil, crate::block::GRASS_BLOCK);
        assert_eq!(sample.cover, crate::block::AIR);
        assert_eq!(sample.results.len(), 1);
        assert!(sample.results[0].next_i64.is_some());
    }
    assert_eq!(reference.samples.len(), 108);
    for tree in TREES {
        for seed in [0, 17, 42] {
            for scenario in SCENARIOS {
                assert!(matrix.contains(&(tree, seed, scenario)));
            }
        }
    }
    let mut providers = BTreeSet::new();
    for sample in &reference.providers {
        assert!(providers.insert((sample.name.as_str(), sample.optional)));
        assert_eq!(sample.pos, [-3, 77, 19]);
    }
    assert_eq!(reference.providers.len(), 12);
    for name in [
        "empty",
        "nested_match",
        "nested_fallback",
        "weighted_one",
        "unknown_property",
        "boolean_property",
    ] {
        for optional in [true, false] {
            assert!(providers.contains(&(name, optional)));
        }
    }
}

fn verify_tree(kind: &str) {
    let samples: Vec<_> = reference()
        .samples
        .iter()
        .filter(|sample| sample.kind == kind)
        .collect();
    assert_eq!(samples.len(), 18, "native 26.2 matrix: {kind}");
    for sample in samples {
        replay::<crate::random::WorldgenRandom>(sample, false);
        replay::<crate::simplex::WorldgenRandom>(sample, false);
    }
}

#[test]
fn native_26_2_oak() {
    verify_tree("oak");
}

#[test]
fn native_26_2_oak_leaf_litter() {
    verify_tree("oak_leaf_litter");
}

#[test]
fn native_26_2_birch_leaf_litter() {
    verify_tree("birch_leaf_litter");
}

#[test]
fn native_26_2_fancy_oak_leaf_litter() {
    verify_tree("fancy_oak_leaf_litter");
}

#[test]
fn native_26_2_dark_oak_leaf_litter() {
    verify_tree("dark_oak_leaf_litter");
}

#[test]
fn native_26_2_azalea() {
    verify_tree("azalea_tree");
}

struct ProviderWorld {
    floor_y: i32,
    reads: RefCell<Vec<Pos>>,
}

impl FallenTreeWorld for ProviderWorld {
    fn get_block(&self, pos: Pos) -> u32 {
        self.reads.borrow_mut().push(pos);
        if pos.1 == self.floor_y {
            crate::block::STONE
        } else {
            crate::block::AIR
        }
    }
    fn set_block(&mut self, _pos: Pos, _state: u32, _flags: i32) -> bool {
        panic!("provider evaluation must not write")
    }
    fn is_face_sturdy_up(&self, _state: u32, _pos: Pos) -> bool {
        panic!("provider evaluation must not query support")
    }
    fn mark_for_postprocessing(&mut self, _pos: Pos) {
        panic!("provider evaluation must not mark blocks")
    }
}

impl StandingTreeWorld for ProviderWorld {
    fn standing_biome(&self, _pos: Pos) -> crate::block_predicate::FeatureResult<u32> {
        Ok(bcore_registry::biome::FOREST)
    }
    fn has_beehive(&mut self, _pos: Pos) -> bool {
        panic!("provider evaluation must not resolve hives")
    }
    fn store_bee(&mut self, _pos: Pos, _ticks: i32) {
        panic!("provider evaluation must not store bees")
    }
    fn schedule_tree_tick(&mut self, _request: [i32; 6]) {
        panic!("provider evaluation must not schedule ticks")
    }
}

fn replay_provider<R: ReplayRandom>(sample: &ProviderSample) {
    let label = format!("26.2 provider {} optional={}", sample.name, sample.optional);
    let provider = Provider::parse(&sample.provider).unwrap();
    let world = ProviderWorld {
        floor_y: sample.pos[1],
        reads: RefCell::default(),
    };
    let mut raw = R::seeded(17);
    let mut draws = Vec::new();
    let mut random = Random {
        raw: &mut raw,
        draws: &mut draws,
    };
    let pos = sample.pos.into();
    let state = if sample.optional {
        provider.sample(&world, &mut random, pos).unwrap()
    } else {
        Some(provider.sample_required(&world, &mut random, pos).unwrap())
    };
    assert_eq!(state, sample.state, "state: {label}");
    assert_eq!(
        *world.reads.borrow(),
        vec![pos; sample.read_count],
        "reads: {label}"
    );
    assert_trace("draws", &draws, &sample.draws, &label);
    assert_eq!(
        raw.next_long(),
        sample.next_i64,
        "RNG continuation: {label}"
    );
}

#[test]
fn native_26_2_optional_required_and_nested_providers() {
    for sample in &reference().providers {
        replay_provider::<crate::random::WorldgenRandom>(sample);
        replay_provider::<crate::simplex::WorldgenRandom>(sample);
    }
}
