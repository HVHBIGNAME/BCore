use bcore_worldgen::{
    block,
    block_entity::BlockEntity,
    dungeon::{self, DungeonWorld},
    ore::OreWorld,
    simplex::WorldgenRandom,
};
use std::collections::BTreeMap;

struct World<'a> {
    terrain: &'a str,
    origin: (i32, i32, i32),
    writes: BTreeMap<(i32, i32, i32), u32>,
    entities: BTreeMap<(i32, i32, i32), BlockEntity>,
}

impl World<'_> {
    fn initial(&self, (x, y, z): (i32, i32, i32)) -> u32 {
        if !(-64..320).contains(&y) {
            return block::AIR;
        }
        let (x, y, z) = (x - self.origin.0, y - self.origin.1, z - self.origin.2);
        if (self.terrain == "floor_hole" && x == 0 && z == 0 && y == -1)
            || (self.terrain == "roof_hole" && x == 0 && z == 0 && y == 4)
            || (self.terrain == "unsupported_floor" && y == -2)
        {
            return block::AIR;
        }
        if self.terrain == "protected" && x == 0 && z == 0 && (-1..=1).contains(&y) {
            return block::BEDROCK;
        }
        if self.terrain == "existing_chest" && x == 1 && y == 0 && z == 1 {
            return dungeon::CHEST;
        }
        if self.terrain != "closed"
            && (y == 0 || y == 1)
            && z.abs() <= if self.terrain == "wide" { 2 } else { 0 }
        {
            return block::AIR;
        }
        block::STONE
    }
}
impl OreWorld for World<'_> {
    fn ocean_floor_wg(&self, _: i32, _: i32) -> i32 {
        panic!("unused by MonsterRoomFeature");
    }
    fn get_block(&self, pos: (i32, i32, i32)) -> Option<u32> {
        Some(
            self.writes
                .get(&pos)
                .copied()
                .unwrap_or_else(|| self.initial(pos)),
        )
    }
    fn set_block(&mut self, pos: (i32, i32, i32), state: u32) -> bool {
        if !(-64..320).contains(&pos.1) {
            return false;
        }
        if state == self.initial(pos) {
            self.writes.remove(&pos);
        } else {
            self.writes.insert(pos, state);
        }
        if self
            .entities
            .get(&pos)
            .is_some_and(|entity| !entity.matches_state(state))
        {
            self.entities.remove(&pos);
        }
        true
    }
}
impl DungeonWorld for World<'_> {
    fn set_block_entity(&mut self, pos: (i32, i32, i32), data: BlockEntity) {
        self.entities.insert(pos, data);
    }
}

#[test]
fn rooms_blocks_rng_and_block_entities_match_native_26_1() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../data/monster_rooms_26_1.json")).unwrap();
    for sample in fixture["samples"].as_array().unwrap() {
        let o = &sample["origin"];
        let origin = (
            o[0].as_i64().unwrap() as i32,
            o[1].as_i64().unwrap() as i32,
            o[2].as_i64().unwrap() as i32,
        );
        let seed = sample["seed"].as_i64().unwrap();
        let terrain = sample["terrain"].as_str().unwrap();
        let mut world = World {
            terrain,
            origin,
            writes: BTreeMap::new(),
            entities: BTreeMap::new(),
        };
        let mut random = WorldgenRandom::new(seed);
        assert_eq!(
            dungeon::place(&mut world, &mut random, origin),
            sample["placed"].as_bool().unwrap(),
            "{seed} {terrain} {origin:?}"
        );
        let mut digest = md5::Context::new();
        for (&(x, y, z), &state) in &world.writes {
            digest.consume(x.to_le_bytes());
            digest.consume(y.to_le_bytes());
            digest.consume(z.to_le_bytes());
            digest.consume(state.to_le_bytes());
        }
        assert_eq!(
            world.writes.len() as u64,
            sample["changed_blocks"].as_u64().unwrap(),
            "{seed} {terrain} {origin:?}"
        );
        assert_eq!(
            format!("{:x}", digest.compute()),
            sample["writes_md5"].as_str().unwrap(),
            "{seed} {terrain} {origin:?}"
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{seed} {terrain} {origin:?}"
        );
        let expected = sample["block_entities"].as_array().unwrap();
        assert_eq!(
            world.entities.len(),
            expected.len(),
            "{seed} {terrain} {origin:?}"
        );
        for (index, (&pos, data)) in world.entities.iter().enumerate() {
            assert_eq!(
                data.full_data(pos),
                expected[index]["nbt"],
                "{seed} {terrain} {pos:?}"
            );
            assert_eq!(
                data.update_data(),
                expected[index]["update"],
                "{seed} {terrain} {pos:?}"
            );
            assert_eq!(
                data.type_id() as u64,
                expected[index]["type"].as_u64().unwrap()
            );
        }
    }
}
