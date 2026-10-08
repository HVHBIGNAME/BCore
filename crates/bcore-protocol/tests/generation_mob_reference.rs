//! Independently repeated native proto saves -> LOAD -> actual pairing codecs.
use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::OnceLock;

use bcore_core::{varint::decode_varint, ChunkPos};
use bcore_protocol::chunk::ChunkColumn;
use bcore_protocol::chunk_store::{decode_chunk_at, encode_chunk};
use bcore_protocol::entity::{LoadedGeneratedMob, CB_ENTITY_ATTRIBUTES};
use bcore_protocol::packet::read_frame;
use bcore_worldgen::generated_entity::{GeneratedEntity, GeneratedMob};
use bcore_worldgen::spawn::SpawnTag;
use bcore_worldgen::structure::template::Nbt;
use serde_json::{json, Value};

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../bcore-worldgen/data/generation_spawn_handoff_26_1.json"
        ))
        .unwrap()
    })
}

fn native_nbt(value: &Value) -> Nbt {
    // Gson's shortest Float decimal must first be read as f32. The physical
    // handoff then retains that exact value and its tag width.
    let logical = SpawnTag::from_native_json(value).unwrap().native_json();
    Nbt::from_logical_typed_json(&logical).unwrap()
}

fn canonical(mut nbt: Nbt) -> Nbt {
    if let Some(Nbt::List { values, .. }) = nbt.compound_mut().unwrap().get_mut("attributes") {
        values.sort_by_key(|v| v.get("id").unwrap().string().unwrap().to_owned());
    }
    nbt
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
        .collect()
}

fn compare_pairing(bytes: &[u8], handoff: &Value) {
    let mut input = Cursor::new(bytes);
    for expected in handoff["packets"].as_array().unwrap() {
        let (id, actual) = read_frame(&mut input).unwrap();
        assert_eq!(i64::from(id), expected["packet_id"].as_i64().unwrap());
        let native = unhex(expected["wire_hex"].as_str().unwrap());
        let (_, prefix) = decode_varint(&native).unwrap();
        if id != CB_ENTITY_ATTRIBUTES {
            assert_eq!(
                actual,
                native[prefix..],
                "native packet {}",
                expected["class"]
            );
            continue;
        }
        // AttributeMap is identity-keyed in Java. Normalize ONLY record order;
        // compare every native snapshot's complete codec bytes and exact count.
        let (entity, at) = decode_varint(&actual).unwrap();
        assert_eq!(i64::from(entity), handoff["runtime_id"].as_i64().unwrap());
        let (count, used) = decode_varint(&actual[at..]).unwrap();
        let mut at = at + used;
        let mut expected_by_id: BTreeMap<_, _> = expected["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| {
                let bytes = unhex(record["hex"].as_str().unwrap());
                (decode_varint(&bytes).unwrap().0, bytes)
            })
            .collect();
        assert_eq!(count as usize, expected_by_id.len());
        for _ in 0..count {
            let key = decode_varint(&actual[at..]).unwrap().0;
            let expected = expected_by_id
                .remove(&key)
                .expect("unique expected native attribute");
            assert_eq!(&actual[at..at + expected.len()], expected);
            at += expected.len();
        }
        assert!(expected_by_id.is_empty());
        assert_eq!(at, actual.len());
    }
    assert_eq!(input.position(), bytes.len() as u64, "extra pairing packet");
}

#[test]
fn native_proto_saves_load_and_pair_for_all_nineteen_creatures() {
    let mut kinds = BTreeMap::<String, usize>::new();
    for case in fixture()["cases"].as_array().unwrap() {
        for native in case["entities"].as_array().unwrap() {
            let saved = native_nbt(&native["typed_nbt"]);
            let mob = GeneratedMob::from_typed_data(saved.typed_json()).unwrap();
            assert_eq!(mob.nbt(), saved);
            assert_eq!(mob.kind().name(), native["type"].as_str().unwrap());
            let loaded = LoadedGeneratedMob::load(&mob).unwrap();
            assert_eq!(
                canonical(loaded.nbt().clone()),
                canonical(native_nbt(&native["handoff"]["loaded"]["typed_nbt"])),
                "LOAD {} at {}",
                native["type"],
                case["input"]["name"]
            );
            let id = native["handoff"]["runtime_id"].as_i64().unwrap() as i32;
            compare_pairing(&loaded.pairing_packets(id).unwrap(), &native["handoff"]);
            *kinds.entry(mob.kind().name().into()).or_default() += 1;
        }
    }
    assert_eq!(kinds.len(), 19);
    assert!(kinds.values().all(|&count| count == 24));
}

#[test]
fn every_native_proto_mob_survives_bcc_and_serde_without_changing_identity() {
    let mut count = 0;
    for case in fixture()["cases"].as_array().unwrap() {
        let mut column = ChunkColumn::flat();
        let owner = ChunkPos::new(0, 0);
        for native in case["entities"].as_array().unwrap() {
            let mob = GeneratedMob::from_typed_data(native_nbt(&native["typed_nbt"]).typed_json())
                .unwrap();
            let uuid = mob.uuid();
            let restored: GeneratedMob =
                serde_json::from_slice(&serde_json::to_vec(&mob).unwrap()).unwrap();
            assert_eq!(mob, restored);
            assert_eq!(restored.uuid(), uuid);
            let source = GeneratedEntity::Mob(Box::new(mob));
            assert!(!source.valid_for(ChunkPos::new(1, 0)));
            assert!(column.add_entity(owner, source));
            count += 1;
        }
        let bytes = encode_chunk(owner.x, owner.z, &column);
        let (_, _, restored) = decode_chunk_at(&bytes).unwrap();
        assert_eq!(restored.entities(), column.entities());
        assert_eq!(encode_chunk(owner.x, owner.z, &restored), bytes);
        for (mob, native) in restored
            .entities()
            .iter()
            .zip(case["entities"].as_array().unwrap())
        {
            let GeneratedEntity::Mob(mob) = mob else {
                panic!("restored entity kind");
            };
            compare_pairing(
                &LoadedGeneratedMob::load(mob)
                    .unwrap()
                    .pairing_packets(913)
                    .unwrap(),
                &native["handoff"],
            );
        }
    }
    assert_eq!(count, 456);
}

#[test]
fn invalid_saved_mob_types_widths_and_geometry_are_rejected() {
    let original = native_nbt(&fixture()["cases"][0]["entities"][0]["typed_nbt"]).typed_json();
    for (field, invalid) in [
        ("id", json!([8, "minecraft:unknown"])),
        ("Health", json!([5, -1.0])),
        ("Health", json!([6, 8.0])),
        ("UUID", json!([11, [0, 1, 2]])),
        ("LeftHanded", json!([1, 2])),
        ("Age", json!([4, 0])),
        (
            "Pos",
            json!([9, {"element_type":6,"values":[[6,1e30],[6,65.0],[6,1.0]]}]),
        ),
        (
            "Rotation",
            json!([9, {"element_type":6,"values":[[6,0.0],[6,0.0]]}]),
        ),
    ] {
        let mut value = original.clone();
        value[1][field] = invalid;
        assert!(
            GeneratedMob::from_typed_data(value.clone()).is_err(),
            "{field}"
        );
        assert!(serde_json::from_value::<GeneratedMob>(json!({"saved_data":value})).is_err());
    }
    let mut top = original;
    top[1]["Pos"][1]["values"][1] = json!([6, 320.0]);
    let entity = GeneratedEntity::Mob(Box::new(GeneratedMob::from_typed_data(top).unwrap()));
    let mut column = ChunkColumn::flat();
    assert!(column.add_entity(ChunkPos::new(0, 0), entity));
    let (_, _, saved) = decode_chunk_at(&encode_chunk(0, 0, &column)).unwrap();
    assert_eq!(saved.entities()[0].position()[1], 320.0);
}
