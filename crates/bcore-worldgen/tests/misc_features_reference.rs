use std::cell::Cell;
use std::collections::BTreeMap;

use bcore_worldgen::block_predicate::{
    catalog, FeatureEnvironment, FeatureResult, RegistryEnvironment,
};
use bcore_worldgen::dripstone::CaveRandomState;
use bcore_worldgen::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::placement::ConfiguredFeatureDispatcher;
use bcore_worldgen::simplex::WorldgenRandom;
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use bcore_worldgen::{misc_features, MAX_Y, MIN_Y};
use serde::Deserialize;
use serde_json::{json, Value};

const PRIORITY: &str = include_str!("../data/misc_features_priority_26_1.json");
const EXTRA: &str = include_str!("../data/misc_features_extra_26_1.json");
const SHAPES: &str = include_str!("../data/misc_features_shapes_26_1.json");

#[derive(Deserialize)]
struct Terrain {
    layers: Vec<[i32; 3]>,
    overrides: Vec<[i32; 4]>,
    biome: String,
    write_policy: String,
    deny_origin: bool,
}

struct World {
    terrain: Terrain,
    origin: Pos,
    initial: BTreeMap<Pos, u32>,
    blocks: BTreeMap<Pos, u32>,
    writes: Vec<[i32; 6]>,
    ticks: Vec<[i32; 6]>,
    marks: Vec<[i32; 3]>,
    biome: u32,
    lower_biome: u32,
    split_y: i32,
    guards: Cell<usize>,
}

fn biome_id(name: &str) -> u32 {
    catalog().documents["biome_ids"][name].as_u64().unwrap() as u32
}

impl World {
    fn new(sample: &Value) -> Self {
        let terrain: Terrain = serde_json::from_value(sample["terrain"].clone()).unwrap();
        Self {
            origin: position(&sample["origin"]),
            biome: biome_id(&terrain.biome),
            lower_biome: biome_id(sample["lower_biome"].as_str().unwrap()),
            split_y: sample["split_y"].as_i64().unwrap() as i32,
            initial: terrain
                .overrides
                .iter()
                .map(|&[x, y, z, state]| ((x, y, z), state as u32))
                .collect(),
            terrain,
            blocks: BTreeMap::new(),
            writes: Vec::new(),
            ticks: Vec::new(),
            marks: Vec::new(),
            guards: Cell::new(0),
        }
    }

    fn initial(&self, pos: Pos) -> u32 {
        if !(MIN_Y..=MAX_Y).contains(&pos.1) {
            return catalog().default_state("void_air").unwrap();
        }
        if let Some(state) = self.initial.get(&pos) {
            return *state;
        }
        for &[min, max, state] in &self.terrain.layers {
            if (min..=max).contains(&pos.1) {
                return state as u32;
            }
        }
        catalog().default_state("air").unwrap()
    }

    fn check(&self, sample: &Value) {
        let label = format!(
            "{} / {} seed={} at {}",
            sample["name"], sample["scenario"], sample["seed"], sample["origin"]
        );
        assert_eq!(
            self.writes.len() as u64,
            sample["write_count"].as_u64().unwrap(),
            "write count {label}"
        );
        assert_eq!(
            json!(self.writes.iter().take(32).collect::<Vec<_>>()),
            sample["write_prefix"],
            "write prefix {label}"
        );
        assert_eq!(
            digest(self.writes.iter().map(|row| row.as_slice())),
            sample["write_md5"].as_str().unwrap(),
            "ordered writes {label}"
        );
        let changed: Vec<_> = self
            .blocks
            .iter()
            .filter(|(pos, state)| **state != self.initial(**pos))
            .map(|(&(x, y, z), &state)| [x, y, z, state as i32])
            .collect();
        assert_eq!(
            changed.len() as u64,
            sample["changed_blocks"].as_u64().unwrap(),
            "changed count {label}"
        );
        assert_eq!(
            digest(changed.iter().map(|row| row.as_slice())),
            sample["blocks_md5"].as_str().unwrap(),
            "final blocks {label}"
        );
        assert_eq!(
            json!(self.ticks),
            sample["ticks"],
            "native requested ticks {label}"
        );
        assert_eq!(json!(self.marks), sample["marks"], "postprocessing {label}");
    }
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.feature_height(FeatureHeightmap::OceanFloorWg, x, z)
    }
    fn get_block(&self, pos: Pos) -> Option<u32> {
        Some(
            self.blocks
                .get(&pos)
                .copied()
                .unwrap_or_else(|| self.initial(pos)),
        )
    }
    fn set_block(&mut self, _pos: Pos, _state: u32) -> bool {
        panic!("native flags must be retained")
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, pos: Pos) -> u32 {
        if pos.1 < self.split_y {
            self.lower_biome
        } else {
            self.biome
        }
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        for y in (MIN_Y..=MAX_Y).rev() {
            let state = self.get_block((x, y, z)).unwrap();
            let info = catalog().info(state).unwrap();
            let opaque = match kind {
                FeatureHeightmap::WorldSurface | FeatureHeightmap::WorldSurfaceWg => !info.is_air(),
                FeatureHeightmap::OceanFloor | FeatureHeightmap::OceanFloorWg => {
                    info.blocks_motion()
                }
                FeatureHeightmap::MotionBlocking => info.blocks_motion() || info.fluid_amount > 0,
                FeatureHeightmap::MotionBlockingNoLeaves => {
                    (info.blocks_motion() || info.fluid_amount > 0)
                        && !catalog().in_block_tag(state, "leaves").unwrap()
                }
            };
            if opaque {
                return y + 1;
            }
        }
        MIN_Y
    }
    fn can_write_feature(&self, _pos: Pos) -> bool {
        self.guards.set(self.guards.get() + 1);
        !self.terrain.deny_origin
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        let accepted = (MIN_Y..=MAX_Y).contains(&pos.1)
            && self.terrain.write_policy != "reject"
            && (self.terrain.write_policy != "checker" || (pos.0 + pos.2) & 1 == 0)
            && (self.terrain.write_policy != "source_chunk"
                || (pos.0 >> 4, pos.2 >> 4) == (self.origin.0 >> 4, self.origin.2 >> 4));
        self.writes.push([
            pos.0,
            pos.1,
            pos.2,
            state as i32,
            flags,
            i32::from(accepted),
        ]);
        if accepted {
            self.blocks.insert(pos, state);
        }
        accepted
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        self.marks.push([pos.0, pos.1, pos.2]);
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        let [x, y, z] = request.block_pos;
        let (id, fluid) = match request.target {
            TickTarget::Block(id) => (id, 0),
            TickTarget::Fluid(id) => (id, 1),
        };
        self.ticks.push([x, y, z, id as i32, request.delay, fluid]);
        true
    }
}

struct Environment {
    observations: Vec<[i32; 9]>,
    cursor: Cell<usize>,
    light: i32,
    sea_level: i32,
}

impl Environment {
    fn new(sample: &Value) -> Self {
        Self {
            observations: serde_json::from_value(sample["environment"].clone()).unwrap(),
            cursor: Cell::new(0),
            light: sample["terrain"]["light"].as_i64().unwrap() as i32,
            sea_level: sample["sea_level"].as_i64().unwrap() as i32,
        }
    }
    fn consume(
        &self,
        world: &dyn FeatureWorld,
        op: i32,
        biome: u32,
        pos: Pos,
        edge: bool,
    ) -> FeatureResult<bool> {
        let i = self.cursor.get();
        let row = self
            .observations
            .get(i)
            .unwrap_or_else(|| panic!("unexpected native callback {op} at {pos:?}"));
        assert_eq!(
            &row[..6],
            &[op, biome as i32, pos.0, pos.1, pos.2, i32::from(edge)],
            "callback order #{i}"
        );
        assert_eq!(
            world.get_block(pos).unwrap(),
            row[7] as u32,
            "callback input state #{i}"
        );
        assert_eq!(
            world.get_block((pos.0, pos.1 - 1, pos.2)).unwrap(),
            row[8] as u32,
            "callback support state #{i}"
        );
        self.cursor.set(i + 1);
        Ok(row[6] != 0)
    }
}

impl FeatureEnvironment for Environment {
    fn sea_level(&self) -> i32 {
        self.sea_level
    }
    fn raw_brightness(&self, _world: &dyn FeatureWorld, _pos: Pos) -> FeatureResult<i32> {
        Ok(self.light)
    }
    fn should_freeze_in_biome(
        &self,
        world: &dyn FeatureWorld,
        biome: u32,
        pos: Pos,
        edge: bool,
    ) -> FeatureResult<bool> {
        self.consume(world, 0, biome, pos, edge)
    }
    fn should_snow(&self, world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
        self.consume(world, 1, world.feature_biome(pos), pos, false)
    }
}

fn position(value: &Value) -> Pos {
    (
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
        value[2].as_i64().unwrap() as i32,
    )
}

fn digest<'a>(rows: impl Iterator<Item = &'a [i32]>) -> String {
    let mut hash = md5::Context::new();
    for row in rows {
        for n in row {
            hash.consume(n.to_le_bytes());
        }
    }
    format!("{:x}", hash.compute())
}

#[derive(Default)]
struct Dispatcher(CaveRandomState);

impl ConfiguredFeatureDispatcher for Dispatcher {
    fn next_gaussian(&mut self, random: &mut WorldgenRandom) -> FeatureResult<f64> {
        Ok(self.0.next_gaussian(random))
    }
    fn place_configured(
        &mut self,
        _document: &Value,
        _name: Option<&str>,
        _world: &mut dyn FeatureWorld,
        _random: &mut WorldgenRandom,
        _origin: Pos,
        _environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        panic!("misc kernels do not delegate configured features")
    }
}

fn verify_fixture(fixture: &str) -> usize {
    let data: Value = serde_json::from_str(fixture).unwrap();
    assert_eq!(data["minecraft"], "26.1");
    assert_eq!(data["protocol"], 775);
    assert_eq!(
        data["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    let samples = data["samples"].as_array().unwrap();
    for sample in samples {
        let name = sample["name"].as_str().unwrap();
        let document = &data["configured"][name];
        assert!(
            misc_features::supports_configured(document).unwrap(),
            "{name}"
        );
        for through_base in [false, true] {
            let mut world = World::new(sample);
            let environment = Environment::new(sample);
            let mut random = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
            let mut dispatcher =
                bcore_worldgen::base_features::BaseFeatures::with_dispatcher(Dispatcher::default());
            let result = if through_base {
                dispatcher.place_configured(
                    document,
                    Some(name),
                    &mut world,
                    &mut random,
                    position(&sample["origin"]),
                    &environment,
                )
            } else {
                misc_features::place_configured_with(
                    document,
                    &mut world,
                    &mut random,
                    position(&sample["origin"]),
                    &environment,
                    &mut dispatcher,
                )
            }
            .unwrap_or_else(|e| {
                panic!("{name} / {} (base={through_base}): {e}", sample["scenario"])
            });
            assert_eq!(
                world.guards.get(),
                1,
                "native Feature.place checks the origin exactly once (base={through_base}, {name})"
            );
            assert_eq!(
                result,
                sample["result"].as_bool().unwrap(),
                "result {name} / {}",
                sample["scenario"]
            );
            world.check(sample);
            assert_eq!(
                random.next_long(),
                sample["next_i64"].as_i64().unwrap(),
                "RNG {name} / {} seed={}",
                sample["scenario"],
                sample["seed"]
            );
            assert_eq!(
                WorldgenRandom::new(987654321).next_long(),
                sample["world_next_i64"].as_i64().unwrap(),
                "independent world RNG {name}"
            );
            assert_eq!(
                environment.cursor.get(),
                environment.observations.len(),
                "unconsumed native biome callbacks {name}"
            );
            if let Some(bits) = sample["next_gaussian_bits"].as_str() {
                assert_eq!(
                    dispatcher
                        .additional
                        .0
                        .next_gaussian(&mut random)
                        .to_bits()
                        .to_string(),
                    bits,
                    "Gaussian continuation {name} / {}",
                    sample["scenario"]
                );
            }
        }
    }
    samples.len()
}

#[test]
fn native_priority_magma_bamboo_vines_and_freeze() {
    assert_eq!(verify_fixture(PRIORITY), 384);
}

#[test]
fn native_aquatic_coral_mushrooms_piles_and_gaussian_continuation() {
    assert_eq!(verify_fixture(EXTRA), 543);
}

#[test]
fn native_rocks_blue_ice_spikes_and_icebergs() {
    assert_eq!(verify_fixture(SHAPES), 590);
}

#[test]
fn native_survival_of_six_new_plants_on_every_block_state_and_hanging_vines() {
    let data: Value = serde_json::from_str(EXTRA).unwrap();
    let mut world = World::new(&data["samples"][0]);
    world.initial.clear();
    world.terrain.layers.clear();
    for plant in data["survival_support"].as_array().unwrap() {
        let state = plant["state"].as_u64().unwrap() as u32;
        assert_eq!(catalog().state(&plant["state_json"]).unwrap(), state);
        let mut end = 0;
        for row in plant["ranges"].as_array().unwrap() {
            let [start, stop, expected]: [u32; 3] = serde_json::from_value(row.clone()).unwrap();
            assert_eq!(start, end);
            end = stop;
            for soil in start..stop {
                world.initial.insert((0, 63, 0), soil);
                let result = bcore_worldgen::block_predicate::would_survive(
                    &world,
                    state,
                    (0, 64, 0),
                    &RegistryEnvironment,
                )
                .unwrap_or_else(|e| panic!("plant {state} on {soil}: {e}"));
                assert_eq!(result, expected != 0, "native plant {state} on {soil}");
            }
        }
        assert_eq!(
            end,
            catalog().documents["state_count"].as_u64().unwrap() as u32
        );
    }
    for sample in data["vine_survival"].as_array().unwrap() {
        let world = World::new(sample);
        let result = bcore_worldgen::block_predicate::would_survive(
            &world,
            sample["state"].as_u64().unwrap() as u32,
            position(&sample["origin"]),
            &RegistryEnvironment,
        )
        .unwrap();
        assert_eq!(
            result,
            sample["result"].as_bool().unwrap(),
            "vine survival {sample}"
        );
    }
}

#[test]
fn nested_provider_capabilities_and_native_codec_rejections() {
    let data: Value = serde_json::from_str(EXTRA).unwrap();
    for rejected in data["native_codec_rejections"].as_array().unwrap() {
        assert!(
            misc_features::supports_configured(rejected).is_err(),
            "native codec rejection {rejected}"
        );
    }
    for name in ["pickle_normal", "pile_normal"] {
        assert!(
            misc_features::requirements(&data["configured"][name])
                .unwrap()
                .unwrap()
                .gaussian
        );
    }
    let mut unsupported = data["configured"]["huge_red_mushroom"].clone();
    unsupported["config"]["can_place_on"] =
        json!({"type":"minecraft:would_survive", "state":{"Name":"minecraft:torch"}});
    assert!(matches!(
        misc_features::supports_configured(&unsupported),
        Err(FeatureError::Unsupported(_))
    ));
    unsupported["config"]["can_place_on"] =
        json!({"type":"minecraft:would_survive", "state":{"Name":"minecraft:brown_mushroom"}});
    assert!(
        misc_features::requirements(&unsupported)
            .unwrap()
            .unwrap()
            .raw_brightness
    );
}

#[test]
fn capability_validation_and_missing_environment_are_explicit() {
    let data: Value = serde_json::from_str(PRIORITY).unwrap();
    let document = &data["configured"]["freeze_top_layer"];
    let needs = misc_features::requirements(document).unwrap().unwrap();
    assert!(needs.freezing && needs.snow);
    let mut world = World::new(&data["samples"][0]);
    let mut random = WorldgenRandom::new(17);
    assert!(matches!(
        misc_features::place_configured(
            document,
            &mut world,
            &mut random,
            (0, 64, 0),
            &RegistryEnvironment
        ),
        Err(FeatureError::MissingData(_))
    ));
    assert!(world.writes.is_empty());
    assert_eq!(random.next_long(), WorldgenRandom::new(17).next_long());
    for bad in [
        json!({"type":"minecraft:bamboo","config":{"probability":1.01}}),
        json!({"type":"minecraft:underwater_magma","config":{"floor_search_range":513,"placement_radius_around_floor":2,"placement_probability_per_valid_position":0.5}}),
        json!({"type":"minecraft:underwater_magma","config":{"floor_search_range":16,"placement_radius_around_floor":65,"placement_probability_per_valid_position":0.5}}),
    ] {
        assert!(matches!(
            misc_features::supports_configured(&bad),
            Err(FeatureError::InvalidConfig(_))
        ));
    }
    assert!(
        !misc_features::supports_configured(&json!({"type":"other:vines","config":{}})).unwrap()
    );
    assert!(
        !misc_features::supports_configured(&json!({"type":"minecraft:geode","config":{}}))
            .unwrap()
    );
}
