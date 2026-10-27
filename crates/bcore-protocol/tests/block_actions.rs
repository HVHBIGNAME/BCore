//! Actual 26.2 packets drive tick-owned mutations, viewer replies and BCC restart.
#[path = "support/block_client.rs"]
mod client;
#[path = "support/block_server.rs"]
mod server;

use std::io::Cursor;
use std::time::Duration;

use bcore_core::BlockPos;
use bcore_protocol::blocks::packets::*;
use bcore_protocol::chunk::block_state;
use bcore_protocol::packet::read_varint;

use client::{changed, Client};
use server::Fixture;

const FLOOR: BlockPos = BlockPos::new(-1, 64, -9);
const PLACED: BlockPos = BlockPos::new(-1, 65, -9);

#[test]
fn rapid_creative_break_burst_is_ordered_published_and_durable() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    let mut viewer = Client::join(live.address, "Viewer", 2, true);
    builder.mode(1);
    builder.move_to(6.5, 65.0, -7.5);
    builder.clear();
    viewer.clear();
    let positions: Vec<_> = (3..11)
        .flat_map(|x| (-12..-8).map(move |z| BlockPos::new(x, 64, z)))
        .collect();
    let started = std::time::Instant::now();
    for (index, &pos) in positions.iter().enumerate() {
        builder.action(0, pos, 100 + index as i32);
    }
    builder.ack(100 + positions.len() as i32 - 1);
    let elapsed = started.elapsed();
    assert!(viewer.wait(|packets| positions.iter().all(|&pos| changed(
        packets,
        pos,
        block_state::AIR
    ))));
    for &pos in &positions {
        assert!(changed(&builder.seen, pos, block_state::AIR));
        assert_eq!(fixture.read(pos), block_state::AIR);
    }
    println!(
        "RUNTIME_BURST={}",
        serde_json::json!({
            "actions": positions.len(), "acknowledged_and_saved_ms": elapsed.as_secs_f64() * 1000.0,
            "actor_light_packets": builder.seen.iter().filter(|packet| packet.id == CB_LIGHT_UPDATE).count(),
            "actor_ack_packets": builder.seen.iter().filter(|packet| packet.id == CB_BLOCK_ACK).count(),
            "viewer_updates": positions.len(), "saved_states_match": true,
        })
    );
    drop(builder);
    drop(viewer);
    live.stop();
}

#[test]
fn creative_can_break_grass_surface_and_place_a_supported_block_on_it() {
    let fixture = Fixture::new();
    let mut column = fixture.store.load(0, -1).unwrap().unwrap();
    column.set(4, 64, 7, block_state::GRASS_BLOCK);
    column.set(5, 64, 7, block_state::GRASS_BLOCK);
    fixture.store.save(0, -1, &column).unwrap();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    builder.mode(1);
    builder.move_to(4.5, 65.0, -6.5);
    builder.hotbar(0, "minecraft:stone");
    builder.clear();
    builder.place(BlockPos::new(5, 64, -9), 1, 0, [0.5; 3], false, 1);
    builder.ack(1);
    assert_eq!(fixture.read(BlockPos::new(5, 65, -9)), block_state::STONE);
    builder.clear();
    builder.action(0, BlockPos::new(4, 64, -9), 2);
    builder.ack(2);
    assert_eq!(fixture.read(BlockPos::new(4, 64, -9)), block_state::AIR);
    drop(builder);
    live.stop();
}

#[test]
fn fragmented_block_action_survives_socket_timeouts_and_preserves_the_next_frame() {
    use std::io::Write;
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    builder.mode(1);
    builder.move_to(3.5, 65.0, -6.5);
    builder.clear();
    let mut payload = vec![0];
    payload.extend_from_slice(&client::packed(FLOOR));
    payload.extend_from_slice(&[1, 7]);
    let mut frame = Vec::new();
    bcore_protocol::packet::write_packet(&mut frame, SB_PLAYER_ACTION, &payload);
    builder.stream.write_all(&frame[..1]).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    builder.stream.write_all(&frame[1..5]).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    builder.stream.write_all(&frame[5..]).unwrap();
    builder.send(bcore_protocol::world::SB_PING_REQUEST, &42i64.to_be_bytes());
    builder.ack(7);
    assert_eq!(fixture.read(FLOOR), block_state::AIR);
    assert!(builder.wait(|packets| packets.iter().any(|packet| packet.id
        == bcore_protocol::world::CB_PING_RESPONSE
        && packet.body == 42i64.to_be_bytes())));
    drop(builder);
    live.stop();
}

#[test]
fn creative_hotbar_place_reaches_other_viewers_and_retained_reads_then_survives_restart() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    let mut viewer = Client::join(live.address, "Viewer", 2, true);
    builder.mode(1);
    builder.move_to(3.5, 65.0, -6.5);
    builder.hotbar(8, "minecraft:diamond_block");
    let reply = builder
        .seen
        .iter()
        .find(|packet| packet.id == CB_PLAYER_INVENTORY)
        .unwrap();
    let mut cursor = Cursor::new(&reply.body);
    assert_eq!(read_varint(&mut cursor).unwrap(), 8);
    assert_eq!(read_varint(&mut cursor).unwrap(), 64);
    assert_eq!(
        read_varint(&mut cursor).unwrap(),
        bcore_registry::registry_id("ITEM", "minecraft:diamond_block").unwrap() as i32
    );
    assert_eq!(read_varint(&mut cursor).unwrap(), 0);
    assert_eq!(read_varint(&mut cursor).unwrap(), 0);
    assert_eq!(cursor.position(), reply.body.len() as u64);
    builder.carried(8);
    let retained = live
        .state
        .world()
        .request_native_chunk(-1, -1, bcore_worldgen::generation::ChunkStatus::Full, false)
        .unwrap()
        .unwrap();
    let original_payload = live.state.world().cached_payload(-1, -1).unwrap();
    builder.clear();
    viewer.clear();
    builder.place(FLOOR, 1, 0, [0.5, 1.0, 0.5], false, 257);
    builder.ack(257);
    assert!(changed(&builder.seen, PLACED, block_state::DIAMOND_BLOCK));
    assert!(viewer.wait(|packets| changed(packets, PLACED, block_state::DIAMOND_BLOCK)));
    assert_eq!(
        retained.view().unwrap().chunk.get(15, 65, 7),
        Some(block_state::DIAMOND_BLOCK)
    );
    assert_eq!(fixture.read(PLACED), block_state::DIAMOND_BLOCK);
    assert_ne!(
        live.state.world().try_chunk_payload(-1, -1).unwrap(),
        original_payload
    );
    let source_session = builder.session;
    live.state.world().native_requests().unwrap().shutdown();
    drop(builder);
    drop(viewer);
    live.stop();
    let saved_bytes = fixture.bytes();
    let live = fixture.live();
    let viewer = Client::join(live.address, "Viewer", 3, true);
    assert_ne!(viewer.session, source_session);
    let packet = viewer
        .seen
        .iter()
        .find(|packet| {
            packet.id == bcore_protocol::world::CB_MAP_CHUNK
                && packet.body[..4] == (-1i32).to_be_bytes()
                && packet.body[4..8] == (-1i32).to_be_bytes()
        })
        .unwrap();
    assert_eq!(
        packet.body,
        fixture
            .store
            .load(-1, -1)
            .unwrap()
            .unwrap()
            .encode_payload(-1, -1)
    );
    assert_eq!(
        live.state
            .world()
            .try_chunk(-1, -1)
            .unwrap()
            .0
            .get(15, 65, 7),
        Some(block_state::DIAMOND_BLOCK)
    );
    drop(viewer);
    live.stop();
    assert_eq!(
        fixture.bytes(),
        saved_bytes,
        "restart/read never overwrites the edit"
    );
}

#[test]
fn creative_start_break_is_instant_and_abort_stop_do_not_destroy_unchanged_blocks() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    builder.mode(1);
    builder.move_to(3.5, 65.0, -6.5);
    let before = fixture.bytes();
    for (action, sequence) in [(1, 1), (2, 2)] {
        builder.clear();
        builder.action(action, FLOOR, sequence);
        builder.ack(sequence);
        assert!(changed(&builder.seen, FLOOR, block_state::STONE));
        assert_eq!(fixture.bytes(), before);
    }
    builder.clear();
    builder.action(0, FLOOR, 3);
    builder.ack(3);
    assert!(changed(&builder.seen, FLOOR, block_state::AIR));
    assert_eq!(fixture.read(FLOOR), block_state::AIR);
    drop(builder);
    live.stop();
    let restart = fixture.live();
    assert_eq!(
        restart
            .state
            .world()
            .try_chunk(-1, -1)
            .unwrap()
            .0
            .get(15, 64, 7),
        Some(block_state::AIR)
    );
    restart.stop();
}

#[test]
fn block_state_mode_reach_hand_hit_loaded_world_and_pending_be_rejections_preserve_saves() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    let mut viewer = Client::join(live.address, "Viewer", 2, true);
    builder.move_to(3.5, 65.0, -6.5);
    let before = fixture.bytes();
    builder.clear();
    builder.action(0, FLOOR, 1);
    builder.ack(1);
    assert!(
        changed(&builder.seen, FLOOR, block_state::STONE),
        "survival rejected"
    );
    builder.mode(1);
    builder.hotbar(0, "minecraft:stone");
    builder.carried(0);
    for (sequence, hand, offsets, border) in [
        (2, 1, [0.5, 1.0, 0.5], false),
        (3, 0, [f32::NAN, 1.0, 0.5], false),
        (4, 0, [0.5, 1.0, 0.5], true),
        (5, 0, [2.0, 1.0, 0.5], false),
    ] {
        builder.clear();
        builder.place(FLOOR, 1, hand, offsets, border, sequence);
        builder.ack(sequence);
        assert_eq!(fixture.bytes(), before);
    }
    for (sequence, target) in [
        (6, BlockPos::new(31, 64, -9)),
        (7, BlockPos::new(2000, 64, -9)),
        (8, BlockPos::new(2, 65, -9)),
        (9, BlockPos::new(-1, 320, -9)),
    ] {
        builder.clear();
        builder.action(0, target, sequence);
        builder.ack(sequence);
        assert_eq!(fixture.bytes(), before);
    }
    // The adjacent floor is already occupied.
    builder.clear();
    builder.place(FLOOR, 5, 0, [1.0, 0.5, 0.5], false, 10);
    builder.ack(10);
    // Standing-player collisions are denied, including the other client.
    builder.move_to(3.5, 65.0, -8.5);
    builder.clear();
    builder.place(BlockPos::new(3, 64, -9), 1, 0, [0.5, 1.0, 0.5], false, 11);
    builder.ack(11);
    builder.move_to(7.5, 65.0, -4.5);
    builder.clear();
    builder.place(BlockPos::new(10, 64, -4), 1, 0, [0.5, 1.0, 0.5], false, 12);
    builder.ack(12);
    assert_eq!(fixture.bytes(), before);
    assert_eq!(
        fixture.store.saved_chunks().unwrap().len(),
        9,
        "unloaded packets never generate"
    );
    let original = fixture
        .originals
        .iter()
        .find(|(pos, _)| *pos == (0, -1))
        .unwrap();
    assert_eq!(
        fixture
            .store
            .load(0, -1)
            .unwrap()
            .unwrap()
            .pending_block_entities(),
        original.1.pending_block_entities()
    );
    viewer.clear();
    assert!(
        !viewer.collect(Duration::from_millis(150), |packets| packets
            .iter()
            .any(|packet| packet.id == CB_BLOCK_UPDATE)),
        "rejected actions never publish to other viewers"
    );
    drop(builder);
    drop(viewer);
    live.stop();
}

#[test]
fn placement_crosses_a_chunk_boundary_and_does_not_publish_to_a_teleported_old_view() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    let mut far = Client::join(live.address, "Far", 2, true);
    builder.mode(1);
    builder.move_to(13.5, 65.0, -5.5);
    builder.hotbar(0, "minecraft:diamond_block");
    far.confirm_teleports = false;
    far.clear();
    far.command("tp 1024 65 -8.5");
    assert!(far.wait(|packets| packets
        .iter()
        .any(|packet| packet.id == bcore_protocol::world::CB_POSITION)));
    far.clear();
    let pos = BlockPos::new(16, 65, -9);
    builder.clear();
    builder.place(BlockPos::new(15, 65, -9), 5, 0, [1.0, 0.5, 0.5], false, 129);
    builder.ack(129);
    assert!(changed(&builder.seen, pos, block_state::DIAMOND_BLOCK));
    assert_eq!(fixture.read(pos), block_state::DIAMOND_BLOCK);
    assert!(!far.collect(Duration::from_millis(150), |packets| changed(
        packets,
        pos,
        block_state::DIAMOND_BLOCK
    )));
    assert_eq!(fixture.store.saved_chunks().unwrap().len(), 9);
    drop(builder);
    drop(far);
    live.stop();
}

#[test]
fn player_loaded_is_required_and_unsupported_creative_items_are_corrected_without_mutation() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, false);
    builder.mode(1);
    builder.move_to(3.5, 65.0, -6.5);
    let before = fixture.bytes();
    builder.clear();
    builder.action(0, FLOOR, 1);
    builder.ack(1);
    assert_eq!(fixture.bytes(), before);
    builder.send(bcore_protocol::world::SB_PLAYER_LOADED, &[]);
    builder.hotbar(0, "minecraft:chest");
    let packet = builder
        .seen
        .iter()
        .find(|packet| packet.id == CB_PLAYER_INVENTORY)
        .unwrap();
    assert_eq!(
        packet.body,
        [0, 0],
        "unsupported BE item retains the previous empty hotbar slot"
    );
    builder.clear();
    builder.place(FLOOR, 1, 0, [0.5, 1.0, 0.5], false, 2);
    builder.ack(2);
    assert_eq!(fixture.bytes(), before);
    builder.hotbar(0, "minecraft:diamond_block");
    builder.clear();
    builder.place(FLOOR, 1, 0, [0.5, 1.0, 0.5], false, 3);
    builder.ack(3);
    assert_eq!(fixture.read(PLACED), block_state::DIAMOND_BLOCK);
    drop(builder);
    live.stop();
}

#[test]
fn malformed_actual_packets_close_the_connection_without_mutating_or_regenerating_chunks() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let before = fixture.bytes();
    let mut negative_sequence = vec![0];
    negative_sequence.extend_from_slice(&client::packed(FLOOR));
    negative_sequence.push(1);
    bcore_core::varint::encode_varint(-1, &mut negative_sequence);
    for (n, id, payload) in [
        (0, SB_SET_CARRIED_ITEM, vec![0, 9]),
        (1, SB_SET_CREATIVE_SLOT, vec![0, 36, 65, 1, 0, 0]),
        (2, SB_PLAYER_ACTION, negative_sequence),
        (3, SB_USE_ITEM_ON, vec![0; 9]),
    ] {
        let mut bad = Client::join(live.address, &format!("Bad{n}"), n as u8 + 1, true);
        bad.send(id, &payload);
        bad.expect_closed();
        assert_eq!(fixture.bytes(), before);
    }
    assert_eq!(fixture.store.saved_chunks().unwrap().len(), 9);
    live.stop();
}
