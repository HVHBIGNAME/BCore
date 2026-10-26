use std::io::Cursor;

use bcore_core::BlockPos;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{bindings, packets};
use crate::packet::read_frame;

fn capture() -> Value {
    serde_json::from_str(include_str!("../../data/pillar_placement_26_2.json")).unwrap()
}

#[test]
fn pillar_fixture_and_all_five_helpers_have_repeat_authenticated_byte_identities() {
    let raw = include_bytes!("../../data/pillar_placement_26_2.json");
    let provenance: Value = serde_json::from_str(include_str!(
        "../../data/pillar_placement_provenance_26_2.json"
    ))
    .unwrap();
    assert_eq!(provenance["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    assert_eq!(provenance["version"]["protocol_version"], 776);
    assert_eq!(provenance["version"]["world_version"], 4903);
    assert_eq!(provenance["repeat_matches"], true);
    assert_eq!(provenance["block_items"], 56);
    assert_eq!(provenance["state_count"], 168);
    assert_eq!(provenance["cases"], 336);
    assert_eq!(
        provenance["libraries_sha256"].as_object().unwrap().len(),
        39
    );
    assert_eq!(provenance["fixture_bytes"], raw.len());
    let digest = format!("{:x}", Sha256::digest(raw));
    assert_eq!(provenance["raw_capture_sha256"], digest);
    for run in ["run-1", "run-2"] {
        assert_eq!(provenance["run_sha256"][run], digest);
    }
    let helpers: [(&str, &[u8]); 5] = [
        (
            "scripts/TreeReference.java",
            include_bytes!("../../../../scripts/TreeReference.java"),
        ),
        (
            "scripts/NativeWorldgenRegistries.java",
            include_bytes!("../../../../scripts/NativeWorldgenRegistries.java"),
        ),
        (
            "scripts/migration-26.2/ProtocolSchema.java",
            include_bytes!("../../../../scripts/migration-26.2/ProtocolSchema.java"),
        ),
        (
            "scripts/migration-26.2/BlockActionReference.java",
            include_bytes!("../../../../scripts/migration-26.2/BlockActionReference.java"),
        ),
        (
            "scripts/migration-26.2/PillarPlacementReference.java",
            include_bytes!("../../../../scripts/migration-26.2/PillarPlacementReference.java"),
        ),
    ];
    for (path, bytes) in helpers {
        assert_eq!(
            provenance["source_sha256"][path],
            format!("{:x}", Sha256::digest(bytes)),
            "{path}"
        );
    }
}

#[test]
fn all_336_native_pillar_placements_and_block_update_payloads_match() {
    let data = capture();
    assert_eq!(data["minecraft"], bcore_registry::MC_VERSION);
    assert_eq!(data["protocol"], bcore_registry::PROTOCOL_VERSION);
    assert_eq!(data["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 336);
    for row in cases {
        let input = &row["input"];
        let item = input["item_id"].as_u64().unwrap() as u32;
        let face = input["face"].as_u64().unwrap() as u8;
        let state = input["state_id"].as_u64().unwrap() as u32;
        assert_eq!(
            bindings::placement_state(item, 64, face),
            Some(state),
            "{}",
            row["name"]
        );
        assert!(bindings::basic_state(state));
        let properties = &bcore_registry::vanilla().states[state as usize].properties;
        assert_eq!(properties["axis"], input["axis"].as_str().unwrap());
        let pos = BlockPos::new(
            input["pos"][0].as_i64().unwrap() as i32,
            input["pos"][1].as_i64().unwrap() as i32,
            input["pos"][2].as_i64().unwrap() as i32,
        );
        let payload: Vec<u8> = row["payload_hex"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(row["native_decode_consumed_all"], true);
        assert_eq!(row["protocol_id"], packets::CB_BLOCK_UPDATE);
        assert_eq!(
            read_frame(&mut Cursor::new(packets::encode_block_update(pos, state))).unwrap(),
            (packets::CB_BLOCK_UPDATE, payload),
            "{}",
            row["name"]
        );
    }
}

#[test]
fn all_pillar_axes_respect_limits_and_do_not_admit_other_block_families() {
    let data = capture();
    let registry = bcore_registry::vanilla();
    let items = data["axis_full_cube_block_items"].as_array().unwrap();
    assert_eq!(items.len(), 56);
    for row in items {
        let item = row["item_id"].as_u64().unwrap() as u32;
        let maximum = row["max_stack_size"].as_u64().unwrap() as u8;
        assert_eq!(
            bindings::block_item(item, maximum),
            Some(row["default_state_id"].as_u64().unwrap() as u32)
        );
        for face in 0..6 {
            assert!(bindings::placement_state(item, maximum, face).is_some());
            assert_eq!(bindings::placement_state(item, 0, face), None);
            assert_eq!(bindings::placement_state(item, maximum + 1, face), None);
        }
        for face in [6, 255] {
            assert_eq!(bindings::placement_state(item, maximum, face), None);
        }
    }
    for name in [
        "minecraft:iron_chain",
        "minecraft:oak_stairs",
        "minecraft:oak_slab",
        "minecraft:chest",
        "minecraft:end_rod",
    ] {
        let item = bcore_registry::registry_id("ITEM", name).unwrap();
        assert_eq!(bindings::block_item(item, 1), None, "{name}");
        for state in registry.states.iter().filter(|state| state.name == name) {
            assert!(!bindings::basic_state(state.id), "{name}");
        }
    }
    assert_eq!(
        bindings::block_item(registry.registries["ITEM"].len() as u32, 1),
        None
    );
}
