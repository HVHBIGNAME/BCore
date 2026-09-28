use bcore_core::varint::decode_varint;
use bcore_protocol::entity::{
    encode_entity_metadata, encode_entity_teleport, encode_remove_entities, encode_spawn_entity,
    ItemEntity, MetadataEntry, MobKind, Position,
};
use serde_json::Value;

fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn entity_packets_match_native_26_1_game_codec() {
    let fixture: Value =
        serde_json::from_str(include_str!("../data/entity_packets_26_1.json")).unwrap();
    assert_eq!(fixture["minecraft"], "26.1");
    assert_eq!(
        fixture["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(fixture["minecart_default_metadata_empty"], true);
    let samples = fixture["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 27);
    for sample in samples {
        let id = || sample["id"].as_i64().unwrap() as i32;
        let position = || Position {
            x: sample["position"][0].as_f64().unwrap(),
            y: sample["position"][1].as_f64().unwrap(),
            z: sample["position"][2].as_f64().unwrap(),
        };
        let packet = match sample["kind"].as_str().unwrap() {
            "spawn" => encode_spawn_entity(
                id(),
                hex(&sample["uuid"].as_str().unwrap().replace('-', ""))
                    .try_into()
                    .unwrap(),
                sample["entity_type"].as_i64().unwrap() as i32,
                position(),
            ),
            "remove" => encode_remove_entities(
                &sample["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_i64().unwrap() as i32)
                    .collect::<Vec<_>>(),
            ),
            "metadata" => encode_entity_metadata(
                id(),
                &sample["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|entry| MetadataEntry {
                        index: entry["index"].as_u64().unwrap() as u8,
                        type_id: entry["type_id"].as_i64().unwrap() as i32,
                        value: hex(entry["value_hex"].as_str().unwrap()),
                    })
                    .collect::<Vec<_>>(),
            ),
            "teleport" => encode_entity_teleport(
                id(),
                position(),
                sample["yaw"].as_f64().unwrap() as f32,
                sample["pitch"].as_f64().unwrap() as f32,
                sample["on_ground"].as_bool().unwrap(),
            ),
            kind => panic!("unexpected packet kind {kind}"),
        };
        let (length, prefix) = decode_varint(&packet).unwrap();
        assert_eq!(length as usize, packet.len() - prefix);
        let expected = hex(sample["wire_hex"].as_str().unwrap());
        assert_eq!(&packet[prefix..], expected, "{sample}");
        let (packet_id, _) = decode_varint(&expected).unwrap();
        assert_eq!(packet_id as i64, sample["packet_id"].as_i64().unwrap());
    }
    let types = &fixture["entity_types"];
    assert_eq!(
        ItemEntity::new(
            1,
            Position {
                x: 0.0,
                y: 0.0,
                z: 0.0
            }
        )
        .entity_type as i64,
        types["item"].as_i64().unwrap()
    );
    assert_eq!(
        MobKind::Cow.entity_type() as i64,
        types["cow"].as_i64().unwrap()
    );
    assert_eq!(
        MobKind::Zombie.entity_type() as i64,
        types["zombie"].as_i64().unwrap()
    );
}
