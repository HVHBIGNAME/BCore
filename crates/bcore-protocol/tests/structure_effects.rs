use bcore_core::ChunkPos;
use bcore_protocol::chunk::ChunkColumn;
use bcore_protocol::chunk_store::{decode_chunk, decode_chunk_at, encode_chunk, ChunkStoreError};
use bcore_protocol::nbt::encode_feature_block_entity_update;
use bcore_worldgen::generation::{FeatureBlockEntity, StructureEntityRequest};
use bcore_worldgen::structure::template::{Mirror, Nbt, Rotation, TemplateBlockEntity};
use bcore_worldgen::structure::template_pool::StructureAssets;
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn native_city_inventories_and_loot_keep_typed_data_through_storage() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/jigsaw_reference_26_1.json"
    ))
    .unwrap();
    let assets = StructureAssets::bundled();
    let mut fixed_inventories = 0;
    let mut loot_tables = 0;
    let mut entities = 0;
    for sample in fixture["cities"].as_array().unwrap() {
        let mut columns = BTreeMap::new();
        for row in sample["block_entities"].as_array().unwrap() {
            let full: Nbt = serde_json::from_value(row["nbt"].clone()).unwrap();
            let id = full.get("id").and_then(Nbt::string).unwrap();
            if !matches!(id, "minecraft:chest" | "minecraft:furnace") {
                continue;
            }
            fixed_inventories += usize::from(
                full.get("Items")
                    .and_then(Nbt::list)
                    .is_some_and(|items| !items.is_empty()),
            );
            loot_tables += usize::from(full.get("LootTable").is_some());
            let [x, y, z] = ["x", "y", "z"].map(|key| full.get(key).and_then(Nbt::int).unwrap());
            let owner = ChunkPos::new(x >> 4, z >> 4);
            let state = assets.blocks.default_state(id).unwrap();
            let template =
                TemplateBlockEntity::from_load(&assets.blocks, state, (x, y, z), full.clone())
                    .unwrap();
            assert_eq!(
                template.full_data(),
                full,
                "native saved inventory at {x},{y},{z}"
            );
            let data = FeatureBlockEntity::from_template(&template).unwrap();
            assert_eq!(Nbt::from_typed_json(&data.typed_data).unwrap(), full);
            assert_eq!(encode_feature_block_entity_update(&data), [10, 0]);
            let column = columns
                .entry((owner.x, owner.z))
                .or_insert_with(ChunkColumn::flat);
            assert!(column.set((x & 15) as usize, y, (z & 15) as usize, state));
            assert!(column.set_feature_block_entity(
                owner,
                (x & 15) as usize,
                y,
                (z & 15) as usize,
                data
            ));
            entities += 1;
        }
        for ((x, z), column) in columns {
            let bytes = encode_chunk(x, z, &column);
            let (rx, rz, loaded) = decode_chunk_at(&bytes).unwrap();
            assert_eq!((rx, rz), (x, z));
            assert_eq!(loaded, column);
            assert_eq!(encode_chunk(x, z, &loaded), bytes);
            for data in loaded.feature_block_entities().values() {
                assert_eq!(encode_feature_block_entity_update(data), [10, 0]);
            }
        }
    }
    assert!(entities >= 10);
    assert!(
        fixed_inventories >= 4,
        "native furnace/chest inventories must be exercised"
    );
    assert!(loot_tables >= 4);
}

#[test]
fn native_decorated_pot_full_and_update_tags_survive_bcc_storage() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/decorated_pot_block_entities_26_1.json"
    ))
    .unwrap();
    let rows = fixture["block_entity_loads"].as_array().unwrap();
    assert_eq!(rows.len(), 80);
    for row in rows {
        let p: [i32; 3] = serde_json::from_value(row["pos"].clone()).unwrap();
        let owner = ChunkPos::new(p[0] >> 4, p[2] >> 4);
        let local = ((p[0] & 15) as usize, p[1], (p[2] & 15) as usize);
        let state = row["state"].as_u64().unwrap() as u32;
        let load: Nbt = serde_json::from_value(row["load"]["nbt"].clone()).unwrap();
        let full: Nbt = serde_json::from_value(row["full"]["nbt"].clone()).unwrap();
        let update: Nbt = serde_json::from_value(row["update"]["nbt"].clone()).unwrap();
        let entity = TemplateBlockEntity::from_load(
            &StructureAssets::bundled().blocks,
            state,
            (p[0], p[1], p[2]),
            load,
        )
        .unwrap();
        let data = FeatureBlockEntity::from_template(&entity).unwrap();
        let expected_update = bcore_protocol::nbt::encode_typed_nbt(&update).unwrap();
        assert_eq!(
            encode_feature_block_entity_update(&data),
            expected_update,
            "{} wire update",
            row["name"]
        );
        let mut column = ChunkColumn::flat();
        assert!(column.set(local.0, local.1, local.2, state));
        assert!(column.set_feature_block_entity(owner, local.0, local.1, local.2, data));
        let bytes = encode_chunk(owner.x, owner.z, &column);
        let loaded = decode_chunk(&bytes).unwrap();
        assert_eq!(loaded, column);
        let data = &loaded.feature_block_entities()[&local];
        assert_eq!(data.full_nbt().unwrap(), full, "{} saved tag", row["name"]);
        assert_eq!(encode_feature_block_entity_update(data), expected_update);
        assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
    }
}

#[test]
fn native_scattered_piece_flags_and_cached_references_survive_bcc_storage() {
    use bcore_worldgen::structure::mineshaft::region::StructureData;
    use bcore_worldgen::structure::scattered::ScatteredStart;
    let mut passes = 0;
    for source in [
        include_str!("../../bcore-worldgen/data/scattered_structures_26_1.json"),
        include_str!("../../bcore-worldgen/data/scattered_jungle_temple_26_1.json"),
    ] {
        let fixture: Value = serde_json::from_str(source).unwrap();
        for row in fixture["placements"].as_array().unwrap() {
            let initial: Nbt = serde_json::from_value(row["initial"]["nbt"].clone()).unwrap();
            let mut start = ScatteredStart::from_nbt(&initial).unwrap();
            let bounds = start.reference_bounds();
            let owner = ChunkPos::new(start.source[0], start.source[1]);
            for pass in row["passes"].as_array().unwrap() {
                let saved: Nbt = serde_json::from_value(pass["after"]["nbt"].clone()).unwrap();
                start.piece = ScatteredStart::from_nbt(&saved).unwrap().piece;
                assert_eq!(start.to_nbt(), saved);
                let mut data = StructureData::default();
                data.scattered_starts
                    .insert(start.kind.name().into(), start.clone());
                if start.references_chunk(owner) {
                    data.scattered_references
                        .insert(start.kind.name().into(), vec![start.source]);
                }
                let mut column = ChunkColumn::flat();
                assert!(column.set_structures(owner, data.clone()));
                let bytes = encode_chunk(owner.x, owner.z, &column);
                let loaded = decode_chunk(&bytes).unwrap();
                assert_eq!(loaded, column);
                let mut restored = loaded.structures().scattered_starts[start.kind.name()].clone();
                assert_eq!(restored.to_nbt(), saved);
                assert_eq!(restored.reference_bounds(), bounds);
                assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
                data.scattered_references
                    .insert(start.kind.name().into(), vec![[owner.x + 9, owner.z]]);
                assert!(!column.set_structures(owner, data));
                passes += 1;
            }
        }
    }
    assert_eq!(passes, 107);
}

fn native_requests() -> Vec<(ChunkPos, StructureEntityRequest)> {
    let mut requests = Vec::new();
    for template in StructureAssets::bundled().templates.values() {
        for entity in &template.entities {
            let origin = [-32.0, 80.0, 48.0];
            let position = std::array::from_fn(|axis| origin[axis] + entity.pos[axis]);
            let block_pos = std::array::from_fn(|axis| {
                origin[axis] as i32
                    + [entity.block_pos.0, entity.block_pos.1, entity.block_pos.2][axis]
            });
            let owner = ChunkPos::new(
                position[0].floor() as i32 >> 4,
                position[2].floor() as i32 >> 4,
            );
            let mut nbt = entity.nbt.clone();
            let fields = nbt.compound_mut().unwrap();
            fields.remove("UUID");
            fields.insert(
                "Pos".into(),
                Nbt::List {
                    element_type: 6,
                    values: position.into_iter().map(Nbt::Double).collect(),
                },
            );
            let request = StructureEntityRequest {
                source_chunk: [owner.x, owner.z],
                position_bits: position.map(f64::to_bits),
                block_pos,
                typed_data: nbt.typed_json(),
                finalize: true,
                rotation: Rotation::Clockwise90,
                mirror: Mirror::FrontBack,
            };
            assert!(request.valid_for(owner));
            requests.push((owner, request));
        }
    }
    assert!(
        !requests.is_empty(),
        "native village templates must supply entity NBT"
    );
    requests
}

#[test]
fn native_template_entity_requests_round_trip_with_both_extension_flags() {
    for (owner, request) in native_requests() {
        for with_marks in [false, true] {
            let mut column = ChunkColumn::flat();
            assert!(column.add_structure_entity(owner, request.clone()));
            assert!(column.add_structure_entity(owner, request.clone()));
            if with_marks {
                assert!(column.mark_postprocessing(0, 80, 0));
                assert!(column.mark_postprocessing(0, 80, 0));
            }
            let bytes = encode_chunk(owner.x, owner.z, &column);
            assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 4);
            assert_eq!(
                u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
                if with_marks { 3 } else { 2 }
            );
            let loaded = decode_chunk(&bytes).unwrap();
            assert_eq!(loaded, column);
            assert_eq!(
                loaded.structure_entities(),
                &[request.clone(), request.clone()]
            );
            assert!(
                loaded.entities().is_empty(),
                "pending requests must not become fake spawned entities"
            );
            assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
        }
    }
}

fn checksum(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let hash = bytes[..end].iter().fold(0x811c9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x01000193)
    });
    bytes[end..].copy_from_slice(&hash.to_le_bytes());
}

#[test]
fn corrupt_structure_requests_cannot_hide_behind_a_valid_checksum() {
    let (owner, request) = native_requests().remove(0);
    let mut column = ChunkColumn::flat();
    assert!(column.add_structure_entity(owner, request.clone()));
    let bytes = encode_chunk(owner.x, owner.z, &column);
    let body = serde_json::to_vec(&request).unwrap();
    let offset = bytes
        .windows(body.len())
        .position(|window| window == body)
        .unwrap();
    for mutation in 0..4 {
        let mut record = serde_json::to_value(&request).unwrap();
        match mutation {
            0 => record["position_bits"][0] = Value::from(f64::NAN.to_bits()),
            1 => {
                record["position_bits"][0] = Value::from((f64::from(owner.x * 16) + 32.0).to_bits())
            }
            2 => {
                record["typed_data"][1]["UUID"] = serde_json::json!([11, [0, 0, 0, 0]]);
            }
            _ => {
                record["typed_data"][1]
                    .as_object_mut()
                    .unwrap()
                    .remove("id");
            }
        }
        let replacement = serde_json::to_vec(&record).unwrap();
        let mut invalid = bytes.clone();
        invalid.splice(offset..offset + body.len(), replacement.iter().copied());
        invalid[offset - 4..offset].copy_from_slice(&(replacement.len() as u32).to_le_bytes());
        checksum(&mut invalid);
        assert!(
            matches!(
                decode_chunk(&invalid),
                Err(ChunkStoreError::InvalidStructureEntity)
            ),
            "mutation {mutation}"
        );
    }
    let mut invalid = bytes;
    invalid[offset - 4..offset].copy_from_slice(&u32::MAX.to_le_bytes());
    checksum(&mut invalid);
    assert!(matches!(
        decode_chunk(&invalid),
        Err(ChunkStoreError::InvalidStructureEntity)
    ));
}
