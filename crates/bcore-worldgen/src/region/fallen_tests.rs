use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use super::{postprocess_on_write, FeatureRegion};
use crate::{
    block,
    tree::fallen::{self, FallenTreeWorld as TreeWorld, Pos},
    ChunkPos, GeneratedChunk, WorldGenerator, MAX_Y, MIN_Y,
};

#[derive(serde::Deserialize)]
struct Reference {
    jar_sha256: String,
    samples: Vec<Sample>,
}

#[derive(serde::Deserialize)]
struct Sample {
    kind: String,
    seed: i64,
    flat_ground: bool,
    placed: bool,
    states_md5: String,
    next_i64: i64,
    origin: [i32; 3],
    chunk: [i32; 2],
    floor_y: i32,
    soil: u32,
    terrain: String,
    initial_blocks: Vec<[i32; 4]>,
    writes: Vec<[i32; 4]>,
    read_count: usize,
    reads_md5: String,
    write_count: usize,
    write_calls_md5: String,
    postprocessing: Vec<[i32; 3]>,
    cached_up_support: Vec<(u32, bool)>,
}

fn reference() -> Reference {
    serde_json::from_str(include_str!("../../data/fallen_trees_26_1.json")).unwrap()
}

type Chunks = BTreeMap<(i32, i32), Arc<GeneratedChunk>>;

fn fixture_region(sample: &Sample) -> (FeatureRegion, Chunks) {
    let mut chunks = BTreeMap::new();
    for cx in sample.chunk[0] - 1..=sample.chunk[0] + 1 {
        for cz in sample.chunk[1] - 1..=sample.chunk[1] + 1 {
            let mut chunk = GeneratedChunk::new(ChunkPos::new(cx, cz));
            if sample.flat_ground && (MIN_Y..=MAX_Y).contains(&sample.floor_y) {
                for x in 0..16 {
                    for z in 0..16 {
                        chunk.set(x, sample.floor_y, z, sample.soil);
                    }
                }
            }
            chunks.insert((cx, cz), chunk);
        }
    }
    for &[x, y, z, state] in &sample.initial_blocks {
        if (MIN_Y..=MAX_Y).contains(&y) {
            chunks.get_mut(&(x >> 4, z >> 4)).unwrap().set(
                (x & 15) as usize,
                y,
                (z & 15) as usize,
                state as u32,
            );
        }
    }
    let bases: Chunks = chunks.into_iter().map(|(p, c)| (p, Arc::new(c))).collect();
    let region = FeatureRegion {
        generator: WorldGenerator::new(sample.seed),
        biome_zoom: crate::biome_zoom::BiomeZoom::new(sample.seed),
        chunks: RefCell::new(bases.clone()),
        tree_effects: Default::default(),
        light_updates: Vec::new(),
        access: Default::default(),
    };
    (region, bases)
}

fn trace_md5<const N: usize>(rows: impl IntoIterator<Item = [i32; N]>) -> String {
    let mut digest = md5::Context::new();
    for row in rows {
        for value in row {
            digest.consume(value.to_le_bytes());
        }
    }
    format!("{:x}", digest.compute())
}

struct AuditedRegion<'a> {
    region: &'a mut FeatureRegion,
    reads: RefCell<Vec<[i32; 4]>>,
    writes: Vec<[i32; 6]>,
    marked: Vec<[i32; 3]>,
    refuse_writes: bool,
}

impl TreeWorld for AuditedRegion<'_> {
    fn get_block(&self, pos: Pos) -> u32 {
        let state = TreeWorld::get_block(self.region, pos);
        self.reads
            .borrow_mut()
            .push([pos.0, pos.1, pos.2, state as i32]);
        state
    }

    fn set_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        // The ten native "reject_writes" cases deliberately inject a read-only
        // world. All other writes, including height failures, use the adapter.
        let accepted = !self.refuse_writes && TreeWorld::set_block(self.region, pos, state, flags);
        self.writes.push([
            pos.0,
            pos.1,
            pos.2,
            state as i32,
            flags,
            i32::from(accepted),
        ]);
        if accepted {
            assert_eq!(
                TreeWorld::get_block(self.region, pos),
                state,
                "read after write"
            );
        }
        accepted
    }

    fn is_face_sturdy_up(&self, state: u32, at: Pos) -> bool {
        TreeWorld::is_face_sturdy_up(self.region, state, at)
    }

    fn mark_for_postprocessing(&mut self, pos: Pos) {
        TreeWorld::mark_for_postprocessing(self.region, pos);
        self.marked.push([pos.0, pos.1, pos.2]);
    }
}

#[test]
fn native_cases_match_through_region_cache_and_live_feature_rng() {
    let reference = reference();
    assert_eq!(
        reference.jar_sha256,
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(reference.samples.len(), 655);
    let mut counts = BTreeMap::new();
    let mut refused = 0;
    for sample in &reference.samples {
        let label = format!(
            "{} {} {:?} {} floor={} soil={}",
            sample.kind, sample.seed, sample.origin, sample.terrain, sample.floor_y, sample.soil
        );
        let (mut region, bases) = fixture_region(sample);
        let before: BTreeMap<_, _> = bases.iter().map(|(&p, c)| (p, (**c).clone())).collect();
        let [x, y, z] = sample.origin;
        for &(state, supported) in &sample.cached_up_support {
            assert_eq!(
                TreeWorld::is_face_sturdy_up(&region, state, (x, y, z)),
                supported,
                "native UP/FULL state={state}: {label}"
            );
        }
        // This is the backend used by features::decorate_ores and region-backed
        // structure placement, not a copied/reseeded random::WorldgenRandom.
        let mut random = crate::simplex::WorldgenRandom::new(sample.seed);
        {
            let mut world = AuditedRegion {
                region: &mut region,
                reads: RefCell::new(Vec::new()),
                writes: Vec::new(),
                marked: Vec::new(),
                refuse_writes: sample.terrain == "reject_writes",
            };
            refused += usize::from(world.refuse_writes);
            assert_eq!(
                fallen::place(
                    &mut world,
                    &mut random,
                    &fallen::config_for(&sample.kind).unwrap(),
                    (x, y, z)
                ),
                sample.placed,
                "return: {label}"
            );
            assert_eq!(
                random.next_long(),
                sample.next_i64,
                "RNG continuation: {label}"
            );
            assert_eq!(
                world.reads.borrow().len(),
                sample.read_count,
                "read count: {label}"
            );
            assert_eq!(
                trace_md5(world.reads.borrow().iter().copied()),
                sample.reads_md5,
                "ordered reads: {label}"
            );
            assert_eq!(
                world.writes.len(),
                sample.write_count,
                "write count: {label}"
            );
            assert_eq!(
                trace_md5(world.writes.iter().copied()),
                sample.write_calls_md5,
                "writes/flags/acceptance: {label}"
            );
            assert_eq!(
                world.marked, sample.postprocessing,
                "postprocessing requests: {label}"
            );
        }
        for (pos, base) in &bases {
            assert_eq!(
                base.as_ref(),
                &before[pos],
                "immutable base cache at {pos:?}: {label}"
            );
        }
        let mut expected = before;
        for &[x, y, z, state] in &sample.writes {
            assert!(expected.get_mut(&(x >> 4, z >> 4)).unwrap().set(
                (x & 15) as usize,
                y,
                (z & 15) as usize,
                state as u32
            ));
        }
        for &[x, y, z] in &sample.postprocessing {
            expected
                .get_mut(&(x >> 4, z >> 4))
                .unwrap()
                .postprocessing
                .push(((x & 15) as usize, y, (z & 15) as usize));
        }
        let actual = region.chunks.borrow();
        assert_eq!(
            actual.len(),
            expected.len(),
            "unexpected neighbour request: {label}"
        );
        for (pos, expected) in &expected {
            assert_eq!(
                actual[pos].as_ref(),
                expected,
                "owning chunk {pos:?}: {label}"
            );
        }
        let mut digest = md5::Context::new();
        for &state in actual[&(sample.chunk[0], sample.chunk[1])].states() {
            digest.consume(state.to_le_bytes());
        }
        assert_eq!(
            format!("{:x}", digest.compute()),
            sample.states_md5,
            "chunk checksum: {label}"
        );
        *counts.entry(sample.kind.as_str()).or_insert(0) += 1;
    }
    assert_eq!(refused, 10);
    assert_eq!(counts.len(), 5);
    for (kind, count) in counts {
        assert_eq!(count, 131);
        eprintln!("Region adapter: {count} native cases for {kind}");
    }
    eprintln!(
        "Region adapter: 645 writable cases plus 10 explicit read-only fault-injection cases"
    );
}

#[test]
fn missing_neighbour_loads_real_terrain_and_writes_do_not_mutate_global_base_cache() {
    let generator = WorldGenerator::new(13_579);
    let mut region = FeatureRegion::new(
        generator,
        Arc::new(GeneratedChunk::new(ChunkPos::new(0, 0))),
    );
    let neighbour = ChunkPos::new(-1, 0);
    let base = crate::terrain_cache::get(generator, neighbour);
    let before = (*base).clone();
    let y = (MIN_Y..=MAX_Y)
        .find(|&y| base.get(15, y, 8).unwrap() != block::AIR)
        .unwrap();
    let original = base.get(15, y, 8).unwrap();
    assert!(!region.chunks.borrow().contains_key(&(-1, 0)));
    assert_eq!(TreeWorld::get_block(&region, (-1, y, 8)), original);
    assert!(Arc::ptr_eq(&region.chunks.borrow()[&(-1, 0)], &base));

    assert!(TreeWorld::set_block(
        &mut region,
        (-1, y, 8),
        block::OAK_LOG,
        3
    ));
    assert_eq!(TreeWorld::get_block(&region, (-1, y, 8)), block::OAK_LOG);
    assert_eq!(TreeWorld::get_block(&region, (15, y, 8)), block::AIR);
    assert!(!Arc::ptr_eq(&region.chunks.borrow()[&(-1, 0)], &base));
    assert_eq!(base.as_ref(), &before);
    let cached = crate::terrain_cache::get(generator, neighbour);
    assert!(Arc::ptr_eq(&base, &cached));
    assert_eq!(cached.get(15, y, 8), Some(original));
}

#[test]
fn write_flags_and_postprocessing_use_the_owning_chunk_without_claiming_ticks() {
    let fixture = reference();
    let (mut region, bases) = fixture_region(&fixture.samples[0]);
    let pos = (-1, 65, 16);
    assert!(TreeWorld::set_block(&mut region, pos, 2336, 19));
    assert!(region.chunks.borrow()[&(-1, 1)].postprocessing.is_empty());
    assert!(TreeWorld::set_block(&mut region, pos, 2337, 3));
    assert_eq!(
        region.chunks.borrow()[&(-1, 1)].postprocessing,
        vec![(15, 65, 0)]
    );
    assert!(TreeWorld::set_block(&mut region, (-2, 65, 16), 6998, 3));
    TreeWorld::mark_for_postprocessing(&mut region, pos);
    TreeWorld::mark_for_postprocessing(&mut region, pos);
    assert_eq!(
        region.chunks.borrow()[&(-1, 1)].postprocessing,
        vec![(15, 65, 0), (14, 66, 0), (15, 65, 0), (15, 65, 0)]
    );
    assert!(region.chunks.borrow()[&(0, 0)].postprocessing.is_empty());
    assert!(bases.values().all(|c| c.postprocessing.is_empty()));
    assert_eq!(bases[&(-1, 1)].get(15, 65, 0), Some(block::AIR));

    let count = region.chunks.borrow().len();
    for y in [MIN_Y - 1, MAX_Y + 1] {
        assert!(!TreeWorld::set_block(
            &mut region,
            (1024, y, -1024),
            2337,
            3
        ));
        TreeWorld::mark_for_postprocessing(&mut region, (1024, y, -1024));
        assert_eq!(TreeWorld::get_block(&region, (1024, y, -1024)), block::AIR);
    }
    assert_eq!(region.chunks.borrow().len(), count);
    assert!(TreeWorld::set_block(&mut region, (0, MAX_Y, 0), 14845, 3));
    assert!(region.chunks.borrow()[&(0, 0)].postprocessing.is_empty());
}

#[test]
fn postprocessing_positions_match_every_native_state_and_recorded_boundary() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("../../data/fallen_tree_configs_26_1.json")).unwrap();
    let properties = &data["region_properties"];
    let mut end = 0;
    for row in properties["postprocess_ranges_exclusive_end"]
        .as_array()
        .unwrap()
    {
        let start = row[0].as_u64().unwrap() as u32;
        assert_eq!(start, end);
        end = row[1].as_u64().unwrap() as u32;
        for p in properties["positions"].as_array().unwrap() {
            let (x, y, z) = (
                p[0].as_i64().unwrap() as i32,
                p[1].as_i64().unwrap() as i32,
                p[2].as_i64().unwrap() as i32,
            );
            let expected = row[2].as_array().map(|d| {
                (
                    x + d[0].as_i64().unwrap() as i32,
                    y + d[1].as_i64().unwrap() as i32,
                    z + d[2].as_i64().unwrap() as i32,
                )
            });
            for state in start..end {
                assert_eq!(
                    postprocess_on_write(state, (x, y, z)),
                    expected,
                    "state={state} pos={p}"
                );
            }
        }
    }
    assert_eq!(end as u64, properties["state_count"].as_u64().unwrap());
}

#[test]
fn advanced_live_rng_is_borrowed_across_multiple_fallen_tree_calls() {
    let fixture = reference();
    let (mut live_region, _) = fixture_region(&fixture.samples[0]);
    let (mut reference_region, _) = fixture_region(&fixture.samples[0]);
    let mut live = crate::simplex::WorldgenRandom::new(-91);
    let mut expected = crate::random::WorldgenRandom::from_seed((-91_i64) as u64);
    let seed = live.set_decoration_seed(13_579, -16, 16);
    assert_eq!(seed, expected.set_decoration_seed(13_579, -16, 16));
    live.set_feature_seed(seed, 12, 9);
    expected.set_feature_seed(seed, 12, 9);
    assert_eq!(live.next_int(7) as i32, expected.next_i32_bounded(7));
    assert_eq!(live.next_float().to_bits(), expected.next_f32().to_bits());
    assert_eq!(live.next_long(), expected.next_i64());
    assert_eq!(live.next_double().to_bits(), expected.next_f64().to_bits());
    for (config, origin) in [
        (fallen::OAK, (-1, 65, 16)),
        (fallen::SUPER_BIRCH, (0, 65, 16)),
        (fallen::JUNGLE, (15, 65, 15)),
        (fallen::SPRUCE, (0, 65, 0)),
        (fallen::BIRCH, (1, 65, 0)),
    ] {
        assert_eq!(
            fallen::place(&mut live_region, &mut live, &config, origin),
            fallen::place(&mut reference_region, &mut expected, &config, origin)
        );
        assert_eq!(
            live.next_long(),
            expected.next_i64(),
            "stream after {origin:?}"
        );
        assert_eq!(
            *live_region.chunks.borrow(),
            *reference_region.chunks.borrow()
        );
    }
    for _ in 0..8 {
        assert_eq!(live.next_long(), expected.next_i64());
    }
}
