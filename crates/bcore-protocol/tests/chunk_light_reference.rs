use bcore_core::ChunkPos;
use bcore_protocol::chunk::{block_state, ChunkColumn, LIGHT_SECTION_COUNT, MIN_Y};
use bcore_protocol::chunk_store::{decode_chunk, encode_chunk, ChunkStoreError};
use bcore_worldgen::generation::StructureEntityRequest;
use bcore_worldgen::lighting::{ChunkLight, LightSection};
use bcore_worldgen::structure::template::{Mirror, Nbt, Rotation};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/chunk_light_wire_26_1.json")).unwrap()
    })
}

fn hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2));
    (0..value.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&value[at..at + 2], 16).unwrap())
        .collect()
}

fn light(sample: &Value) -> ChunkLight {
    ChunkLight {
        min_section_y: sample["min_section_y"].as_i64().unwrap() as i32,
        sections: sample["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|section| LightSection {
                sky: section["sky"].as_str().map(hex),
                block: section["block"].as_str().map(hex),
                sky_empty: section["sky_empty"].as_bool().unwrap(),
                block_empty: section["block_empty"].as_bool().unwrap(),
            })
            .collect(),
    }
}

#[test]
fn native_packet_masks_and_bytes_include_lazy_and_materialized_zero_layers() {
    let fixture = fixture();
    assert_eq!(fixture["minecraft"], "26.1");
    assert_eq!(fixture["protocol"], 775);
    assert_eq!(
        fixture["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    for (name, source) in [
        (
            "scripts\\TreeReference.java",
            include_bytes!("../../../scripts/TreeReference.java").as_slice(),
        ),
        (
            "scripts\\ChunkLightPacketReference.java",
            include_bytes!("../../../scripts/ChunkLightPacketReference.java").as_slice(),
        ),
    ] {
        assert_eq!(
            fixture["sources"][name],
            format!("{:x}", Sha256::digest(source))
        );
    }
    assert_eq!(
        fixture["capture_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../scripts/capture_chunk_light_reference.py"
            ))
        )
    );
    let samples = fixture["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 19);
    for sample in samples {
        let mut column = ChunkColumn::flat();
        assert!(column.set_light(light(sample)), "{}", sample["name"]);
        let expected = hex(sample["hex"].as_str().unwrap());
        assert_eq!(
            column.encode_light_payload(),
            expected,
            "{}",
            sample["name"]
        );
        let payload = column.encode_payload(-17, 29);
        assert_eq!(
            &payload[payload.len() - expected.len()..],
            expected,
            "map_chunk: {}",
            sample["name"]
        );
        let (data, empty) = column.lit_sections();
        assert_eq!(
            serde_json::json!(data),
            sample["sky_mask"],
            "{}",
            sample["name"]
        );
        assert_eq!(
            serde_json::json!(empty),
            sample["empty_sky_mask"],
            "{}",
            sample["name"]
        );
    }
}

fn request() -> StructureEntityRequest {
    let position = [0.5f64, 80.0, 0.5];
    let data = Nbt::Compound(
        [
            ("id".into(), Nbt::String("minecraft:pig".into())),
            (
                "Pos".into(),
                Nbt::List {
                    element_type: 6,
                    values: position.into_iter().map(Nbt::Double).collect(),
                },
            ),
        ]
        .into(),
    );
    StructureEntityRequest {
        source_chunk: [0, 0],
        position_bits: position.map(f64::to_bits),
        block_pos: [0, 80, 0],
        typed_data: data.typed_json(),
        finalize: true,
        rotation: Rotation::None,
        mirror: Mirror::None,
    }
}

#[test]
fn native_light_round_trips_with_every_other_bcc_extension_combination() {
    for sample in fixture()["samples"].as_array().unwrap() {
        for other_flags in 0..4u16 {
            let mut column = ChunkColumn::flat();
            if other_flags & 1 != 0 {
                assert!(column.mark_postprocessing(15, MIN_Y, 0));
                assert!(column.mark_postprocessing(15, MIN_Y, 0));
            }
            if other_flags & 2 != 0 {
                assert!(column.add_structure_entity(ChunkPos::new(0, 0), request()));
                assert!(column.add_structure_entity(ChunkPos::new(0, 0), request()));
            }
            assert!(column.set_light(light(sample)));
            let bytes = encode_chunk(0, 0, &column);
            assert_eq!(
                u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
                4 | other_flags
            );
            let loaded = decode_chunk(&bytes).unwrap();
            assert_eq!(loaded, column, "{}, flags {other_flags}", sample["name"]);
            assert_eq!(encode_chunk(0, 0, &loaded), bytes);
            assert_eq!(
                loaded.encode_light_payload(),
                hex(sample["hex"].as_str().unwrap())
            );
        }
    }
}

fn checksum(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let hash = bytes[..end].iter().fold(0x811c9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    });
    bytes[end..].copy_from_slice(&hash.to_le_bytes());
}

#[test]
fn malformed_light_headers_and_flags_are_rejected_after_checksum_validation() {
    let mut column = ChunkColumn::flat();
    assert!(column.set_light(light(&fixture()["samples"][0])));
    let bytes = encode_chunk(0, 0, &column);
    // The all-null native case has a six-byte light header and 26 flag bytes.
    let start = bytes.len() - 4 - 6 - LIGHT_SECTION_COUNT;
    for mutation in 0..6 {
        let mut bad = bytes.clone();
        match mutation {
            0 => bad[start..start + 4].copy_from_slice(&(-6i32).to_le_bytes()),
            1 => bad[start + 4..start + 6].copy_from_slice(&25u16.to_le_bytes()),
            2 => bad[start + 4..start + 6].copy_from_slice(&u16::MAX.to_le_bytes()),
            3 => bad[start + 6] = 16,
            4 => bad[start + 6] = 4, // lazy zero without a sky layer
            _ => bad[start + 6] = 8, // lazy zero without a block layer
        }
        checksum(&mut bad);
        assert!(
            matches!(decode_chunk(&bad), Err(ChunkStoreError::InvalidLight)),
            "mutation {mutation}"
        );
    }
    let mut truncated = bytes;
    let last_flag = truncated.len() - 5;
    truncated[last_flag] = 1; // 2048 sky bytes were promised but are absent
    checksum(&mut truncated);
    assert!(matches!(
        decode_chunk(&truncated),
        Err(ChunkStoreError::Truncated { .. })
    ));
}

#[test]
fn invalid_light_does_not_replace_a_valid_snapshot_and_block_edits_invalidate_it() {
    let sample = fixture()["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "pattern_both")
        .unwrap();
    let valid = light(sample);
    let mut column = ChunkColumn::flat();
    assert!(column.set_light(valid.clone()));
    for mutation in 0..5 {
        let mut bad = valid.clone();
        match mutation {
            0 => bad.min_section_y += 1,
            1 => {
                bad.sections.pop();
            }
            2 => {
                bad.sections[0].sky.as_mut().unwrap().pop();
            }
            3 => bad.sections[0].sky_empty = true,
            _ => {
                bad.sections[0].block = None;
                bad.sections[0].block_empty = true;
            }
        }
        assert!(!column.set_light(bad), "mutation {mutation}");
        assert_eq!(column.light(), Some(&valid));
    }
    assert!(!column.set(16, 80, 0, block_state::STONE));
    assert!(column.set(0, MIN_Y, 0, block_state::BEDROCK));
    assert_eq!(column.light(), Some(&valid));
    assert!(column.set(0, 80, 0, block_state::STONE));
    assert!(column.light().is_none());
}

#[test]
fn generated_light_boundary_survives_column_and_disk_transfer() {
    use bcore_worldgen::generation::{ChunkStatus, StageState};
    let world = bcore_worldgen::GenerationWorld::new(846692123413862008);
    let result = world
        .generate_to_status(ChunkPos::new(0, 0), ChunkStatus::Light)
        .unwrap();
    assert_eq!(
        result.coverage.target.stage(ChunkStatus::Light).state,
        StageState::Complete
    );
    assert_eq!(
        result.coverage.target.stage(ChunkStatus::Full).state,
        StageState::Pending
    );
    let native = result
        .chunk
        .light()
        .expect("executed LIGHT supplies a snapshot");
    let column = ChunkColumn::from_generated(&result.chunk);
    assert_eq!(column.light(), Some(native));
    let loaded = decode_chunk(&encode_chunk(0, 0, &column)).unwrap();
    assert_eq!(loaded.light(), Some(native));
    assert_eq!(loaded.encode_light_payload(), column.encode_light_payload());
}
