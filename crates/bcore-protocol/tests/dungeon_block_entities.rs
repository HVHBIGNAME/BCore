use bcore_protocol::{
    chunk::ChunkColumn,
    chunk_store::{decode_chunk, encode_chunk},
    nbt::encode_block_entity_update,
};
use bcore_worldgen::{
    block_entity::{BlockEntity, SpawnerMob},
    dungeon,
};

fn entity(nbt: &serde_json::Value) -> BlockEntity {
    match nbt["id"].as_str().unwrap() {
        "minecraft:chest" => BlockEntity::DungeonChest {
            loot_seed: nbt["LootTableSeed"].as_i64().unwrap_or(0),
        },
        "minecraft:mob_spawner" => BlockEntity::Spawner {
            mob: match nbt["SpawnData"]["entity"]["id"].as_str().unwrap() {
                "minecraft:skeleton" => SpawnerMob::Skeleton,
                "minecraft:zombie" => SpawnerMob::Zombie,
                "minecraft:spider" => SpawnerMob::Spider,
                other => panic!("unexpected mob {other}"),
            },
        },
        other => panic!("unexpected block entity {other}"),
    }
}

fn take<'a>(data: &mut &'a [u8], count: usize) -> &'a [u8] {
    let (out, rest) = data.split_at(count);
    *data = rest;
    out
}
fn string(data: &mut &[u8]) -> String {
    let len = u16::from_be_bytes(take(data, 2).try_into().unwrap()) as usize;
    String::from_utf8(take(data, len).to_vec()).unwrap()
}
fn read_compound(data: &mut &[u8]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    loop {
        let kind = take(data, 1)[0];
        if kind == 0 {
            break;
        }
        let key = string(data);
        let value = match kind {
            2 => serde_json::json!(i16::from_be_bytes(take(data, 2).try_into().unwrap())),
            8 => serde_json::json!(string(data)),
            10 => read_compound(data),
            other => panic!("unexpected NBT type {other}; native spawner settings must be shorts"),
        };
        assert!(map.insert(key, value).is_none());
    }
    map.into()
}

fn varint(data: &mut &[u8]) -> usize {
    let (n, bytes) = bcore_core::varint::decode_varint(data).unwrap();
    take(data, bytes);
    usize::try_from(n).unwrap()
}

fn skip_chunk_sections<'a>(data: &mut &'a [u8]) -> &'a [u8] {
    take(data, 8); // chunk coordinates
    for _ in 0..varint(data) {
        varint(data); // heightmap kind
        let count = varint(data);
        take(data, count * 8);
    }
    let size = varint(data);
    take(data, size)
}

#[test]
fn all_native_room_entities_keep_update_nbt_and_persisted_data() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../bcore-worldgen/data/monster_rooms_26_1.json"
    ))
    .unwrap();
    for sample in fixture["samples"].as_array().unwrap() {
        for native in sample["block_entities"].as_array().unwrap() {
            let data = entity(&native["nbt"]);
            let encoded = encode_block_entity_update(&data);
            assert_eq!(encoded[0], 10, "anonymous compound root");
            let mut input = &encoded[1..];
            assert_eq!(read_compound(&mut input), native["update"]);
            assert!(input.is_empty());
            let p = &native["pos"];
            let (x, y, z) = (
                p[0].as_i64().unwrap() as i32,
                p[1].as_i64().unwrap() as i32,
                p[2].as_i64().unwrap() as i32,
            );
            let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
            let mut column = ChunkColumn::new(0);
            column.set(
                lx,
                y,
                lz,
                match data {
                    BlockEntity::DungeonChest { .. } => dungeon::CHEST,
                    BlockEntity::Spawner { .. } => dungeon::SPAWNER,
                },
            );
            assert!(column.set_block_entity(lx, y, lz, data.clone()));
            let bytes = encode_chunk(x >> 4, z >> 4, &column);
            let loaded = decode_chunk(&bytes).unwrap();
            assert_eq!(column, loaded);
            assert_eq!(
                loaded.block_entities()[&(lx, y, lz)].full_data((x, y, z)),
                native["nbt"]
            );
            assert_eq!(
                loaded.encode_payload(x >> 4, z >> 4),
                column.encode_payload(x >> 4, z >> 4)
            );
            let payload = loaded.encode_payload(x >> 4, z >> 4);
            let mut packet = payload.as_slice();
            skip_chunk_sections(&mut packet);
            assert_eq!(varint(&mut packet), 1);
            assert_eq!(take(&mut packet, 1)[0], ((lx << 4) | lz) as u8);
            assert_eq!(
                i16::from_be_bytes(take(&mut packet, 2).try_into().unwrap()),
                y as i16
            );
            assert_eq!(
                varint(&mut packet),
                native["type"].as_u64().unwrap() as usize
            );
            assert_eq!(take(&mut packet, 1), [10]);
            assert_eq!(read_compound(&mut packet), native["update"]);
            column.set(lx, y, lz, 0);
            assert!(
                column.block_entities().is_empty(),
                "removing a container must remove its data"
            );
        }
    }
}

#[test]
fn cave_air_is_empty_for_heightmaps_and_lighting() {
    let mut column = ChunkColumn::new(0);
    column.set(8, 64, 8, dungeon::CAVE_AIR);
    assert_eq!(column.surface_y(8, 8), None);
    assert_eq!(column.heightmap(), ChunkColumn::new(0).heightmap());
    assert_eq!(column.lit_sections(), ChunkColumn::new(0).lit_sections());
}
