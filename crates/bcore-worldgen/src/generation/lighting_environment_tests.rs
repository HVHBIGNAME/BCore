use super::*;
use crate::block_predicate::{would_survive, Direction};
use crate::lighting::{biome_temperature, GenerationLight};
use crate::{biome, block, ChunkPos, MAX_Y, MIN_Y, WORLD_HEIGHT};
use std::cell::RefCell;

struct World {
    blocks: BTreeMap<Pos, u32>,
    biome: u32,
    reads: RefCell<Vec<Pos>>,
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, _x: i32, _z: i32) -> i32 {
        64
    }
    fn get_block(&self, pos: Pos) -> Option<u32> {
        self.reads.borrow_mut().push(pos);
        Some(self.blocks.get(&pos).copied().unwrap_or(block::AIR))
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        if (MIN_Y..=MAX_Y).contains(&pos.1) {
            self.blocks.insert(pos, state);
        }
        true
    }
}
impl FeatureWorld for World {
    fn feature_biome(&self, _pos: Pos) -> u32 {
        self.biome
    }
    fn feature_height(&self, _kind: FeatureHeightmap, _x: i32, _z: i32) -> i32 {
        64
    }
    fn can_write_feature(&self, _pos: Pos) -> bool {
        true
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, _flags: i32) -> bool {
        self.set_block(pos, state)
    }
    fn mark_feature_postprocessing(&mut self, _pos: Pos) {}
    fn schedule_feature_tick(&mut self, _request: TickRequest) -> bool {
        true
    }
}

fn int(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn position(v: &Value) -> Pos {
    (int(&v[0]), int(&v[1]), int(&v[2]))
}

#[test]
fn native_features_freeze_snow_temperature_and_survival() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../data/generation_environment_26_1.json")).unwrap();
    let mut world = World {
        blocks: BTreeMap::new(),
        biome: biome::id("plains").unwrap(),
        reads: RefCell::new(Vec::new()),
    };
    let environment = GenerationEnvironment;
    for row in fixture["samples"].as_array().unwrap() {
        let pos = position(&row["pos"]);
        let name = row["biome"].as_str().unwrap();
        world.biome = biome::id(name).unwrap();
        let state = int(&row["state"]) as u32;
        world.set_block(pos, state);
        world.set_block(Direction::Down.step(pos), int(&row["below"]) as u32);
        let expected = u32::from_str_radix(row["temperature_bits"].as_str().unwrap(), 16).unwrap();
        assert_eq!(
            biome_temperature(world.biome, pos, 63).unwrap().to_bits(),
            expected,
            "temperature {name} {pos:?}"
        );
        assert_eq!(
            environment.should_freeze(&world, pos).unwrap(),
            row["freeze"].as_bool().unwrap(),
            "freeze {row}"
        );
        assert_eq!(
            environment
                .should_freeze_with_edge(&world, pos, true)
                .unwrap(),
            row["freeze_edge"].as_bool().unwrap(),
            "edge {row}"
        );
        assert_eq!(
            environment.should_snow(&world, pos).unwrap(),
            row["snow"].as_bool().unwrap(),
            "snow {row}"
        );
        assert_eq!(
            would_survive(&world, state, pos, &environment).unwrap(),
            row["survives"].as_bool().unwrap(),
            "survival {row}"
        );
    }
    world.biome = biome::id("snowy_plains").unwrap();
    let pos = (8, 64, 8);
    for row in fixture["water"].as_array().unwrap() {
        world.set_block(pos, int(&row["state"]) as u32);
        for direction in Direction::HORIZONTAL {
            world.set_block(
                direction.step(pos),
                if row["all_water_neighbors"].as_bool().unwrap() {
                    block::WATER
                } else {
                    block::AIR
                },
            );
        }
        assert_eq!(
            environment.should_freeze(&world, pos).unwrap(),
            row["freeze"].as_bool().unwrap(),
            "source/flowing {row}"
        );
        assert_eq!(
            environment
                .should_freeze_with_edge(&world, pos, true)
                .unwrap(),
            row["freeze_edge"].as_bool().unwrap(),
            "source/flowing edge {row}"
        );
    }
    println!("Verified {} native biome/temperature/freezing/snow/survival observations plus 32 source/flowing water cases", fixture["samples"].as_array().unwrap().len());
}

#[test]
fn explicit_upper_biome_and_native_neighbor_access_order_are_preserved() {
    let pos = (15, 63, 15);
    let mut world = World {
        blocks: BTreeMap::from([(pos, block::WATER)]),
        biome: biome::id("desert").unwrap(),
        reads: RefCell::new(Vec::new()),
    };
    let environment = GenerationEnvironment;
    assert!(!environment.should_freeze(&world, pos).unwrap());
    assert!(world.reads.borrow().is_empty()); // Native warm rejection precedes block reads.
    let cold = biome::id("snowy_plains").unwrap();
    assert!(environment
        .should_freeze_in_biome(&world, cold, pos, false)
        .unwrap());
    world.reads.borrow_mut().clear();
    for direction in Direction::HORIZONTAL {
        world.set_block(direction.step(pos), block::WATER);
    }
    assert!(!environment
        .should_freeze_in_biome(&world, cold, pos, true)
        .unwrap());
    assert_eq!(
        *world.reads.borrow(),
        [pos, (14, 63, 15), (16, 63, 15), (15, 63, 14), (15, 63, 16)]
    );
    world.blocks.remove(&(14, 63, 15));
    world.reads.borrow_mut().clear();
    assert!(environment
        .should_freeze_in_biome(&world, cold, pos, true)
        .unwrap());
    assert_eq!(*world.reads.borrow(), [pos, (14, 63, 15)]);
    assert!(environment
        .should_freeze_in_biome(&world, u32::MAX, pos, false)
        .is_err());
}

#[test]
fn initialized_storage_changes_mushroom_survival_without_changing_the_biome_bridge() {
    let pos = (8, 64, 8);
    let world = World {
        blocks: BTreeMap::from([((8, 63, 8), block::STONE), ((8, 80, 8), block::STONE)]),
        biome: biome::id("snowy_plains").unwrap(),
        reads: RefCell::new(Vec::new()),
    };
    let mushroom = catalog().default_state("brown_mushroom").unwrap();
    assert!(!would_survive(&world, mushroom, pos, &GenerationEnvironment).unwrap());
    let mut light = GenerationLight::default();
    let mut states = vec![0; WORLD_HEIGHT as usize * 256];
    for (&(x, y, z), &state) in &world.blocks {
        states[((y - MIN_Y) * 256 + z * 16 + x) as usize] = state;
    }
    light
        .initialize_chunk(ChunkPos::new(0, 0), &states)
        .unwrap();
    let environment = GenerationEnvironment::with_light(&light);
    assert!(would_survive(&world, mushroom, pos, &environment).unwrap());
    assert_eq!(environment.raw_brightness(&world, pos).unwrap(), 0);
    assert_eq!(
        environment.biome_has_feature(world.biome, "freeze_top_layer"),
        GenerationEnvironment.biome_has_feature(world.biome, "freeze_top_layer")
    );
}
