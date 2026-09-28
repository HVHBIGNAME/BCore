use bcore_core::ChunkPos;
use bcore_protocol::{
    chunk::{block_state, ChunkColumn},
    chunk_store::ChunkStore,
    entity::{CB_REMOVE_ENTITIES, CB_SPAWN_ENTITY},
    packet::{read_frame, read_varint},
    world::{
        ChunkDeliveryState, PlayerView, CB_CHUNK_BATCH_FINISHED, CB_CHUNK_BATCH_START,
        CB_MAP_CHUNK, CB_UNLOAD_CHUNK, CB_UPDATE_VIEW_POSITION,
    },
    world_state::World,
};
use bcore_worldgen::generated_entity::GeneratedEntity;
use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CART_CHUNK: (i32, i32) = (-2, 3);
const EXIT_CHUNK: (i32, i32) = (-1, 3);
type Packet = (i32, Vec<u8>);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let parent = std::env::temp_dir().join("opencode");
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!(
            "bcore-queued-entity-delivery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).expect("new isolated fixture directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A failed test may still have queued readers of these fixtures.
        if !thread::panicking() {
            fs::remove_dir_all(&self.0).expect("remove isolated fixture directory");
        }
    }
}

fn cart_column(marker: u32, carts: &[([i32; 3], i64)]) -> ChunkColumn {
    let mut column = ChunkColumn::flat();
    assert!(column.set(2, 31, 3, marker));
    for &(block_pos, loot_seed) in carts {
        assert!(column.add_entity(
            ChunkPos::new(CART_CHUNK.0, CART_CHUNK.1),
            GeneratedEntity::ChestMinecart {
                block_pos,
                loot_seed
            }
        ));
    }
    column
}

fn wait_cached(world: &World, (x, z): (i32, i32)) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(payload) = world.cached_payload(x, z) {
            return payload;
        }
        assert!(
            Instant::now() < deadline,
            "queued payload ({x}, {z}) did not arrive for seed {}",
            world.seed()
        );
        world.request_payload(x, z);
        thread::sleep(Duration::from_millis(1));
    }
}

fn varints(bytes: &[u8]) -> Vec<i32> {
    let mut input = Cursor::new(bytes);
    let mut values = Vec::new();
    while input.position() < bytes.len() as u64 {
        values.push(read_varint(&mut input).unwrap());
    }
    values
}

#[derive(Debug, PartialEq)]
struct Spawn {
    id: i32,
    uuid: [u8; 16],
    position: [f64; 3],
}

fn decode_spawn(payload: &[u8]) -> Spawn {
    let mut input = Cursor::new(payload);
    let id = read_varint(&mut input).unwrap();
    assert!(id > 0);
    let mut uuid = [0; 16];
    input.read_exact(&mut uuid).unwrap();
    assert_eq!(read_varint(&mut input).unwrap(), 25, "chest minecart type");
    let position = std::array::from_fn(|_| {
        let mut bytes = [0; 8];
        input.read_exact(&mut bytes).unwrap();
        f64::from_be_bytes(bytes)
    });
    assert_eq!(&payload[input.position() as usize..], &[0; 5]);
    Spawn { id, uuid, position }
}

fn stream_one(view: &mut PlayerView, world: &World) -> Vec<Packet> {
    let mut out = Vec::new();
    assert_eq!(view.stream_chunks_from(&mut out, world).unwrap(), 1);
    assert_eq!(
        view.loaded_chunks().copied().collect::<Vec<_>>(),
        [view.chunk()]
    );
    assert!(!view.has_pending_chunks());
    let mut input = Cursor::new(&out);
    let mut packets = Vec::new();
    while input.position() < out.len() as u64 {
        packets.push(read_frame(&mut input).unwrap());
    }
    assert_eq!(packets[0].0, CB_UPDATE_VIEW_POSITION);
    assert_eq!(varints(&packets[0].1), [view.chunk().0, view.chunk().1]);
    packets
}

fn assert_batch(packets: &[Packet], payload: &[u8], positions: &[[f64; 3]]) -> Vec<Spawn> {
    assert_eq!(packets.len(), 3 + positions.len());
    assert_eq!(packets[0], (CB_CHUNK_BATCH_START, Vec::new()));
    assert_eq!(packets[1].0, CB_MAP_CHUNK);
    assert_eq!(
        packets[1].1, payload,
        "map_chunk must match the saved column"
    );
    assert_eq!(packets[2].0, CB_CHUNK_BATCH_FINISHED);
    assert_eq!(varints(&packets[2].1), [1]);
    let spawns: Vec<_> = packets[3..]
        .iter()
        .zip(positions)
        .map(|(packet, position)| {
            assert_eq!(packet.0, CB_SPAWN_ENTITY);
            let spawn = decode_spawn(&packet.1);
            assert_eq!(&spawn.position, position);
            spawn
        })
        .collect();
    for (i, spawn) in spawns.iter().enumerate() {
        assert!(spawns[..i]
            .iter()
            .all(|previous| previous.id != spawn.id && previous.uuid != spawn.uuid));
    }
    spawns
}

fn assert_unload(packets: &[Packet], (x, z): (i32, i32), spawns: &[Spawn]) {
    assert_eq!(
        packets.iter().map(|packet| packet.0).collect::<Vec<_>>(),
        [CB_REMOVE_ENTITIES, CB_UNLOAD_CHUNK]
    );
    let removed = varints(&packets[0].1);
    assert_eq!(removed[0] as usize, spawns.len());
    assert_eq!(
        removed[1..],
        spawns.iter().map(|spawn| spawn.id).collect::<Vec<_>>()
    );
    let coordinates = &packets[1].1;
    assert_eq!(coordinates.len(), 8);
    assert_eq!(i32::from_be_bytes(coordinates[..4].try_into().unwrap()), z);
    assert_eq!(i32::from_be_bytes(coordinates[4..].try_into().unwrap()), x);
}

fn view() -> PlayerView {
    PlayerView::new(-24.5, 64.0, 56.5).with_view_distance(0)
}

#[test]
fn queued_persisted_carts_keep_identity_through_handoff_and_unload() {
    let scratch = Scratch::new();
    let first_store = ChunkStore::at(scratch.0.join("first"));
    let other_store = ChunkStore::at(scratch.0.join("other"));
    let first_column = cart_column(
        block_state::STONE,
        &[([-31, 32, 50], 41), ([-17, 35, 63], -42)],
    );
    // The identical first row makes UUID isolation depend on the world seed.
    let other_column = cart_column(
        block_state::DIRT,
        &[([-31, 32, 50], 41), ([-24, 48, 57], 99)],
    );
    let empty = ChunkColumn::flat();
    for (store, column) in [(&first_store, &first_column), (&other_store, &other_column)] {
        for ((x, z), column) in [(CART_CHUNK, column), (EXIT_CHUNK, &empty)] {
            store.save(x, z, column).unwrap();
            let saved = fs::read(store.chunk_path(x, z)).unwrap();
            assert_eq!(&saved[..6], b"BCC1\x03\x00", "persist BCC version 3");
        }
    }
    let first_payload = first_column.encode_payload(CART_CHUNK.0, CART_CHUNK.1);
    let other_payload = other_column.encode_payload(CART_CHUNK.0, CART_CHUNK.1);
    let empty_payload = empty.encode_payload(EXIT_CHUNK.0, EXIT_CHUNK.1);
    assert_ne!(first_payload, other_payload);
    let first_positions = [[-30.5, 32.5, 50.5], [-16.5, 35.5, 63.5]];
    let other_positions = [[-30.5, 32.5, 50.5], [-23.5, 48.5, 57.5]];

    let first = World::with_store(11, first_store.clone());
    let shared = first.clone();
    let other = World::with_store(-37, other_store.clone());
    for world in [&first, &other] {
        assert_eq!(world.cached_payloads(), 0);
        world.request_payload(CART_CHUNK.0, CART_CHUNK.1);
    }
    assert_eq!(wait_cached(&first, CART_CHUNK), first_payload);
    assert_eq!(wait_cached(&other, CART_CHUNK), other_payload);
    assert_eq!(
        shared.cached_payload(CART_CHUNK.0, CART_CHUNK.1).as_deref(),
        Some(first_payload.as_slice())
    );

    let mut previous = view();
    let packets = stream_one(&mut previous, &first);
    let first_spawns = assert_batch(&packets[1..], &first_payload, &first_positions);
    {
        let mut peer = view();
        let packets = stream_one(&mut peer, &shared);
        assert_eq!(
            assert_batch(&packets[1..], &first_payload, &first_positions),
            first_spawns
        );
    }
    let mut other_view = view();
    let packets = stream_one(&mut other_view, &other);
    let other_spawns = assert_batch(&packets[1..], &other_payload, &other_positions);
    assert!(first_spawns.iter().all(|first| other_spawns
        .iter()
        .all(|other| first.id != other.id && first.uuid != other.uuid)));

    shared.clear_cache();
    assert_eq!(first.cached_payloads(), 0);
    assert_eq!(
        other.cached_payload(CART_CHUNK.0, CART_CHUNK.1).as_deref(),
        Some(other_payload.as_slice())
    );
    assert_eq!(wait_cached(&shared, CART_CHUNK), first_payload);
    {
        let mut peer = view();
        let packets = stream_one(&mut peer, &shared);
        assert_eq!(
            assert_batch(&packets[1..], &first_payload, &first_positions),
            first_spawns
        );
    }

    previous.teleport(-23.5, 72.0, 57.5);
    let delivery: ChunkDeliveryState = previous.into_delivery_state();
    first.clear_cache();
    assert_eq!(shared.cached_payloads(), 0);
    // Only the opaque transfer token retains these identities during this reload.
    assert_eq!(wait_cached(&shared, CART_CHUNK), first_payload);
    let mut adopted = PlayerView::new(-23.5, 72.0, 57.5).with_view_distance(0);
    adopted.adopt_delivery_state(delivery).unwrap();
    assert_eq!(adopted.loaded_chunks().count(), 0);
    assert!(adopted.has_pending_chunks());
    let packets = stream_one(&mut adopted, &shared);
    assert_unload(&packets[1..3], CART_CHUNK, &first_spawns);
    assert_eq!(
        assert_batch(&packets[3..], &first_payload, &first_positions),
        first_spawns
    );

    for (world, view, spawns) in [
        (&shared, &mut adopted, &first_spawns),
        (&other, &mut other_view, &other_spawns),
    ] {
        world.clear_cache();
        assert_eq!(world.cached_payloads(), 0);
        assert_eq!(wait_cached(world, EXIT_CHUNK), empty_payload);
        assert!(world.cached_payload(CART_CHUNK.0, CART_CHUNK.1).is_none());
        view.x += 16.0;
        let packets = stream_one(view, world);
        let _ = assert_batch(&packets[1..4], &empty_payload, &[]);
        assert_unload(&packets[4..], CART_CHUNK, spawns);
        let mut again = Vec::new();
        assert_eq!(view.stream_chunks_from(&mut again, world).unwrap(), 0);
        assert!(
            again.is_empty(),
            "unloaded identities must not be removed twice"
        );
    }
    for (store, column) in [(&first_store, first_column), (&other_store, other_column)] {
        assert_eq!(store.saved_chunks().unwrap(), [CART_CHUNK, EXIT_CHUNK]);
        assert_eq!(
            store.load(CART_CHUNK.0, CART_CHUNK.1).unwrap(),
            Some(column)
        );
    }
}
