use bcore_core::ChunkPos;
use bcore_protocol::chunk::ChunkColumn;
use bcore_protocol::chunk_store::{decode_chunk_at, encode_chunk};
use bcore_protocol::nbt::{encode_feature_block_entity_update, encode_typed_nbt};
use bcore_worldgen::generation::FeatureBlockEntity;
use bcore_worldgen::structure::mineshaft::region::StructureData;
use bcore_worldgen::structure::scattered::{ScatteredPieceData, ScatteredStart};
use bcore_worldgen::structure::template::{Nbt, TemplateBlockEntity};
use bcore_worldgen::structure::template_pool::StructureAssets;
use serde_json::Value;
use std::collections::BTreeMap;

fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn nbt(v: &Value) -> Nbt {
    serde_json::from_value(v["nbt"].clone()).unwrap()
}

#[test]
fn desert_pyramid_native_chests_brushables_and_live_piece_state_survive_bcc() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/desert_pyramid_26_1.json"
    ))
    .unwrap();
    let mut passes = 0;
    let mut brushables = 0;
    let mut chests = 0;
    for row in fixture["placements"].as_array().unwrap() {
        let mut start = ScatteredStart::from_nbt(&nbt(&row["initial"])).unwrap();
        start.reference_bounds();
        let owner = ChunkPos::new(start.source[0], start.source[1]);
        for (index, pass) in row["passes"].as_array().unwrap().iter().enumerate() {
            let saved = nbt(&pass["after"]);
            let loaded = ScatteredStart::from_nbt(&saved).unwrap();
            if row["special"] == "reload" && index > 0 {
                start = loaded;
            } else {
                start.piece = loaded.piece;
            }
            let ScatteredPieceData::DesertPyramid { archaeology, .. } = &mut start.piece.data
            else {
                unreachable!()
            };
            *archaeology = serde_json::from_value(pass["archaeology"].clone()).unwrap();
            let bounds = start.reference_bounds();
            assert_eq!(
                serde_json::json!(bounds.as_array()),
                pass["cached_reference_bounds"]
            );
            let mut structures = StructureData::default();
            structures
                .scattered_starts
                .insert(start.kind.name().into(), start.clone());
            structures
                .scattered_references
                .insert(start.kind.name().into(), vec![start.source]);
            let mut columns = BTreeMap::from([((owner.x, owner.z), ChunkColumn::flat())]);
            assert!(columns
                .get_mut(&(owner.x, owner.z))
                .unwrap()
                .set_structures(owner, structures));
            let states: BTreeMap<_, _> = pass["states"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| ((n(&p[0]), n(&p[1]), n(&p[2])), n(&p[3]) as u32))
                .collect();
            for entry in pass["block_entities"].as_array().unwrap() {
                let p = &entry["pos"];
                let p = (n(&p[0]), n(&p[1]), n(&p[2]));
                let full = nbt(&entry["full"]);
                let update = nbt(&entry["update"]);
                let state = states[&p];
                let owner = ChunkPos::new(p.0 >> 4, p.2 >> 4);
                let entity = TemplateBlockEntity::from_load(
                    &StructureAssets::bundled().blocks,
                    state,
                    p,
                    full.clone(),
                )
                .unwrap();
                let data = FeatureBlockEntity::from_template(&entity).unwrap();
                assert_eq!(data.full_nbt().unwrap(), full);
                assert_eq!(data.update_nbt().unwrap(), update);
                assert_eq!(
                    encode_feature_block_entity_update(&data),
                    encode_typed_nbt(&update).unwrap()
                );
                match entity.id.as_str() {
                    "minecraft:chest" => chests += 1,
                    "minecraft:brushable_block" => brushables += 1,
                    id => panic!("unexpected pyramid BE {id}"),
                }
                let c = columns
                    .entry((owner.x, owner.z))
                    .or_insert_with(ChunkColumn::flat);
                let local = ((p.0 & 15) as usize, p.1, (p.2 & 15) as usize);
                assert!(c.set(local.0, local.1, local.2, state));
                assert!(c.set_feature_block_entity(owner, local.0, local.1, local.2, data));
            }
            for e in pass["effects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[0] == "mark")
            {
                let (x, y, z) = (n(&e[1]), n(&e[2]), n(&e[3]));
                assert!(columns
                    .entry((x >> 4, z >> 4))
                    .or_insert_with(ChunkColumn::flat)
                    .mark_postprocessing((x & 15) as usize, y, (z & 15) as usize));
            }
            for ((x, z), column) in columns {
                let bytes = encode_chunk(x, z, &column);
                let (rx, rz, loaded) = decode_chunk_at(&bytes).unwrap();
                assert_eq!((rx, rz), (x, z));
                assert_eq!(loaded, column);
                assert_eq!(encode_chunk(x, z, &loaded), bytes);
                if (x, z) == (owner.x, owner.z) {
                    let mut restored =
                        loaded.structures().scattered_starts[start.kind.name()].clone();
                    assert_eq!(restored, start);
                    assert_eq!(restored.to_nbt(), saved);
                    assert_eq!(restored.reference_bounds(), bounds);
                }
            }
            passes += 1;
        }
    }
    assert_eq!(passes, 137);
    assert!(
        brushables > 100 && chests > 100,
        "{brushables} brushables, {chests} chests"
    );
}
