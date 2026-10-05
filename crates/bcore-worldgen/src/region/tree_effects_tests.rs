use super::{FeatureRegion, TreeEffectError};
use crate::{
    block,
    block_entity::BlockEntity,
    tick_request::{TickRequest, TickTarget},
    tree::{
        fallen::{FallenTreeWorld, Pos},
        standing::StandingTreeWorld,
    },
    ChunkPos, GeneratedChunk, WorldGenerator, MAX_Y, MIN_Y,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

type Chunks = BTreeMap<(i32, i32), Arc<GeneratedChunk>>;

fn region() -> (FeatureRegion, Chunks) {
    let mut chunks = BTreeMap::new();
    for (x, z) in [(0, 0), (-1, 1), (1, -1), (-2, -1)] {
        let mut chunk = GeneratedChunk::new(ChunkPos::new(x, z));
        assert!(chunk.add_tick_request(TickRequest {
            block_pos: [x * 16, MIN_Y, z * 16],
            target: TickTarget::Fluid(4),
            delay: i32::MIN,
        }));
        chunks.insert((x, z), Arc::new(chunk));
    }
    (
        FeatureRegion {
            generator: WorldGenerator::new(0),
            biome_zoom: crate::biome_zoom::BiomeZoom::new(0),
            chunks: RefCell::new(chunks.clone()),
            tree_effects: Default::default(),
            light_updates: Vec::new(),
            access: Default::default(),
        },
        chunks,
    )
}

fn raw(request: &TickRequest) -> [i32; 6] {
    let [x, y, z] = request.block_pos;
    let (value, fluid) = match request.target {
        TickTarget::Block(value) => (value as i32, 0),
        TickTarget::Fluid(value) => (value as i32, 1),
    };
    [x, y, z, value, request.delay, fluid]
}

fn place_hive(region: &mut FeatureRegion, pos: Pos, ticks: &[i32]) {
    assert!(FallenTreeWorld::set_block(region, pos, 21774, 19));
    assert!(region.has_beehive(pos));
    for &tick in ticks {
        region.store_bee(pos, tick);
    }
}

fn bees(region: &FeatureRegion, (x, y, z): Pos) -> Option<Vec<i32>> {
    match region.chunk(x >> 4, z >> 4).block_entities().get(&(
        (x & 15) as usize,
        y,
        (z & 15) as usize,
    )) {
        Some(BlockEntity::Beehive { ticks_in_hive }) => Some(ticks_in_hive.clone()),
        None => None,
        Some(other) => panic!("unexpected block entity {other:?}"),
    }
}

fn assert_same_arcs(region: &FeatureRegion, before: &Chunks) {
    let actual = region.chunks.borrow();
    assert_eq!(actual.len(), before.len());
    for (pos, old) in before {
        assert!(
            Arc::ptr_eq(old, &actual[pos]),
            "unexpected COW or replacement at {pos:?}"
        );
    }
}

#[test]
fn transfer_appends_native_requests_per_owner_with_duplicates_and_signed_delays() {
    let (mut region, bases) = region();
    let before: BTreeMap<_, _> = bases
        .iter()
        .map(|(&p, c)| (p, c.as_ref().clone()))
        .collect();
    let ages = [i32::MIN, -1, 0, 598, 599, 600, i32::MAX];
    place_hive(&mut region, (-1, 65, 16), &ages);
    place_hive(&mut region, (16, MAX_Y, -1), &[]);
    let rows = [
        [-1, 65, 16, 21768, -1, 0],
        [16, MAX_Y, -1, 4, i32::MIN, 1],
        [-1, 65, 16, 21768, -1, 0],
        [-17, MIN_Y, -1, 0, i32::MAX, 1],
        [0, 65, 0, 29872, 0, 0],
        [16, MAX_Y, -1, 0, -7, 1],
        [-1, 65, 16, 1, 0, 1],
        [-17, MIN_Y, -1, 2, 5, 1],
        [-17, MIN_Y, -1, 3, 5, 1],
    ];
    for row in rows {
        region.schedule_tree_tick(row);
    }
    region.transfer_tree_effects().unwrap();
    assert_eq!(region.tree_effects, Default::default());
    assert_eq!(bees(&region, (-1, 65, 16)), Some(ages.to_vec()));
    assert_eq!(bees(&region, (16, MAX_Y, -1)), Some(Vec::new()));
    for (&owner, chunk) in region.chunks.borrow().iter() {
        let expected: Vec<_> = bases[&owner]
            .tick_requests()
            .iter()
            .map(raw)
            .chain(rows.into_iter().filter(|r| (r[0] >> 4, r[2] >> 4) == owner))
            .collect();
        assert_eq!(
            chunk.tick_requests().iter().map(raw).collect::<Vec<_>>(),
            expected
        );
        assert!(chunk.tick_requests().iter().all(|r| r.valid_for(chunk.pos)));
        assert_eq!(bases[&owner].as_ref(), &before[&owner]);
    }
    let transferred = region.chunks.borrow().clone();
    region.transfer_tree_effects().unwrap();
    assert_same_arcs(&region, &transferred);
}

#[test]
fn empty_transfer_preserves_existing_chunk_data_and_cow_sharing() {
    let (mut region, _) = region();
    let pos = (-1, 65, 16);
    place_hive(&mut region, pos, &[7, 8]);
    region.transfer_tree_effects().unwrap();
    let persisted = region.chunks.borrow().clone();
    region.transfer_tree_effects().unwrap();
    region.transfer_tree_effects().unwrap();
    assert_same_arcs(&region, &persisted);
    assert_eq!(bees(&region, pos), Some(vec![7, 8]));
}

#[test]
fn restaging_after_transfer_keeps_occupants_and_new_requests_exactly_once() {
    let (mut region, _) = region();
    let pos = (-1, 65, 16);
    let row = [-1, 65, 16, 2, 5, 1];
    place_hive(&mut region, pos, &[11, 12]);
    region.schedule_tree_tick(row);
    region.transfer_tree_effects().unwrap();
    let persisted = region.chunks.borrow().clone();
    region.store_bee(pos, i32::MIN);
    assert_eq!(region.tree_effects.beehives[&pos], vec![11, 12, i32::MIN]);
    assert_eq!(bees(&region, pos), Some(vec![11, 12]));
    assert!(region.has_beehive(pos));
    region.store_bee(pos, i32::MAX);
    region.schedule_tree_tick(row);
    region.transfer_tree_effects().unwrap();
    assert_eq!(bees(&region, pos), Some(vec![11, 12, i32::MIN, i32::MAX]));
    let chunk = region.chunk(-1, 1);
    assert_eq!(chunk.tick_requests().len(), 3);
    assert_eq!(
        chunk.tick_requests()[1..]
            .iter()
            .map(raw)
            .collect::<Vec<_>>(),
        vec![row, row]
    );
    assert_eq!(persisted[&(-1, 1)].tick_requests().len(), 2);
    assert_eq!(
        persisted[&(-1, 1)].block_entities()[&(15, 65, 0)],
        BlockEntity::Beehive {
            ticks_in_hive: vec![11, 12]
        }
    );
    drop(chunk);
    let before = region.chunks.borrow().clone();
    region.transfer_tree_effects().unwrap();
    assert_same_arcs(&region, &before);
}

#[test]
fn hive_removal_and_compatible_writes_preserve_requests_and_cow_snapshots() {
    let (mut region, _) = region();
    let a = (-1, 65, 16);
    let b = (16, 65, -1);
    place_hive(&mut region, a, &[1, 2]);
    place_hive(&mut region, b, &[3, 4]);
    for (x, y, z) in [a, b] {
        region.schedule_tree_tick([x, y, z, 21768, 1, 0]);
    }
    assert!(FallenTreeWorld::set_block(&mut region, a, block::AIR, 19));
    assert!(!region.tree_effects.beehives.contains_key(&a));
    region.transfer_tree_effects().unwrap();
    assert_eq!(bees(&region, a), None);
    assert_eq!(bees(&region, b), Some(vec![3, 4]));
    assert!(FallenTreeWorld::set_block(&mut region, b, block::STONE, 19));
    assert_eq!(bees(&region, b), None);
    region.transfer_tree_effects().unwrap();
    for (x, _, z) in [a, b] {
        assert_eq!(region.chunk(x >> 4, z >> 4).tick_requests().len(), 2);
    }

    place_hive(&mut region, a, &[9]);
    region.transfer_tree_effects().unwrap();
    let saved = region.chunks.borrow()[&(-1, 1)].clone();
    assert!(FallenTreeWorld::set_block(&mut region, a, 21774, 19));
    assert_eq!(bees(&region, a), Some(vec![9]));
    region.store_bee(a, -5);
    region.transfer_tree_effects().unwrap();
    assert_eq!(bees(&region, a), Some(vec![9, -5]));
    assert_eq!(
        saved.block_entities()[&(15, 65, 0)],
        BlockEntity::Beehive {
            ticks_in_hive: vec![9]
        }
    );
    let effects = region.tree_effects.clone();
    let chunks = region.chunks.borrow().clone();
    assert!(!FallenTreeWorld::set_block(
        &mut region,
        (-1, MAX_Y + 1, 16),
        block::AIR,
        19
    ));
    assert_eq!(region.tree_effects, effects);
    assert_same_arcs(&region, &chunks);
}

#[test]
fn invalid_tick_batches_preserve_all_pending_and_stored_data_for_retry() {
    let invalid = [
        [-1, MIN_Y - 1, 16, 0, 0, 0],
        [-1, MAX_Y + 1, 16, 0, 0, 0],
        [-1, 65, 16, -1, 0, 0],
        [-1, 65, 16, 29873, 0, 0],
        [-1, 65, 16, i32::MAX, 0, 0],
        [-1, 65, 16, -1, 0, 1],
        [-1, 65, 16, 5, 0, 1],
        [-1, 65, 16, i32::MAX, 0, 1],
        [-1, 65, 16, 0, 0, -1],
        [-1, 65, 16, 0, 0, 2],
    ];
    for row in invalid {
        let (mut region, _) = region();
        let pos = (-1, 65, 16);
        let valid = [-1, 65, 16, 0, -7, 0];
        place_hive(&mut region, pos, &[11]);
        region.schedule_tree_tick(valid);
        region.transfer_tree_effects().unwrap();
        region.store_bee(pos, 97);
        assert!(FallenTreeWorld::set_block(&mut region, pos, 21774, 3));
        region.schedule_tree_tick(valid);
        region.schedule_tree_tick(row);
        let pending = region.tree_effects.clone();
        let stored = region.chunks.borrow().clone();
        for _ in 0..2 {
            assert_eq!(
                region.transfer_tree_effects(),
                Err(TreeEffectError::InvalidTickRequest(row))
            );
            assert_eq!(region.tree_effects, pending);
            assert_same_arcs(&region, &stored);
        }
        assert_eq!(bees(&region, pos), Some(vec![11]));
        assert_eq!(region.tree_effects.tick_requests.pop(), Some(row));
        region.transfer_tree_effects().unwrap();
        assert_eq!(bees(&region, pos), Some(vec![11, 97]));
        assert_eq!(
            region.chunk(-1, 1).tick_requests()[1..]
                .iter()
                .map(raw)
                .collect::<Vec<_>>(),
            vec![valid, valid]
        );
    }
}

#[test]
fn incompatible_hive_keeps_the_entire_transfer_pending() {
    let (mut region, _) = region();
    let valid = (-1, 65, 16);
    let invalid = (16, 65, -1);
    place_hive(&mut region, valid, &[1, 2]);
    region.tree_effects.beehives.insert(invalid, vec![3, 4]);
    region.schedule_tree_tick([-1, 65, 16, 2, 5, 1]);
    let pending = region.tree_effects.clone();
    let stored = region.chunks.borrow().clone();
    assert_eq!(
        region.transfer_tree_effects(),
        Err(TreeEffectError::InvalidBeehive {
            pos: invalid,
            state: block::AIR
        })
    );
    assert_eq!(region.tree_effects, pending);
    assert_same_arcs(&region, &stored);
    assert_eq!(bees(&region, valid), None);
    assert!(region.chunk_mut(1, -1).set(0, 65, 15, 21774));
    region.transfer_tree_effects().unwrap();
    assert_eq!(bees(&region, valid), Some(vec![1, 2]));
    assert_eq!(bees(&region, invalid), Some(vec![3, 4]));
}

#[test]
fn out_of_height_hives_are_recoverable_transfer_errors() {
    for y in [MIN_Y - 1, MAX_Y + 1, i32::MIN, i32::MAX] {
        let (mut region, _) = region();
        let valid = (-1, 65, 16);
        let invalid = (16, y, -1);
        place_hive(&mut region, valid, &[1, 2]);
        region.tree_effects.beehives.insert(invalid, vec![3, 4]);
        region.schedule_tree_tick([-1, 65, 16, 2, 5, 1]);
        let pending = region.tree_effects.clone();
        let stored = region.chunks.borrow().clone();
        assert_eq!(
            region.transfer_tree_effects(),
            Err(TreeEffectError::InvalidBeehive {
                pos: invalid,
                state: block::AIR,
            })
        );
        assert_eq!(region.tree_effects, pending);
        assert_same_arcs(&region, &stored);
        let ages = region.tree_effects.beehives.remove(&invalid).unwrap();
        region.tree_effects.beehives.insert((16, 65, -1), ages);
        assert!(region.chunk_mut(1, -1).set(0, 65, 15, 21774));
        region.transfer_tree_effects().unwrap();
        assert_eq!(bees(&region, valid), Some(vec![1, 2]));
        assert_eq!(bees(&region, (16, 65, -1)), Some(vec![3, 4]));
    }
}

#[test]
fn native_hive_metadata_transfers_all_supported_states_and_unbounded_age_lists() {
    #[derive(serde::Deserialize)]
    struct Reference {
        type_id: u32,
        samples: Vec<Sample>,
    }
    #[derive(serde::Deserialize)]
    struct Sample {
        pos: [i32; 3],
        state: u32,
        ticks_in_hive: Vec<i32>,
        inside_build_height: bool,
        nbt: serde_json::Value,
        update: serde_json::Value,
    }
    let data: Reference =
        serde_json::from_str(include_str!("../../data/beehives_26_1.json")).unwrap();
    assert_eq!(data.samples.len(), 100);
    let mut states = BTreeSet::new();
    let (mut accepted, mut long_lists, mut negative_ages) = (0, 0, 0);
    for sample in &data.samples {
        let [x, y, z] = sample.pos;
        let owner = ChunkPos::new(x >> 4, z >> 4);
        let mut chunk = GeneratedChunk::new(owner);
        assert!(sample.inside_build_height);
        assert!(chunk.set((x & 15) as usize, y, (z & 15) as usize, sample.state));
        let base = Arc::new(chunk);
        let mut region = FeatureRegion::new(WorldGenerator::new(0), base.clone());
        assert!(
            region.has_beehive((x, y, z)),
            "native hive state {}",
            sample.state
        );
        for &ticks in &sample.ticks_in_hive {
            region.store_bee((x, y, z), ticks);
        }
        region.transfer_tree_effects().unwrap();
        region.transfer_tree_effects().unwrap();
        let actual = region.chunk(owner.x, owner.z);
        let entity = &actual.block_entities()[&((x & 15) as usize, y, (z & 15) as usize)];
        assert_eq!(entity.type_id(), data.type_id);
        assert_eq!(entity.full_data((x, y, z)), sample.nbt);
        assert_eq!(entity.update_data(), sample.update);
        assert_eq!(actual.states(), base.states());
        assert!(base.block_entities().is_empty());
        assert_eq!(region.tree_effects, Default::default());
        states.insert(sample.state);
        accepted += 1;
        long_lists += usize::from(sample.ticks_in_hive.len() > 3);
        negative_ages += usize::from(sample.ticks_in_hive.iter().any(|&t| t < 0));
    }
    assert_eq!(states.len(), 48);
    assert_eq!(accepted, 100);
    assert_eq!(long_lists, 2);
    assert_eq!(negative_ages, 2);
    println!("Native hive transfer: {accepted} accepted, 48 states, {long_lists} lists over three occupants, {negative_ages} signed-age cases");
}

#[test]
fn tick_only_transfer_loads_real_neighbor_terrain_without_mutating_its_cache() {
    let generator = WorldGenerator::new(13_579);
    let owner = ChunkPos::new(-1, 0);
    let base = crate::terrain_cache::get(generator, owner);
    let before = base.as_ref().clone();
    let mut region = FeatureRegion::new(
        generator,
        Arc::new(GeneratedChunk::new(ChunkPos::new(0, 0))),
    );
    let request = TickRequest {
        block_pos: [-1, 65, 8],
        target: TickTarget::Fluid(2),
        delay: -5,
    };
    region.schedule_tree_tick(raw(&request));
    assert!(!region.chunks.borrow().contains_key(&(-1, 0)));
    region.transfer_tree_effects().unwrap();
    let mut expected = before.clone();
    assert!(expected.add_tick_request(request));
    assert_eq!(&*region.chunk(-1, 0), &expected);
    assert_eq!(base.as_ref(), &before);
    assert!(!Arc::ptr_eq(&base, &region.chunks.borrow()[&(-1, 0)]));
}

#[test]
#[should_panic(expected = "invalid staged standing-tree effects")]
fn finalization_reports_invalid_requests_instead_of_discarding_them() {
    let (mut region, _) = region();
    region.schedule_tree_tick([0, 65, 0, -1, 0, 0]);
    region.finish_chunk(ChunkPos::new(0, 0));
}

#[derive(serde::Deserialize)]
struct NativeHiveSnapshot {
    state: u32,
    entity_id: u32,
    ticks_in_hive: Option<Vec<i32>>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum NativeHiveAction {
    Write { state: u32, accepted: bool },
    Resolve { present: bool },
    Store { age: i32 },
}

#[derive(serde::Deserialize)]
struct NativeHiveStep {
    #[serde(flatten)]
    action: NativeHiveAction,
    after: NativeHiveSnapshot,
    native_ticks_unchanged: bool,
}

#[derive(serde::Deserialize)]
struct NativeHiveTransition {
    name: String,
    pos: [i32; 3],
    source: [i32; 2],
    flags: i32,
    requests: Vec<[i32; 6]>,
    native_ticks_before: Vec<[i32; 7]>,
    native_ticks_after: Vec<[i32; 7]>,
    poi_callbacks: usize,
    steps: Vec<NativeHiveStep>,
    nbt: serde_json::Value,
}

fn replay_native_hive_transition(sample: &NativeHiveTransition, transfer_every: usize) {
    let [x, y, z] = sample.pos;
    let pos = (x, y, z);
    let owner = ChunkPos::new(x >> 4, z >> 4);
    let source = ChunkPos::new(sample.source[0], sample.source[1]);
    assert_ne!(owner, source, "native write must cross a chunk boundary");
    let base = Arc::new(GeneratedChunk::new(owner));
    let source_base = Arc::new(GeneratedChunk::new(source));
    let mut region = FeatureRegion::new(WorldGenerator::new(0), source_base.clone());
    region
        .chunks
        .get_mut()
        .insert((owner.x, owner.z), base.clone());
    for &row in &sample.requests {
        region.schedule_tree_tick(row);
    }
    let label = format!(
        "{} flags={} transfer_every={transfer_every}",
        sample.name, sample.flags
    );
    for (index, step) in sample.steps.iter().enumerate() {
        let shared = region.chunks.borrow()[&(owner.x, owner.z)].clone();
        let before = shared.as_ref().clone();
        match step.action {
            NativeHiveAction::Write { state, accepted } => assert_eq!(
                FallenTreeWorld::set_block(&mut region, pos, state, sample.flags),
                accepted,
                "{label} step {index}: write result"
            ),
            NativeHiveAction::Resolve { present } => {
                assert_eq!(
                    region.has_beehive(pos),
                    present,
                    "{label} step {index}: typed lookup"
                );
            }
            NativeHiveAction::Store { age } => region.store_bee(pos, age),
        }
        if transfer_every != 0 && (index + 1) % transfer_every == 0 {
            region.transfer_tree_effects().unwrap();
            let stored = region.chunks.borrow().clone();
            region.transfer_tree_effects().unwrap();
            assert_same_arcs(&region, &stored);
        }
        let actual = region
            .tree_effects
            .beehives
            .get(&pos)
            .cloned()
            .or_else(|| bees(&region, pos));
        assert_eq!(
            actual, step.after.ticks_in_hive,
            "{label} step {index}: occupants"
        );
        assert_eq!(
            FallenTreeWorld::get_block(&region, pos),
            step.after.state,
            "{label} step {index}: block state"
        );
        let requests: Vec<_> = region
            .chunk(owner.x, owner.z)
            .tick_requests()
            .iter()
            .map(raw)
            .chain(region.tree_effects.tick_requests.iter().copied())
            .collect();
        assert_eq!(
            requests, sample.requests,
            "{label} step {index}: raw request order/duplicates"
        );
        assert!(
            step.native_ticks_unchanged,
            "native ticks changed in {label} step {index}"
        );
        assert_eq!(
            shared.as_ref(),
            &before,
            "{label} step {index}: mutated COW snapshot"
        );
    }
    FallenTreeWorld::mark_for_postprocessing(&mut region, pos);
    FallenTreeWorld::mark_for_postprocessing(&mut region, pos);
    let finalized = region.finish_chunk(owner);
    let entity = &finalized.block_entities()[&((x & 15) as usize, y, (z & 15) as usize)];
    assert_eq!(
        entity.full_data(pos),
        sample.nbt,
        "{label}: finalized native NBT"
    );
    assert_eq!(
        finalized
            .tick_requests()
            .iter()
            .map(raw)
            .collect::<Vec<_>>(),
        sample.requests,
        "{label}: finalized requests"
    );
    assert!(finalized.postprocessing.is_empty());
    assert_eq!(base.as_ref(), &GeneratedChunk::new(owner));
    assert_eq!(source_base.as_ref(), &GeneratedChunk::new(source));
}

#[test]
fn native_proto_hive_transitions_match_staged_and_transferred_effects() {
    use sha2::{Digest, Sha256};
    #[derive(serde::Deserialize)]
    struct Reference {
        minecraft: String,
        version: u32,
        jar_sha256: String,
        probe_sha256: String,
        source_dependencies: Vec<String>,
        samples: Vec<NativeHiveTransition>,
    }
    let reference: Reference =
        serde_json::from_str(include_str!("../../data/tree_effects_26_1.json")).unwrap();
    assert_eq!(reference.minecraft, "26.1");
    assert_eq!(reference.version, 1);
    assert_eq!(
        reference.jar_sha256,
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    let mut hash = Sha256::new();
    for bytes in [
        &include_bytes!("../../../../scripts/TreeReference.java")[..],
        &include_bytes!("../../../../scripts/NativeEntityLevel.java")[..],
        &include_bytes!("../../../../scripts/NativeWorldgenRegistries.java")[..],
        &include_bytes!("../../../../scripts/TreeEffectReference.java")[..],
    ] {
        hash.update(bytes);
    }
    assert_eq!(reference.probe_sha256, format!("{:x}", hash.finalize()));
    assert_eq!(
        reference.source_dependencies,
        [
            "TreeReference.java",
            "NativeEntityLevel.java",
            "NativeWorldgenRegistries.java",
            "TreeEffectReference.java"
        ]
    );
    assert_eq!(reference.samples.len(), 18);
    let coverage: BTreeSet<_> = reference
        .samples
        .iter()
        .map(|s| (s.name.as_str(), s.flags))
        .collect();
    for name in [
        "same_state",
        "nest_properties",
        "hive_properties",
        "nest_to_hive",
        "hive_to_nest",
        "stone_recreate",
        "air_recreate",
        "repeated_stores",
        "pending_rewrites",
    ] {
        for flags in [19, 3] {
            assert!(
                coverage.contains(&(name, flags)),
                "missing native {name} flags={flags}"
            );
        }
    }
    for sample in &reference.samples {
        assert_eq!(sample.native_ticks_before, sample.native_ticks_after);
        assert_eq!(
            sample.poi_callbacks,
            sample
                .steps
                .iter()
                .filter(|s| matches!(s.action, NativeHiveAction::Write { .. }))
                .count()
        );
        let last = &sample.steps.last().unwrap().after;
        if sample.name == "same_state" {
            assert_eq!(last.ticks_in_hive, Some(vec![9, -5]));
            assert_eq!(last.entity_id, 1);
        } else if sample.name.ends_with("_recreate") {
            assert_eq!(last.ticks_in_hive, Some(vec![-5]));
            assert_eq!(last.entity_id, 2);
        }
        // Never flush, flush every step, or retain a new staged snapshot over an
        // older stored entity between flushes. All must match the same native trace.
        for transfer_every in [0, 1, 2] {
            replay_native_hive_transition(sample, transfer_every);
        }
    }
    println!("Verified 18 native WorldGenRegion/ProtoChunk hive traces with three transfer cadences (54 finalized replays)");
}
