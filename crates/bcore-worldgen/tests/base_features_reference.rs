use std::collections::BTreeMap;

use bcore_worldgen::dripstone::CaveRandomState;
use bcore_worldgen::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::placement::ConfiguredFeatureDispatcher;
use bcore_worldgen::simplex::WorldgenRandom;
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use bcore_worldgen::{base_features, block_predicate, placement, MAX_Y, MIN_Y};
use block_predicate::{catalog, FeatureEnvironment, FeatureResult};
use serde::Deserialize;
use serde_json::{json, Value};

const BASE: &str = include_str!("../data/base_features_26_1.json");
const PLACEMENT: &str = include_str!("../data/placements_26_1.json");

#[derive(Deserialize)]
struct Terrain {
    layers: Vec<[i32; 3]>,
    overrides: Vec<[i32; 4]>,
    biome: String,
    write_policy: String,
    light: i32,
    deny_origin: bool,
}

struct World {
    terrain: Terrain,
    initial: BTreeMap<Pos, u32>,
    blocks: BTreeMap<Pos, u32>,
    writes: Vec<[i32; 6]>,
    marks: Vec<[i32; 3]>,
    ticks: Vec<[i32; 6]>,
    biome: u32,
}

impl World {
    fn new(value: &Value) -> Self {
        let terrain: Terrain = serde_json::from_value(value.clone()).unwrap();
        let biome = catalog().documents["biome_ids"][&terrain.biome]
            .as_u64()
            .unwrap() as u32;
        let initial = terrain
            .overrides
            .iter()
            .map(|&[x, y, z, state]| ((x, y, z), state as u32))
            .collect();
        Self {
            terrain,
            initial,
            blocks: BTreeMap::new(),
            writes: Vec::new(),
            marks: Vec::new(),
            ticks: Vec::new(),
            biome,
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

    fn check(&self, expected: &Value) {
        let label = format!(
            "{} seed={} at {}",
            expected["name"], expected["seed"], expected["origin"]
        );
        assert_eq!(
            self.writes.len() as u64,
            expected["write_count"].as_u64().unwrap(),
            "write count {label}"
        );
        assert_eq!(
            digest(self.writes.iter().map(|row| row.as_slice())),
            expected["write_md5"].as_str().unwrap(),
            "ordered writes {label}"
        );
        let prefix: Vec<_> = self.writes.iter().take(32).collect();
        assert_eq!(json!(prefix), expected["write_prefix"], "prefix {label}");
        let changed: Vec<_> = self
            .blocks
            .iter()
            .filter(|(pos, state)| **state != self.initial(**pos))
            .map(|(&(x, y, z), &state)| [x, y, z, state as i32])
            .collect();
        assert_eq!(
            changed.len() as u64,
            expected["changed_blocks"].as_u64().unwrap(),
            "block count {label}"
        );
        assert_eq!(
            digest(changed.iter().map(|row| row.as_slice())),
            expected["blocks_md5"].as_str().unwrap(),
            "blocks {label}"
        );
        assert_eq!(
            json!(self.marks),
            expected["marks"],
            "postprocessing {label}"
        );
        assert_eq!(json!(self.ticks), expected["ticks"], "ticks {label}");
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
        panic!("feature must retain its native write flags")
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, _pos: Pos) -> u32 {
        self.biome
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
        !self.terrain.deny_origin
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        let accepted = (MIN_Y..=MAX_Y).contains(&pos.1)
            && self.terrain.write_policy != "reject"
            && (self.terrain.write_policy != "checker" || (pos.0 + pos.2) & 1 == 0);
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
        if (MIN_Y..=MAX_Y).contains(&pos.1) {
            self.marks.push([pos.0, pos.1, pos.2]);
        }
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        let [x, y, z] = request.block_pos;
        let (value, fluid) = match request.target {
            TickTarget::Block(id) => (id, 0),
            TickTarget::Fluid(id) => (id, 1),
        };
        self.ticks
            .push([x, y, z, value as i32, request.delay, fluid]);
        true
    }
}

struct Environment(i32);
impl FeatureEnvironment for Environment {
    fn raw_brightness(&self, _world: &dyn FeatureWorld, _pos: Pos) -> FeatureResult<i32> {
        Ok(self.0)
    }
}

#[derive(Default)]
struct Terminal {
    calls: Vec<[i32; 4]>,
    gaussians: CaveRandomState,
}
impl ConfiguredFeatureDispatcher for Terminal {
    fn next_gaussian(&mut self, random: &mut WorldgenRandom) -> FeatureResult<f64> {
        Ok(self.gaussians.next_gaussian(random))
    }
    fn place_configured(
        &mut self,
        _config: &Value,
        _name: Option<&str>,
        world: &mut dyn FeatureWorld,
        random: &mut WorldgenRandom,
        pos: Pos,
        _environment: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        let choice = random.next_int(7) as i32;
        for _ in 0..choice {
            random.next_float();
        }
        self.calls.push([pos.0, pos.1, pos.2, choice]);
        world.set_feature_block(
            pos,
            catalog().default_state(if choice % 2 == 0 {
                "stone"
            } else {
                "gold_block"
            })?,
            2,
        );
        Ok(choice % 2 == 0)
    }
}

fn digest<'a>(rows: impl Iterator<Item = &'a [i32]>) -> String {
    let mut md = md5::Context::new();
    for row in rows {
        for value in row {
            md.consume(value.to_le_bytes());
        }
    }
    format!("{:x}", md.compute())
}

fn origin(value: &Value) -> Pos {
    (
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
        value[2].as_i64().unwrap() as i32,
    )
}

#[test]
fn every_pinned_placed_definition_compiles() {
    let catalog = block_predicate::catalog();
    for (name, value) in catalog.documents["placed_feature"].as_object().unwrap() {
        placement::PlacementProgram::parse(value).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn every_pinned_simple_block_provider_compiles() {
    let catalog = block_predicate::catalog();
    for (name, value) in catalog.documents["configured_feature"].as_object().unwrap() {
        if value["type"] == "minecraft:simple_block" {
            base_features::StateProvider::parse(&value["config"]["to_place"])
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}

#[test]
fn native_overworld_modifier_streams_and_boundaries() {
    let fixture: Value = serde_json::from_str(PLACEMENT).unwrap();
    for sample in fixture["samples"].as_array().unwrap() {
        let name = sample["name"].as_str().unwrap();
        let config = fixture["custom_placed"]
            .get(name)
            .unwrap_or_else(|| catalog().placed(name).unwrap());
        let mut world = World::new(&sample["terrain"]);
        let mut random = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        let environment = Environment(world.terrain.light);
        let mut terminal = Terminal::default();
        let result = placement::place(
            config,
            Some(name),
            &mut world,
            &mut random,
            origin(&sample["origin"]),
            &environment,
            &mut terminal,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            result,
            sample["result"].as_bool().unwrap(),
            "result {sample}"
        );
        assert_eq!(
            terminal.calls.len() as u64,
            sample["calls"].as_u64().unwrap(),
            "call count {sample}"
        );
        assert_eq!(
            digest(terminal.calls.iter().map(|row| row.as_slice())),
            sample["calls_md5"].as_str().unwrap(),
            "call order {sample}"
        );
        assert_eq!(
            json!(terminal.calls.iter().take(32).collect::<Vec<_>>()),
            sample["call_prefix"],
            "calls {sample}"
        );
        world.check(sample);
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "RNG {sample}"
        );
    }
}

#[test]
fn native_configured_features_and_actual_plant_patches() {
    let fixture: Value = serde_json::from_str(BASE).unwrap();
    let mut unsupported = Vec::new();
    for sample in fixture["samples"].as_array().unwrap() {
        let name = sample["name"].as_str().unwrap();
        let mut world = World::new(&sample["terrain"]);
        let environment = Environment(world.terrain.light);
        let mut random = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        let mut features = base_features::BaseFeatures::default();
        let result = if sample["placed"] == true {
            placement::place_named(
                name,
                &mut world,
                &mut random,
                origin(&sample["origin"]),
                &environment,
                &mut features,
            )
        } else {
            let config = fixture["custom_configured"]
                .get(name)
                .unwrap_or_else(|| catalog().configured(name).unwrap());
            features.place_configured(
                config,
                Some(name),
                &mut world,
                &mut random,
                origin(&sample["origin"]),
                &environment,
            )
        };
        let result = match result {
            Ok(value) => value,
            Err(error) => {
                unsupported.push(format!("{name}: {error}"));
                continue;
            }
        };
        assert_eq!(
            result,
            sample["result"].as_bool().unwrap(),
            "result {sample}"
        );
        world.check(sample);
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "RNG {sample}"
        );
    }
    assert!(
        unsupported.is_empty(),
        "native cases not executed: {unsupported:#?}"
    );
}

#[test]
fn native_survival_soils_light_fluids_and_upper_halves() {
    let fixture: Value = serde_json::from_str(BASE).unwrap();
    let mut errors = Vec::new();
    for sample in fixture["survival"].as_array().unwrap() {
        let state = catalog().state(&sample["state"]).unwrap();
        assert_eq!(state as u64, sample["state_id"].as_u64().unwrap());
        let world = World::new(&sample["terrain"]);
        match block_predicate::would_survive(
            &world,
            state,
            origin(&sample["origin"]),
            &Environment(world.terrain.light),
        ) {
            Ok(result) => assert_eq!(
                result,
                sample["result"].as_bool().unwrap(),
                "survival {sample}"
            ),
            Err(error) => errors.push(format!("{}: {error}", sample["state"])),
        }
    }
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn native_state_providers_and_independent_legacy_noise() {
    let fixture: Value = serde_json::from_str(BASE).unwrap();
    for sample in fixture["providers"].as_array().unwrap() {
        let provider = base_features::StateProvider::parse(&sample["provider"]).unwrap();
        let world = World::new(&sample["terrain"]);
        let mut random = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        let pos = origin(&sample["origin"]);
        let actual = provider
            .sample_optional(&world, &mut random, pos, &Environment(world.terrain.light))
            .unwrap();
        assert_eq!(
            actual,
            sample["state"].as_u64().map(|v| v as u32),
            "provider {sample}"
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "provider RNG {sample}"
        );
        if let Some(expected) = sample["noise_bits"].as_str() {
            use base_features::StateProvider::{Dual, Noise, Threshold};
            let (noise, scale) = match provider {
                Noise(n, s, _) | Threshold(n, s, ..) | Dual(n, s, ..) => (n, s),
                _ => unreachable!(),
            };
            let value = noise.value(
                pos.0 as f64 * scale as f64,
                pos.1 as f64 * scale as f64,
                pos.2 as f64 * scale as f64,
            );
            assert_eq!(
                value.to_bits(),
                expected.parse::<u64>().unwrap(),
                "noise {sample}"
            );
        }
    }
}

#[test]
fn missing_operations_are_explicit_and_do_not_restart_partial_streams() {
    let fixture: Value = serde_json::from_str(BASE).unwrap();
    let terrain = &fixture["samples"][0]["terrain"];
    let mut world = World::new(terrain);
    let environment = Environment(world.terrain.light);
    let mut random = WorldgenRandom::new(17);
    let mut terminal = Terminal::default();
    let program = json!({"feature":{"type":"missing:kernel","config":{}},"placement":[{"type":"minecraft:count","count":3}]});
    let mut attempts = 0;
    let mut dispatcher = |config: &Value,
                          name: Option<&str>,
                          world: &mut dyn FeatureWorld,
                          random: &mut WorldgenRandom,
                          pos: Pos,
                          environment: &dyn FeatureEnvironment|
     -> FeatureResult<bool> {
        attempts += 1;
        if attempts == 2 {
            return Err(FeatureError::Unsupported("second attempt".into()));
        }
        terminal.place_configured(config, name, world, random, pos, environment)
    };
    let result = placement::place(
        &program,
        Some("probe"),
        &mut world,
        &mut random,
        (8, 64, 8),
        &environment,
        &mut dispatcher,
    );
    assert!(matches!(result, Err(FeatureError::Unsupported(_))));
    assert_eq!(attempts, 2);
    assert_eq!(world.writes.len(), 1);
    let mut expected = WorldgenRandom::new(17);
    let count = expected.next_int(7);
    for _ in 0..count {
        expected.next_float();
    }
    assert_eq!(random.next_long(), expected.next_long());
    let unsupported = json!({"type":"minecraft:carving_mask"});
    let bad_program =
        json!({"feature":{"type":"minecraft:simple_block","config":{}}, "placement":[unsupported]});
    assert!(matches!(
        placement::PlacementProgram::parse(&bad_program),
        Err(FeatureError::Unsupported(_))
    ));
    assert!(matches!(
        block_predicate::BlockPredicate::parse(&json!({"type":"unknown:predicate"})),
        Err(FeatureError::Unsupported(_))
    ));
}
