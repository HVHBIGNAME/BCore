//! Path inclusion keeps this independently testable before the core export is wired.
pub use bcore_worldgen::{feature_world, simplex, MAX_Y, MIN_Y};
#[path = "../src/sculk.rs"]
mod sculk;

use bcore_worldgen::block_predicate::{FeatureEnvironment, RegistryEnvironment};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::placement;
use bcore_worldgen::tick_request::TickRequest;
use feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use sculk::{Config, GeneratedBlockEntity, IntProvider};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use simplex::WorldgenRandom;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/sculk_26_1.json")).unwrap())
}
fn states() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/sculk_states_26_1.json")).unwrap()
    })
}
fn id(name: &str) -> u32 {
    states()["blocks"][format!("minecraft:{name}")][2]
        .as_u64()
        .unwrap() as u32
}
fn position(value: &Value) -> Pos {
    serde_json::from_value(value.clone()).unwrap()
}

struct Grid {
    terrain: String,
    origin: Pos,
    radius: i32,
    initial_blocks: BTreeMap<Pos, u32>,
    blocks: BTreeMap<Pos, u32>,
    trace: Vec<[i32; 6]>,
    marks: Vec<Pos>,
    ticks: Vec<TickRequest>,
    entities: BTreeMap<Pos, GeneratedBlockEntity>,
    missing: Option<Pos>,
    biome: Option<u32>,
    ids: BTreeMap<String, u32>,
}

impl Grid {
    fn new(terrain: &str, origin: Pos, radius: i32) -> Self {
        let mut ids: BTreeMap<_, _> = [
            "air",
            "void_air",
            "cave_air",
            "stone",
            "deepslate",
            "water",
            "lava",
            "bedrock",
            "sculk",
            "tuff",
            "calcite",
            "gravel",
            "dirt",
            "clay",
            "glass",
            "oak_slab",
            "fire",
            "short_grass",
            "deepslate_bricks",
            "deepslate_tiles",
            "cobbled_deepslate",
            "cracked_deepslate_bricks",
            "cracked_deepslate_tiles",
            "polished_deepslate",
            "pointed_dripstone",
            "bamboo",
            "scaffolding",
            "powder_snow",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), id(name)))
        .collect();
        for name in ["flowing_water", "vein_down"] {
            ids.insert(
                name.into(),
                states()["test_states"][name].as_u64().unwrap() as u32,
            );
        }
        Self {
            terrain: terrain.into(),
            origin,
            radius,
            initial_blocks: BTreeMap::new(),
            blocks: BTreeMap::new(),
            trace: Vec::new(),
            marks: Vec::new(),
            ticks: Vec::new(),
            entities: BTreeMap::new(),
            missing: None,
            biome: None,
            ids,
        }
    }

    fn initial(&self, (x, y, z): Pos) -> u32 {
        let id = |name: &str| self.ids[name];
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return id("void_air");
        }
        if let Some(&state) = self.initial_blocks.get(&(x, y, z)) {
            return state;
        }
        let (dx, dy, dz) = (x - self.origin.0, y - self.origin.1, z - self.origin.2);
        match self.terrain.as_str() {
            "layers" => {
                return id(if y.rem_euclid(8) < 2 {
                    if y < 0 {
                        "deepslate"
                    } else {
                        "stone"
                    }
                } else {
                    "cave_air"
                })
            }
            "air" => return id("air"),
            "solid" => return id("deepslate"),
            "sculk_floor" => return id(if dy < 0 { "sculk" } else { "air" }),
            "sculk_water" => {
                return id(if dy < 0 {
                    "sculk"
                } else if dy < 4 {
                    "water"
                } else {
                    "air"
                })
            }
            "origin_sculk" if (dx, dy, dz) == (0, 0, 0) => return id("sculk"),
            "origin_vein" if (dx, dy, dz) == (0, 0, 0) => return id("vein_down"),
            "ceiling" => {
                return id(if dy > 0 {
                    "deepslate"
                } else if (y >> 4) * 16 + 15 <= self.origin.1 {
                    "air"
                } else {
                    "cave_air"
                })
            }
            "walls" => {
                return id(if dx.abs() >= 3 || dz.abs() >= 4 {
                    "deepslate"
                } else {
                    "cave_air"
                })
            }
            "search" => {
                return id(if (dx, dy, dz) == (2, 0, 0) {
                    "stone"
                } else {
                    "air"
                })
            }
            _ => {}
        }
        if matches!(self.terrain.as_str(), "cave" | "mixed")
            && (dy >= 6 || (dx - 4).abs() <= 1 && (dz - 3).abs() <= 1)
        {
            return id("deepslate");
        }
        if dy < 0 {
            if self.terrain == "bedrock" {
                return id("bedrock");
            }
            if self.terrain == "mixed" && dy == -1 {
                let blocks = [
                    "stone",
                    "deepslate",
                    "tuff",
                    "calcite",
                    "gravel",
                    "dirt",
                    "clay",
                    "bedrock",
                    "glass",
                    "oak_slab",
                    "fire",
                    "short_grass",
                ];
                return id(blocks[(x * 3 + z * 5).rem_euclid(blocks.len() as i32) as usize]);
            }
            if self.terrain == "city" {
                let blocks = [
                    "deepslate_bricks",
                    "deepslate_tiles",
                    "cobbled_deepslate",
                    "cracked_deepslate_bricks",
                    "cracked_deepslate_tiles",
                    "polished_deepslate",
                ];
                return id(blocks[(x + z * 3).rem_euclid(6) as usize]);
            }
            if self.terrain == "dynamic" && dy == -1 {
                let blocks = ["pointed_dripstone", "bamboo", "scaffolding", "powder_snow"];
                return if dx == 0 && dz == 0 {
                    id("deepslate")
                } else {
                    id(blocks[(x + z).rem_euclid(4) as usize])
                };
            }
            return id(if y < 0 { "deepslate" } else { "stone" });
        }
        if dy < 4 && matches!(self.terrain.as_str(), "water" | "lava" | "flowing_water") {
            return id(&self.terrain);
        }
        id(if matches!(self.terrain.as_str(), "cave" | "mixed") {
            "cave_air"
        } else {
            "air"
        })
    }

    fn writes(&self) -> Vec<[i32; 4]> {
        self.blocks
            .iter()
            .filter(|(pos, state)| **state != self.initial(**pos))
            .map(|(&(x, y, z), &state)| [x, y, z, state as i32])
            .collect()
    }

    fn canonical_marks(&self) -> Vec<[i32; 4]> {
        let mut marks = BTreeMap::<Pos, i32>::new();
        for &pos in &self.marks {
            *marks.entry(pos).or_default() += 1;
        }
        marks
            .into_iter()
            .map(|((x, y, z), count)| [x, y, z, count])
            .collect()
    }

    fn section_marks(&self) -> Vec<[i32; 3]> {
        let mut sections = BTreeMap::<Pos, Vec<[i32; 3]>>::new();
        for &(x, y, z) in &self.marks {
            sections
                .entry((x >> 4, z >> 4, y >> 4))
                .or_default()
                .push([x, y, z]);
        }
        sections.into_values().flatten().collect()
    }

    fn compare(&self, sample: &Value) {
        let name = sample["name"].as_str().unwrap();
        assert_eq!(
            json!(self.writes()),
            sample["writes"],
            "{name}: canonical writes"
        );
        assert_eq!(
            self.trace.len(),
            sample["write_calls"].as_u64().unwrap() as usize,
            "{name}: write count"
        );
        assert_eq!(
            hash(&self.trace),
            sample["write_trace_sha256"].as_str().unwrap(),
            "{name}: ordered writes/flags/returns"
        );
        assert_eq!(
            json!(self.canonical_marks()),
            sample["marks"],
            "{name}: mark multiset"
        );
        assert_eq!(
            self.marks.len(),
            sample["mark_calls"].as_u64().unwrap() as usize,
            "{name}: mark count"
        );
        assert_eq!(
            hash(&self.section_marks()),
            sample["section_marks_sha256"].as_str().unwrap(),
            "{name}: section-local mark order"
        );
        assert_eq!(
            json!(self.ticks),
            sample["tick_requests"],
            "{name}: raw ticks"
        );
        assert_eq!(
            sample["native_ticks"],
            json!([]),
            "{name}: deferred native proto ticks"
        );
        let mut entities = Vec::new();
        for (&pos, entity) in &self.entities {
            let state = self.get_block(pos).unwrap();
            entities.push(
                json!({"pos": pos, "state": state, "type_id": entity.type_id,
                "nbt": entity.full_data, "update": entity.update_data,
                "pending_nbt": {"id": "DUMMY", "x": pos.0, "y": pos.1, "z": pos.2}}),
            );
        }
        assert_eq!(
            json!(entities),
            sample["block_entities"],
            "{name}: generated block entities"
        );
    }
}

impl OreWorld for Grid {
    fn ocean_floor_wg(&self, _: i32, _: i32) -> i32 {
        panic!("unexpected sculk height query")
    }
    fn get_block(&self, pos: Pos) -> Option<u32> {
        (self.missing != Some(pos)).then(|| {
            self.blocks
                .get(&pos)
                .copied()
                .unwrap_or_else(|| self.initial(pos))
        })
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("sculk write bypassed flags")
    }
}

impl FeatureWorld for Grid {
    fn feature_biome(&self, _: Pos) -> u32 {
        self.biome.expect("unexpected configured sculk biome query")
    }
    fn feature_height(&self, _: FeatureHeightmap, _: i32, _: i32) -> i32 {
        panic!("unexpected sculk height query")
    }
    fn can_write_feature(&self, (x, _, z): Pos) -> bool {
        ((x >> 4) - (self.origin.0 >> 4))
            .abs()
            .max(((z >> 4) - (self.origin.2 >> 4)).abs())
            <= self.radius
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        let accepted = self.can_write_feature(pos);
        self.trace.push([
            pos.0,
            pos.1,
            pos.2,
            state as i32,
            flags,
            i32::from(accepted),
        ]);
        if accepted {
            if (MIN_Y..=MAX_Y).contains(&pos.1) {
                self.blocks.insert(pos, state);
                if let Some(entity) = sculk::generated_block_entity(state, pos).unwrap() {
                    self.entities.insert(pos, entity);
                } else {
                    self.entities.remove(&pos);
                }
            }
            if flags & 16 == 0 {
                let offset = &states()["postprocess_offsets"][state.to_string()];
                if offset.is_array() {
                    let delta = position(offset);
                    self.mark_feature_postprocessing((
                        pos.0 + delta.0,
                        pos.1 + delta.1,
                        pos.2 + delta.2,
                    ));
                }
            }
        }
        accepted
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        if (MIN_Y..=MAX_Y).contains(&pos.1) {
            self.marks.push(pos);
        }
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        self.ticks.push(request);
        true
    }
}

fn hash<const N: usize>(rows: &[[i32; N]]) -> String {
    let mut hash = Sha256::new();
    for row in rows {
        for value in row {
            hash.update(value.to_le_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}

#[test]
fn registered_features_match_native_writes_rng_marks_and_entities() {
    for sample in fixture()["samples"].as_array().unwrap() {
        let origin = position(&sample["origin"]);
        let mut world = Grid::new(
            sample["terrain"].as_str().unwrap(),
            origin,
            sample["write_radius"].as_i64().unwrap() as i32,
        );
        let mut random = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        for _ in 0..sample["advance"].as_u64().unwrap() {
            random.next_long();
        }
        let mut results = Vec::new();
        for _ in sample["placed"].as_array().unwrap() {
            let result = if let Some(config) = sample.get("config") {
                sculk::place(
                    &mut world,
                    &mut random,
                    origin,
                    &Config::from_json("minecraft:sculk_patch", config).unwrap(),
                )
            } else {
                sculk::place_named(
                    &mut world,
                    &mut random,
                    origin,
                    sample["kind"].as_str().unwrap(),
                )
            };
            results.push(result.unwrap_or_else(|error| panic!("{}: {error}", sample["name"])));
        }
        assert_eq!(
            json!(results),
            sample["placed"],
            "{}: return",
            sample["name"]
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{}: RNG continuation",
            sample["name"]
        );
        world.compare(sample);
    }
}

#[test]
fn native_capture_provenance_and_coverage() {
    let mut hash = Sha256::new();
    for source in [
        include_bytes!("../../../scripts/TreeReference.java").as_slice(),
        include_bytes!("../../../scripts/NativeEntityLevel.java"),
        include_bytes!("../../../scripts/NativeWorldgenRegistries.java"),
        include_bytes!("../../../scripts/TreeEffectReference.java"),
        include_bytes!("../../../scripts/SculkReference.java"),
    ] {
        hash.update(source);
    }
    let digest = format!("{:x}", hash.finalize());
    for data in [fixture(), states()] {
        assert_eq!(data["minecraft"], "26.1");
        assert_eq!(
            data["jar_sha256"],
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        assert_eq!(data["probe_sha256"], digest);
    }
    let samples = fixture()["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 85);
    assert_eq!(fixture()["kernels"].as_array().unwrap().len(), 13);
    assert_eq!(fixture()["streams"].as_array().unwrap().len(), 6);
    assert_eq!(
        samples
            .iter()
            .map(|sample| sample["name"].as_str().unwrap())
            .collect::<BTreeSet<_>>()
            .len(),
        samples.len()
    );
    let generated_types: BTreeSet<_> = samples
        .iter()
        .flat_map(|sample| sample["block_entities"].as_array().unwrap())
        .map(|entity| entity["type_id"].as_u64().unwrap())
        .collect();
    assert_eq!(generated_types, BTreeSet::from([35, 37, 38]));
    assert!(samples
        .iter()
        .any(|sample| sample["writes"].as_array().unwrap().len() > 300));
}

#[test]
fn worldgen_cursor_updates_match_native_snapshots() {
    for sample in fixture()["kernels"].as_array().unwrap() {
        let origin = position(&sample["origin"]);
        let mut world = Grid::new(sample["terrain"].as_str().unwrap(), origin, 1);
        for block in sample["initial_blocks"].as_array().unwrap() {
            let [x, y, z, state]: [i32; 4] = serde_json::from_value(block.clone()).unwrap();
            world.initial_blocks.insert((x, y, z), state as u32);
        }
        let mut random = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        let mut spreader = sculk::WorldgenSpreader::default();
        for addition in sample["additions"].as_array().unwrap() {
            let [x, y, z, charge]: [i32; 4] = serde_json::from_value(addition.clone()).unwrap();
            spreader.add_cursors((x, y, z), charge);
        }
        let mut snapshots = vec![json!(spreader.cursors())];
        for (index, spread) in sample["updates"].as_array().unwrap().iter().enumerate() {
            for action in sample["actions"].as_array().unwrap() {
                if action["before_update"].as_u64().unwrap() as usize == index {
                    world.set_feature_block(
                        position(&action["pos"]),
                        action["state"].as_u64().unwrap() as u32,
                        3,
                    );
                }
            }
            spreader
                .update(&mut world, &mut random, origin, spread.as_bool().unwrap())
                .unwrap_or_else(|error| panic!("{} update {index}: {error}", sample["name"]));
            snapshots.push(json!(spreader.cursors()));
        }
        assert_eq!(
            json!(snapshots),
            sample["cursors"],
            "{}: cursor snapshots",
            sample["name"]
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{}: kernel RNG",
            sample["name"]
        );
        world.compare(sample);
    }
}

#[test]
fn complete_placed_streams_preserve_child_rng_interleaving() {
    for sample in fixture()["streams"].as_array().unwrap() {
        let origin = position(&sample["origin"]);
        let mut world = Grid::new(sample["terrain"].as_str().unwrap(), origin, 1);
        world.biome = Some(
            states()["biome_ids"][sample["biome"].as_str().unwrap()]
                .as_u64()
                .unwrap() as u32,
        );
        let mut random = WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        let mut origins = Vec::new();
        let mut dispatcher = |document: &Value,
                              _name: Option<&str>,
                              world: &mut dyn FeatureWorld,
                              random: &mut WorldgenRandom,
                              origin: Pos,
                              _environment: &dyn FeatureEnvironment| {
            origins.push([origin.0, origin.1, origin.2]);
            sculk::place(
                world,
                random,
                origin,
                &Config::from_json(document["type"].as_str().unwrap(), &document["config"])?,
            )
        };
        let mut results = Vec::new();
        for root in sample["roots"].as_array().unwrap() {
            let name = format!("minecraft:{}", root.as_str().unwrap());
            results.push(
                placement::place_named(
                    &name,
                    &mut world,
                    &mut random,
                    position(&sample["stream_origin"]),
                    &RegistryEnvironment,
                    &mut dispatcher,
                )
                .unwrap_or_else(|error| panic!("{}: {error}", sample["name"])),
            );
        }
        assert_eq!(
            json!(results),
            sample["placed"],
            "{}: placed stream return",
            sample["name"]
        );
        assert_eq!(
            json!(origins),
            sample["feature_origins"],
            "{}: lazy feature origins",
            sample["name"]
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{}: complete stream RNG",
            sample["name"]
        );
        world.compare(sample);
    }
}

#[test]
fn generated_defaults_preserve_native_nbt_types() {
    for template in states()["block_entities"].as_array().unwrap() {
        let state = template["state"].as_u64().unwrap() as u32;
        let entity = sculk::generated_block_entity(state, (0, 0, 0))
            .unwrap()
            .unwrap();
        assert_eq!(entity.type_id, template["type_id"].as_u64().unwrap() as u32);
        assert_eq!(entity.full_data, template["nbt"]);
        assert_eq!(entity.typed_data, template["typed_nbt"]);
        assert_eq!(entity.update_data, template["update"]);
        let moved = sculk::generated_block_entity(state, (-17, -64, 16))
            .unwrap()
            .unwrap();
        assert_eq!(moved.full_data["x"], -17);
        assert_eq!(moved.typed_data[1]["y"], json!([3, -64]));
        if entity.type_id != 37 {
            assert_eq!(
                moved.typed_data[1]["listener"][1]["selector"][1]["tick"],
                json!([4, -1])
            );
        }
    }
    assert!(sculk::generated_block_entity(id("sculk_vein"), (0, 0, 0))
        .unwrap()
        .is_none());
}

#[test]
fn invalid_configuration_and_missing_reads_are_explicit() {
    let origin = (8, -32, 8);
    let mut world = Grid::new("flat", origin, 1);
    let mut random = WorldgenRandom::new(17);
    let mut control = WorldgenRandom::new(17);
    let Config::Patch(mut config) = Config::named("sculk_patch_deep_dark").unwrap() else {
        unreachable!()
    };
    config.extra_rare_growths = IntProvider::Uniform {
        min_inclusive: 3,
        max_inclusive: 1,
    };
    assert!(matches!(
        sculk::place(&mut world, &mut random, origin, &Config::Patch(config)),
        Err(FeatureError::InvalidConfig(_))
    ));
    assert!(world.trace.is_empty());
    world.missing = Some((8, -33, 8));
    assert!(matches!(
        sculk::place_named(&mut world, &mut random, origin, "sculk_patch_deep_dark"),
        Err(FeatureError::MissingData(_))
    ));
    assert!(world.trace.is_empty());
    assert_eq!(random.next_long(), control.next_long());
    assert!(matches!(
        Config::named("mod:sculk_patch_deep_dark"),
        Err(FeatureError::Unsupported(_))
    ));
}
