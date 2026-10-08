use bcore_core::{varint::decode_varint, ChunkPos};
use bcore_protocol::{
    chunk::{ChunkColumn, MAX_Y, MIN_Y},
    chunk_store::{decode_chunk, encode_chunk, ChunkStoreError},
    nbt::encode_feature_block_entity_update,
};
use bcore_worldgen::{
    block_entity::BlockEntity,
    generation::FeatureBlockEntity,
    tick_request::{TickRequest, TickTarget},
};
use serde_json::{json, Value};

const OWNER: ChunkPos = ChunkPos::new(-2, 2);

fn reference() -> Value {
    serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/sculk_states_26_1.json"
    ))
    .unwrap()
}

fn native_entity(sample: &Value, pos: [i32; 3]) -> FeatureBlockEntity {
    let mut full_data = sample["nbt"].clone();
    let mut typed_data = sample["typed_nbt"].clone();
    for (key, coordinate) in ["x", "y", "z"].into_iter().zip(pos) {
        full_data[key] = coordinate.into();
        typed_data[1][key][1] = coordinate.into();
    }
    FeatureBlockEntity {
        type_id: sample["type_id"].as_u64().unwrap() as u32,
        valid_states: serde_json::from_value(sample["valid_states"][0].clone()).unwrap(),
        full_data,
        typed_data,
        update_data: sample["update"].clone(),
        template_load_data: None,
        typed_update_data: None,
    }
}

fn column(sample: &Value) -> (ChunkColumn, FeatureBlockEntity) {
    let mut column = ChunkColumn::flat();
    let data = native_entity(sample, [-17, 65, 47]);
    assert!(column.set(15, 65, 15, sample["state"].as_u64().unwrap() as u32));
    assert!(column.set_feature_block_entity(OWNER, 15, 65, 15, data.clone()));
    (column, data)
}

fn take<'a>(input: &mut &'a [u8], count: usize) -> &'a [u8] {
    let (value, rest) = input.split_at(count);
    *input = rest;
    value
}

fn varint(input: &mut &[u8]) -> usize {
    let (value, count) = decode_varint(input).unwrap();
    take(input, count);
    usize::try_from(value).unwrap()
}

fn empty_updates(payload: &[u8]) -> Vec<(u8, i16, u32)> {
    let mut input = &payload[8..];
    for _ in 0..varint(&mut input) {
        varint(&mut input);
        let count = varint(&mut input);
        take(&mut input, count * 8);
    }
    let count = varint(&mut input);
    take(&mut input, count);
    let mut entries = Vec::new();
    for _ in 0..varint(&mut input) {
        let xz = take(&mut input, 1)[0];
        let y = i16::from_be_bytes(take(&mut input, 2).try_into().unwrap());
        let type_id = varint(&mut input) as u32;
        assert_eq!(
            take(&mut input, 1),
            [0],
            "native BlockEntityInfo projects an empty update to null"
        );
        entries.push((xz, y, type_id));
    }
    for _ in 0..4 {
        let count = varint(&mut input);
        take(&mut input, count * 8);
    }
    for _ in 0..2 {
        for _ in 0..varint(&mut input) {
            let count = varint(&mut input);
            take(&mut input, count);
        }
    }
    assert!(input.is_empty());
    entries
}

#[test]
fn all_native_sculk_states_keep_typed_nbt_and_send_native_empty_updates() {
    let fixture = reference();
    let mut checked = 0;
    for sample in fixture["block_entities"].as_array().unwrap() {
        assert_eq!(sample["typed_update"], json!([10, {}]));
        assert_eq!(sample["update_packet_null"], true);
        let [first, end]: [u32; 2] =
            serde_json::from_value(sample["valid_states"][0].clone()).unwrap();
        for state in first..end {
            let y = if state % 2 == 0 { MIN_Y } else { MAX_Y };
            let data = native_entity(sample, [-17, y, 47]);
            let mut column = ChunkColumn::flat();
            assert!(column.set(15, y, 15, state));
            assert!(column.set_feature_block_entity(OWNER, 15, y, 15, data.clone()));
            assert_eq!(encode_feature_block_entity_update(&data), [10, 0]);
            let bytes = encode_chunk(OWNER.x, OWNER.z, &column);
            let loaded = decode_chunk(&bytes).unwrap();
            assert_eq!(loaded, column);
            assert_eq!(loaded.feature_block_entities()[&(15, y, 15)], data);
            assert_eq!(encode_chunk(OWNER.x, OWNER.z, &loaded), bytes);
            assert_eq!(
                empty_updates(&loaded.encode_payload(OWNER.x, OWNER.z)),
                [(255, y as i16, data.type_id)]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 106);
}

#[test]
fn mixed_entities_ticks_and_duplicate_marks_round_trip_without_execution() {
    let fixture = reference();
    let (mut column, _) = column(&fixture["block_entities"][0]);
    assert!(column.set(0, 65, 0, 21_768));
    assert!(column.set_block_entity(
        0,
        65,
        0,
        BlockEntity::Beehive {
            ticks_in_hive: vec![97, -1]
        }
    ));
    let marks = [(15, MIN_Y, 15), (0, MAX_Y, 0), (15, MIN_Y, 15)];
    for (x, y, z) in marks {
        assert!(column.mark_postprocessing(x, y, z));
    }
    assert!(!column.mark_postprocessing(16, 65, 0));
    assert!(!column.mark_postprocessing(0, MAX_Y + 1, 0));
    let request = TickRequest {
        block_pos: [-17, MIN_Y, 47],
        target: TickTarget::Fluid(2),
        delay: -5,
    };
    assert!(column.add_tick_request(OWNER, request));
    assert!(column.add_tick_request(OWNER, request));
    let bytes = encode_chunk(OWNER.x, OWNER.z, &column);
    assert_eq!(&bytes[4..8], [5, 0, 1, 0]);
    let mut loaded = decode_chunk(&bytes).unwrap();
    assert_eq!(loaded, column);
    assert_eq!(encode_chunk(OWNER.x, OWNER.z, &loaded), bytes);
    assert_eq!(
        empty_updates(&loaded.encode_payload(OWNER.x, OWNER.z)),
        [(0, 65, 34), (255, 65, 35)]
    );
    assert!(loaded.set(15, 65, 15, 24_785));
    assert_eq!(
        loaded.feature_block_entities(),
        column.feature_block_entities()
    );
    assert!(loaded.set(15, 65, 15, 0));
    assert!(loaded.feature_block_entities().is_empty());
    assert_eq!(loaded.block_entities(), column.block_entities());
    assert_eq!(loaded.postprocessing_positions(), marks);
    assert_eq!(loaded.tick_requests(), [request, request]);
}

fn checksum(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let value = bytes[..end].iter().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    bytes[end..].copy_from_slice(&value.to_le_bytes());
}

fn metadata_offset(bytes: &[u8], data: &FeatureBlockEntity) -> (usize, usize) {
    let json = serde_json::to_vec(data).unwrap();
    (
        bytes
            .windows(json.len())
            .position(|window| window == json)
            .unwrap(),
        json.len(),
    )
}

#[test]
fn valid_checksums_cannot_hide_wrong_nbt_types_states_ids_or_owners() {
    let fixture = reference();
    let (column, data) = column(&fixture["block_entities"][0]);
    let original = encode_chunk(OWNER.x, OWNER.z, &column);
    let (at, length) = metadata_offset(&original, &data);
    for change in ["width", "range", "type", "position", "id", "update"] {
        let mut value = serde_json::to_value(&data).unwrap();
        match change {
            "width" => value["typed_data"][1]["listener"][1]["selector"][1]["tick"][0] = json!(3),
            "range" => value["valid_states"][1] = json!(24_785),
            "type" => value["type_id"] = json!(37),
            "position" => value["full_data"]["x"] = json!(-33),
            "id" => value["full_data"]["id"] = json!("minecraft:sculk_catalyst"),
            "update" => value["update_data"] = json!({"warning_level": 0}),
            _ => unreachable!(),
        }
        let replacement = serde_json::to_vec(&value).unwrap();
        let mut bytes = original.clone();
        bytes[at - 4..at].copy_from_slice(&(replacement.len() as u32).to_le_bytes());
        bytes.splice(at..at + length, replacement);
        checksum(&mut bytes);
        assert!(
            matches!(
                decode_chunk(&bytes),
                Err(ChunkStoreError::InvalidBlockEntity)
            ),
            "accepted corrupt {change}"
        );
    }
    let mut other_owner = original.clone();
    other_owner[8..12].copy_from_slice(&(-3i32).to_le_bytes());
    checksum(&mut other_owner);
    assert!(matches!(
        decode_chunk(&other_owner),
        Err(ChunkStoreError::InvalidBlockEntity)
    ));

    let mut oversized = original.clone();
    oversized[at - 4..at].copy_from_slice(&u32::MAX.to_le_bytes());
    checksum(&mut oversized);
    assert!(matches!(
        decode_chunk(&oversized),
        Err(ChunkStoreError::InvalidBlockEntity)
    ));

    let mut duplicate = original.clone();
    let entry_start = at - 10;
    duplicate[entry_start - 4..entry_start].copy_from_slice(&2u32.to_le_bytes());
    duplicate.splice(
        at + length..at + length,
        original[entry_start..at + length].iter().copied(),
    );
    checksum(&mut duplicate);
    assert!(matches!(
        decode_chunk(&duplicate),
        Err(ChunkStoreError::InvalidBlockEntity)
    ));
}

#[test]
fn malformed_postprocessing_extensions_are_rejected() {
    let mut column = ChunkColumn::flat();
    assert!(column.mark_postprocessing(1, 65, 2));
    let original = encode_chunk(0, 0, &column);
    let count_at = original.len() - 4 - 4 - 5;
    let mut bad_y = original.clone();
    bad_y[count_at + 5..count_at + 9].copy_from_slice(&(MAX_Y + 1).to_le_bytes());
    checksum(&mut bad_y);
    assert!(matches!(
        decode_chunk(&bad_y),
        Err(ChunkStoreError::InvalidPostprocessing)
    ));
    let mut bad_count = original.clone();
    bad_count[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    checksum(&mut bad_count);
    assert!(matches!(
        decode_chunk(&bad_count),
        Err(ChunkStoreError::InvalidPostprocessing)
    ));
    let mut bad_flags = original;
    bad_flags[6..8].copy_from_slice(&0x8000u16.to_le_bytes());
    checksum(&mut bad_flags);
    assert!(matches!(
        decode_chunk(&bad_flags),
        Err(ChunkStoreError::UnsupportedFlags(0x8000))
    ));
}
