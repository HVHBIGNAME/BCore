use std::collections::BTreeMap;

use bcore_worldgen::{
    block,
    features::OreKind,
    ore::{self, OreConfig, OreWorld},
    simplex::WorldgenRandom,
};

struct ReferenceWorld<'a> {
    terrain: &'a str,
    cave_air: u32,
    writes: BTreeMap<(i32, i32, i32), u32>,
}

impl ReferenceWorld<'_> {
    fn assert_writes(&self, sample: &serde_json::Value) {
        let mut digest = md5::Context::new();
        for (&(x, y, z), &state) in &self.writes {
            digest.consume(x.to_le_bytes());
            digest.consume(y.to_le_bytes());
            digest.consume(z.to_le_bytes());
            digest.consume(state.to_le_bytes());
        }
        assert_eq!(
            self.writes.len() as u64,
            sample["changed_blocks"].as_u64().unwrap(),
            "{sample}"
        );
        assert_eq!(
            format!("{:x}", digest.compute()),
            sample["writes_md5"].as_str().unwrap(),
            "{sample}"
        );
    }

    fn initial(&self, (x, y, _z): (i32, i32, i32)) -> u32 {
        let surface = if self.terrain == "ceiling" { 320 } else { 64 };
        if y < -64 || y >= surface || self.terrain == "air" {
            return block::AIR;
        }
        if x == 8 {
            match self.terrain {
                "cave" => return block::AIR,
                "cave_air" => return self.cave_air,
                "water" => return block::WATER,
                "bedrock" => return block::BEDROCK,
                _ => {}
            }
        }
        match self.terrain {
            "deepslate" => block::DEEPSLATE,
            "deepslate_x" => 27923,
            "deepslate_z" => 27925,
            "tuff" => block::TUFF,
            _ => block::STONE,
        }
    }
}

impl OreWorld for ReferenceWorld<'_> {
    fn ocean_floor_wg(&self, x: i32, _z: i32) -> i32 {
        if self.terrain == "air"
            || (x == 8 && matches!(self.terrain, "cave" | "cave_air" | "water"))
        {
            -64
        } else if self.terrain == "ceiling" {
            320
        } else {
            64
        }
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
        assert!((-64..320).contains(&pos.1));
        if state == self.initial(pos) {
            self.writes.remove(&pos);
        } else {
            self.writes.insert(pos, state);
        }
        true
    }
}

#[test]
fn configured_ores_match_native_26_1() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../data/ores_26_1.json")).unwrap();
    for (name, expected) in [
        ("base_stone_overworld", ore::BASE_STONE_OVERWORLD),
        ("stone_ore_replaceables", ore::STONE_ORE_REPLACEABLES),
        (
            "deepslate_ore_replaceables",
            ore::DEEPSLATE_ORE_REPLACEABLES,
        ),
    ] {
        let states: Vec<u32> = fixture["tags"][name]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_u64().unwrap() as u32)
            .collect();
        assert_eq!(states, expected, "{name}");
    }
    for sample in fixture["samples"].as_array().unwrap() {
        let name = sample["kind"].as_str().unwrap();
        let native = &fixture["configurations"][name];
        let kind = match name {
            "ore_coal" | "ore_coal_buried" => OreKind::Coal,
            "ore_diamond_buried" => OreKind::Diamond,
            "ore_iron" => OreKind::Iron,
            "ore_copper_large" => OreKind::Copper,
            "ore_granite" => OreKind::Granite,
            "ore_dirt" => OreKind::Dirt,
            other => panic!("unknown reference config: {other}"),
        };
        let config = OreConfig {
            kind,
            size: native["size"].as_u64().unwrap() as usize,
            discard_on_air_exposure: native["discard_chance_on_air_exposure"].as_f64().unwrap()
                as f32,
        };
        let mut world = ReferenceWorld {
            terrain: sample["terrain"].as_str().unwrap(),
            cave_air: fixture["cave_air_state"].as_u64().unwrap() as u32,
            writes: BTreeMap::new(),
        };
        let mut rng = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        let o = &sample["origin"];
        let origin = (
            o[0].as_i64().unwrap() as i32,
            o[1].as_i64().unwrap() as i32,
            o[2].as_i64().unwrap() as i32,
        );
        for expected in sample["placed"].as_array().unwrap() {
            assert_eq!(
                ore::place(&mut world, &mut rng, origin, config),
                expected.as_bool().unwrap(),
                "{sample}"
            );
        }
        world.assert_writes(sample);
        assert_eq!(
            rng.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{sample}"
        );
    }
}

#[test]
fn placed_ores_match_native_modifier_and_shape_streams() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../data/ore_placements_26_1.json")).unwrap();
    for (name, state) in [
        ("CLAY", block::CLAY),
        ("INFESTED_STONE", block::INFESTED_STONE),
        ("INFESTED_DEEPSLATE", block::INFESTED_DEEPSLATE),
    ] {
        assert_eq!(
            state as u64,
            fixture["block_states"][name].as_u64().unwrap()
        );
    }
    for sample in fixture["samples"].as_array().unwrap() {
        let mut world = ReferenceWorld {
            terrain: sample["terrain"].as_str().unwrap(),
            cave_air: 0,
            writes: BTreeMap::new(),
        };
        let name = sample["name"].as_str().unwrap();
        let source = bcore_core::ChunkPos::new(
            sample["chunk"][0].as_i64().unwrap() as i32,
            sample["chunk"][1].as_i64().unwrap() as i32,
        );
        let seed = sample["seed"].as_i64().unwrap();
        let index = sample["index"].as_i64().unwrap() as i32;
        let step = sample["step"].as_i64().unwrap() as i32;
        assert_eq!(
            bcore_worldgen::feature_sorter::sorter().within_step_index(name),
            Some((step as usize, index as usize))
        );
        let mut rng = WorldgenRandom::new(seed);
        let decoration_seed = rng.set_decoration_seed(seed, source.x * 16, source.z * 16);
        rng.set_feature_seed(decoration_seed, index, step);
        let placed = bcore_worldgen::features::place_ore_feature(
            &mut world,
            &mut rng,
            source,
            name,
            |_, _| true,
        )
        .unwrap();
        assert_eq!(placed, sample["placed"].as_bool().unwrap(), "{sample}");
        world.assert_writes(sample);
        assert_eq!(
            rng.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{sample}"
        );
    }
}
