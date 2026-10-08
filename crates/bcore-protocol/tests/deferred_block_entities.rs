use std::collections::BTreeMap;

use bcore_core::{varint::decode_varint, ChunkPos};
use bcore_protocol::chunk::{ChunkColumn, LIGHT_SECTION_COUNT, MIN_Y};
use bcore_protocol::chunk_store::{decode_chunk, encode_chunk, ChunkStoreError};
use bcore_worldgen::block_entity::PendingBlockEntity;
use bcore_worldgen::generation::{FeatureBlockEntity, StructureEntityRequest};
use bcore_worldgen::lighting::{ChunkLight, LightSection};
use bcore_worldgen::structure::template::{Mirror, Nbt, Rotation, TemplateBlockEntity};
use bcore_worldgen::structure::template_pool::StructureAssets;
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use serde_json::Value;

fn fixture() -> Value {
    let data: Value = serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/deferred_block_entities_26_1_v1.json"
    ))
    .unwrap();
    assert_eq!(data["cases"].as_array().unwrap().len(), 39);
    data
}

fn nbt(v: &Value) -> Nbt {
    serde_json::from_value(v["nbt"].clone()).unwrap()
}

fn observation<'a>(row: &'a Value, boundary: &str) -> &'a Value {
    row["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["boundary"] == boundary)
        .unwrap()
}

fn pending_column(row: &Value) -> (ChunkPos, (usize, i32, usize), ChunkColumn) {
    let p: [i32; 3] = serde_json::from_value(row["pos"].clone()).unwrap();
    let owner = ChunkPos::new(p[0] >> 4, p[2] >> 4);
    let local = ((p[0] & 15) as usize, p[1], (p[2] & 15) as usize);
    let mut column = ChunkColumn::new(1);
    assert!(column.set(
        local.0,
        local.1,
        local.2,
        row["state"].as_u64().unwrap() as u32
    ));
    assert!(column.set_pending_block_entity(
        owner,
        local.0,
        local.1,
        local.2,
        PendingBlockEntity::from_nbt(&nbt(&row["pending"]))
    ));
    (owner, local, column)
}

fn take<'a>(input: &mut &'a [u8], len: usize) -> &'a [u8] {
    let (out, rest) = input.split_at(len);
    *input = rest;
    out
}

fn varint(input: &mut &[u8]) -> usize {
    let (value, len) = decode_varint(input).unwrap();
    take(input, len);
    usize::try_from(value).unwrap()
}

fn block_entity_prefix(payload: &[u8]) -> (usize, &[u8]) {
    let mut input = &payload[8..];
    for _ in 0..varint(&mut input) {
        varint(&mut input);
        let len = varint(&mut input);
        take(&mut input, len * 8);
    }
    let sections = varint(&mut input);
    take(&mut input, sections);
    let count = varint(&mut input);
    (count, input)
}

fn unhex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn deferred_proto_storage_is_inert_and_native_packet_entries_appear_only_after_materialization() {
    let data = fixture();
    let mut checked = 0;
    for row in data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "basic")
    {
        let initial = observation(row, "after conversion/no lookup");
        let (owner, local, column) = pending_column(initial);
        let bytes = encode_chunk(owner.x, owner.z, &column);
        assert_eq!(&bytes[4..8], [5, 0, 0, 0]);
        let mut loaded = decode_chunk(&bytes).unwrap();
        assert_eq!(loaded, column);
        assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
        let packet = loaded.encode_payload(owner.x, owner.z);
        assert_eq!(
            block_entity_prefix(&packet).0,
            0,
            "a pending DUMMY has no update packet"
        );
        assert_eq!(
            observation(row, "packet before level save")["packet"]["count"],
            0
        );
        assert_eq!(
            loaded, column,
            "network projection must not create a runtime entity"
        );
        assert!(loaded
            .materialize_block_entity(owner, local.0, local.1, local.2)
            .unwrap());
        assert!(loaded.pending_block_entities().is_empty());
        let expected = &observation(row, "level save")["after"];
        if let Some(entity) = loaded.feature_block_entities().get(&local) {
            assert_eq!(entity.full_nbt().unwrap(), nbt(&expected["full"]));
            assert_eq!(entity.update_nbt().unwrap(), nbt(&expected["update"]));
        } else {
            let p = (
                owner.x * 16 + local.0 as i32,
                local.1,
                owner.z * 16 + local.2 as i32,
            );
            assert_eq!(
                loaded.block_entities()[&local].full_data(p),
                nbt(&expected["full"]).to_json()
            );
        }
        let expected_packet = &observation(row, "packet after level save")["packet"];
        assert_eq!(expected_packet["count"], 1);
        let wire = unhex(expected_packet["entries"][0]["wire_hex"].as_str().unwrap());
        let packet = loaded.encode_payload(owner.x, owner.z);
        let (count, encoded) = block_entity_prefix(&packet);
        assert_eq!(count, 1);
        assert_eq!(
            &encoded[..wire.len()],
            wire,
            "{} actual native BlockEntityInfo bytes",
            row["name"]
        );
        let saved = encode_chunk(owner.x, owner.z, &loaded);
        assert_eq!(decode_chunk(&saved).unwrap(), loaded);
        let before = loaded.clone();
        assert!(loaded
            .materialize_block_entity(owner, local.0, local.1, local.2)
            .unwrap());
        assert_eq!(loaded, before, "second lookup is inert");
        checked += 1;
    }
    assert_eq!(checked, 16);
}

#[test]
fn deferred_saved_nbt_numeric_widths_survive_bcc_before_and_after_native_load_boundary() {
    let data = fixture();
    let mut checked = 0;
    for row in data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "saved")
    {
        let initial = observation(row, "installed saved pending NBT");
        let (owner, local, column) = pending_column(initial);
        let raw = nbt(&initial["pending"]);
        assert_eq!(raw.get("keepPacked"), Some(&Nbt::Byte(1)));
        if let Some(seed) = raw.get("LootTableSeed") {
            assert_eq!(seed, &Nbt::Long(i64::MIN + 17));
        }
        let encoded = encode_chunk(owner.x, owner.z, &column);
        let mut loaded = decode_chunk(&encoded).unwrap();
        assert_eq!(
            loaded.pending_block_entities()[&local].full_nbt().unwrap(),
            raw
        );
        assert_eq!(encode_chunk(owner.x, owner.z, &loaded), encoded);
        let original = loaded.clone();
        assert!(loaded
            .materialize_block_entity(
                ChunkPos::new(owner.x + 1, owner.z),
                local.0,
                local.1,
                local.2
            )
            .is_err());
        assert_eq!(loaded, original, "wrong owner cannot consume a pending tag");
        assert!(loaded
            .materialize_block_entity(owner, local.0, local.1, local.2)
            .unwrap());
        let expected = &observation(row, "saved pending region lookup")["after"];
        assert_eq!(
            loaded.feature_block_entities()[&local].full_nbt().unwrap(),
            nbt(&expected["full"])
        );
        assert_eq!(
            loaded.feature_block_entities()[&local]
                .update_nbt()
                .unwrap(),
            nbt(&expected["update"])
        );
        assert!(loaded.pending_block_entities().is_empty());
        assert_eq!(
            decode_chunk(&encode_chunk(owner.x, owner.z, &loaded)).unwrap(),
            loaded
        );
        checked += 1;
    }
    assert_eq!(checked, 4);
}

#[test]
fn deferred_bcc_preserves_raw_ticks_marks_structure_requests_and_native_light() {
    let data = fixture();
    let row = &data["cases"][0];
    let (owner, local, mut column) = pending_column(observation(row, "write/no lookup"));
    let p = [
        owner.x * 16 + local.0 as i32,
        local.1,
        owner.z * 16 + local.2 as i32,
    ];
    let tick = TickRequest {
        block_pos: p,
        target: TickTarget::Fluid(2),
        delay: -5,
    };
    assert!(column.add_tick_request(owner, tick));
    assert!(column.add_tick_request(owner, tick));
    assert!(column.mark_postprocessing(local.0, local.1, local.2));
    assert!(column.mark_postprocessing(local.0, local.1, local.2));
    let position = p.map(|v| f64::from(v) + 0.5);
    let request = StructureEntityRequest {
        source_chunk: [owner.x - 1, owner.z],
        position_bits: position.map(f64::to_bits),
        block_pos: p,
        typed_data: Nbt::Compound(BTreeMap::from([
            ("id".into(), Nbt::String("minecraft:pig".into())),
            (
                "Pos".into(),
                Nbt::List {
                    element_type: 6,
                    values: position.into_iter().map(Nbt::Double).collect(),
                },
            ),
        ]))
        .typed_json(),
        finalize: true,
        rotation: Rotation::None,
        mirror: Mirror::None,
    };
    assert!(column.add_structure_entity(owner, request.clone()));
    assert!(column.set_light(ChunkLight {
        min_section_y: MIN_Y / 16 - 1,
        sections: vec![LightSection::default(); LIGHT_SECTION_COUNT],
    }));
    let bytes = encode_chunk(owner.x, owner.z, &column);
    assert_eq!(
        &bytes[4..8],
        [5, 0, 7, 0],
        "existing extension flags are unchanged"
    );
    let mut loaded = decode_chunk(&bytes).unwrap();
    assert_eq!(loaded, column);
    assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
    assert!(loaded
        .materialize_block_entity(owner, local.0, local.1, local.2)
        .unwrap());
    assert_eq!(loaded.tick_requests(), [tick, tick]);
    assert_eq!(loaded.postprocessing_positions(), [local, local]);
    assert_eq!(loaded.structure_entities(), [request]);
    assert_eq!(loaded.light(), column.light());
    assert_eq!(loaded.states(), column.states());
    assert_eq!(loaded.structures(), column.structures());
    assert!(loaded.set(local.0, local.1, local.2, 0));
    assert!(loaded.feature_block_entities().is_empty());
    let mut pending = column;
    assert!(pending.set(local.0, local.1, local.2, 0));
    assert!(pending.pending_block_entities().is_empty());
}

#[test]
fn deferred_typed_hive_all_native_states_preserve_payload_across_bcc_versions() {
    let data = fixture();
    let row = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "BEE_NEST/source-callback")
        .unwrap();
    let after = &observation(row, "source getBlockEntity/mutation")["after"];
    let [x, y, z]: [i32; 3] = serde_json::from_value(after["pos"].clone()).unwrap();
    let pos = (x, y, z);
    let owner = ChunkPos::new(x >> 4, z >> 4);
    let local = ((x & 15) as usize, y, (z & 15) as usize);
    let initial_state = after["state"].as_u64().unwrap() as u32;
    let full = nbt(&after["full"]);
    let entity = TemplateBlockEntity::from_load(
        &StructureAssets::bundled().blocks,
        initial_state,
        pos,
        full.clone(),
    )
    .unwrap();
    let mut column = ChunkColumn::new(1);
    assert!(column.set(local.0, y, local.2, initial_state));
    assert!(column.set_feature_block_entity(
        owner,
        local.0,
        y,
        local.2,
        FeatureBlockEntity::from_template(&entity).unwrap(),
    ));
    let hive_reference: Value =
        serde_json::from_str(include_str!("../../bcore-worldgen/data/beehives_26_1.json")).unwrap();
    let states = hive_reference["states"].as_array().unwrap();
    assert_eq!(states.len(), 48);
    for state in states {
        let state = state["state"].as_u64().unwrap() as u32;
        assert!(column.set(local.0, y, local.2, state));
        let hive = &column.feature_block_entities()[&local];
        assert!(hive.valid_for(state, pos), "native hive state {state}");
        assert_eq!(hive.full_nbt().unwrap(), full);
        for version in [4u16, 5] {
            let mut bytes = encode_chunk(owner.x, owner.z, &column);
            bytes[4..6].copy_from_slice(&version.to_le_bytes());
            checksum(&mut bytes);
            let restored = decode_chunk(&bytes).unwrap();
            assert_eq!(restored, column);
            assert!(restored.pending_block_entities().is_empty());
            let payload = restored.encode_payload(owner.x, owner.z);
            let (count, mut entries) = block_entity_prefix(&payload);
            assert_eq!(count, 1);
            take(&mut entries, 3);
            assert_eq!(varint(&mut entries), 34);
            assert_eq!(take(&mut entries, 1), [0]);
        }
    }
}

fn checksum(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let hash = bytes[..end].iter().fold(0x811c_9dc5u32, |hash, b| {
        (hash ^ u32::from(*b)).wrapping_mul(0x0100_0193)
    });
    bytes[end..].copy_from_slice(&hash.to_le_bytes());
}

#[test]
fn deferred_decoder_rejects_wrong_owner_duplicates_bad_widths_and_legacy_pending_kinds() {
    let data = fixture();
    let (owner, local, column) = pending_column(observation(&data["cases"][0], "write/no lookup"));
    let original = encode_chunk(owner.x, owner.z, &column);
    let pending = &column.pending_block_entities()[&local];
    let json = serde_json::to_vec(pending).unwrap();
    let at = original
        .windows(json.len())
        .position(|w| w == json)
        .unwrap();
    assert_eq!(original[at - 5], 5, "new pending entry kind");
    for change in 0..4 {
        let mut malformed = original.clone();
        match change {
            0 => malformed[4..6].copy_from_slice(&4u16.to_le_bytes()),
            1 => malformed[at - 4..at].copy_from_slice(&u32::MAX.to_le_bytes()),
            2 => {
                let entry = original[at - 10..at + json.len()].to_vec();
                malformed.splice(at + json.len()..at + json.len(), entry);
                malformed[at - 14..at - 10].copy_from_slice(&2u32.to_le_bytes());
            }
            _ => {
                let mut invalid = pending.clone();
                invalid.typed_data[1]["x"][0] = 4.into(); // owner coordinates must be TAG_Int
                let bytes = serde_json::to_vec(&invalid).unwrap();
                malformed[at - 4..at].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
                malformed.splice(at..at + json.len(), bytes);
            }
        }
        checksum(&mut malformed);
        assert!(
            matches!(
                decode_chunk(&malformed),
                Err(ChunkStoreError::InvalidBlockEntity)
            ),
            "case {change}"
        );
    }
    let mut wrong_owner = original.clone();
    wrong_owner[8..12].copy_from_slice(&(owner.x + 1).to_le_bytes());
    checksum(&mut wrong_owner);
    assert!(matches!(
        decode_chunk(&wrong_owner),
        Err(ChunkStoreError::InvalidBlockEntity)
    ));
}

#[test]
fn deferred_v5_reader_preserves_materialized_v4_and_leaves_unsupported_pending_data_intact() {
    let data = fixture();
    let (owner, local, mut column) =
        pending_column(observation(&data["cases"][0], "write/no lookup"));
    let state = column.get(local.0, local.1, local.2).unwrap();
    let pos = (
        owner.x * 16 + local.0 as i32,
        local.1,
        owner.z * 16 + local.2 as i32,
    );
    let mut unknown = column.pending_block_entities()[&local].full_nbt().unwrap();
    unknown.compound_mut().unwrap().insert(
        "id".into(),
        Nbt::String("minecraft:unimplemented_pending".into()),
    );
    assert!(column.set_pending_block_entity(
        owner,
        local.0,
        local.1,
        local.2,
        PendingBlockEntity::from_nbt(&unknown)
    ));
    let mut loaded = decode_chunk(&encode_chunk(owner.x, owner.z, &column)).unwrap();
    assert!(loaded
        .materialize_block_entity(owner, local.0, local.1, local.2)
        .is_err());
    assert_eq!(loaded, column);
    assert_eq!(
        block_entity_prefix(&loaded.encode_payload(owner.x, owner.z)).0,
        0
    );

    let full = nbt(&observation(&data["cases"][0], "region lookup")["full"]);
    let entity =
        TemplateBlockEntity::from_load(&StructureAssets::bundled().blocks, state, pos, full)
            .unwrap();
    assert!(column.set_feature_block_entity(
        owner,
        local.0,
        local.1,
        local.2,
        FeatureBlockEntity::from_template(&entity).unwrap()
    ));
    assert!(column.pending_block_entities().is_empty());
    let mut legacy = encode_chunk(owner.x, owner.z, &column);
    legacy[4..6].copy_from_slice(&4u16.to_le_bytes());
    checksum(&mut legacy);
    let restored = decode_chunk(&legacy).unwrap();
    assert_eq!(
        restored, column,
        "v4 materialized entries must not be reinterpreted as DUMMY"
    );
    assert!(restored.pending_block_entities().is_empty());
}
