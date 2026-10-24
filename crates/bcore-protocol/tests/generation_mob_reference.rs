//! Historical 26.1 native LOAD/pairing and target-registry codec regressions.
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, OnceLock};

use bcore_core::{varint::decode_varint, ChunkPos};
use bcore_protocol::chunk::ChunkColumn;
use bcore_protocol::chunk_store::{decode_chunk_at, encode_chunk};
use bcore_protocol::entity::{
    GenerationMobCatalog, LoadedGeneratedMob, CB_ENTITY_ATTRIBUTES, CB_ENTITY_EQUIPMENT,
    CB_ENTITY_METADATA, CB_SPAWN_ENTITY,
};
use bcore_protocol::packet::{read_frame, read_string, read_varint};
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

fn historical_catalog() -> &'static Arc<GenerationMobCatalog> {
    static DATA: OnceLock<Arc<GenerationMobCatalog>> = OnceLock::new();
    DATA.get_or_init(|| {
        let fixture = fixture();
        assert_eq!(fixture["minecraft"], "26.1");
        assert_eq!(fixture["protocol"], 775);
        assert_eq!(
            fixture["provenance"]["jar_sha256"],
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        let source = include_str!("../data/generation_mob_catalog_26_1.json");
        let identity: Value = serde_json::from_str(source).unwrap();
        assert_eq!(identity["minecraft"], fixture["minecraft"]);
        assert_eq!(identity["protocol"], fixture["protocol"]);
        assert_eq!(
            identity["provenance"]["jar_sha256"],
            fixture["provenance"]["jar_sha256"]
        );
        let types = fixture["catalog"]["types"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["name"].as_str().unwrap().to_owned(),
                    i32::try_from(row["id"].as_i64().unwrap()).unwrap(),
                )
            })
            .collect();
        Arc::new(GenerationMobCatalog::from_sources(source, types).unwrap())
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
            let loaded =
                LoadedGeneratedMob::load_with_catalog(&mob, Arc::clone(historical_catalog()))
                    .unwrap();
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
                &LoadedGeneratedMob::load_with_catalog(mob, Arc::clone(historical_catalog()))
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

fn saved_mob(name: &str) -> Nbt {
    fixture()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|case| case["entities"].as_array().unwrap())
        .find(|entity| entity["type"] == name)
        .map(|entity| native_nbt(&entity["typed_nbt"]))
        .unwrap()
}

#[test]
fn shifted_generated_types_use_the_shared_target_registry() {
    for (name, expected) in [
        ("minecraft:turtle", bcore_registry::entity_type::TURTLE),
        ("minecraft:wolf", bcore_registry::entity_type::WOLF),
    ] {
        let mob = GeneratedMob::from_typed_data(saved_mob(name).typed_json()).unwrap();
        let bytes = LoadedGeneratedMob::load(&mob)
            .unwrap()
            .pairing_packets(913)
            .unwrap();
        let (packet, spawn) = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(packet, CB_SPAWN_ENTITY);
        let mut spawn = Cursor::new(spawn);
        assert_eq!(read_varint(&mut spawn).unwrap(), 913);
        let mut uuid = [0; 16];
        spawn.read_exact(&mut uuid).unwrap();
        assert_eq!(uuid, mob.uuid());
        assert_eq!(read_varint(&mut spawn).unwrap(), expected as i32);
        assert_eq!(
            bcore_registry::registry_id("ENTITY_TYPE", name),
            Some(expected)
        );
    }
}

/// Constructed target-registry input using a historical physical NBT save.
fn target_pairing_fixture() -> Vec<u8> {
    let mut saved = saved_mob("minecraft:horse");
    let attribute = Nbt::Compound(BTreeMap::from([
        ("id".into(), Nbt::String("minecraft:movement_speed".into())),
        ("base".into(), Nbt::Double(0.25)),
        (
            "modifiers".into(),
            Nbt::List {
                element_type: 10,
                values: vec![Nbt::Compound(BTreeMap::from([
                    ("id".into(), Nbt::String("bcore:pairing_test".into())),
                    ("amount".into(), Nbt::Double(0.5)),
                    (
                        "operation".into(),
                        Nbt::String("add_multiplied_total".into()),
                    ),
                ]))],
            },
        ),
    ]));
    let fields = saved.compound_mut().unwrap();
    fields.insert(
        "attributes".into(),
        Nbt::List {
            element_type: 10,
            values: vec![attribute],
        },
    );
    let equipment = [
        ("mainhand", "minecraft:dirt", 2),
        ("head", "minecraft:iron_helmet", 1),
        ("saddle", "minecraft:saddle", 1),
    ];
    fields.insert(
        "equipment".into(),
        Nbt::Compound(
            equipment
                .iter()
                .map(|&(slot, name, count)| {
                    (
                        slot.into(),
                        Nbt::Compound(BTreeMap::from([
                            ("id".into(), Nbt::String(name.into())),
                            ("count".into(), Nbt::Int(count)),
                            ("components".into(), Nbt::empty_compound()),
                        ])),
                    )
                })
                .collect(),
        ),
    );
    let mob = GeneratedMob::from_typed_data(saved.typed_json()).unwrap();
    LoadedGeneratedMob::load(&mob)
        .unwrap()
        .pairing_packets(913)
        .unwrap()
}

#[test]
fn target_pairing_resolves_attribute_and_equipment_resource_names() {
    let mut input = Cursor::new(target_pairing_fixture());
    assert_eq!(read_frame(&mut input).unwrap().0, CB_SPAWN_ENTITY);
    assert_eq!(read_frame(&mut input).unwrap().0, CB_ENTITY_METADATA);
    let (packet, attributes) = read_frame(&mut input).unwrap();
    assert_eq!(packet, CB_ENTITY_ATTRIBUTES);
    let mut attributes = Cursor::new(attributes);
    assert_eq!(read_varint(&mut attributes).unwrap(), 913);
    assert_eq!(read_varint(&mut attributes).unwrap(), 1);
    assert_eq!(
        read_varint(&mut attributes).unwrap(),
        bcore_registry::attribute::MOVEMENT_SPEED as i32
    );
    let mut number = [0; 8];
    attributes.read_exact(&mut number).unwrap();
    assert_eq!(f64::from_be_bytes(number), 0.25);
    assert_eq!(read_varint(&mut attributes).unwrap(), 1);
    assert_eq!(
        read_string(&mut attributes, 32767).unwrap(),
        "bcore:pairing_test"
    );
    attributes.read_exact(&mut number).unwrap();
    assert_eq!(f64::from_be_bytes(number), 0.5);
    assert_eq!(read_varint(&mut attributes).unwrap(), 2);
    assert_eq!(attributes.position(), attributes.get_ref().len() as u64);

    let (packet, encoded) = read_frame(&mut input).unwrap();
    assert_eq!(packet, CB_ENTITY_EQUIPMENT);
    let mut encoded = Cursor::new(encoded);
    assert_eq!(read_varint(&mut encoded).unwrap(), 913);
    for (slot, count, item) in [
        (0x80, 2, bcore_registry::item::DIRT),
        (0x85, 1, bcore_registry::item::IRON_HELMET),
        (0x07, 1, bcore_registry::item::SADDLE),
    ] {
        let mut actual_slot = [0];
        encoded.read_exact(&mut actual_slot).unwrap();
        assert_eq!(actual_slot[0], slot);
        assert_eq!(read_varint(&mut encoded).unwrap(), count);
        assert_eq!(read_varint(&mut encoded).unwrap(), item as i32);
        assert_eq!(read_varint(&mut encoded).unwrap(), 0, "added components");
        assert_eq!(read_varint(&mut encoded).unwrap(), 0, "removed components");
    }
    assert_eq!(encoded.position(), encoded.get_ref().len() as u64);
    assert_eq!(input.position(), input.get_ref().len() as u64);
}
