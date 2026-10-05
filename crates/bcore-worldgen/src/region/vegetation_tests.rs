use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::FeatureRegion;
use crate::{
    decoration::{place_tree_feature, TreeFeatureWorld, TreeHeightmap},
    simplex::WorldgenRandom,
    tree::{fallen::FallenTreeWorld, TreeRandom},
    ChunkPos, GeneratedChunk, WorldGenerator, MAX_Y, MIN_Y,
};

#[derive(Deserialize)]
struct Reference {
    minecraft: String,
    jar_sha256: String,
    probe_sha256: String,
    roots: Vec<String>,
    configured_features: BTreeMap<String, serde_json::Value>,
    placed_features: BTreeMap<String, serde_json::Value>,
    samples: Vec<Sample>,
}

#[derive(Deserialize)]
struct Sample {
    root: String,
    seed: i64,
    biome: String,
    biome_id: u32,
    terrain: String,
    floor_y: i32,
    soil: u32,
    cover: u32,
    initial_blocks: Vec<[i32; 4]>,
    sources: Vec<[i32; 2]>,
    streams: Vec<Stream>,
    leaves: Vec<Leaf>,
    draws: Vec<[i32; 3]>,
    events_md5: String,
    event_prefix: Vec<Vec<i32>>,
    event_count: usize,
    writes: Vec<[i32; 4]>,
    postprocessing: Vec<[i32; 3]>,
}

#[derive(Deserialize)]
struct Stream {
    source: [i32; 2],
    decoration_seed: i64,
    slot: [i32; 2],
    advance: bool,
    placed: Option<bool>,
    blocked: Option<Leaf>,
    next_i64: i64,
}

#[derive(Deserialize)]
struct Leaf {
    kind: String,
    origin: [i32; 3],
    placement_attempt: Option<usize>,
}

fn reference() -> Reference {
    serde_json::from_str(include_str!("../../data/vegetation_26_1.json")).unwrap()
}

type Chunks = BTreeMap<(i32, i32), Arc<GeneratedChunk>>;

fn fixture_region(sample: &Sample) -> (FeatureRegion, Chunks) {
    let min_x = sample.sources.iter().map(|p| p[0]).min().unwrap() - 1;
    let max_x = sample.sources.iter().map(|p| p[0]).max().unwrap() + 1;
    let min_z = sample.sources.iter().map(|p| p[1]).min().unwrap() - 1;
    let max_z = sample.sources.iter().map(|p| p[1]).max().unwrap() + 1;
    let biome = crate::biome::id(&sample.biome).unwrap();
    let mut chunks = BTreeMap::new();
    for cx in min_x..=max_x {
        for cz in min_z..=max_z {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(cx, cz));
            chunk.noise_biomes = Some(vec![biome; 1536]);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, sample.floor_y, z, sample.soil);
                    chunk.set(x, sample.floor_y + 1, z, sample.cover);
                }
            }
            chunks.insert((cx, cz), Arc::new(chunk));
        }
    }
    for &[x, y, z, state] in &sample.initial_blocks {
        Arc::make_mut(chunks.get_mut(&(x >> 4, z >> 4)).unwrap()).set(
            (x & 15) as usize,
            y,
            (z & 15) as usize,
            state as u32,
        );
    }
    let region = FeatureRegion {
        generator: WorldGenerator::new(sample.seed),
        biome_zoom: crate::biome_zoom::BiomeZoom::new(sample.seed),
        chunks: RefCell::new(chunks.clone()),
        tree_effects: Default::default(),
        light_updates: Vec::new(),
        access: Default::default(),
    };
    (region, chunks)
}

struct AuditedRegion<'a> {
    region: &'a mut FeatureRegion,
    deny_origin: bool,
    events: RefCell<Vec<Vec<i32>>>,
}

impl AuditedRegion<'_> {
    fn event(&self, event: &[i32]) {
        self.events.borrow_mut().push(event.to_vec());
    }
}

impl FallenTreeWorld for AuditedRegion<'_> {
    fn get_block(&self, (x, y, z): (i32, i32, i32)) -> u32 {
        let state = FallenTreeWorld::get_block(self.region, (x, y, z));
        self.event(&[3, x, y, z, state as i32]);
        state
    }

    fn set_block(&mut self, (x, y, z): (i32, i32, i32), state: u32, flags: i32) -> bool {
        let accepted = FallenTreeWorld::set_block(self.region, (x, y, z), state, flags);
        self.event(&[5, x, y, z, state as i32, flags, i32::from(accepted)]);
        accepted
    }

    fn is_face_sturdy_up(&self, state: u32, pos: (i32, i32, i32)) -> bool {
        FallenTreeWorld::is_face_sturdy_up(self.region, state, pos)
    }

    fn mark_for_postprocessing(&mut self, (x, y, z): (i32, i32, i32)) {
        self.event(&[6, x, y, z]);
        FallenTreeWorld::mark_for_postprocessing(self.region, (x, y, z));
    }
}

impl TreeFeatureWorld for AuditedRegion<'_> {
    fn tree_height(&self, heightmap: TreeHeightmap, x: i32, z: i32) -> i32 {
        let height = self.region.tree_height(heightmap, x, z);
        let kind = i32::from(heightmap == TreeHeightmap::WorldSurface);
        self.event(&[1, kind, x, z, height]);
        height
    }

    fn tree_biome(&self, (x, y, z): (i32, i32, i32)) -> u32 {
        let biome = self.region.tree_biome((x, y, z));
        let key = format!("minecraft:{}", crate::biome::name(biome));
        let key_hash = key.encode_utf16().fold(0_i32, |hash, ch| {
            hash.wrapping_mul(31).wrapping_add(i32::from(ch))
        });
        self.event(&[2, x, y, z, key_hash]);
        biome
    }

    fn can_place_tree(&self, (x, y, z): (i32, i32, i32)) -> bool {
        let accepted = !self.deny_origin && self.region.can_place_tree((x, y, z));
        self.event(&[4, x, y, z, i32::from(accepted)]);
        accepted
    }
}

struct AuditedRandom<'a> {
    random: &'a mut WorldgenRandom,
    draws: &'a mut Vec<[i32; 3]>,
}

impl TreeRandom for AuditedRandom<'_> {
    fn next_i32_bounded(&mut self, bound: i32) -> i32 {
        let value = self.random.next_int(bound as usize) as i32;
        self.draws.push([0, bound, value]);
        value
    }

    fn next_f32(&mut self) -> f32 {
        let value = self.random.next_float();
        self.draws.push([1, 0, value.to_bits() as i32]);
        value
    }
}

fn replay(sample: &Sample, region: &mut FeatureRegion) {
    let label = format!(
        "{} seed={} {} {:?}",
        sample.root, sample.seed, sample.terrain, sample.sources
    );
    let mut world = AuditedRegion {
        region,
        deny_origin: sample.terrain == "deny_origin",
        events: RefCell::new(Vec::new()),
    };
    let mut draws = Vec::new();
    for (index, stream) in sample.streams.iter().enumerate() {
        assert_eq!(
            stream.source, sample.sources[index],
            "source order: {label}"
        );
        let [cx, cz] = stream.source;
        let mut random = WorldgenRandom::new(sample.seed);
        let decoration_seed =
            random.set_decoration_seed(sample.seed, cx.wrapping_mul(16), cz.wrapping_mul(16));
        assert_eq!(
            decoration_seed, stream.decoration_seed,
            "decoration seed: {label}"
        );
        let [step, feature] = stream.slot;
        assert_eq!(
            crate::feature_sorter::sorter().within_step_index(&sample.root),
            Some((step as usize, feature as usize)),
            "native feature slot: {label}"
        );
        random.set_feature_seed(decoration_seed, feature, step);
        if stream.advance {
            random.next_int(17);
            random.next_float();
            random.next_int(1_073_741_825);
        }
        let result = place_tree_feature(
            &mut world,
            &mut AuditedRandom {
                random: &mut random,
                draws: &mut draws,
            },
            ChunkPos::new(cx, cz),
            &sample.root,
        );
        if let Some(blocked) = &stream.blocked {
            let error = result.expect_err(&label);
            assert_eq!(
                error.configured_feature, blocked.kind,
                "blocked child: {label}"
            );
            assert_eq!(
                <[i32; 3]>::from(error.origin),
                blocked.origin,
                "blocked origin: {label}"
            );
            assert!(stream.placed.is_none());
            assert_eq!(index + 1, sample.streams.len(), "replay must stop: {label}");
        } else {
            assert_eq!(
                result.unwrap(),
                stream.placed,
                "native placement result: {label}"
            );
        }
        assert_eq!(
            random.next_long(),
            stream.next_i64,
            "native RNG continuation: {label}"
        );
    }
    assert_eq!(draws, sample.draws, "every bounded/float draw: {label}");
    let events = world.events.into_inner();
    assert_eq!(
        &events[..events.len().min(8)],
        sample.event_prefix,
        "ordered event prefix: {label}"
    );
    assert_eq!(
        events.len(),
        sample.event_count,
        "world event count: {label}"
    );
    let mut digest = md5::Context::new();
    for event in events {
        for value in event {
            digest.consume(value.to_le_bytes());
        }
    }
    assert_eq!(
        format!("{:x}", digest.compute()),
        sample.events_md5,
        "ordered world events: {label}"
    );
}

fn expected_chunks(sample: &Sample, mut bases: Chunks) -> Chunks {
    for &[x, y, z, state] in &sample.writes {
        assert!((MIN_Y..=MAX_Y).contains(&y));
        Arc::make_mut(bases.get_mut(&(x >> 4, z >> 4)).unwrap()).set(
            (x & 15) as usize,
            y,
            (z & 15) as usize,
            state as u32,
        );
    }
    for &[x, y, z] in &sample.postprocessing {
        Arc::make_mut(bases.get_mut(&(x >> 4, z >> 4)).unwrap())
            .postprocessing
            .push(((x & 15) as usize, y, (z & 15) as usize));
    }
    bases
}

#[test]
fn native_placed_tree_streams_match_filters_selectors_fallen_writes_and_rng() {
    let reference = reference();
    assert_eq!(reference.minecraft, "26.1");
    assert_eq!(
        reference.jar_sha256,
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    let mut hash = Sha256::new();
    for source in [
        &include_bytes!("../../../../scripts/TreeReference.java")[..],
        &include_bytes!("../../../../scripts/OreReference.java")[..],
        &include_bytes!("../../../../scripts/NativeWorldgenRegistries.java")[..],
        &include_bytes!("../../../../scripts/vegetation-bounded/VegetationReference.java")[..],
    ] {
        hash.update(source);
    }
    assert_eq!(format!("{:x}", hash.finalize()), reference.probe_sha256);
    assert_eq!(reference.roots.len(), 10);
    assert_eq!(reference.samples.len(), 504);
    let mut complete = 0;
    let mut blocked = 0;
    let mut biome_id_differences = std::collections::BTreeSet::new();
    for sample in &reference.samples {
        let bundled_id = crate::biome::id(&sample.biome).unwrap();
        if bundled_id != sample.biome_id {
            biome_id_differences.insert((&sample.biome, bundled_id, sample.biome_id));
        }
        let (mut region, bases) = fixture_region(sample);
        let before: BTreeMap<_, _> = bases
            .iter()
            .map(|(&p, c)| (p, c.as_ref().clone()))
            .collect();
        replay(sample, &mut region);
        assert_eq!(
            *region.chunks.borrow(),
            expected_chunks(sample, bases.clone()),
            "{} {} {}",
            sample.root,
            sample.seed,
            sample.terrain
        );
        for (pos, base) in &bases {
            assert_eq!(
                base.as_ref(),
                &before[pos],
                "immutable terrain base {pos:?}"
            );
        }
        if sample.streams.last().unwrap().blocked.is_some() {
            blocked += 1;
        } else {
            assert_eq!(sample.streams.len(), sample.sources.len());
            complete += 1;
        }
    }
    assert!(complete > 0 && blocked > 0);
    println!(
        "native vegetation: {complete} complete samples, {blocked} explicitly blocked prefixes"
    );
    for (biome, bundled, native) in biome_id_differences {
        println!("named-biome fixture {biome}: bundled id={bundled}, native id={native}; raw registry parity is outside this fixture");
    }
}

#[test]
fn independent_targets_keep_incoming_writes_when_replaying_the_same_native_source_plan() {
    let reference = reference();
    let mut incoming_targets = 0;
    let mut fallen_kinds = std::collections::BTreeSet::new();
    for sample in reference.samples.iter().filter(|s| s.sources.len() == 9) {
        assert!(sample.streams.iter().all(|s| s.blocked.is_none()));
        let (mut together, bases) = fixture_region(sample);
        replay(sample, &mut together);
        let expected = expected_chunks(sample, bases);
        let targets: std::collections::BTreeSet<_> = sample
            .writes
            .iter()
            .map(|&[x, _, z, _]| (x >> 4, z >> 4))
            .collect();
        for leaf in &sample.leaves {
            assert!(leaf.kind.starts_with("fallen_"));
            fallen_kinds.insert(leaf.kind.clone());
        }
        for target in targets {
            // A fresh region and all nine full streams for each requested target.
            // This tests a fixed explicit plan, not target-relative ChunkPyramid
            // dependency discovery or finish_chunk's legacy vegetation pass.
            let (mut independent, _) = fixture_region(sample);
            replay(sample, &mut independent);
            assert_eq!(independent.chunks.borrow()[&target], expected[&target]);
            assert_eq!(
                independent.chunks.borrow()[&target],
                together.chunks.borrow()[&target]
            );
            if target != (0, 0) {
                incoming_targets += 1;
            }
        }
    }
    assert_eq!(fallen_kinds.len(), 5);
    assert!(
        incoming_targets > 0,
        "native cross-chunk writes must be exercised"
    );
    println!("native vegetation: {incoming_targets} independently replayed incoming targets");
}

#[test]
fn unknown_tree_feature_does_not_consume_or_mutate_a_live_stream() {
    let reference = reference();
    let (mut region, bases) = fixture_region(&reference.samples[0]);
    let mut random = WorldgenRandom::new(42);
    let mut expected = WorldgenRandom::new(42);
    assert_eq!(
        place_tree_feature(
            &mut region,
            &mut random,
            ChunkPos::new(0, 0),
            "unknown_tree"
        ),
        Ok(None)
    );
    assert_eq!(random.next_long(), expected.next_long());
    assert_eq!(*region.chunks.borrow(), bases);
}

#[test]
fn native_fixture_covers_every_child_and_delayed_incoming_attempts() {
    use std::collections::BTreeSet;
    let reference = reference();
    for root in &reference.roots {
        let config_name = reference.placed_features[root]["feature"]
            .as_str()
            .unwrap()
            .strip_prefix("minecraft:")
            .unwrap();
        let config = &reference.configured_features[config_name]["config"];
        let mut expected = BTreeSet::new();
        for child in config["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| &c["feature"])
            .chain(std::iter::once(&config["default"]))
        {
            let placed = if let Some(id) = child.as_str() {
                &reference.placed_features[id.strip_prefix("minecraft:").unwrap()]
            } else {
                child
            };
            expected.insert(
                placed["feature"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("minecraft:")
                    .unwrap(),
            );
        }
        let actual: BTreeSet<_> = reference
            .samples
            .iter()
            .filter(|s| &s.root == root)
            .flat_map(|s| s.leaves.iter().map(|leaf| leaf.kind.as_str()))
            .collect();
        assert_eq!(actual, expected, "native child coverage for {root}");
    }
    assert!(
        reference.samples.iter().any(|s| {
            s.streams.last().unwrap().blocked.is_some()
                && s.leaves.iter().any(|leaf| leaf.kind.starts_with("fallen_"))
                && !s.writes.is_empty()
        }),
        "the stop boundary must preserve preceding fallen-tree effects"
    );
    let delayed = reference
        .samples
        .iter()
        .find(|s| s.terrain == "delayed_single_support" && s.sources.len() == 1)
        .expect("native incoming tree after rejected attempts");
    assert!(delayed.streams.iter().all(|s| s.blocked.is_none()));
    assert!(delayed.leaves[0].placement_attempt.unwrap() >= 3);
    assert!(delayed
        .writes
        .iter()
        .any(|&[x, _, z, _]| (x >> 4, z >> 4) != (0, 0)));
}
