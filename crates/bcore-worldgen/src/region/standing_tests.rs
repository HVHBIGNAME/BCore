use super::FeatureRegion;
use crate::{
    decoration::{self, TreeFeatureWorld, TreeHeightmap, UnsupportedTree},
    simplex::WorldgenRandom,
    tick_request::TickTarget,
    tree::{
        fallen::{FallenTreeWorld, Pos},
        standing::{self, StandingTreeWorld},
        TreeRandom,
    },
    ChunkPos, GeneratedChunk, WorldGenerator,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Deserialize)]
struct Reference {
    version: u32,
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
    biome: String,
    initial_blocks: Vec<[i32; 4]>,
    sources: Vec<[i32; 2]>,
    results: Vec<Outcome>,
    writes: Vec<[i32; 4]>,
    write_count: usize,
    standing_log_writes: usize,
    fallen_log_writes: usize,
    write_calls_md5: String,
    write_prefix: Vec<[i32; 6]>,
    postprocessing: Vec<[i32; 3]>,
    tick_requests: Vec<[i32; 6]>,
    draws: Vec<[i32; 3]>,
    block_entities: Vec<Entity>,
}
#[derive(Deserialize)]
struct Outcome {
    placed: bool,
    next_i64: Option<i64>,
    origin: Option<[i32; 3]>,
    decoration_seed: Option<i64>,
    slot: Option<[i32; 2]>,
}
#[derive(Deserialize)]
struct Entity {
    pos: [i32; 3],
    data: serde_json::Value,
}

fn reference() -> Reference {
    serde_json::from_str(include_str!("../../data/standing_trees_26_1.json")).unwrap()
}
type Chunks = BTreeMap<(i32, i32), Arc<GeneratedChunk>>;

fn region(sample: &Sample) -> (FeatureRegion, Chunks) {
    let sources = if sample.sources.is_empty() {
        vec![[sample.origin[0] >> 4, sample.origin[2] >> 4]]
    } else {
        sample.sources.clone()
    };
    let xmin = sources.iter().map(|p| p[0]).min().unwrap() - 2;
    let xmax = sources.iter().map(|p| p[0]).max().unwrap() + 2;
    let zmin = sources.iter().map(|p| p[1]).min().unwrap() - 2;
    let zmax = sources.iter().map(|p| p[1]).max().unwrap() + 2;
    let mut chunks = BTreeMap::new();
    for cx in xmin..=xmax {
        for cz in zmin..=zmax {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(cx, cz));
            chunk.noise_biomes = Some(vec![crate::biome::id(&sample.biome).unwrap(); 1536]);
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
    (
        FeatureRegion {
            generator: WorldGenerator::new(sample.seed),
            biome_zoom: crate::biome_zoom::BiomeZoom::new(sample.seed),
            chunks: RefCell::new(chunks.clone()),
            tree_effects: Default::default(),
            light_updates: Vec::new(),
            access: Default::default(),
        },
        chunks,
    )
}

struct World<'a> {
    region: &'a mut FeatureRegion,
    reject: bool,
    writes: Vec<[i32; 6]>,
}
impl FallenTreeWorld for World<'_> {
    fn get_block(&self, pos: Pos) -> u32 {
        FallenTreeWorld::get_block(self.region, pos)
    }
    fn set_block(&mut self, (x, y, z): Pos, state: u32, flags: i32) -> bool {
        let accepted =
            !self.reject && FallenTreeWorld::set_block(self.region, (x, y, z), state, flags);
        self.writes
            .push([x, y, z, state as i32, flags, i32::from(accepted)]);
        accepted
    }
    fn is_face_sturdy_up(&self, state: u32, pos: Pos) -> bool {
        FallenTreeWorld::is_face_sturdy_up(self.region, state, pos)
    }
    fn mark_for_postprocessing(&mut self, pos: Pos) {
        FallenTreeWorld::mark_for_postprocessing(self.region, pos);
    }
}
impl StandingTreeWorld for World<'_> {
    fn has_beehive(&mut self, pos: Pos) -> bool {
        self.region.has_beehive(pos)
    }
    fn store_bee(&mut self, pos: Pos, ticks: i32) {
        self.region.store_bee(pos, ticks);
    }
    fn schedule_tree_tick(&mut self, request: [i32; 6]) {
        self.region.schedule_tree_tick(request);
    }
}
impl TreeFeatureWorld for World<'_> {
    fn tree_height(&self, kind: TreeHeightmap, x: i32, z: i32) -> i32 {
        self.region.tree_height(kind, x, z)
    }
    fn tree_biome(&self, pos: Pos) -> u32 {
        self.region.tree_biome(pos)
    }
    fn can_place_tree(&self, pos: Pos) -> bool {
        self.region.can_place_tree(pos)
    }
    fn place_standing_tree<R: TreeRandom + ?Sized>(
        &mut self,
        random: &mut R,
        configured_feature: &'static str,
        origin: Pos,
    ) -> Result<bool, UnsupportedTree> {
        standing::place(self, random, configured_feature, origin)
            .map_err(|shape| UnsupportedTree {
                configured_feature,
                origin,
                unsupported_shape: Some(shape),
            })?
            .ok_or(UnsupportedTree {
                configured_feature,
                origin,
                unsupported_shape: None,
            })
    }
}

trait ReplayRandom: TreeRandom {
    fn new(seed: i64) -> Self;
    fn next_long(&mut self) -> i64;
    fn seed_feature(&mut self, seed: i64, source: [i32; 2], slot: [i32; 2]) -> i64;
}

impl ReplayRandom for WorldgenRandom {
    fn new(seed: i64) -> Self {
        Self::new(seed)
    }
    fn next_long(&mut self) -> i64 {
        self.next_long()
    }
    fn seed_feature(&mut self, seed: i64, [cx, cz]: [i32; 2], [step, feature]: [i32; 2]) -> i64 {
        let seed = self.set_decoration_seed(seed, cx * 16, cz * 16);
        self.set_feature_seed(seed, feature, step);
        seed
    }
}

impl ReplayRandom for crate::random::WorldgenRandom {
    fn new(seed: i64) -> Self {
        Self::from_seed(seed as u64)
    }
    fn next_long(&mut self) -> i64 {
        self.next_i64()
    }
    fn seed_feature(&mut self, seed: i64, [cx, cz]: [i32; 2], [step, feature]: [i32; 2]) -> i64 {
        let seed = self.set_decoration_seed(seed, cx * 16, cz * 16);
        self.set_feature_seed(seed, feature, step);
        seed
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

fn run<R: ReplayRandom>(sample: &Sample, region: &mut FeatureRegion) {
    let label = format!(
        "{} seed={} {} {:?}",
        sample.kind, sample.seed, sample.scenario, sample.origin
    );
    let mut world = World {
        region,
        reject: sample.scenario.ends_with("reject_writes"),
        writes: Vec::new(),
    };
    let mut draws = Vec::new();
    let mut raw = R::new(sample.seed);
    if sample.scenario.starts_with("advanced_") {
        raw.next_i32_bounded(17);
        raw.next_f32();
        raw.next_i32_bounded(1_073_741_825);
    }
    for (index, outcome) in sample.results.iter().enumerate() {
        let result = if sample.sources.is_empty() {
            standing::place(
                &mut world,
                &mut Random {
                    raw: &mut raw,
                    draws: &mut draws,
                },
                &sample.kind,
                outcome.origin.unwrap().into(),
            )
            .unwrap_or_else(|e| panic!("{label}: {e}"))
            .unwrap()
        } else {
            let [cx, cz] = sample.sources[index];
            let [step, feature] = outcome.slot.unwrap();
            let seed = raw.seed_feature(sample.seed, [cx, cz], [step, feature]);
            assert_eq!(Some(seed), outcome.decoration_seed, "{label}");
            assert_eq!(
                crate::feature_sorter::sorter().within_step_index(&sample.kind),
                Some((step as usize, feature as usize))
            );
            decoration::place_tree_feature(
                &mut world,
                &mut Random {
                    raw: &mut raw,
                    draws: &mut draws,
                },
                ChunkPos::new(cx, cz),
                &sample.kind,
            )
            .unwrap_or_else(|e| panic!("{label}: {e}"))
            .unwrap()
        };
        assert_eq!(result, outcome.placed, "return: {label}");
        if let Some(expected) = outcome.next_i64 {
            assert_eq!(raw.next_long(), expected, "RNG continuation: {label}");
        }
    }
    if draws != sample.draws {
        let first = draws.iter().zip(&sample.draws).position(|(a, b)| a != b);
        panic!(
            "draw trace: {label}; actual={}, expected={}, first difference={first:?}",
            draws.len(),
            sample.draws.len()
        );
    }
    if let Some(index) = world
        .writes
        .iter()
        .zip(&sample.write_prefix)
        .position(|(a, b)| a != b)
    {
        panic!(
            "write prefix: {label}, index={index}, actual={:?}, expected={:?}",
            &world.writes[index.saturating_sub(2)..world.writes.len().min(index + 4)],
            &sample.write_prefix[index.saturating_sub(2)..sample.write_prefix.len().min(index + 4)]
        );
    }
    assert!(
        world.writes.len() >= sample.write_prefix.len(),
        "truncated writes: {label}"
    );
    assert_eq!(
        world.writes.len(),
        sample.write_count,
        "write count: {label}"
    );
    let mut digest = md5::Context::new();
    for row in &world.writes {
        for value in row {
            digest.consume(value.to_le_bytes());
        }
    }
    assert_eq!(
        format!("{:x}", digest.compute()),
        sample.write_calls_md5,
        "ordered writes/flags: {label}"
    );
    assert_effects(sample, world.region);
}

fn assert_effects(sample: &Sample, region: &FeatureRegion) {
    let label = format!("{} {} {}", sample.kind, sample.seed, sample.scenario);
    let effects = &region.tree_effects;
    assert_eq!(
        effects.tick_requests, sample.tick_requests,
        "native tick requests: {label}"
    );
    assert_eq!(
        effects.beehives.len(),
        sample.block_entities.len(),
        "native bee nest count: {label}"
    );
    for entity in &sample.block_entities {
        assert_eq!(
            effects.beehive_data(entity.pos.into()).unwrap(),
            entity.data,
            "complete native bee metadata: {label}"
        );
    }
}

fn assert_chunk_effects(sample: &Sample, chunk: &GeneratedChunk) {
    let expected_ticks: Vec<_> = sample
        .tick_requests
        .iter()
        .copied()
        .filter(|p| p[0] >> 4 == chunk.pos.x && p[2] >> 4 == chunk.pos.z)
        .collect();
    let actual_ticks: Vec<_> = chunk
        .tick_requests()
        .iter()
        .map(|request| {
            assert!(request.valid_for(chunk.pos));
            let [x, y, z] = request.block_pos;
            let (value, fluid) = match request.target {
                TickTarget::Block(value) => (value as i32, 0),
                TickTarget::Fluid(value) => (value as i32, 1),
            };
            [x, y, z, value, request.delay, fluid]
        })
        .collect();
    assert_eq!(
        actual_ticks, expected_ticks,
        "stored native requests {:?}: {} {} {}",
        chunk.pos, sample.kind, sample.seed, sample.scenario
    );
    let expected_bees: BTreeMap<_, _> = sample
        .block_entities
        .iter()
        .filter(|entity| entity.pos[0] >> 4 == chunk.pos.x && entity.pos[2] >> 4 == chunk.pos.z)
        .map(|entity| (entity.pos, entity.data.clone()))
        .collect();
    let actual_bees: BTreeMap<_, _> = chunk
        .block_entities()
        .iter()
        .map(|(&(x, y, z), data)| {
            let pos = [chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32];
            assert_eq!(data.type_id(), 34);
            assert_eq!(data.update_data(), serde_json::json!({}));
            (pos, data.full_data(pos.into()))
        })
        .collect();
    assert_eq!(
        actual_bees, expected_bees,
        "stored native bee NBT {:?}: {} {} {}",
        chunk.pos, sample.kind, sample.seed, sample.scenario
    );
}

fn effect_target(sample: &Sample) -> ChunkPos {
    let [x, _, z] = sample
        .block_entities
        .first()
        .map(|entity| entity.pos)
        .or_else(|| sample.tick_requests.first().map(|p| [p[0], p[1], p[2]]))
        .unwrap_or(sample.origin);
    ChunkPos::new(x >> 4, z >> 4)
}

fn expected_chunks(sample: &Sample, bases: &Chunks) -> Chunks {
    let mut expected = bases.clone();
    for &[x, y, z, state] in &sample.writes {
        Arc::make_mut(expected.get_mut(&(x >> 4, z >> 4)).unwrap()).set(
            (x & 15) as usize,
            y,
            (z & 15) as usize,
            state as u32,
        );
    }
    for &[x, y, z] in &sample.postprocessing {
        Arc::make_mut(expected.get_mut(&(x >> 4, z >> 4)).unwrap())
            .postprocessing
            .push(((x & 15) as usize, y, (z & 15) as usize));
    }
    expected
}

fn assert_chunks(sample: &Sample, region: &FeatureRegion, expected: &Chunks) {
    let actual = region.chunks.borrow();
    assert_eq!(
        actual.len(),
        expected.len(),
        "unexpected terrain request: {} {}",
        sample.kind,
        sample.seed
    );
    for (&pos, chunk) in actual.iter() {
        assert_eq!(
            chunk.as_ref(),
            expected[&pos].as_ref(),
            "full chunk {pos:?}: {} {} {}",
            sample.kind,
            sample.seed,
            sample.scenario
        );
    }
}

fn probe_sha256() -> String {
    let mut sha = Sha256::new();
    for bytes in [
        &include_bytes!("../../../../scripts/TreeReference.java")[..],
        &include_bytes!("../../../../scripts/OreReference.java")[..],
        &include_bytes!("../../../../scripts/NativeWorldgenRegistries.java")[..],
        &include_bytes!("../../../../scripts/VegetationReference.java")[..],
    ] {
        sha.update(bytes);
    }
    format!("{:x}", sha.finalize())
}

#[test]
fn complete_native_standing_and_mixed_selectors_match_live_region() {
    let reference = reference();
    assert_eq!(reference.version, 2);
    assert_eq!(
        reference.jar_sha256,
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(probe_sha256(), reference.probe_sha256);
    for sample in &reference.samples {
        let (mut region, bases) = region(sample);
        let before: BTreeMap<_, _> = bases
            .iter()
            .map(|(&pos, c)| (pos, c.as_ref().clone()))
            .collect();
        let expected = expected_chunks(sample, &bases);
        run::<WorldgenRandom>(sample, &mut region);
        assert_chunks(sample, &region, &expected);
        region.transfer_tree_effects().unwrap();
        assert_eq!(region.tree_effects, Default::default());
        for (&pos, chunk) in region.chunks.borrow().iter() {
            assert_chunk_effects(sample, chunk);
            assert_eq!(chunk.states(), expected[&pos].states());
            assert_eq!(chunk.postprocessing, expected[&pos].postprocessing);
        }
        let transferred = region.chunks.borrow().clone();
        region.transfer_tree_effects().unwrap();
        for (&pos, chunk) in region.chunks.borrow().iter() {
            assert!(
                Arc::ptr_eq(chunk, &transferred[&pos]),
                "empty retransfer cloned {pos:?}"
            );
        }
        *region.chunks.get_mut() = bases.clone();
        region.tree_effects = Default::default();
        run::<crate::random::WorldgenRandom>(sample, &mut region);
        assert_chunks(sample, &region, &expected);
        let finalized = region.finish_chunk(effect_target(sample));
        assert_chunk_effects(sample, &finalized);
        for &pos in bases.keys() {
            assert_eq!(
                bases[&pos].as_ref(),
                &before[&pos],
                "immutable terrain {pos:?}"
            );
        }
    }
    println!(
        "Verified {} complete native standing/placed-tree samples on both RNG backends, with owning-chunk transfer, idempotent retransfer and finalization",
        reference.samples.len()
    );
}

#[test]
fn native_matrix_covers_mixed_branches_and_retains_source_order() {
    let reference = reference();
    let mut kinds = BTreeSet::new();
    let mut roots = BTreeSet::new();
    let mut standing_roots = BTreeSet::new();
    let mut fallen_roots = BTreeSet::new();
    let mut mixed_roots = BTreeSet::new();
    let mut scenarios = BTreeSet::new();
    let mut bees = 0;
    let mut multiple_sources = 0;
    for sample in &reference.samples {
        bees += sample.block_entities.len();
        if sample.sources.is_empty() {
            kinds.insert(sample.kind.as_str());
            scenarios.insert(sample.scenario.as_str());
            assert_eq!(
                sample
                    .results
                    .iter()
                    .filter(|r| r.next_i64.is_some())
                    .count(),
                1
            );
            if sample.scenario == "repeat_live" {
                assert_eq!(sample.results.len(), 3);
            }
        } else {
            roots.insert(sample.kind.as_str());
            assert_eq!(sample.sources.len(), sample.results.len());
            if sample.standing_log_writes > 0 {
                standing_roots.insert(sample.kind.as_str());
            }
            if sample.fallen_log_writes > 0 {
                fallen_roots.insert(sample.kind.as_str());
            }
            if sample.standing_log_writes > 0 && sample.fallen_log_writes > 0 {
                mixed_roots.insert(sample.kind.as_str());
            }
            if sample.sources.len() > 1 {
                assert_eq!(sample.sources.len(), 9);
                multiple_sources += 1;
            }
        }
    }
    println!("Complete native mixed standing/fallen roots: {mixed_roots:?}; bee nests: {bees}");
    assert_eq!(kinds.len(), 13);
    assert_eq!(roots.len(), 8);
    assert_eq!(standing_roots, roots);
    assert_eq!(fallen_roots, roots);
    assert_eq!(mixed_roots.len(), 7);
    assert_eq!(multiple_sources, 9);
    assert!(bees > 0);
    for name in [
        "obstructed",
        "wet_foliage",
        "reject_writes",
        "flowing_water",
        "persistent_leaf",
        "vine_cover",
        "advanced_stream",
        "advanced_reject_writes",
        "repeat_live",
        "existing_logs",
    ] {
        assert!(scenarios.contains(name), "missing native scenario {name}");
    }
    let plans: Vec<_> = reference
        .samples
        .iter()
        .filter(|s| {
            s.kind == "trees_birch_and_oak_leaf_litter" && s.seed == 49 && s.sources.len() == 9
        })
        .collect();
    assert_eq!(plans.len(), 2);
    assert!(plans[0].sources.iter().eq(plans[1].sources.iter().rev()));
    assert_ne!(plans[0].write_calls_md5, plans[1].write_calls_md5);
    let blocks: serde_json::Value =
        serde_json::from_str(include_str!("../../data/standing_blocks_26_1.json")).unwrap();
    assert_eq!(blocks["jar_sha256"], reference.jar_sha256);
    assert_eq!(blocks["probe_sha256"], probe_sha256());
    assert_eq!(blocks["state_count"], 29_873);
    assert_eq!(
        blocks["configurations"].as_object().unwrap().len(),
        kinds.len()
    );
    let successful_bees = reference
        .samples
        .iter()
        .find(|s| s.scenario == "advanced_stream" && !s.block_entities.is_empty())
        .unwrap();
    let rejected_bees = reference
        .samples
        .iter()
        .find(|s| {
            s.kind == successful_bees.kind
                && s.seed == successful_bees.seed
                && s.scenario == "advanced_reject_writes"
        })
        .unwrap();
    assert!(rejected_bees.block_entities.is_empty());
    assert_eq!(
        &successful_bees.draws[..rejected_bees.draws.len()],
        rejected_bees.draws
    );
    assert_eq!(
        successful_bees.draws.len() - rejected_bees.draws.len(),
        successful_bees.block_entities[0].data["bees"]
            .as_array()
            .unwrap()
            .len()
            + 1
    );
    let no_attachment = reference
        .samples
        .iter()
        .find(|s| s.scenario == "existing_logs")
        .unwrap();
    assert!(no_attachment.results[0].placed);
    assert_eq!(no_attachment.writes.len(), 1);
}

fn replay_sources(sample: &Sample, region: &mut FeatureRegion) {
    for (&source, outcome) in sample.sources.iter().zip(&sample.results) {
        let mut random = WorldgenRandom::new(sample.seed);
        let seed = random.seed_feature(sample.seed, source, outcome.slot.unwrap());
        assert_eq!(Some(seed), outcome.decoration_seed);
        assert_eq!(
            decoration::place_tree_feature(
                region,
                &mut random,
                ChunkPos::new(source[0], source[1]),
                &sample.kind
            )
            .unwrap(),
            Some(outcome.placed),
            "production region dispatch: {} {} {source:?}",
            sample.kind,
            sample.seed
        );
        assert_eq!(
            Some(random.next_long()),
            outcome.next_i64,
            "production region RNG: {} {} {source:?}",
            sample.kind,
            sample.seed
        );
    }
}

#[test]
fn fixed_source_plan_replays_complete_stream_for_each_requested_target() {
    let reference = reference();
    let mut checked = 0;
    let mut incoming = 0;
    for sample in reference.samples.iter().filter(|s| s.sources.len() > 1) {
        let (_, bases) = region(sample);
        let expected = expected_chunks(sample, &bases);
        let targets: BTreeSet<_> = sample
            .writes
            .iter()
            .map(|p| (p[0] >> 4, p[2] >> 4))
            .collect();
        for (cx, cz) in targets {
            let mut actual =
                FeatureRegion::new(WorldGenerator::new(sample.seed), bases[&(cx, cz)].clone());
            actual.chunks.get_mut().extend(bases.clone());
            replay_sources(sample, &mut actual);
            assert_chunks(sample, &actual, &expected);
            assert_effects(sample, &actual);
            let finalized = actual.finish_chunk(ChunkPos::new(cx, cz));
            assert_chunk_effects(sample, &finalized);
            incoming += usize::from(!sample.sources.contains(&[cx, cz]));
            checked += 1;
        }
    }
    assert!(incoming > 0, "no independently requested incoming target");
    println!("Verified {checked} independent finalized targets, including {incoming} incoming-only targets, with complete source plans and stored native effects");
}

#[test]
fn unsupported_configurations_and_edge_effects_remain_explicit() {
    let reference = reference();
    let sample = &reference.samples[0];
    let (mut region, bases) = region(sample);
    let mut random = WorldgenRandom::new(42);
    let mut control = WorldgenRandom::new(42);
    random.next_int(31);
    control.next_int(31);
    for name in ["minecraft:unknown_tree", "unknown_tree"] {
        assert_eq!(
            standing::place(&mut region, &mut random, name, sample.origin.into()),
            Ok(None)
        );
    }
    assert_eq!(random.next_long(), control.next_long());
    assert_chunks(sample, &region, &bases);
    assert_eq!(region.tree_effects, Default::default());

    let [x, y, z, _] = sample.writes.iter().min_by_key(|p| p[0]).copied().unwrap();
    let edge = (x - 1, y, z);
    FallenTreeWorld::set_block(&mut region, edge, crate::block::LAVA, 19);
    let error = standing::place(
        &mut region,
        &mut WorldgenRandom::new(sample.seed),
        &sample.kind,
        sample.origin.into(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        standing::UnsupportedShape {
            pos: edge,
            state: crate::block::LAVA
        }
    );
    assert_eq!(
        FallenTreeWorld::get_block(&region, sample.origin.into()),
        crate::block::OAK_LOG
    );
}

#[test]
fn native_beehive_metadata_survives_finalization_and_noop_postprocessing() {
    let reference = reference();
    let sample = reference
        .samples
        .iter()
        .find(|s| !s.block_entities.is_empty())
        .unwrap();
    let (mut region, bases) = region(sample);
    run::<WorldgenRandom>(sample, &mut region);
    let [x, y, z] = sample.block_entities[0].pos;
    FallenTreeWorld::mark_for_postprocessing(&mut region, (x, y, z));
    FallenTreeWorld::mark_for_postprocessing(&mut region, (x, y, z));
    let finalized = region.finish_chunk(ChunkPos::new(x >> 4, z >> 4));
    assert_chunk_effects(sample, &finalized);
    assert!(finalized.postprocessing.is_empty());
    assert!(bases
        .values()
        .all(|c| c.block_entities().is_empty() && c.tick_requests().is_empty()));
}
