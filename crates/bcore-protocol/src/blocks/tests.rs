use std::io::Cursor;

use bcore_core::BlockPos;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{bindings, packets::*};
use crate::packet::read_frame;

fn capture() -> Value {
    serde_json::from_str(include_str!("../../data/block_actions_26_2.json")).unwrap()
}

fn hex(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn position(value: &Value) -> BlockPos {
    BlockPos::new(
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
        value[2].as_i64().unwrap() as i32,
    )
}

fn stack(input: &Value) -> Option<SimpleStack> {
    let count = input["count"].as_u64().unwrap() as u8;
    (count != 0).then(|| SimpleStack {
        item: input["item_id"].as_u64().unwrap() as u32,
        count,
    })
}

#[test]
fn native_codec_fixture_and_helpers_have_verified_unchanged_byte_identities() {
    let raw = include_bytes!("../../data/block_actions_26_2.json");
    let provenance: Value = serde_json::from_str(include_str!(
        "../../data/block_actions_provenance_26_2.json"
    ))
    .unwrap();
    assert_eq!(provenance["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    assert_eq!(provenance["repeat_matches"], true);
    assert_eq!(provenance["cases"], 25);
    assert_eq!(provenance["block_items"], 197);
    assert_eq!(provenance["fixture_bytes"], raw.len());
    let digest = format!("{:x}", Sha256::digest(raw));
    assert_eq!(provenance["raw_capture_sha256"], digest);
    assert_eq!(provenance["run_sha256"]["run-1"], digest);
    assert_eq!(provenance["run_sha256"]["run-2"], digest);
    assert_eq!(
        provenance["source_sha256"]["scripts/migration-26.2/BlockActionReference.java"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../../scripts/migration-26.2/BlockActionReference.java"
            ))
        )
    );
}

#[test]
fn all_25_native_action_hotbar_and_reply_payloads_match_the_wire_boundary() {
    let data = capture();
    assert_eq!(data["minecraft"], bcore_registry::MC_VERSION);
    assert_eq!(data["protocol"], bcore_registry::PROTOCOL_VERSION);
    assert_eq!(data["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    let rows = data["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 25);
    for row in rows {
        let id = row["protocol_id"].as_i64().unwrap() as i32;
        let input = &row["input"];
        let payload = hex(&row["payload_hex"]);
        assert_eq!(row["native_decode_consumed_all"], true);
        let expected = match id {
            SB_PLAYER_ACTION => Some(BlockPacket::Action {
                action: input["action"].as_u64().unwrap() as u8,
                pos: position(&input["pos"]),
                face: input["face"].as_u64().unwrap() as u8,
                sequence: input["sequence"].as_i64().unwrap() as i32,
            }),
            SB_USE_ITEM_ON => Some(BlockPacket::Place {
                hand: input["hand"].as_u64().unwrap() as u8,
                hit: BlockHit {
                    pos: position(&input["pos"]),
                    face: input["face"].as_u64().unwrap() as u8,
                    offsets: std::array::from_fn(|i| input["offsets"][i].as_f64().unwrap() as f32),
                    inside: input["inside"].as_bool().unwrap(),
                    world_border: input["world_border"].as_bool().unwrap(),
                },
                sequence: input["sequence"].as_i64().unwrap() as i32,
            }),
            SB_SET_CARRIED_ITEM => Some(BlockPacket::Carried {
                slot: input["slot"].as_u64().unwrap() as u8,
            }),
            SB_SET_CREATIVE_SLOT => Some(BlockPacket::Creative {
                slot: input["slot"].as_u64().unwrap() as u8 - 36,
                stack: stack(input),
            }),
            _ => None,
        };
        if let Some(expected) = expected {
            assert_eq!(
                parse(id, &payload).unwrap(),
                Some(expected),
                "{}",
                row["name"]
            );
        } else {
            let frame = match id {
                CB_BLOCK_ACK => encode_ack(input["sequence"].as_i64().unwrap() as i32),
                CB_BLOCK_UPDATE => encode_block_update(
                    position(&input["pos"]),
                    input["state_id"].as_u64().unwrap() as u32,
                ),
                CB_PLAYER_INVENTORY => {
                    encode_inventory(input["slot"].as_u64().unwrap() as u8, stack(input))
                }
                _ => panic!("uncovered native packet {id}"),
            };
            assert_eq!(
                read_frame(&mut Cursor::new(frame)).unwrap(),
                (id, payload),
                "{}",
                row["name"]
            );
        }
    }
    assert_eq!(data["actions"][7]["name"], "STAB");
}

#[test]
fn truncation_and_trailing_data_are_rejected_for_every_supported_native_request() {
    for row in capture()["cases"].as_array().unwrap() {
        let id = row["protocol_id"].as_i64().unwrap() as i32;
        let payload = hex(&row["payload_hex"]);
        if parse(id, &payload).unwrap().is_none() {
            continue;
        }
        for length in 0..payload.len() {
            assert!(
                parse(id, &payload[..length]).is_err(),
                "{} prefix {length}",
                row["name"]
            );
        }
        let mut trailing = payload.clone();
        trailing.push(0);
        assert!(
            parse(id, &trailing).is_err(),
            "{} trailing data",
            row["name"]
        );
    }
}

#[test]
fn unknown_slots_items_patches_faces_hands_and_negative_sequences_fail_closed() {
    for payload in [&[0, 9][..], &[255, 255][..]] {
        assert!(parse(SB_SET_CARRIED_ITEM, payload).is_err());
    }
    assert!(parse(SB_SET_CREATIVE_SLOT, &[0, 35, 0]).is_err());
    assert!(parse(SB_SET_CREATIVE_SLOT, &[0, 36, 65, 1, 0, 0]).is_err());
    assert!(parse(SB_SET_CREATIVE_SLOT, &[0, 36, 1, 1, 1, 0]).is_err());
    let mut action = vec![0];
    action.extend_from_slice(&0i64.to_be_bytes());
    action.extend_from_slice(&[6, 0]);
    assert!(parse(SB_PLAYER_ACTION, &action).is_err());
    action.truncate(9);
    action.push(0);
    bcore_core::varint::encode_varint(-1, &mut action);
    assert!(parse(SB_PLAYER_ACTION, &action).is_err());
    let mut place = vec![2];
    place.extend_from_slice(&[0; 24]);
    assert!(parse(SB_USE_ITEM_ON, &place).is_err());
}

#[test]
fn malformed_wire_fuzz_is_bounded_and_does_not_panic() {
    let mut random = 0x1234_5678u32;
    for n in 0..4096 {
        let data: Vec<_> = (0..n % 80)
            .map(|_| {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (random >> 24) as u8
            })
            .collect();
        for id in [
            SB_PLAYER_ACTION,
            SB_USE_ITEM_ON,
            SB_SET_CARRIED_ITEM,
            SB_SET_CREATIVE_SLOT,
        ] {
            let _ = parse(id, &data);
        }
    }
}

#[test]
fn native_bindings_are_stateless_non_be_cubes_with_correct_item_stack_limits() {
    let data = capture();
    for row in data["stateless_full_cube_block_items"].as_array().unwrap() {
        let item = row["item_id"].as_u64().unwrap() as u32;
        let state = row["state_id"].as_u64().unwrap() as u32;
        let maximum = row["max_stack_size"].as_u64().unwrap() as u8;
        assert_eq!(bindings::block_item(item, maximum), Some(state));
        assert_eq!(bindings::block_item(item, 0), None);
        assert_eq!(bindings::block_item(item, maximum + 1), None);
        assert!(bindings::basic_state(state));
    }
    assert!(!bindings::basic_state(bcore_registry::block_state::CHEST));
    assert!(!bindings::basic_state(bcore_registry::block_state::AIR));
    assert_eq!(
        bindings::block_item(bcore_registry::vanilla().registries["ITEM"].len() as u32, 1),
        None
    );
}

#[test]
fn native_standing_bounds_hit_offsets_and_aabb_reach_have_strict_edges() {
    let bounds = bindings::player_bounds();
    assert_eq!(bounds.block_range_default, 4.5);
    assert_eq!(bounds.creative_block_range_addition, 0.5);
    assert_eq!(bounds.width, f64::from(0.6f32));
    assert_eq!(bounds.height, f64::from(1.8f32));
    assert_eq!(bounds.eye_height, f64::from(1.62f32));
    let pos = BlockPos::new(-17, -64, 32);
    assert!(within_reach([-10.000_001, -63.5, 32.5], pos, 6.0));
    assert!(!within_reach([-10.0, -63.5, 32.5], pos, 6.0));
    assert!(!within_reach([f64::NAN, -63.5, 32.5], pos, 6.0));
    assert!(!within_reach([-17.0, -63.5, 32.5], pos, f64::INFINITY));
    let hit = BlockHit {
        pos,
        face: 1,
        offsets: [0.25, 0.5, 1.0],
        inside: false,
        world_border: false,
    };
    assert!(hit_is_valid(hit));
    assert!(!hit_is_valid(BlockHit {
        world_border: true,
        ..hit
    }));
    assert!(!hit_is_valid(BlockHit {
        offsets: [f32::NAN, 0.5, 1.0],
        ..hit
    }));
    assert!(!hit_is_valid(BlockHit {
        offsets: [2.0, 0.5, 1.0],
        ..hit
    }));
}
