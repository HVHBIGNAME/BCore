use super::*;
use crate::chunk::ChunkColumn;
use crate::entity::{CB_REMOVE_ENTITIES, CB_SPAWN_ENTITY};
use crate::packet::read_frame;
use bcore_core::ChunkPos;
use bcore_worldgen::generated_entity::GeneratedEntity;
use std::io::{self, Cursor};
use std::sync::Arc;

fn column() -> ChunkColumn {
    let mut column = ChunkColumn::flat();
    assert!(column.add_entity(
        ChunkPos::new(-1, 0),
        GeneratedEntity::ChestMinecart {
            block_pos: [-1, 32, 1],
            loot_seed: 42,
        }
    ));
    column
}

fn world() -> World {
    let world = World::flat_fixture([(0, 0), (5, 0)]);
    world.cache_column(-1, 0, &column());
    world
}

fn view() -> PlayerView {
    PlayerView::new(-0.5, 33.0, 1.5).with_view_distance(0)
}

fn frames(bytes: &[u8]) -> Vec<(i32, Vec<u8>)> {
    let mut input = Cursor::new(bytes);
    let mut packets = Vec::new();
    while input.position() < bytes.len() as u64 {
        packets.push(read_frame(&mut input).unwrap());
    }
    packets
}

#[test]
fn generated_cart_spawns_after_its_chunk_exactly_once() {
    let world = world();
    let mut view = view();
    let mut out = Vec::new();
    assert_eq!(view.stream_chunks_from(&mut out, &world).unwrap(), 1);
    let packets = frames(&out);
    assert_eq!(
        packets.iter().map(|p| p.0).collect::<Vec<_>>(),
        [
            CB_UPDATE_VIEW_POSITION,
            CB_CHUNK_BATCH_START,
            CB_MAP_CHUNK,
            CB_CHUNK_BATCH_FINISHED,
            CB_SPAWN_ENTITY
        ]
    );
    let cart = &view.tracked_entities[&(-1, 0)][0];
    assert!(cart.id > crate::join::PLAY_LOGIN_ENTITY_ID);
    assert_eq!(frames(&cart.spawn_packet())[0], packets[4]);
    assert_eq!(cart.generated.data()["LootTableSeed"], 42);
    let mut again = Vec::new();
    assert_eq!(view.stream_chunks_from(&mut again, &world).unwrap(), 0);
    assert!(again.is_empty());
}

#[test]
fn leaving_a_chunk_removes_only_its_entities() {
    let world = world();
    let mut view = view();
    view.stream_chunks_from(&mut Vec::new(), &world).unwrap();
    let old = view.tracked_entities[&(-1, 0)][0].id;
    view.x = 0.5;
    let mut out = Vec::new();
    assert_eq!(view.stream_chunks_from(&mut out, &world).unwrap(), 1);
    let packets = frames(&out);
    let removed = packets.iter().find(|p| p.0 == CB_REMOVE_ENTITIES).unwrap();
    assert_eq!(*removed, frames(&encode_remove_entities(&[old]))[0]);
    assert!(packets.iter().all(|p| p.0 != CB_SPAWN_ENTITY));
    assert_eq!(packets.iter().filter(|p| p.0 == CB_UNLOAD_CHUNK).count(), 1);
    assert!(view.tracked_entities.is_empty());
    assert_eq!(view.loaded_chunks().copied().collect::<Vec<_>>(), [(0, 0)]);
}

#[test]
fn teleport_removes_before_resending_even_within_the_same_chunk() {
    let world = world();
    let mut view = view();
    view.stream_chunks_from(&mut Vec::new(), &world).unwrap();
    let id = view.tracked_entities[&(-1, 0)][0].id;
    view.teleport(-0.5, 40.0, 1.5);
    assert!(view.loaded.is_empty());
    assert!(view.has_pending_chunks());
    let mut out = Vec::new();
    view.stream_chunks_from(&mut out, &world).unwrap();
    let packets = frames(&out);
    assert_eq!(
        packets.iter().map(|p| p.0).collect::<Vec<_>>(),
        [
            CB_UPDATE_VIEW_POSITION,
            CB_REMOVE_ENTITIES,
            CB_UNLOAD_CHUNK,
            CB_CHUNK_BATCH_START,
            CB_MAP_CHUNK,
            CB_CHUNK_BATCH_FINISHED,
            CB_SPAWN_ENTITY,
        ]
    );
    assert_eq!(view.tracked_entities[&(-1, 0)][0].id, id);
    assert!(view.retired_chunks.is_empty());
    assert!(!view.has_pending_chunks());
}

#[test]
fn viewers_keep_one_identity_across_payload_eviction() {
    let world = world();
    let mut a = view();
    let mut b = view();
    a.stream_chunks_from(&mut Vec::new(), &world).unwrap();
    b.stream_chunks_from(&mut Vec::new(), &world).unwrap();
    assert!(Arc::ptr_eq(
        &a.tracked_entities[&(-1, 0)],
        &b.tracked_entities[&(-1, 0)]
    ));
    let old_id = a.tracked_entities[&(-1, 0)][0].id;
    let old_uuid = a.tracked_entities[&(-1, 0)][0].uuid;
    let weak = Arc::downgrade(&a.tracked_entities[&(-1, 0)]);
    world.clear_cache();
    world.cache_column(-1, 0, &column());
    let mut c = view();
    c.stream_chunks_from(&mut Vec::new(), &world).unwrap();
    assert!(Arc::ptr_eq(
        &a.tracked_entities[&(-1, 0)],
        &c.tracked_entities[&(-1, 0)]
    ));
    drop((a, b, c));
    world.clear_cache();
    assert!(weak.upgrade().is_none());
    let new = world.cache_column(-1, 0, &column());
    assert_ne!(new.entities[0].id, old_id);
    assert_eq!(new.entities[0].uuid, old_uuid);
}

#[test]
fn persisted_data_restores_uuid_and_duplicate_rows_have_distinct_identities() {
    let mut original = column();
    let duplicate = original.entities()[0].clone();
    assert!(original.add_entity(ChunkPos::new(-1, 0), duplicate));
    let bytes = crate::chunk_store::encode_chunk(-1, 0, &original);
    let (x, z, restored) = crate::chunk_store::decode_chunk_at(&bytes).unwrap();
    assert_eq!((x, z), (-1, 0));
    let a = World::in_memory(0).cache_column(-1, 0, &original);
    let b = World::in_memory(0).cache_column(-1, 0, &restored);
    let c = World::in_memory(1).cache_column(-1, 0, &restored);
    assert_ne!(a.entities[0].id, b.entities[0].id);
    assert_eq!(a.entities[0].uuid, b.entities[0].uuid);
    assert_ne!(a.entities[0].uuid, a.entities[1].uuid);
    assert_ne!(a.entities[0].uuid, c.entities[0].uuid);
}

#[test]
fn concurrent_payload_misses_publish_one_entity_identity() {
    let world = World::in_memory(0);
    let barrier = std::sync::Barrier::new(4);
    let results = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    world.cache_column(-1, 0, &column()).entities
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(results
        .iter()
        .all(|entities| Arc::ptr_eq(entities, &results[0])));
}

struct FailedWrite;
impl Write for FailedWrite {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "test writer"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn failed_writes_do_not_commit_chunk_or_entity_delivery() {
    let world = world();
    let mut view = view();
    assert!(view.stream_chunks_from(&mut FailedWrite, &world).is_err());
    assert!(view.loaded.is_empty());
    assert!(view.tracked_entities.is_empty());
    view.stream_chunks_from(&mut Vec::new(), &world).unwrap();
    view.teleport(-0.5, 40.0, 1.5);
    assert!(view.stream_chunks_from(&mut FailedWrite, &world).is_err());
    assert_eq!(view.retired_chunks.len(), 1);
    assert_eq!(view.tracked_entities.len(), 1);
    let mut out = Vec::new();
    view.stream_chunks_from(&mut out, &world).unwrap();
    let packets = frames(&out);
    assert_eq!(
        packets.iter().filter(|p| p.0 == CB_REMOVE_ENTITIES).count(),
        1
    );
    assert_eq!(packets.iter().filter(|p| p.0 == CB_SPAWN_ENTITY).count(), 1);
}

#[test]
fn adopted_cart_keeps_its_identity_through_cache_eviction_and_unload() {
    let world = world();
    let mut previous = view();
    let mut delivered = Vec::new();
    previous.stream_chunks_from(&mut delivered, &world).unwrap();
    let cart = &previous.tracked_entities[&(-1, 0)][0];
    let id = cart.id;
    assert_eq!(
        frames(&delivered).last().unwrap(),
        &frames(&cart.spawn_packet())[0]
    );
    let weak = Arc::downgrade(&previous.tracked_entities[&(-1, 0)]);
    let state = previous.into_delivery_state();
    world.clear_cache();
    assert!(
        weak.upgrade().is_some(),
        "the transfer owns the delivered identity"
    );

    let mut adopted = view();
    adopted.adopt_delivery_state(state).unwrap();
    assert!(Arc::ptr_eq(
        &weak.upgrade().unwrap(),
        &adopted.tracked_entities[&(-1, 0)]
    ));
    let mut again = Vec::new();
    assert_eq!(adopted.stream_chunks_from(&mut again, &world).unwrap(), 0);
    assert!(again.is_empty());

    world.cache_column(0, 0, &ChunkColumn::flat());
    adopted.x = 0.5;
    assert!(adopted
        .stream_chunks_from(&mut FailedWrite, &world)
        .is_err());
    assert_eq!(
        adopted.loaded_chunks().copied().collect::<Vec<_>>(),
        [(-1, 0)]
    );
    assert!(weak.upgrade().is_some());

    let mut out = Vec::new();
    assert_eq!(adopted.stream_chunks_from(&mut out, &world).unwrap(), 1);
    let packets = frames(&out);
    let removals: Vec<_> = packets
        .iter()
        .filter(|p| p.0 == CB_REMOVE_ENTITIES)
        .cloned()
        .collect();
    assert_eq!(removals, frames(&encode_remove_entities(&[id])));
    assert_eq!(packets.last().unwrap().0, CB_UNLOAD_CHUNK);
    assert!(packets.iter().all(|p| p.0 != CB_SPAWN_ENTITY));
    assert_eq!(
        adopted.loaded_chunks().copied().collect::<Vec<_>>(),
        [(0, 0)]
    );
    assert!(adopted.tracked_entities.is_empty());
    assert!(
        weak.upgrade().is_none(),
        "successful unload releases the identity"
    );
}

#[test]
fn adoption_preserves_pending_teleport_retirements_and_teleport_ids() {
    let world = world();
    let mut previous = view();
    previous
        .stream_chunks_from(&mut Vec::new(), &world)
        .unwrap();
    let id = previous.tracked_entities[&(-1, 0)][0].id;
    let weak = Arc::downgrade(&previous.tracked_entities[&(-1, 0)]);
    previous.teleport(80.5, 40.0, 1.5);
    previous.teleport(-0.5, 40.0, 1.5);
    assert!(previous
        .stream_chunks_from(&mut FailedWrite, &world)
        .is_err());

    let state = previous.into_delivery_state();
    world.clear_cache();
    let mut adopted = PlayerView::new(-0.5, 40.0, 1.5).with_view_distance(0);
    adopted.adopt_delivery_state(state).unwrap();
    assert!(adopted.loaded.is_empty());
    assert_eq!(
        adopted.retired_chunks.iter().copied().collect::<Vec<_>>(),
        [(-1, 0)]
    );
    assert!(adopted.has_pending_chunks());
    assert!(Arc::ptr_eq(
        &weak.upgrade().unwrap(),
        &adopted.tracked_entities[&(-1, 0)]
    ));
    let teleport = frames(&adopted.teleport(-0.5, 45.0, 1.5));
    assert_eq!(
        bcore_core::varint::decode_varint(&teleport[0].1).unwrap().0,
        4
    );

    world.cache_column(-1, 0, &column());
    assert!(adopted
        .stream_chunks_from(&mut FailedWrite, &world)
        .is_err());
    assert!(adopted.loaded.is_empty());
    assert!(adopted.retired_chunks.contains(&(-1, 0)));
    assert_eq!(adopted.tracked_entities[&(-1, 0)][0].id, id);
    let mut out = Vec::new();
    assert_eq!(adopted.stream_chunks_from(&mut out, &world).unwrap(), 1);
    let packets = frames(&out);
    assert_eq!(
        packets.iter().map(|p| p.0).collect::<Vec<_>>(),
        [
            CB_UPDATE_VIEW_POSITION,
            CB_REMOVE_ENTITIES,
            CB_UNLOAD_CHUNK,
            CB_CHUNK_BATCH_START,
            CB_MAP_CHUNK,
            CB_CHUNK_BATCH_FINISHED,
            CB_SPAWN_ENTITY,
        ]
    );
    assert_eq!(packets[1], frames(&encode_remove_entities(&[id]))[0]);
    assert!(Arc::ptr_eq(
        &weak.upgrade().unwrap(),
        &adopted.tracked_entities[&(-1, 0)]
    ));
    assert!(adopted.retired_chunks.is_empty());
    assert!(!adopted.has_pending_chunks());
}

#[test]
fn adoption_rejects_existing_delivery_or_teleport_state_without_losing_ownership() {
    for (delivered, teleported) in [(true, false), (true, true), (false, true)] {
        let world = world();
        let mut previous = view();
        previous
            .stream_chunks_from(&mut Vec::new(), &world)
            .unwrap();
        let weak = Arc::downgrade(&previous.tracked_entities[&(-1, 0)]);
        let mut destination = view();
        if delivered {
            destination
                .stream_chunks_from(&mut Vec::new(), &world)
                .unwrap();
        }
        if teleported {
            destination.teleport(80.5, 40.0, 1.5);
        }
        let loaded = destination.loaded.clone();
        let retired = destination.retired_chunks.clone();
        let next_teleport_id = destination.next_teleport_id;
        let state = destination
            .adopt_delivery_state(previous.into_delivery_state())
            .expect_err("destination already has connection state");
        assert_eq!(destination.loaded, loaded);
        assert_eq!(destination.retired_chunks, retired);
        assert_eq!(destination.next_teleport_id, next_teleport_id);
        if delivered {
            assert!(Arc::ptr_eq(
                &weak.upgrade().unwrap(),
                &destination.tracked_entities[&(-1, 0)]
            ));
        }
        drop(destination);
        world.clear_cache();

        let mut empty = view();
        empty.adopt_delivery_state(state).unwrap();
        assert!(Arc::ptr_eq(
            &weak.upgrade().unwrap(),
            &empty.tracked_entities[&(-1, 0)]
        ));
        let mut out = Vec::new();
        assert_eq!(empty.stream_chunks_from(&mut out, &world).unwrap(), 0);
        assert!(out.is_empty());
    }
}

#[test]
fn adopting_after_a_failed_initial_write_leaves_delivery_pending() {
    let world = world();
    let mut previous = view();
    assert!(previous
        .stream_chunks_from(&mut FailedWrite, &world)
        .is_err());
    let mut adopted = view();
    adopted
        .adopt_delivery_state(previous.into_delivery_state())
        .unwrap();
    assert!(adopted.loaded.is_empty());
    assert!(adopted.tracked_entities.is_empty());
    assert!(adopted.has_pending_chunks());

    let mut out = Vec::new();
    assert_eq!(adopted.stream_chunks_from(&mut out, &world).unwrap(), 1);
    assert_eq!(
        frames(&out)
            .iter()
            .filter(|p| p.0 == CB_SPAWN_ENTITY)
            .count(),
        1
    );
    assert!(!adopted.has_pending_chunks());
}
