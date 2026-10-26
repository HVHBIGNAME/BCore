//! Stateful pillar placement uses actual packets, retained holders and BCC saves.
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
use bcore_worldgen::generation::ChunkStatus;

use client::{changed, Client};
use server::Fixture;

fn native_state(item: &str, face: u8) -> u32 {
    let capture: serde_json::Value =
        serde_json::from_str(include_str!("../data/pillar_placement_26_2.json")).unwrap();
    capture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["input"]["item"] == item && row["input"]["face"] == face)
        .unwrap()["input"]["state_id"]
        .as_u64()
        .unwrap() as u32
}

#[test]
fn all_six_pillar_faces_reach_viewers_and_canonical_holders_then_survive_restart() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    let mut viewer = Client::join(live.address, "Viewer", 2, true);
    builder.mode(1);
    builder.move_to(13.5, 65.0, -5.5);
    let anchor = BlockPos::new(15, 66, -9);
    builder.hotbar(0, "minecraft:diamond_block");
    builder.place(BlockPos::new(15, 65, -9), 1, 0, [0.5, 1.0, 0.5], false, 1);
    builder.ack(1);
    assert_eq!(fixture.read(anchor), block_state::DIAMOND_BLOCK);
    builder.clear();
    builder.action(0, BlockPos::new(15, 65, -9), 2);
    builder.ack(2);
    assert_eq!(fixture.read(BlockPos::new(15, 65, -9)), block_state::AIR);
    builder.hotbar(8, "minecraft:oak_log");
    builder.carried(8);
    let retained = [0, 1].map(|x| {
        live.state
            .world()
            .request_native_chunk(x, -1, ChunkStatus::Full, false)
            .unwrap()
            .unwrap()
    });
    let cached = live.state.world().try_chunk_payload(0, -1).unwrap();
    let mut expected = Vec::new();
    for face in 0..6 {
        let pos = adjacent(anchor, face);
        let state = native_state("minecraft:oak_log", face);
        builder.clear();
        viewer.clear();
        builder.place(anchor, face, 0, [0.5; 3], false, 10 + i32::from(face));
        builder.ack(10 + i32::from(face));
        assert!(changed(&builder.seen, pos, state), "face {face}");
        assert!(
            viewer.wait(|packets| changed(packets, pos, state)),
            "face {face}"
        );
        let (x, y, z) = pos.local();
        assert_eq!(
            retained[pos.chunk().x as usize]
                .view()
                .unwrap()
                .chunk
                .get(x as usize, y, z as usize),
            Some(state)
        );
        assert_eq!(fixture.read(pos), state);
        expected.push((pos, state));
    }
    assert_ne!(live.state.world().try_chunk_payload(0, -1).unwrap(), cached);
    assert_eq!(fixture.store.saved_chunks().unwrap().len(), 9);
    assert_eq!(
        fixture
            .store
            .load(0, -1)
            .unwrap()
            .unwrap()
            .pending_block_entities(),
        fixture
            .originals
            .iter()
            .find(|(pos, _)| *pos == (0, -1))
            .unwrap()
            .1
            .pending_block_entities()
    );
    let session = builder.session;
    drop(builder);
    drop(viewer);
    live.stop();
    let saved = fixture.bytes();
    let live = fixture.live();
    let viewer = Client::join(live.address, "Viewer", 3, true);
    assert_ne!(viewer.session, session);
    for (pos, state) in expected {
        let (x, y, z) = pos.local();
        assert_eq!(
            live.state
                .world()
                .try_chunk(pos.chunk().x, pos.chunk().z)
                .unwrap()
                .0
                .get(x as usize, y, z as usize),
            Some(state)
        );
    }
    for x in [0i32, 1] {
        let body = &viewer
            .seen
            .iter()
            .find(|packet| {
                packet.id == bcore_protocol::world::CB_MAP_CHUNK
                    && packet.body[..4] == x.to_be_bytes()
                    && packet.body[4..8] == (-1i32).to_be_bytes()
            })
            .unwrap()
            .body;
        assert_eq!(
            body,
            &fixture
                .store
                .load(x, -1)
                .unwrap()
                .unwrap()
                .encode_payload(x, -1)
        );
    }
    drop(viewer);
    live.stop();
    assert_eq!(
        fixture.bytes(),
        saved,
        "restart never rewrites pillar saves"
    );
}

#[test]
fn negative_pillar_chains_break_each_axis_and_reject_unsupported_requests_without_writes() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    let mut viewer = Client::join(live.address, "Viewer", 2, true);
    builder.mode(1);
    builder.move_to(3.5, 65.0, -6.5);
    builder.hotbar(0, "minecraft:deepslate");
    let mut clicked = BlockPos::new(-1, 64, -9);
    let mut placed = Vec::new();
    for (face, sequence) in [(1, 1), (4, 2), (2, 3)] {
        let pos = adjacent(clicked, face);
        let state = native_state("minecraft:deepslate", face);
        builder.clear();
        builder.place(clicked, face, 0, [0.5; 3], false, sequence);
        builder.ack(sequence);
        assert!(changed(&builder.seen, pos, state));
        assert!(viewer.wait(|packets| changed(packets, pos, state)));
        assert_eq!(fixture.read(pos), state);
        placed.push((pos, state));
        clicked = pos;
    }
    let before = fixture.bytes();
    for item in [
        "minecraft:oak_stairs",
        "minecraft:iron_chain",
        "minecraft:chest",
    ] {
        builder.hotbar(0, item);
        let mut payload = Cursor::new(
            &builder
                .seen
                .iter()
                .find(|packet| packet.id == CB_PLAYER_INVENTORY)
                .unwrap()
                .body,
        );
        assert_eq!(read_varint(&mut payload).unwrap(), 0);
        assert_eq!(read_varint(&mut payload).unwrap(), 64);
        assert_eq!(
            read_varint(&mut payload).unwrap(),
            bcore_registry::registry_id("ITEM", "minecraft:deepslate").unwrap() as i32
        );
    }
    for (n, &(pos, state)) in placed.iter().enumerate() {
        for action in [1, 2] {
            let sequence = 10 + n as i32 * 2 + i32::from(action);
            builder.clear();
            builder.action(action, pos, sequence);
            builder.ack(sequence);
            assert!(changed(&builder.seen, pos, state));
        }
    }
    builder.mode(0);
    builder.clear();
    builder.action(0, placed[0].0, 20);
    builder.ack(20);
    builder.mode(1);
    for (sequence, hand, hit, offsets) in [
        (21, 1, placed[0].0, [0.5; 3]),
        (22, 0, placed[0].0, [f32::NAN, 0.5, 0.5]),
        (23, 0, BlockPos::new(2000, 64, -9), [0.5; 3]),
        (24, 0, BlockPos::new(2, 65, -9), [0.5; 3]),
    ] {
        builder.clear();
        builder.place(hit, 1, hand, offsets, false, sequence);
        builder.ack(sequence);
    }
    builder.move_to(3.5, 65.0, -8.5);
    builder.clear();
    builder.place(BlockPos::new(3, 64, -9), 1, 0, [0.5; 3], false, 25);
    builder.ack(25);
    assert_eq!(fixture.bytes(), before);
    viewer.clear();
    assert!(
        !viewer.collect(Duration::from_millis(150), |packets| packets
            .iter()
            .any(|packet| packet.id == CB_BLOCK_UPDATE))
    );
    builder.move_to(3.5, 65.0, -6.5);
    for (n, &(pos, _)) in placed.iter().enumerate() {
        builder.clear();
        builder.action(0, pos, 30 + n as i32);
        builder.ack(30 + n as i32);
        assert!(changed(&builder.seen, pos, block_state::AIR));
        assert!(viewer.wait(|packets| changed(packets, pos, block_state::AIR)));
        assert_eq!(fixture.read(pos), block_state::AIR);
    }
    assert_eq!(fixture.store.saved_chunks().unwrap().len(), 9);
    drop(builder);
    drop(viewer);
    live.stop();
}

#[test]
fn pillars_do_not_publish_to_old_views_and_malformed_stacks_disconnect_without_writes() {
    let fixture = Fixture::new();
    let live = fixture.live();
    let mut builder = Client::join(live.address, "Builder", 1, true);
    let mut far = Client::join(live.address, "Far", 2, true);
    builder.mode(1);
    builder.move_to(13.5, 65.0, -5.5);
    builder.hotbar(0, "minecraft:basalt");
    far.confirm_teleports = false;
    far.clear();
    far.command("tp 1024 65 -8.5");
    assert!(far.wait(|packets| packets
        .iter()
        .any(|packet| packet.id == bcore_protocol::world::CB_POSITION)));
    far.clear();
    let clicked = BlockPos::new(15, 65, -9);
    let pos = adjacent(clicked, 5);
    let state = native_state("minecraft:basalt", 5);
    builder.clear();
    builder.place(clicked, 5, 0, [0.5; 3], false, 1);
    builder.ack(1);
    assert!(changed(&builder.seen, pos, state));
    assert_eq!(fixture.read(pos), state);
    assert!(!far.collect(Duration::from_millis(150), |packets| changed(
        packets, pos, state
    )));
    let before = fixture.bytes();
    let mut bad = Client::join(live.address, "Malformed", 3, true);
    bad.send(SB_SET_CREATIVE_SLOT, &[0, 36, 65, 8, 0, 0]);
    bad.expect_closed();
    assert_eq!(fixture.bytes(), before);
    assert_eq!(fixture.store.saved_chunks().unwrap().len(), 9);
    drop(bad);
    drop(builder);
    drop(far);
    live.stop();
}
