use super::{
    fallen::{FallenTreeWorld, Pos},
    standing::{self, StandingTreeWorld, TreeEffects},
    TreeRandom,
};
use crate::{block, MAX_Y, MIN_Y};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Reference {
    jar_sha256: String,
    probe_sha256: String,
    samples: Vec<Sample>,
}
#[derive(Deserialize)]
struct Sample {
    kind: String,
    seed: i64,
    scenario: String,
    origin: [i32; 3],
    floor_y: i32,
    soil: u32,
    cover: u32,
    initial_blocks: Vec<[i32; 4]>,
    writes: Vec<[i32; 4]>,
    write_prefix: Vec<[i32; 6]>,
    write_calls_md5: String,
    postprocessing: Vec<[i32; 3]>,
    tick_requests: Vec<[i32; 6]>,
    draws: Vec<[i32; 3]>,
    results: Vec<Outcome>,
    block_entities: Vec<Entity>,
    write_source: Option<[i32; 2]>,
    raw_brightness: i32,
}
#[derive(Deserialize)]
struct Outcome {
    placed: bool,
    origin: [i32; 3],
    next_i64: Option<i64>,
}
#[derive(Deserialize)]
struct Entity {
    pos: [i32; 3],
    data: serde_json::Value,
}

struct World<'a> {
    sample: &'a Sample,
    initial: BTreeMap<Pos, u32>,
    writes: BTreeMap<Pos, u32>,
    calls: Vec<[i32; 6]>,
    marks: Vec<[i32; 3]>,
    effects: TreeEffects,
}
impl FallenTreeWorld for World<'_> {
    fn get_block(&self, p: Pos) -> u32 {
        if !(MIN_Y..=MAX_Y).contains(&p.1) {
            return block::AIR;
        }
        self.writes
            .get(&p)
            .or_else(|| self.initial.get(&p))
            .copied()
            .unwrap_or_else(|| {
                if p.1 == self.sample.floor_y {
                    self.sample.soil
                } else if p.1 == self.sample.floor_y + 1 {
                    self.sample.cover
                } else {
                    block::AIR
                }
            })
    }
    fn set_block(&mut self, p: Pos, state: u32, flags: i32) -> bool {
        let accepted = !self.sample.scenario.ends_with("reject_writes")
            && (MIN_Y..=MAX_Y).contains(&p.1)
            && self
                .sample
                .write_source
                .is_none_or(|[x, z]| ((p.0 >> 4) - x).abs() <= 1 && ((p.2 >> 4) - z).abs() <= 1);
        self.calls
            .push([p.0, p.1, p.2, state as i32, flags, i32::from(accepted)]);
        if accepted {
            let old = self.get_block(p);
            self.writes.insert(p, state);
            if super::extra_data::block(old).first != super::extra_data::block(state).first {
                self.effects.beehives.remove(&p);
                if (21768..21816).contains(&state) {
                    self.effects.beehives.insert(p, Vec::new());
                }
            }
            if flags & 16 == 0 {
                match state {
                    2336 | 2337 => self.marks.push(p.into()),
                    6998 | 14845 => self.marks.push([p.0, p.1 + 1, p.2]),
                    _ => {}
                }
            }
        }
        accepted
    }
    fn is_face_sturdy_up(&self, state: u32, _p: Pos) -> bool {
        crate::structure::mineshaft::blocks::is_face_sturdy_up(state)
    }
    fn mark_for_postprocessing(&mut self, p: Pos) {
        self.marks.push(p.into());
    }
}
impl StandingTreeWorld for World<'_> {
    fn has_beehive(&mut self, p: Pos) -> bool {
        self.effects.beehives.contains_key(&p)
    }
    fn store_bee(&mut self, p: Pos, age: i32) {
        self.effects.beehives.get_mut(&p).unwrap().push(age);
    }
    fn schedule_tree_tick(&mut self, request: [i32; 6]) {
        self.effects.tick_requests.push(request);
    }
    fn tree_raw_brightness(&self, _p: Pos) -> Option<i32> {
        Some(self.sample.raw_brightness)
    }
}

trait ReplayRandom: TreeRandom {
    fn seeded(seed: i64) -> Self;
    fn next_long(&mut self) -> i64;
}
impl ReplayRandom for crate::random::WorldgenRandom {
    fn seeded(seed: i64) -> Self {
        Self::from_seed(seed as u64)
    }
    fn next_long(&mut self) -> i64 {
        self.next_i64()
    }
}
impl ReplayRandom for crate::simplex::WorldgenRandom {
    fn seeded(seed: i64) -> Self {
        Self::new(seed)
    }
    fn next_long(&mut self) -> i64 {
        self.next_long()
    }
}
struct Random<'a, R> {
    raw: &'a mut R,
    draws: &'a mut Vec<[i32; 3]>,
}
impl<R: TreeRandom> TreeRandom for Random<'_, R> {
    fn next_i32_bounded(&mut self, bound: i32) -> i32 {
        let value = self.raw.next_i32_bounded(bound);
        self.draws.push([0, bound, value]);
        value
    }
    fn next_f32(&mut self) -> f32 {
        let value = self.raw.next_f32();
        self.draws.push([1, 0, value.to_bits() as i32]);
        value
    }
}

fn assert_trace<const N: usize>(
    name: &str,
    actual: &[[i32; N]],
    expected: &[[i32; N]],
    label: &str,
) {
    let index = actual
        .iter()
        .zip(expected)
        .position(|(a, b)| a != b)
        .unwrap_or(actual.len().min(expected.len()));
    assert!(actual==expected,"{name}: {label}; count actual={}, native={}; first difference {index}: actual={:?}, native={:?}",
        actual.len(),expected.len(),&actual[index.saturating_sub(2)..actual.len().min(index+4)],&expected[index.saturating_sub(2)..expected.len().min(index+4)]);
}

fn replay<R: ReplayRandom>(sample: &Sample) {
    let label = format!(
        "{} seed={} {} {:?}",
        sample.kind, sample.seed, sample.scenario, sample.origin
    );
    let mut world = World {
        sample,
        initial: sample
            .initial_blocks
            .iter()
            .map(|&[x, y, z, s]| ((x, y, z), s as u32))
            .collect(),
        writes: BTreeMap::new(),
        calls: Vec::new(),
        marks: Vec::new(),
        effects: TreeEffects::default(),
    };
    let mut raw = R::seeded(sample.seed);
    if sample.scenario.starts_with("advanced_") {
        raw.next_i32_bounded(17);
        raw.next_f32();
        raw.next_i32_bounded(1_073_741_825);
    }
    let mut draws = Vec::new();
    let mut continuation = None;
    for outcome in &sample.results {
        let result = standing::place(
            &mut world,
            &mut Random {
                raw: &mut raw,
                draws: &mut draws,
            },
            &sample.kind,
            outcome.origin.into(),
        )
        .unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(result, Some(outcome.placed), "placement: {label}");
        if let Some(next) = outcome.next_i64 {
            continuation = Some((raw.next_long(), next));
        }
    }
    assert_trace("draws", &draws, &sample.draws, &label);
    assert_eq!(
        continuation.map(|p| p.0),
        continuation.map(|p| p.1),
        "RNG continuation: {label}"
    );
    assert_trace("writes", &world.calls, &sample.write_prefix, &label);
    let mut digest = md5::Context::new();
    for call in &world.calls {
        for value in call {
            digest.consume(value.to_le_bytes());
        }
    }
    assert_eq!(
        format!("{:x}", digest.compute()),
        sample.write_calls_md5,
        "ordered writes: {label}"
    );
    let states: Vec<_> = world
        .writes
        .iter()
        .map(|(&(x, y, z), &s)| [x, y, z, s as i32])
        .collect();
    assert_trace("final states", &states, &sample.writes, &label);
    assert_trace("marks", &world.marks, &sample.postprocessing, &label);
    assert_trace(
        "ticks",
        &world.effects.tick_requests,
        &sample.tick_requests,
        &label,
    );
    assert_eq!(
        world.effects.beehives.len(),
        sample.block_entities.len(),
        "block entity count: {label}"
    );
    for entity in &sample.block_entities {
        assert_eq!(
            world.effects.beehive_data(entity.pos.into()).unwrap(),
            entity.data,
            "native bee NBT: {label}"
        );
    }
}

pub(super) fn probe_sha256(extra_source: &[u8]) -> String {
    let mut sha = Sha256::new();
    for bytes in [
        &include_bytes!("../../../../scripts/TreeReference.java")[..],
        &include_bytes!("../../../../scripts/OreReference.java")[..],
        &include_bytes!("../../../../scripts/NativeWorldgenRegistries.java")[..],
        &include_bytes!("../../../../scripts/VegetationReference.java")[..],
        &include_bytes!("../../../../scripts/ExtraTreeReference.java")[..],
    ] {
        sha.update(bytes);
    }
    sha.update(extra_source);
    format!("{:x}", sha.finalize())
}

fn verify(kinds: &[&str]) {
    let reference: Reference =
        serde_json::from_str(include_str!("../../data/extra_trees_26_1.json")).unwrap();
    assert_eq!(
        reference.jar_sha256,
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(probe_sha256(&[]), reference.probe_sha256);
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../../data/extra_tree_catalog_26_1.json")).unwrap();
    assert_eq!(catalog["jar_sha256"], reference.jar_sha256);
    assert_eq!(catalog["probe_sha256"], reference.probe_sha256);
    for kind in kinds {
        let cases: Vec<_> = reference
            .samples
            .iter()
            .filter(|s| s.kind == *kind)
            .collect();
        let expected = if matches!(*kind, "mangrove" | "tall_mangrove") {
            36
        } else {
            28
        };
        assert_eq!(cases.len(), expected, "native matrix: {kind}");
        for sample in cases {
            replay::<crate::random::WorldgenRandom>(sample);
            replay::<crate::simplex::WorldgenRandom>(sample);
        }
        println!("Verified {expected} full native {kind} cases on both RNG backends");
    }
}

#[test]
fn native_extra_jungle_bush() {
    verify(&["jungle_bush"]);
}
#[test]
fn native_extra_jungle_tree() {
    verify(&["jungle_tree"]);
}
#[test]
fn native_extra_mega_jungle() {
    verify(&["mega_jungle_tree"]);
}
#[test]
fn native_extra_dark_oak() {
    verify(&["dark_oak"]);
}
#[test]
fn native_extra_mega_pine() {
    verify(&["mega_pine"]);
}
#[test]
fn native_extra_mega_spruce() {
    verify(&["mega_spruce"]);
}
#[test]
fn native_extra_azalea() {
    verify(&["azalea_tree"]);
}
#[test]
fn native_extra_cherry() {
    verify(&["cherry", "cherry_bees_005"]);
}
#[test]
fn native_extra_birch_variants() {
    verify(&["birch_bees_002", "birch_bees_005", "birch_leaf_litter"]);
}
#[test]
fn native_extra_oak_variants() {
    verify(&["oak_bees_002", "oak_leaf_litter", "dark_oak_leaf_litter"]);
}
#[test]
fn native_extra_fancy_variants() {
    verify(&[
        "fancy_oak_bees",
        "fancy_oak_bees_002",
        "fancy_oak_leaf_litter",
        "super_birch_bees",
    ]);
}
#[test]
fn native_extra_no_vine_jungle() {
    verify(&["jungle_tree_no_vine"]);
}
#[test]
fn native_extra_swamp_oak() {
    verify(&["swamp_oak"]);
}
#[test]
fn native_extra_pale_oak_bonemeal() {
    verify(&["pale_oak_bonemeal"]);
}
#[test]
fn native_extra_mangrove() {
    verify(&["mangrove"]);
}
#[test]
fn native_extra_tall_mangrove() {
    verify(&["tall_mangrove"]);
}
