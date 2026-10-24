use super::*;
use crate::block;
use crate::block_entity::{BlockEntity, PendingBlockEntity};
use crate::generation::{
    graph::layer_positions, ChunkAccessKind, ChunkPyramid, ReturnedChunk, StageState,
};
use crate::region::FeatureRegion;
use crate::structure::template::Nbt;
use crate::tick_request::{TickRequest, TickTarget};
use std::time::{Duration, Instant};

type Point = (i32, i32, ChunkStatus, bool);

#[derive(Default)]
struct ControlState {
    gates: BTreeMap<Point, bool>,
    panic_at: Option<Point>,
    cancelled: bool,
}

/// Barriers around real worker stage dispatch, outside both runtime and core
/// locks. Timeouts only detect stuck tests; no elapsed time controls generation.
#[derive(Default)]
pub(super) struct WorkerControl {
    state: Mutex<ControlState>,
    changed: Condvar,
}

impl WorkerControl {
    pub(super) fn before(&self, pos: ChunkPos, status: ChunkStatus) {
        self.checkpoint((pos.x, pos.z, status, false));
    }

    pub(super) fn after(&self, pos: ChunkPos, status: ChunkStatus) {
        self.checkpoint((pos.x, pos.z, status, true));
    }

    fn checkpoint(&self, point: Point) {
        let mut state = lock(&self.state);
        if state.panic_at == Some(point) {
            state.panic_at = None;
            drop(state);
            panic!("controlled worker unwind");
        }
        if let Some(reached) = state.gates.get_mut(&point) {
            *reached = true;
            self.changed.notify_all();
        }
        while state.gates.contains_key(&point) && !state.cancelled {
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn arm(&self, pos: ChunkPos, status: ChunkStatus, after: bool) {
        lock(&self.state)
            .gates
            .insert((pos.x, pos.z, status, after), false);
    }

    fn reached(&self, pos: ChunkPos, status: ChunkStatus, after: bool) {
        let point = (pos.x, pos.z, status, after);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut state = lock(&self.state);
        while state.gates.get(&point) != Some(&true) {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("worker barrier not reached");
            state = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn release(&self, pos: ChunkPos, status: ChunkStatus, after: bool) {
        lock(&self.state)
            .gates
            .remove(&(pos.x, pos.z, status, after));
        self.changed.notify_all();
    }

    pub(super) fn cancel(&self) {
        let mut state = lock(&self.state);
        state.cancelled = true;
        state.gates.clear();
        self.changed.notify_all();
    }
}

fn stage(status: ChunkStatus, pos: ChunkPos, region: &mut FeatureRegion) -> Result<(), String> {
    match status {
        ChunkStatus::Noise => {
            region.owned_chunk_mut(pos).set(3, 50, 3, block::STONE);
        }
        ChunkStatus::Features => {
            let chunk = region.owned_chunk_mut(pos);
            let catalog = crate::block_predicate::catalog();
            chunk.set(1, 80, 1, catalog.default_state("sculk_sensor").unwrap());
            chunk.set_pending_block_entity(
                1,
                80,
                1,
                PendingBlockEntity::dummy((pos.x * 16 + 1, 80, pos.z * 16 + 1)),
            );
            chunk.set(2, 80, 1, catalog.default_state("chest").unwrap());
            chunk.set_block_entity(2, 80, 1, BlockEntity::DungeonChest { loot_seed: 91 });
            chunk.postprocessing.extend([(1, 80, 1), (1, 80, 1)]);
            let tick = TickRequest {
                block_pos: [pos.x * 16 + 1, 80, pos.z * 16 + 1],
                target: TickTarget::Block(block::STONE),
                delay: 7,
            };
            chunk.add_tick_request(tick);
            chunk.add_tick_request(tick);
        }
        ChunkStatus::Spawn => {
            region.owned_chunk_mut(pos).set(4, 90, 4, block::STONE);
        }
        _ => {}
    }
    Ok(())
}

fn fixture() -> (Arc<GenerationWorld>, NativeChunkRequests) {
    let world = Arc::new(GenerationWorld::new(2417));
    world.state.lock().unwrap().test_stage = Some(stage);
    let runtime = NativeChunkRequests::new(world.clone()).unwrap();
    (world, runtime)
}

fn wait_state(runtime: &NativeChunkRequests, predicate: impl Fn(&RequestState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut state = lock(&runtime.shared.state);
    while !predicate(&state) {
        state.check_open().unwrap();
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("runtime condition not reached");
        state = runtime
            .shared
            .wake
            .wait_timeout(state, remaining)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

fn queued_full(runtime: &NativeChunkRequests, pos: ChunkPos) {
    wait_state(runtime, |state| {
        state
            .main
            .iter()
            .any(|task| matches!(task, MainTask::Full(task) if task.pos == pos))
    });
}

fn full_queue(runtime: &NativeChunkRequests) -> Vec<ChunkPos> {
    runtime
        .snapshot()
        .main_tasks
        .into_iter()
        .filter(|task| task.kind == MainTaskKind::FullConversion)
        .map(|task| task.pos)
        .collect()
}

fn idle_worker(runtime: &NativeChunkRequests) {
    wait_state(runtime, |state| {
        state.pending.is_empty() && state.worker_queue.is_empty() && !state.worker_active
    });
}

fn light_before_spawn(
    runtime: &NativeChunkRequests,
    pos: ChunkPos,
) -> (ChunkFuture, ReturnedChunk) {
    runtime.shared.control.arm(pos, ChunkStatus::Spawn, false);
    let future = runtime
        .request_future(pos, ChunkStatus::Light, true)
        .unwrap();
    let object = runtime.wait(&future).unwrap().unwrap();
    runtime
        .shared
        .control
        .reached(pos, ChunkStatus::Spawn, false);
    let cached = runtime
        .get_chunk(pos, ChunkStatus::Light, true)
        .unwrap()
        .unwrap();
    assert!(cached.same_object(&object));
    (future, object)
}

#[test]
fn native_ticket_table_uses_accumulated_full_radii_and_allowed_ceilings() {
    assert_eq!(
        ChunkStatus::ALL.map(ticket_level),
        [44, 44, 36, 36, 35, 35, 35, 34, 34, 33, 33, 33]
    );
    assert_eq!(maximum_loading_level(), 44);
    for level in 0..=33 {
        assert_eq!(generation_status(level), Some(ChunkStatus::Full));
    }
    assert_eq!(generation_status(34), Some(ChunkStatus::InitializeLight));
    assert_eq!(generation_status(35), Some(ChunkStatus::Carvers));
    assert_eq!(generation_status(36), Some(ChunkStatus::Biomes));
    for level in 37..=44 {
        assert_eq!(generation_status(level), Some(ChunkStatus::StructureStarts));
    }
    assert_eq!(generation_status(45), None);
    assert_eq!(generation_status(u8::MAX), None);
}

#[test]
fn saved_admission_resolves_retained_full_accessibility_and_cancels_queued_conversion() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(3, -2);
    let first = layer_positions(
        pos,
        ChunkPyramid::Generation
            .step(ChunkStatus::Full)
            .layer_radius(ChunkStatus::Empty)
            .unwrap(),
    )
    .next()
    .unwrap();
    runtime.shared.control.arm(first, ChunkStatus::Empty, false);
    let full = runtime
        .request_future(pos, ChunkStatus::Full, true)
        .unwrap();
    runtime
        .shared
        .control
        .reached(first, ChunkStatus::Empty, false);
    let accessibility = runtime
        .holder(pos)
        .unwrap()
        .full_accessibility_future
        .unwrap();
    assert!(!full.is_done());
    assert!(!accessibility.is_done());
    let positions: Vec<_> = layer_positions(pos, 1).collect();
    let data = positions
        .iter()
        .map(|&pos| crate::generation::SavedChunkData {
            pos,
            states: vec![block::AIR; crate::WORLD_HEIGHT as usize * 256],
            biomes: vec![0; crate::WORLD_HEIGHT as usize * 4],
            block_entities: Default::default(),
            feature_block_entities: Default::default(),
            pending_block_entities: Default::default(),
            entities: Vec::new(),
            structure_entities: Vec::new(),
            tick_requests: Vec::new(),
            postprocessing: Vec::new(),
            light: None,
            structures: Default::default(),
        })
        .collect();
    world.admit_saved_chunks(data).unwrap();
    for saved in positions {
        runtime.admit_saved_chunk(saved).unwrap();
    }
    let old = full
        .get_now()
        .expect("retained FULL must resolve")
        .unwrap()
        .unwrap();
    let current = runtime
        .get_chunk(pos, ChunkStatus::Full, false)
        .unwrap()
        .unwrap();
    assert!(old.same_object(&current));
    let accessible = accessibility
        .get_now()
        .expect("retained dependencies must resolve")
        .unwrap()
        .unwrap();
    assert!(accessible.same_object(&current));
    runtime
        .shared
        .control
        .release(first, ChunkStatus::Empty, false);
    idle_worker(&runtime);
    assert!(full_queue(&runtime).is_empty());
    assert_eq!(runtime.holder(pos).unwrap().generation_target, None);
    assert_eq!(
        world
            .progress(pos)
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::Empty)
            .attempts,
        0
    );
    runtime.shutdown();
    assert!(full.join().unwrap().unwrap().same_object(&current));
}

#[test]
fn retained_ticket_minima_relax_all_eight_neighbours_and_overlaps() {
    let sources = [
        (ChunkPos::new(-3, 7), 33),
        (ChunkPos::new(6, -2), 35),
        (ChunkPos::new(-5, -4), 34),
    ];
    let mut tickets = LoadingTickets::default();
    for (pos, level) in sources {
        tickets.add(pos, level);
    }
    let settled: BTreeMap<_, _> = tickets
        .settle()
        .into_iter()
        .map(|(p, l)| ((p.x, p.z), l))
        .collect();
    for x in -18i32..=20 {
        for z in -18i32..=22 {
            let expected = sources
                .iter()
                .map(|(pos, level)| u32::from(*level) + x.abs_diff(pos.x).max(z.abs_diff(pos.z)))
                .min()
                .unwrap();
            assert_eq!(
                settled.get(&(x, z)).copied(),
                (expected <= 44).then_some(expected as u8)
            );
        }
    }
    tickets.add(sources[0].0, 44);
    tickets.add(sources[0].0, 33);
    assert!(tickets.settle().is_empty());
    tickets.add(sources[1].0, 33);
    assert!(!tickets.settle().is_empty());
}

#[test]
fn permission_does_not_demand_initialize_light() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(7, -4);
    let object = runtime
        .get_chunk(pos, ChunkStatus::Features, true)
        .unwrap()
        .unwrap();
    idle_worker(&runtime);
    assert_eq!(
        runtime.holder(pos).unwrap().highest_allowed_status,
        Some(ChunkStatus::InitializeLight)
    );
    assert_eq!(
        object.view().unwrap().persisted_status,
        ChunkStatus::Features
    );
    assert_eq!(
        world
            .progress(pos)
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::InitializeLight)
            .state,
        StageState::Pending
    );
    assert!(runtime.snapshot().main_queue.is_empty());
}

#[test]
fn full_accessibility_updates_all_ceilings_before_z_outer_range_demands() {
    let (_, runtime) = fixture();
    let pos = ChunkPos::new(-8, 13);
    runtime.shared.control.arm(pos, ChunkStatus::Spawn, false);
    let future = runtime
        .request_future(pos, ChunkStatus::Light, true)
        .unwrap();
    let holder = runtime.holder(pos).unwrap();
    assert_eq!(holder.generation_target, Some(ChunkStatus::Full));
    assert!(holder.full_accessible);
    let state = lock(&runtime.shared.state);
    let (changes, demands) = &state.admission_batches[0];
    assert_eq!(changes.len(), 529);
    let expected: Vec<_> = (-1..=1)
        .flat_map(|dz| {
            (-1..=1).map(move |dx| {
                (
                    ChunkPos::new(pos.x + dx, pos.z + dz),
                    if dx == 0 && dz == 0 {
                        ChunkStatus::Full
                    } else {
                        ChunkStatus::InitializeLight
                    },
                )
            })
        })
        .collect();
    assert_eq!(demands, &expected);
    for &(p, status) in demands {
        assert!(state.holders[&(p.x, p.z)].allowed.unwrap() >= status);
    }
    assert!(
        state.holders[&(pos.x, pos.z)].futures[ChunkStatus::Light.index()]
            .as_ref()
            .unwrap()
            .same_future(&future)
    );
    drop(state);
    runtime.shutdown();
}

#[test]
fn coalescing_and_target_upgrade_preserve_requested_futures_and_once_only_claims() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(2, -3);
    runtime
        .shared
        .control
        .arm(pos, ChunkStatus::Features, false);
    let first = runtime
        .request_future(pos, ChunkStatus::Features, true)
        .unwrap();
    runtime.poll_task().unwrap();
    runtime
        .shared
        .control
        .reached(pos, ChunkStatus::Features, false);
    let duplicate = runtime
        .request_future(pos, ChunkStatus::Features, true)
        .unwrap();
    assert!(duplicate.same_future(&first));
    let initialize = runtime
        .request_future(pos, ChunkStatus::InitializeLight, true)
        .unwrap();
    let light = runtime
        .request_future(pos, ChunkStatus::Light, true)
        .unwrap();
    let full = runtime
        .request_future(pos, ChunkStatus::Full, true)
        .unwrap();
    assert!(!light.same_future(&full));
    assert!(!initialize.same_future(&light));
    assert_eq!(
        runtime.holder(pos).unwrap().generation_target,
        Some(ChunkStatus::Full)
    );
    runtime
        .shared
        .control
        .release(pos, ChunkStatus::Features, false);
    assert_eq!(
        runtime.wait(&full).unwrap().unwrap().kind(),
        ChunkAccessKind::Level
    );
    assert_eq!(
        first.join().unwrap().unwrap().kind(),
        ChunkAccessKind::Proto
    );
    let state = world.state.lock().unwrap();
    assert!(state.holders.values().all(|holder| holder
        .progress
        .stages
        .iter()
        .all(|s| s.attempts <= 1)));
    assert_eq!(
        state.holders[&(pos.x, pos.z)]
            .progress
            .stage(ChunkStatus::Full)
            .state,
        StageState::Partial
    );
}

#[test]
fn proto_save_preserves_pending_nbt_without_level_envelopes() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(-2, 3);
    let proto = runtime
        .get_chunk(pos, ChunkStatus::Features, true)
        .unwrap()
        .unwrap();
    let saved = proto.prepare_save(0).unwrap();
    assert_eq!(saved.view.kind, ChunkAccessKind::Proto);
    assert_eq!(saved.block_entity_tags.len(), 2);
    assert!(saved
        .block_entity_tags
        .values()
        .all(|tag| tag.get("keepPacked").is_none()));
    assert_eq!(saved.view.chunk.pending_block_entities().len(), 1);

    // An input tag may already carry save metadata. Proto saving must not
    // synthesize, remove or rewrite it, even across repeated observations.
    let mut pending = saved.block_entity_tags[&(1, 80, 1)].clone();
    pending
        .compound_mut()
        .unwrap()
        .insert("keepPacked".into(), Nbt::Byte(0));
    pending
        .compound_mut()
        .unwrap()
        .insert("retained_seed".into(), Nbt::Long(i64::MIN));
    {
        let mut state = world.state.lock().unwrap();
        let chunk = state.region.owned_chunk_mut(pos);
        assert!(chunk.set_pending_block_entity(1, 80, 1, PendingBlockEntity::from_nbt(&pending)));
        // Native getBlockEntityNbtForSaving prefers the live map if both maps
        // contain this position; observing it does not consume the pending tag.
        chunk.pending_block_entities.insert(
            (2, 80, 1),
            PendingBlockEntity::dummy((pos.x * 16 + 2, 80, pos.z * 16 + 1)),
        );
    }
    for _ in 0..2 {
        let observed = proto.prepare_save(0).unwrap();
        assert_eq!(observed.block_entity_tags[&(1, 80, 1)], pending);
        assert_eq!(
            observed.block_entity_tags[&(2, 80, 1)],
            saved.block_entity_tags[&(2, 80, 1)]
        );
        assert_eq!(observed.view.chunk.pending_block_entities().len(), 2);
        assert_eq!(observed.block_ticks, saved.block_ticks);
    }
}

#[test]
fn cache_and_held_future_retain_old_proto_across_full_and_level_save() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(-6, 9);
    let (old_future, old) = light_before_spawn(&runtime, pos);
    let before = old.view().unwrap();
    assert_eq!(before.persisted_status, ChunkStatus::Light);
    assert_eq!(before.chunk.get(4, 90, 4), Some(block::AIR));
    runtime
        .shared
        .control
        .release(pos, ChunkStatus::Spawn, false);
    queued_full(&runtime, pos);
    assert_eq!(old.view().unwrap().persisted_status, ChunkStatus::Spawn);
    assert_eq!(old.view().unwrap().chunk.get(4, 90, 4), Some(block::STONE));
    let pointer = world
        .chunk_access(pos, ChunkStatus::Light)
        .unwrap()
        .unwrap()
        .read(|v| v.chunk.states().as_ptr() as usize)
        .unwrap();
    let level = runtime
        .get_chunk(pos, ChunkStatus::Full, true)
        .unwrap()
        .unwrap();
    assert_eq!(level.kind(), ChunkAccessKind::Level);
    assert!(!old.same_object(&level));
    let cache_hit = runtime
        .get_chunk(pos, ChunkStatus::Light, true)
        .unwrap()
        .unwrap();
    assert!(cache_hit.same_object(&old));
    let replaced = runtime
        .request_future(pos, ChunkStatus::Light, true)
        .unwrap();
    assert!(!replaced.same_future(&old_future));
    for status in ChunkStatus::ALL.into_iter().take(ChunkStatus::Full.index()) {
        assert!(runtime
            .request_future(pos, status, true)
            .unwrap()
            .same_future(&replaced));
    }
    assert_eq!(
        replaced.join().unwrap().unwrap().kind(),
        ChunkAccessKind::ImposterProto
    );
    assert!(old_future.join().unwrap().unwrap().same_object(&old));
    assert_eq!(
        world
            .chunk_access(pos, ChunkStatus::Light)
            .unwrap()
            .unwrap()
            .read(|v| v.kind)
            .unwrap(),
        ChunkAccessKind::ImposterProto
    );
    let level_save = level.prepare_save(0).unwrap();
    assert!(level_save.view.chunk.pending_block_entities().is_empty());
    assert!(level_save.view.chunk.worldgen_heightmaps().is_none());
    assert_eq!(
        level_save.block_entity_tags[&(1, 80, 1)]
            .get("id")
            .and_then(Nbt::string),
        Some("minecraft:sculk_sensor")
    );
    let old_save = old.prepare_save(0).unwrap();
    assert_eq!(old_save.view.persisted_status, ChunkStatus::Spawn);
    assert!(old_save.view.chunk.worldgen_heightmaps().is_some());
    assert_eq!(
        old_save.block_entity_tags[&(1, 80, 1)]
            .get("id")
            .and_then(Nbt::string),
        Some("DUMMY")
    );
    assert!(old_save
        .block_entity_tags
        .values()
        .all(|tag| tag.get("keepPacked").is_none()));
    assert!(level_save
        .block_entity_tags
        .values()
        .all(|tag| tag.get("keepPacked") == Some(&Nbt::Byte(0))));
    assert_eq!(old_save.block_ticks, level_save.block_ticks);
    assert_eq!(
        old_save.view.chunk.tick_requests(),
        before.chunk.tick_requests()
    );
    assert_eq!(
        old_save.view.chunk.postprocessing_positions(),
        before.chunk.postprocessing_positions()
    );
    {
        let mut state = world.state.lock().unwrap();
        let chunk = state.region.owned_chunk_mut(pos);
        chunk.set(7, 95, 7, block::STONE);
        let BlockEntity::DungeonChest { loot_seed } =
            chunk.block_entities.get_mut(&(2, 80, 1)).unwrap()
        else {
            panic!("live chest")
        };
        *loot_seed = 193;
    }
    let shared = old.view().unwrap();
    assert_eq!(shared.chunk.get(7, 95, 7), Some(block::STONE));
    assert_eq!(
        shared.chunk.block_entities()[&(2, 80, 1)],
        BlockEntity::DungeonChest { loot_seed: 193 }
    );
    assert!(
        !shared
            .chunk
            .feature_block_entities()
            .contains_key(&(1, 80, 1)),
        "newly materialized level object is not an old proto member"
    );
    assert_eq!(
        world
            .chunk_access(pos, ChunkStatus::Full)
            .unwrap()
            .unwrap()
            .read(|v| v.chunk.states().as_ptr() as usize)
            .unwrap(),
        pointer
    );
    assert!(!level.view().unwrap().coverage.is_complete());
}

#[test]
fn ready_future_miss_and_cache_hit_do_not_poll_pending_full() {
    let (_, runtime) = fixture();
    let pos = ChunkPos::new(4, 6);
    let (_, old) = light_before_spawn(&runtime, pos);
    runtime
        .shared
        .control
        .release(pos, ChunkStatus::Spawn, false);
    queued_full(&runtime, pos);
    let before = runtime.snapshot();
    assert!(runtime
        .get_chunk(pos, ChunkStatus::Light, true)
        .unwrap()
        .unwrap()
        .same_object(&old));
    assert_eq!(
        runtime
            .get_chunk(pos, ChunkStatus::Biomes, true)
            .unwrap()
            .unwrap()
            .kind(),
        ChunkAccessKind::Proto
    );
    let after = runtime.snapshot();
    assert_eq!(after.main_bodies_executed, before.main_bodies_executed);
    assert_eq!(after.main_queue, before.main_queue);
}

#[test]
fn cache_exact_status_eviction_hits_not_promoted_and_update_invalidation() {
    let (_, runtime) = fixture();
    let pos = ChunkPos::new(3, 8);
    runtime.get_chunk(pos, ChunkStatus::Full, true).unwrap();
    for status in [
        ChunkStatus::Empty,
        ChunkStatus::StructureStarts,
        ChunkStatus::Biomes,
        ChunkStatus::Features,
    ] {
        runtime.get_chunk(pos, status, true).unwrap();
    }
    let before = runtime.snapshot();
    runtime.get_chunk(pos, ChunkStatus::Empty, true).unwrap();
    assert_eq!(runtime.snapshot().cache, before.cache);
    runtime.get_chunk(pos, ChunkStatus::Carvers, true).unwrap();
    assert!(!runtime
        .snapshot()
        .cache
        .contains(&(pos, ChunkStatus::Empty)));
    assert_eq!(runtime.snapshot().cache[0], (pos, ChunkStatus::Carvers));
    assert_eq!(
        runtime.snapshot().cache_invalidations,
        before.cache_invalidations
    );
    // A weaker new source already covered by a propagated stronger level does
    // not change the distance/visible map and must not invalidate the cache.
    let near = ChunkPos::new(pos.x + 1, pos.z);
    runtime
        .request_future(near, ChunkStatus::StructureStarts, true)
        .unwrap();
    runtime.poll_task().unwrap();
    assert_eq!(
        runtime.snapshot().cache_invalidations,
        before.cache_invalidations
    );
    assert_eq!(runtime.snapshot().cache.len(), 4);
    runtime
        .request_future(ChunkPos::new(40, -30), ChunkStatus::Empty, true)
        .unwrap();
    assert!(runtime.snapshot().cache.is_empty());
    assert_eq!(
        runtime.snapshot().cache_invalidations,
        before.cache_invalidations + 1
    );
}

#[test]
fn create_false_null_cache_does_not_prevent_later_creation() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(-17, 24);
    assert!(runtime
        .get_chunk(pos, ChunkStatus::Empty, false)
        .unwrap()
        .is_none());
    let cached = runtime.snapshot().cache;
    assert!(runtime
        .get_chunk(pos, ChunkStatus::Empty, false)
        .unwrap()
        .is_none());
    assert_eq!(runtime.snapshot().cache, cached);
    assert_eq!(runtime.snapshot().holders, 0);
    assert!(world.progress(pos).unwrap().is_none());
    assert!(runtime
        .get_chunk(pos, ChunkStatus::Empty, true)
        .unwrap()
        .is_some());
    assert_eq!(runtime.snapshot().holders, 1);
}

#[test]
fn a_later_incomplete_wait_executes_older_full_but_not_its_own_queued_full() {
    let (world, runtime) = fixture();
    let origin = ChunkPos::new(-2, -7);
    let neighbour = ChunkPos::new(-1, -7);
    let (_, old) = light_before_spawn(&runtime, origin);
    runtime
        .shared
        .control
        .release(origin, ChunkStatus::Spawn, false);
    queued_full(&runtime, origin);
    runtime
        .shared
        .control
        .arm(neighbour, ChunkStatus::Light, false);
    runtime
        .shared
        .control
        .arm(neighbour, ChunkStatus::Spawn, false);
    let next = runtime
        .request_future(neighbour, ChunkStatus::Light, true)
        .unwrap();
    runtime
        .shared
        .control
        .reached(neighbour, ChunkStatus::Light, false);
    assert!(!next.is_done());
    thread::scope(|scope| {
        scope.spawn(|| {
            wait_state(&runtime, |state| state.main_bodies_executed == 1);
            runtime
                .shared
                .control
                .release(neighbour, ChunkStatus::Light, false);
        });
        let object = runtime.wait(&next).unwrap().unwrap();
        assert_eq!(object.kind(), ChunkAccessKind::Proto);
    });
    runtime
        .shared
        .control
        .reached(neighbour, ChunkStatus::Spawn, false);
    assert_eq!(
        world
            .chunk_access(origin, ChunkStatus::Full)
            .unwrap()
            .unwrap()
            .read(|v| v.kind)
            .unwrap(),
        ChunkAccessKind::Level
    );
    assert_eq!(old.kind(), ChunkAccessKind::Proto);
    runtime
        .shared
        .control
        .release(neighbour, ChunkStatus::Spawn, false);
    queued_full(&runtime, neighbour);
    let ready = runtime
        .request_future(origin, ChunkStatus::Light, true)
        .unwrap();
    assert_eq!(
        runtime.wait(&ready).unwrap().unwrap().kind(),
        ChunkAccessKind::ImposterProto
    );
    assert_eq!(runtime.snapshot().main_bodies_executed, 1);
    assert_eq!(full_queue(&runtime), vec![neighbour]);
}

#[test]
fn main_executor_runs_one_fifo_body_and_ready_full_leaves_the_next_pending() {
    let (_, runtime) = fixture();
    let first = ChunkPos::new(6, 2);
    let second = ChunkPos::new(30, 2);
    let (_, _) = light_before_spawn(&runtime, first);
    runtime
        .shared
        .control
        .release(first, ChunkStatus::Spawn, false);
    queued_full(&runtime, first);
    // Nonblocking admission drives worker tasks without pumping older main work.
    let second_light = runtime
        .request_future(second, ChunkStatus::Light, true)
        .unwrap();
    assert!(second_light.join().unwrap().is_some());
    queued_full(&runtime, second);
    assert_eq!(runtime.snapshot().main_queue, vec![first, second]);
    runtime.poll_task().unwrap();
    let queued = runtime.snapshot().main_tasks;
    assert_eq!(
        queued,
        vec![
            MainTaskSnapshot {
                pos: second,
                kind: MainTaskKind::FullConversion
            },
            MainTaskSnapshot {
                pos: first,
                kind: MainTaskKind::FullPromotion
            },
        ]
    );
    assert_eq!(
        runtime
            .get_chunk(first, ChunkStatus::Full, true)
            .unwrap()
            .unwrap()
            .kind(),
        ChunkAccessKind::Level
    );
    assert_eq!(runtime.snapshot().main_bodies_executed, 1);
    assert_eq!(runtime.snapshot().main_tasks, queued);
    runtime.poll_task().unwrap();
    assert!(full_queue(&runtime).is_empty());
    assert_eq!(
        runtime.snapshot().main_tasks,
        vec![
            MainTaskSnapshot {
                pos: first,
                kind: MainTaskKind::FullPromotion
            },
            MainTaskSnapshot {
                pos: second,
                kind: MainTaskKind::FullPromotion
            },
        ]
    );
    assert!(runtime.poll_task().unwrap());
    assert!(runtime.holder(first).unwrap().promotion_callback_executed);
    assert!(!runtime.holder(second).unwrap().promotion_callback_executed);
    assert!(runtime.poll_task().unwrap());
    assert!(runtime.holder(second).unwrap().promotion_callback_executed);
    assert!(runtime.snapshot().main_queue.is_empty());
    assert_eq!(runtime.snapshot().main_bodies_executed, 2);
    assert_eq!(runtime.snapshot().promotion_callbacks_executed, 2);
}

#[test]
fn accessibility_status_and_supplier_futures_are_distinct_and_ready_waits_keep_promotion_queued() {
    let (_, runtime) = fixture();
    let pos = ChunkPos::new(8, -9);
    let (light, proto) = light_before_spawn(&runtime, pos);
    let full = runtime
        .request_future(pos, ChunkStatus::Full, true)
        .unwrap();
    let holder = runtime.holder(pos).unwrap();
    let accessible = holder.full_accessibility_future.unwrap();
    assert!(!accessible.same_future(&full));
    assert!(!accessible.same_future(&light));
    assert!(!accessible.is_done());
    assert!(!full.is_done());
    assert!(holder.full_supplier_future.is_none());
    runtime
        .shared
        .control
        .release(pos, ChunkStatus::Spawn, false);
    queued_full(&runtime, pos);
    let supplier = runtime.holder(pos).unwrap().full_supplier_future.unwrap();
    assert!(!supplier.same_future(&full));
    assert!(!supplier.same_future(&accessible));
    assert!(!supplier.is_done());
    let level = runtime.wait(&full).unwrap().unwrap();
    assert_eq!(level.kind(), ChunkAccessKind::Level);
    assert!(light.join().unwrap().unwrap().same_object(&proto));
    let after_full = runtime.snapshot();
    assert_eq!(
        after_full.main_tasks,
        vec![MainTaskSnapshot {
            pos,
            kind: MainTaskKind::FullPromotion
        }]
    );
    assert!(runtime
        .wait(&accessible)
        .unwrap()
        .unwrap()
        .same_object(&level));
    assert!(runtime
        .wait(&supplier)
        .unwrap()
        .unwrap()
        .same_object(&level));
    let after_waits = runtime.snapshot();
    assert_eq!(after_waits.main_tasks, after_full.main_tasks);
    assert_eq!(
        after_waits.main_bodies_executed,
        after_full.main_bodies_executed
    );
    assert_eq!(after_waits.promotion_callbacks_executed, 0);
    assert!(runtime.holder(pos).unwrap().promotion_callback_queued);
    runtime.shutdown();
    assert!(accessible
        .get_now()
        .unwrap()
        .unwrap()
        .unwrap()
        .same_object(&level));
    assert!(supplier
        .get_now()
        .unwrap()
        .unwrap()
        .unwrap()
        .same_object(&level));
    assert!(!runtime.holder(pos).unwrap().promotion_callback_executed);
}

#[test]
fn worker_prioritizes_stronger_tickets_then_equal_priority_submission_order() {
    let (_, runtime) = fixture();
    let blocker = ChunkPos::new(-50, -50);
    let first = ChunkPos::new(30, 30);
    let second = ChunkPos::new(40, 40);
    let strong = ChunkPos::new(0, 0);
    runtime
        .shared
        .control
        .arm(blocker, ChunkStatus::Empty, false);
    runtime.shared.control.arm(first, ChunkStatus::Empty, false);
    runtime
        .shared
        .control
        .arm(strong, ChunkStatus::Features, false);
    let blocked = runtime
        .request_future(blocker, ChunkStatus::Empty, true)
        .unwrap();
    runtime.poll_task().unwrap();
    runtime
        .shared
        .control
        .reached(blocker, ChunkStatus::Empty, false);
    let first_future = runtime
        .request_future(first, ChunkStatus::Empty, true)
        .unwrap();
    let second_future = runtime
        .request_future(second, ChunkStatus::Empty, true)
        .unwrap();
    let strong_future = runtime
        .request_future(strong, ChunkStatus::Features, true)
        .unwrap();
    runtime.poll_task().unwrap();
    runtime
        .shared
        .control
        .release(blocker, ChunkStatus::Empty, false);
    runtime
        .shared
        .control
        .reached(strong, ChunkStatus::Features, false);
    assert!(blocked.is_done());
    assert!(!first_future.is_done());
    assert!(!second_future.is_done());
    runtime
        .shared
        .control
        .release(strong, ChunkStatus::Features, false);
    runtime
        .shared
        .control
        .reached(first, ChunkStatus::Empty, false);
    assert!(strong_future.is_done());
    assert!(!second_future.is_done());
    runtime
        .shared
        .control
        .release(first, ChunkStatus::Empty, false);
    assert!(first_future.join().unwrap().is_some());
    assert!(second_future.join().unwrap().is_some());
}

#[test]
fn returned_read_allows_reentrant_requests_without_holding_core_borrows() {
    let (_, runtime) = fixture();
    let pos = ChunkPos::new(2, 14);
    let object = runtime
        .get_chunk(pos, ChunkStatus::Empty, true)
        .unwrap()
        .unwrap();
    object
        .read(|view| {
            assert_eq!(view.persisted_status, ChunkStatus::Empty);
            assert!(runtime
                .get_chunk(pos, ChunkStatus::Empty, true)
                .unwrap()
                .unwrap()
                .same_object(&object));
            assert_eq!(object.view().unwrap().persisted_status, ChunkStatus::Empty);
        })
        .unwrap();
}

#[test]
fn rejected_coordinates_and_off_thread_requests_have_no_ticket_or_generation_effects() {
    let (world, runtime) = fixture();
    for pos in [ChunkPos::new(i32::MAX, 0), ChunkPos::new(0, i32::MIN)] {
        assert!(matches!(
            runtime.get_chunk(pos, ChunkStatus::Light, true),
            Err(NativeRequestError::Generation(
                GenerationError::CoordinateOutOfRange(_)
            ))
        ));
        assert!(matches!(
            runtime.request_future(pos, ChunkStatus::Empty, false),
            Err(NativeRequestError::Generation(
                GenerationError::CoordinateOutOfRange(_)
            ))
        ));
    }
    thread::scope(|scope| {
        scope
            .spawn(|| {
                assert!(matches!(
                    runtime.request_future(ChunkPos::new(0, 0), ChunkStatus::Empty, true),
                    Err(NativeRequestError::WrongThread)
                ))
            })
            .join()
            .unwrap();
    });
    assert_eq!(runtime.snapshot().holders, 0);
    assert!(world.state.lock().unwrap().holders.is_empty());
    assert!(!lock(&runtime.shared.state).tickets.has_pending());
}

#[test]
fn stage_failure_resolves_requested_dependants_once_and_keeps_worker_usable() {
    let (world, runtime) = fixture();
    world.state.lock().unwrap().test_stage = Some(|status, _, _| {
        if status == ChunkStatus::Features {
            Err("controlled feature failure".into())
        } else {
            Ok(())
        }
    });
    let pos = ChunkPos::new(12, -12);
    let light = runtime
        .request_future(pos, ChunkStatus::Light, true)
        .unwrap();
    let full = runtime
        .request_future(pos, ChunkStatus::Full, true)
        .unwrap();
    let error = runtime.wait(&full).unwrap_err();
    assert!(matches!(
        error,
        NativeRequestError::Generation(GenerationError::StageFailed {
            status: ChunkStatus::Features,
            ..
        })
    ));
    assert_eq!(light.join().unwrap_err(), error);
    assert_eq!(
        runtime.get_chunk(pos, ChunkStatus::Full, true).unwrap_err(),
        error
    );
    assert!(world
        .state
        .lock()
        .unwrap()
        .holders
        .values()
        .all(|holder| holder.progress.stages.iter().all(|s| s.attempts <= 1)));
    assert!(runtime
        .get_chunk(ChunkPos::new(80, 80), ChunkStatus::Empty, true)
        .unwrap()
        .is_some());
}

#[test]
fn shutdown_cancels_pending_worker_and_main_futures_without_losing_ready_results() {
    let (_, runtime) = fixture();
    let pos = ChunkPos::new(-4, -10);
    let (ready, object) = light_before_spawn(&runtime, pos);
    let pending = runtime
        .request_future(pos, ChunkStatus::Full, true)
        .unwrap();
    let waiting = pending.clone();
    let waiter = thread::spawn(move || waiting.join());
    runtime.shutdown();
    assert_eq!(
        waiter.join().unwrap().unwrap_err(),
        NativeRequestError::Closed
    );
    assert!(ready.join().unwrap().unwrap().same_object(&object));
    assert!(lock(&runtime.worker).is_none());
    assert!(!runtime.snapshot().worker_active);
    assert_eq!(runtime.snapshot().pending_generation_tasks, 0);
    assert!(runtime.snapshot().main_queue.is_empty());
    assert!(matches!(
        runtime.get_chunk(pos, ChunkStatus::Light, true),
        Err(NativeRequestError::Closed)
    ));
    runtime.shutdown();
}

#[test]
fn dropping_runtime_cancels_unflushed_futures_and_joins_worker() {
    let (world, runtime) = fixture();
    let future = runtime
        .request_future(ChunkPos::new(11, -21), ChunkStatus::Empty, true)
        .unwrap();
    assert!(!future.is_done());
    let baseline = Arc::strong_count(&world);
    drop(runtime);
    assert_eq!(future.join().unwrap_err(), NativeRequestError::Closed);
    assert!(
        Arc::strong_count(&world) < baseline,
        "worker no longer retains the core"
    );
}

#[test]
fn worker_unwind_resolves_all_waiters_and_is_joined_on_shutdown() {
    let (_, runtime) = fixture();
    let pos = ChunkPos::new(15, 3);
    lock(&runtime.shared.control.state).panic_at = Some((pos.x, pos.z, ChunkStatus::Empty, false));
    let future = runtime
        .request_future(pos, ChunkStatus::Empty, true)
        .unwrap();
    if let Err(error) = runtime.poll_task() {
        assert!(matches!(error, NativeRequestError::WorkerPanicked(_)));
    }
    assert!(matches!(
        future.join(),
        Err(NativeRequestError::WorkerPanicked(_))
    ));
    assert!(runtime.snapshot().closed);
    runtime.shutdown();
    assert!(!runtime.snapshot().worker_active);
    assert!(lock(&runtime.worker).is_none());
}

#[test]
fn foreign_future_cannot_drive_another_worlds_main_queue() {
    let (_, first) = fixture();
    let (_, second) = fixture();
    let unloaded = first
        .request_future(ChunkPos::new(9, 17), ChunkStatus::Empty, false)
        .unwrap();
    assert_eq!(
        second.wait(&unloaded).unwrap_err(),
        NativeRequestError::ForeignFuture
    );
    assert_eq!(second.snapshot().holders, 0);
}

#[test]
fn admission_observing_core_poison_closes_unflushed_futures() {
    let (world, runtime) = fixture();
    let ready_pos = ChunkPos::new(1, 2);
    let ready = runtime
        .request_future(ready_pos, ChunkStatus::Empty, true)
        .unwrap();
    runtime.wait(&ready).unwrap();
    idle_worker(&runtime);
    let pending_pos = ChunkPos::new(25, 27);
    let pending = runtime
        .request_future(pending_pos, ChunkStatus::Empty, true)
        .unwrap();
    assert!(!pending.is_done());
    let access = world
        .chunk_access(ready_pos, ChunkStatus::Empty)
        .unwrap()
        .unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        access.read(|_| panic!("poison the core from a borrowed callback"))
    }));
    assert!(panic.is_err());
    let error = runtime
        .request_future(pending_pos, ChunkStatus::Empty, true)
        .unwrap_err();
    assert_eq!(
        error,
        NativeRequestError::Generation(GenerationError::Poisoned)
    );
    assert_eq!(pending.get_now().unwrap().unwrap_err(), error);
    assert!(ready.get_now().unwrap().is_ok());
    assert!(runtime.snapshot().closed);
    runtime.shutdown();
    assert!(lock(&runtime.worker).is_none());
}

#[test]
fn shutdown_resolves_an_already_queued_full_body_without_converting() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(-11, 6);
    light_before_spawn(&runtime, pos);
    runtime
        .shared
        .control
        .release(pos, ChunkStatus::Spawn, false);
    queued_full(&runtime, pos);
    let full = runtime
        .request_future(pos, ChunkStatus::Full, true)
        .unwrap();
    assert!(!full.is_done());
    let holder = runtime.holder(pos).unwrap();
    let accessible = holder.full_accessibility_future.unwrap();
    let supplier = holder.full_supplier_future.unwrap();
    runtime.shutdown();
    assert_eq!(full.join().unwrap_err(), NativeRequestError::Closed);
    assert_eq!(
        accessible.get_now().unwrap().unwrap_err(),
        NativeRequestError::Closed
    );
    assert_eq!(
        supplier.get_now().unwrap().unwrap_err(),
        NativeRequestError::Closed
    );
    assert!(runtime.snapshot().main_queue.is_empty());
    assert_eq!(runtime.snapshot().main_bodies_executed, 0);
    assert_eq!(
        world
            .progress(pos)
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::Full)
            .state,
        StageState::Pending
    );
}

#[test]
fn failed_main_conversion_resolves_full_once_and_keeps_completed_proto_future() {
    let (world, runtime) = fixture();
    world.state.lock().unwrap().test_stage = Some(|status, pos, region| {
        stage(status, pos, region)?;
        if status == ChunkStatus::Spawn {
            region.owned_chunk_mut(pos).tick_requests.push(TickRequest {
                block_pos: [(pos.x + 2) * 16, 80, pos.z * 16],
                target: TickTarget::Block(block::STONE),
                delay: 0,
            });
        }
        Ok(())
    });
    let pos = ChunkPos::new(-14, 5);
    let (light, proto) = light_before_spawn(&runtime, pos);
    runtime
        .shared
        .control
        .release(pos, ChunkStatus::Spawn, false);
    queued_full(&runtime, pos);
    let error = runtime.get_chunk(pos, ChunkStatus::Full, true).unwrap_err();
    assert!(matches!(
        error,
        NativeRequestError::Generation(GenerationError::StageFailed {
            status: ChunkStatus::Full,
            ..
        })
    ));
    assert_eq!(
        runtime.get_chunk(pos, ChunkStatus::Full, true).unwrap_err(),
        error
    );
    assert!(light.join().unwrap().unwrap().same_object(&proto));
    assert_eq!(proto.kind(), ChunkAccessKind::Proto);
    assert_eq!(
        world
            .progress(pos)
            .unwrap()
            .unwrap()
            .stage(ChunkStatus::Full)
            .attempts,
        1
    );
    assert_eq!(runtime.snapshot().main_bodies_executed, 1);
}

#[test]
fn unexpected_main_unwind_closes_every_pending_future() {
    let (world, runtime) = fixture();
    let pos = ChunkPos::new(-19, 8);
    let (light, proto) = light_before_spawn(&runtime, pos);
    runtime
        .shared
        .control
        .release(pos, ChunkStatus::Spawn, false);
    queued_full(&runtime, pos);
    let full = runtime
        .request_future(pos, ChunkStatus::Full, true)
        .unwrap();
    let holder = runtime.holder(pos).unwrap();
    let accessible = holder.full_accessibility_future.unwrap();
    let supplier = holder.full_supplier_future.unwrap();
    // run_stage's claim assertion is outside its stage-body catch_unwind.
    idle_worker(&runtime);
    world
        .state
        .lock()
        .unwrap()
        .holders
        .get_mut(&(pos.x, pos.z))
        .unwrap()
        .progress
        .stages[ChunkStatus::Full.index()]
    .state = StageState::Running;
    let error = runtime.poll_task().unwrap_err();
    assert!(matches!(error, NativeRequestError::MainPanicked(_)));
    for pending in [full, accessible, supplier] {
        assert_eq!(pending.get_now().unwrap().unwrap_err(), error);
    }
    assert!(light
        .get_now()
        .unwrap()
        .unwrap()
        .unwrap()
        .same_object(&proto));
    assert!(runtime.snapshot().closed);
    runtime.shutdown();
    assert!(!runtime.snapshot().worker_active);
    assert!(lock(&runtime.worker).is_none());
}
