use bcore_core::{varint::decode_varint, ChunkPos};
use bcore_protocol::{
    chunk::{block_state, ChunkColumn, MAX_Y, MIN_Y},
    chunk_store::{decode_chunk, decode_chunk_at, encode_chunk, ChunkStore, ChunkStoreError},
    nbt::encode_block_entity_update,
    world_state::{ChunkOrigin, World},
};
use bcore_worldgen::{
    block_entity::BlockEntity,
    tick_request::{TickRequest, TickTarget},
    WorldGenerator,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BEE_NEST: u32 = 21768;
const HIVE: (usize, i32, usize) = (1, 65, 2);

fn reference() -> Value {
    serde_json::from_str(include_str!("../../bcore-worldgen/data/beehives_26_1.json")).unwrap()
}

fn hive_column(ticks: &[i32]) -> ChunkColumn {
    let mut column = ChunkColumn::flat();
    assert!(column.set(HIVE.0, HIVE.1, HIVE.2, BEE_NEST));
    assert!(column.set_block_entity(
        HIVE.0,
        HIVE.1,
        HIVE.2,
        BlockEntity::Beehive {
            ticks_in_hive: ticks.to_vec()
        },
    ));
    column
}

fn take<'a>(input: &mut &'a [u8], count: usize) -> &'a [u8] {
    let (bytes, rest) = input.split_at(count);
    *input = rest;
    bytes
}

fn varint(input: &mut &[u8]) -> usize {
    let (value, count) = decode_varint(input).unwrap();
    take(input, count);
    usize::try_from(value).unwrap()
}

fn assert_hive_packet(payload: &[u8], [x, y, z]: [i32; 3]) {
    let mut input = payload;
    assert_eq!(
        i32::from_be_bytes(take(&mut input, 4).try_into().unwrap()),
        x >> 4
    );
    assert_eq!(
        i32::from_be_bytes(take(&mut input, 4).try_into().unwrap()),
        z >> 4
    );
    for _ in 0..varint(&mut input) {
        varint(&mut input); // heightmap kind
        let count = varint(&mut input);
        take(&mut input, count * 8);
    }
    let sections = varint(&mut input);
    take(&mut input, sections);
    assert_eq!(varint(&mut input), 1);
    assert_eq!(take(&mut input, 1), [(((x & 15) << 4) | (z & 15)) as u8]);
    assert_eq!(
        i16::from_be_bytes(take(&mut input, 2).try_into().unwrap()),
        y as i16
    );
    assert_eq!(varint(&mut input), 34);
    assert_eq!(
        take(&mut input, 1),
        [0],
        "native chunk packet projects the empty hive update to null"
    );
    for _ in 0..4 {
        let longs = varint(&mut input);
        take(&mut input, longs * 8);
    }
    for _ in 0..2 {
        for _ in 0..varint(&mut input) {
            let bytes = varint(&mut input);
            take(&mut input, bytes);
        }
    }
    assert!(
        input.is_empty(),
        "map_chunk must end after its light arrays"
    );
}

#[test]
fn native_beehives_keep_occupants_but_send_only_empty_updates() {
    let fixture = reference();
    let samples = fixture["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 100);
    assert_eq!(fixture["states"].as_array().unwrap().len(), 48);
    assert_eq!(fixture["type_id"], 34);
    for sample in samples {
        let pos =
            std::array::from_fn(|i| i32::try_from(sample["pos"][i].as_i64().unwrap()).unwrap());
        let [x, y, z] = pos;
        let data = BlockEntity::Beehive {
            ticks_in_hive: sample["ticks_in_hive"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| i32::try_from(value.as_i64().unwrap()).unwrap())
                .collect(),
        };
        assert_eq!(data.full_data((x, y, z)), sample["nbt"], "{sample}");
        assert_eq!(data.update_data(), sample["update"]);
        assert_eq!(sample["update_nbt_hex"], "0a00");
        assert_eq!(encode_block_entity_update(&data), [10, 0]);

        let mut column = ChunkColumn::flat();
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let state = u32::try_from(sample["state"].as_u64().unwrap()).unwrap();
        let in_height = sample["inside_build_height"].as_bool().unwrap();
        assert_eq!(column.set(lx, y, lz, state), in_height);
        assert_eq!(column.set_block_entity(lx, y, lz, data), in_height);
        if !in_height {
            continue;
        }
        let bytes = encode_chunk(x >> 4, z >> 4, &column);
        assert_eq!(&bytes[..6], b"BCC1\x05\x00");
        let (cx, cz, loaded) = decode_chunk_at(&bytes).unwrap();
        assert_eq!((cx, cz), (x >> 4, z >> 4));
        assert_eq!(loaded, column);
        assert_eq!(
            loaded.block_entities()[&(lx, y, lz)].full_data((x, y, z)),
            sample["nbt"]
        );
        assert_eq!(encode_chunk(cx, cz, &loaded), bytes);
        assert_hive_packet(&loaded.encode_payload(cx, cz), pos);
    }
}

#[test]
fn native_occupant_list_sizes_and_state_changes_are_preserved() {
    let fixture = reference();
    let mut counts = Vec::new();
    for case in fixture["codec_cases"].as_array().unwrap() {
        if case["kind"] != "occupant_list" {
            continue;
        }
        assert_eq!(case["encode_accepted"], true);
        assert_eq!(case["decode_accepted"], true);
        let count = case["count"].as_u64().unwrap() as usize;
        counts.push(count);
        let ticks: Vec<_> = (0..count)
            .map(|i| [i32::MIN, -1, 0, 599, i32::MAX][i % 5])
            .collect();
        let column = hive_column(&ticks);
        let mut loaded = decode_chunk(&encode_chunk(-2, 3, &column)).unwrap();
        assert_eq!(loaded, column);
        for state in fixture["states"].as_array().unwrap() {
            assert!(loaded.set(
                HIVE.0,
                HIVE.1,
                HIVE.2,
                state["state"].as_u64().unwrap() as u32
            ));
            assert_eq!(loaded.block_entities(), column.block_entities());
        }
        for case in fixture["state_validation"].as_array().unwrap() {
            assert_eq!(case["constructor_accepted"], false);
            assert!(loaded.set(
                HIVE.0,
                HIVE.1,
                HIVE.2,
                case["state"].as_u64().unwrap() as u32
            ));
            assert!(loaded.block_entities().is_empty());
            assert!(!loaded.set_block_entity(
                HIVE.0,
                HIVE.1,
                HIVE.2,
                column.block_entities()[&HIVE].clone()
            ));
        }
    }
    assert_eq!(counts, [0, 1, 2, 3, 4, 64]);
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let parent = std::env::temp_dir().join("opencode");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "bcore-tree-effects-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

#[test]
fn generated_beehive_and_requests_survive_bcc_and_queued_world_load() {
    let fixture = reference();
    let native = fixture["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sample| sample["kind"] == "signed_tick_bounds" && sample["state"] == BEE_NEST)
        .expect("native nest with signed occupant ages");
    let [x, y, z] =
        std::array::from_fn(|i| i32::try_from(native["pos"][i].as_i64().unwrap()).unwrap());
    let owner = ChunkPos::new(x >> 4, z >> 4);
    let hive = ((x & 15) as usize, y, (z & 15) as usize);
    let state = u32::try_from(native["state"].as_u64().unwrap()).unwrap();
    let beehive = BlockEntity::Beehive {
        ticks_in_hive: serde_json::from_value(native["ticks_in_hive"].clone()).unwrap(),
    };
    let first = TickRequest {
        block_pos: [owner.x * 16 + 15, MAX_Y, owner.z * 16 + 15],
        target: TickTarget::Block(29_872),
        delay: i32::MIN,
    };
    let requests = [
        first,
        TickRequest {
            block_pos: [owner.x * 16, MIN_Y, owner.z * 16],
            target: TickTarget::Fluid(0),
            delay: 0,
        },
        TickRequest {
            target: TickTarget::Fluid(4),
            delay: i32::MAX,
            ..first
        },
        first,
        TickRequest {
            block_pos: [x, y, z],
            target: TickTarget::Block(state),
            delay: -1,
        },
    ];
    let mut generated = WorldGenerator::new(123).generate_chunk(owner);
    assert!(generated.set(hive.0, hive.1, hive.2, state));
    assert!(generated.set_block_entity(hive.0, hive.1, hive.2, beehive.clone()));
    for outside_y in [MIN_Y - 1, MAX_Y + 1] {
        assert!(!generated.set(hive.0, outside_y, hive.2, state));
        assert!(!generated.set_block_entity(hive.0, outside_y, hive.2, beehive.clone()));
    }
    let mut expected = generated.tick_requests().to_vec();
    assert!(generated.set(15, MAX_Y, 15, block_state::STONE));
    for request in requests {
        assert!(generated.add_tick_request(request));
        expected.push(request);
    }
    assert!(generated.set(15, MAX_Y, 15, block_state::AIR));
    assert_eq!(generated.tick_requests(), expected);
    assert_eq!(
        generated.block_entities()[&hive].full_data((x, y, z)),
        native["nbt"]
    );
    let mut column = ChunkColumn::from_generated(&generated);
    assert_eq!(column.tick_requests(), expected);
    for outside_y in [MIN_Y - 1, MAX_Y + 1] {
        assert!(!column.set(hive.0, outside_y, hive.2, state));
        assert!(!column.set_block_entity(hive.0, outside_y, hive.2, beehive.clone()));
    }
    assert_eq!(column.get(hive.0, hive.1, hive.2), Some(state));
    assert_eq!(
        column.block_entities()[&hive].full_data((x, y, z)),
        native["nbt"]
    );
    let wire = column.encode_payload(owner.x, owner.z);
    assert!(column.add_tick_request(owner, first));
    expected.push(first);
    assert_eq!(
        column.encode_payload(owner.x, owner.z),
        wire,
        "tick requests are server-only"
    );
    let mut edited = column.clone();
    assert!(edited.set(hive.0, hive.1, hive.2, block_state::AIR));
    assert!(edited.block_entities().is_empty());
    assert_eq!(
        edited.tick_requests(),
        expected,
        "block edits must not cancel deferred requests"
    );

    let bytes = encode_chunk(owner.x, owner.z, &column);
    let loaded = decode_chunk(&bytes).unwrap();
    assert_eq!(loaded, column);
    assert_eq!(loaded.tick_requests(), expected);
    assert_eq!(
        loaded.block_entities()[&hive].full_data((x, y, z)),
        native["nbt"]
    );
    assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
    let scratch = Scratch::new();
    let store = ChunkStore::at(&scratch.0);
    store.save(owner.x, owner.z, &column).unwrap();
    assert_eq!(fs::read(store.chunk_path(owner.x, owner.z)).unwrap(), bytes);
    let world = World::with_store(123, store.clone());
    assert!(world.cached_payload(owner.x, owner.z).is_none());
    world.request_payload(owner.x, owner.z);
    let deadline = Instant::now() + Duration::from_secs(5);
    let queued = loop {
        if let Some(payload) = world.cached_payload(owner.x, owner.z) {
            break payload;
        }
        assert!(
            Instant::now() < deadline,
            "queued saved tree effects did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(queued, wire);
    assert_hive_packet(&queued, [x, y, z]);
    let (restored, origin) = world.chunk(owner.x, owner.z);
    assert_eq!(origin, ChunkOrigin::Loaded);
    assert_eq!(restored, column);
    assert_eq!(restored.tick_requests(), expected);
    assert_eq!(
        restored.block_entities()[&hive].full_data((x, y, z)),
        native["nbt"]
    );
    assert_eq!(restored.get(hive.0, hive.1, hive.2), Some(state));
    assert_eq!(fs::read(store.chunk_path(owner.x, owner.z)).unwrap(), bytes);
    assert_eq!(store.saved_chunks().unwrap(), [(owner.x, owner.z)]);
}

#[test]
fn tick_request_append_rejects_invalid_owners_heights_and_target_ids() {
    let owner = ChunkPos::new(-2, 3);
    let request = TickRequest {
        block_pos: [-31, 65, 50],
        target: TickTarget::Block(0),
        delay: -1,
    };
    let mut column = ChunkColumn::flat();
    assert!(column.add_tick_request(owner, request));
    for invalid in [
        TickRequest {
            block_pos: [-33, 65, 50],
            ..request
        },
        TickRequest {
            block_pos: [-31, 65, 64],
            ..request
        },
        TickRequest {
            block_pos: [-31, MIN_Y - 1, 50],
            ..request
        },
        TickRequest {
            block_pos: [-31, MAX_Y + 1, 50],
            ..request
        },
        TickRequest {
            target: TickTarget::Block(29_873),
            ..request
        },
        TickRequest {
            target: TickTarget::Block(u32::MAX),
            ..request
        },
        TickRequest {
            target: TickTarget::Fluid(5),
            ..request
        },
        TickRequest {
            target: TickTarget::Fluid(u32::MAX),
            ..request
        },
    ] {
        assert!(!column.add_tick_request(owner, invalid), "{invalid:?}");
        assert_eq!(column.tick_requests(), [request]);
    }
}

fn rechecksum(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let hash = bytes[..end].iter().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    bytes[end..].copy_from_slice(&hash.to_le_bytes());
}

#[test]
fn malformed_beehive_records_are_rejected() {
    let ticks = [i32::MIN, -1, 42, i32::MAX];
    let bytes = encode_chunk(-2, 3, &hive_column(&ticks));
    // One hive entry, followed by empty entity/structure/tick counts and checksum.
    let record = bytes.len() - 16 - (1 + 4 + 1 + 4 + ticks.len() * 4);
    assert_eq!(bytes[record], 0x12);
    assert_eq!(bytes[record + 5], 3);
    for (offset, replacement) in [
        (record + 1, (MIN_Y - 1).to_le_bytes().to_vec()),
        (record + 1, (MAX_Y + 1).to_le_bytes().to_vec()),
        (record + 5, vec![255]),
    ] {
        let mut invalid = bytes.clone();
        invalid[offset..offset + replacement.len()].copy_from_slice(&replacement);
        rechecksum(&mut invalid);
        assert!(matches!(
            decode_chunk(&invalid),
            Err(ChunkStoreError::InvalidBlockEntity)
        ));
    }
    let mut invalid = bytes.clone();
    invalid[record + 6..record + 10].copy_from_slice(&u32::MAX.to_le_bytes());
    rechecksum(&mut invalid);
    assert!(matches!(
        decode_chunk(&invalid),
        Err(ChunkStoreError::Truncated { .. }) | Err(ChunkStoreError::InvalidBlockEntity)
    ));

    let palette_len = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    let state = (28..28 + palette_len * 4)
        .step_by(4)
        .find(|&at| bytes[at..at + 4] == BEE_NEST.to_le_bytes())
        .unwrap();
    let mut invalid = bytes.clone();
    invalid[state..state + 4].copy_from_slice(&block_state::DIRT.to_le_bytes());
    rechecksum(&mut invalid);
    assert!(matches!(
        decode_chunk(&invalid),
        Err(ChunkStoreError::InvalidBlockEntity)
    ));

    for version in [0u16, 6] {
        let mut invalid = bytes.clone();
        invalid[4..6].copy_from_slice(&version.to_le_bytes());
        rechecksum(&mut invalid);
        assert!(matches!(
            decode_chunk(&invalid),
            Err(ChunkStoreError::UnsupportedVersion(actual)) if actual == version
        ));
    }
    let mut legacy = bytes;
    legacy[4..6].copy_from_slice(&3u16.to_le_bytes());
    legacy.truncate(legacy.len() - 4); // remove the v4 tick count; reuse its slot for checksum
    rechecksum(&mut legacy);
    assert!(matches!(
        decode_chunk(&legacy),
        Err(ChunkStoreError::InvalidBlockEntity)
    ));
}

#[test]
fn malformed_tick_records_are_rejected() {
    let owner = ChunkPos::new(-2, 3);
    let mut column = ChunkColumn::flat();
    assert!(column.add_tick_request(
        owner,
        TickRequest {
            block_pos: [-31, 65, 50],
            target: TickTarget::Block(BEE_NEST),
            delay: -1,
        }
    ));
    let bytes = encode_chunk(owner.x, owner.z, &column);
    let record = bytes.len() - 4 - 14;
    for (offset, replacement) in [
        (record, vec![255]),
        (record + 2, (MIN_Y - 1).to_le_bytes().to_vec()),
        (record + 2, (MAX_Y + 1).to_le_bytes().to_vec()),
        (record + 6, 29_873u32.to_le_bytes().to_vec()),
        (record - 4, u32::MAX.to_le_bytes().to_vec()),
        (8, i32::MAX.to_le_bytes().to_vec()),
        (12, i32::MIN.to_le_bytes().to_vec()),
    ] {
        let mut invalid = bytes.clone();
        invalid[offset..offset + replacement.len()].copy_from_slice(&replacement);
        rechecksum(&mut invalid);
        assert!(
            matches!(
                decode_chunk(&invalid),
                Err(ChunkStoreError::InvalidTickRequest)
            ),
            "offset {offset}"
        );
    }
    let mut invalid = bytes.clone();
    invalid[record] = 2;
    invalid[record + 6..record + 10].copy_from_slice(&5u32.to_le_bytes());
    rechecksum(&mut invalid);
    assert!(matches!(
        decode_chunk(&invalid),
        Err(ChunkStoreError::InvalidTickRequest)
    ));

    let mut truncated = bytes.clone();
    truncated.remove(truncated.len() - 5);
    rechecksum(&mut truncated);
    assert!(matches!(
        decode_chunk(&truncated),
        Err(ChunkStoreError::InvalidTickRequest)
    ));
    let mut trailing = bytes;
    trailing[record - 4..record].copy_from_slice(&0u32.to_le_bytes());
    rechecksum(&mut trailing);
    assert!(matches!(
        decode_chunk(&trailing),
        Err(ChunkStoreError::TrailingData)
    ));
}

#[test]
#[should_panic(expected = "tick request must belong to the saved chunk")]
fn saving_tick_requests_under_a_different_owner_is_rejected() {
    let owner = ChunkPos::new(-2, 3);
    let mut column = ChunkColumn::flat();
    assert!(column.add_tick_request(
        owner,
        TickRequest {
            block_pos: [-31, 65, 50],
            target: TickTarget::Fluid(2),
            delay: 0,
        }
    ));
    encode_chunk(owner.x + 1, owner.z, &column);
}
