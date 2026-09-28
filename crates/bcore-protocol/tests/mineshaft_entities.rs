use bcore_core::ChunkPos;
use bcore_protocol::{
    chunk::ChunkColumn,
    chunk_store::{decode_chunk_at, encode_chunk},
};
use bcore_worldgen::{
    block_entity::{BlockEntity, SpawnerMob},
    generated_entity::GeneratedEntity,
    structure::mineshaft::{region::StructureData, MineType, MineshaftLayout},
    WorldGenerator,
};
use serde_json::Value;

#[test]
fn native_minecart_loot_and_cave_spawners_survive_persistence() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/mineshaft_blocks_26_1.json"
    ))
    .unwrap();
    let mut count = 0;
    for sample in fixture["samples"].as_array().unwrap() {
        for entity in sample["entities"].as_array().unwrap() {
            let p = std::array::from_fn::<_, 3, _>(|i| {
                entity["pos"][i].as_f64().unwrap().floor() as i32
            });
            let owner = ChunkPos::new(p[0] >> 4, p[2] >> 4);
            let mut column = ChunkColumn::flat();
            assert!(column.add_entity(
                owner,
                GeneratedEntity::ChestMinecart {
                    block_pos: p,
                    loot_seed: entity["loot_seed"].as_i64().unwrap()
                }
            ));
            let bytes = encode_chunk(owner.x, owner.z, &column);
            let (_, _, loaded) = decode_chunk_at(&bytes).unwrap();
            assert_eq!(loaded.entities()[0].data(), entity["nbt"]);
            assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
            count += 1;
        }
        for be in sample["block_entities"].as_array().unwrap() {
            let (x, y, z) = (
                be["x"].as_i64().unwrap() as i32,
                be["y"].as_i64().unwrap() as i32,
                be["z"].as_i64().unwrap() as i32,
            );
            let owner = ChunkPos::new(x >> 4, z >> 4);
            let local = ((x & 15) as usize, y, (z & 15) as usize);
            let mut column = ChunkColumn::flat();
            column.set(local.0, y, local.2, bcore_worldgen::dungeon::SPAWNER);
            assert!(column.set_block_entity(
                local.0,
                y,
                local.2,
                BlockEntity::Spawner {
                    mob: SpawnerMob::CaveSpider
                }
            ));
            let (_, _, loaded) = decode_chunk_at(&encode_chunk(owner.x, owner.z, &column)).unwrap();
            assert_eq!(loaded.block_entities()[&local].full_data((x, y, z)), *be);
        }
    }
    assert!(count >= 9);
}

#[test]
fn normal_and_mesa_starts_and_references_survive_persistence() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/mineshafts_26_1.json"
    ))
    .unwrap();
    for sample in fixture["samples"].as_array().unwrap() {
        let seed = sample["seed"].as_i64().unwrap();
        let owner = ChunkPos::new(
            sample["chunk"][0].as_i64().unwrap() as i32,
            sample["chunk"][1].as_i64().unwrap() as i32,
        );
        let kind = if sample["type"] == "NORMAL" {
            MineType::Normal
        } else {
            MineType::Mesa
        };
        let mut random = bcore_worldgen::structure::placement::large_feature_random(seed, owner);
        let (layout, _) = MineshaftLayout::start(&mut random, owner, kind, 63, -64, |x, z| {
            WorldGenerator::new(seed).base_height_vanilla(x, z)
        });
        let data = StructureData {
            mineshaft_start: Some(layout),
            references: vec![([owner.x, owner.z], kind)],
        };
        let mut column = ChunkColumn::flat();
        assert!(column.set_structures(owner, data.clone()));
        let bytes = encode_chunk(owner.x, owner.z, &column);
        let (_, _, loaded) = decode_chunk_at(&bytes).unwrap();
        assert_eq!(loaded.structures(), &data);
        assert_eq!(
            loaded.structures().native_data(owner),
            data.native_data(owner)
        );
        assert_eq!(encode_chunk(owner.x, owner.z, &loaded), bytes);
    }
}
