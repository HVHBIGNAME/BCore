use super::*;
use crate::dripstone::CaveRandomState;
use crate::feature_world::{FeatureError, FeatureHeightmap};
use crate::generation::FeatureBlockEntity;
use crate::ore::OreWorld;
use crate::placement::ConfiguredFeatureDispatcher;
use crate::structure::template::{Nbt, TemplateBlockEntity, TemplateEffect};
use crate::tick_request::{TickRequest, TickTarget};
use crate::{block, MAX_Y, MIN_Y};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::OnceLock;

fn native() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        let bytes = include_bytes!("../../data/template_features_26_2.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            "d2cb8b52dfd0882595d5041965a3d074cae1e00afc6534d52c1c28a5034f0875"
        );
        let data: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(data["minecraft"], "26.2");
        assert_eq!(data["protocol"], 776);
        assert_eq!(data["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
        assert_eq!(data["gameplay_ticks"], 0);
        data
    })
}

fn native_effects() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        let bytes = include_bytes!("../../data/template_feature_effects_26_2.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            "eb463eb97028370258f4fc1c7c26ff715cb6a40a89dca7fc35b6cb6c64c14531"
        );
        let data: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(data["schema"], "bcore.template-feature-effects.v1");
        assert_eq!(
            data["reference_sha256"],
            "d2cb8b52dfd0882595d5041965a3d074cae1e00afc6534d52c1c28a5034f0875"
        );
        assert_eq!(data["minecraft"], "26.2");
        assert_eq!(data["protocol"], 776);
        assert_eq!(data["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
        assert_eq!(data["gameplay_ticks"], 0);
        assert_eq!(
            data["manager_source"],
            "initialized-server-structure-manager"
        );
        assert_eq!(data["samples"].as_array().unwrap().len(), 153);
        data
    })
}

struct World {
    terrain: String,
    fill: u32,
    writes: BTreeMap<Pos, u32>,
    calls: Vec<[i32; 6]>,
    ticks: Vec<[i32; 6]>,
    implicit_marks: Vec<[i32; 3]>,
    explicit_marks: BTreeMap<(i32, i32, i32), Vec<[i32; 3]>>,
    neighbours: Vec<(String, [i32; 3], u32)>,
    entities: BTreeMap<Pos, FeatureBlockEntity>,
}

impl World {
    fn from_sample(sample: &Value) -> Self {
        Self {
            terrain: sample["terrain"].as_str().unwrap().into(),
            fill: sample["fill"].as_u64().unwrap() as u32,
            writes: BTreeMap::new(),
            calls: Vec::new(),
            ticks: Vec::new(),
            implicit_marks: Vec::new(),
            explicit_marks: BTreeMap::new(),
            neighbours: Vec::new(),
            entities: BTreeMap::new(),
        }
    }

    fn height(&self, x: i32, z: i32) -> i32 {
        if self.terrain == "slope" {
            64 + (x + 2 * z).rem_euclid(5)
        } else {
            65
        }
    }

    fn read(&self, pos: Pos) -> u32 {
        if !(MIN_Y..=MAX_Y).contains(&pos.1) {
            return block::VOID_AIR;
        }
        self.writes.get(&pos).copied().unwrap_or_else(|| {
            if self.terrain == "slope" {
                if pos.1 < self.height(pos.0, pos.2) {
                    block::STONE
                } else {
                    block::AIR
                }
            } else {
                self.fill
            }
        })
    }

    fn tick(request: TickRequest) -> [i32; 6] {
        let [x, y, z] = request.block_pos;
        let (target, fluid) = match request.target {
            TickTarget::Block(id) => (id, 0),
            TickTarget::Fluid(id) => (id, 1),
        };
        [x, y, z, target as i32, request.delay, fluid]
    }
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.height(x, z)
    }
    fn get_block(&self, pos: Pos) -> Option<u32> {
        Some(self.read(pos))
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.set_feature_block(pos, state, 3)
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, _pos: Pos) -> u32 {
        crate::biome_id::PLAINS
    }
    fn feature_height(&self, _kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.height(x, z)
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        (MIN_Y..=MAX_Y).contains(&pos.1)
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        let accepted = self.terrain != "reject_writes" && self.can_write_feature(pos);
        self.calls.push([
            pos.0,
            pos.1,
            pos.2,
            state as i32,
            flags,
            i32::from(accepted),
        ]);
        if accepted {
            let assets = StructureAssets::bundled();
            let previous = self.read(pos);
            self.writes.insert(pos, state);
            if assets.blocks.flags(state).unwrap() & 8 == 0 {
                self.entities.remove(&pos);
            } else if !self.entities.contains_key(&pos)
                || assets.blocks.state(previous).unwrap().name
                    != assets.blocks.state(state).unwrap().name
            {
                let entity = TemplateBlockEntity::from_load(
                    &assets.blocks,
                    state,
                    pos,
                    Nbt::empty_compound(),
                )
                .unwrap();
                self.entities
                    .insert(pos, FeatureBlockEntity::from_template(&entity).unwrap());
            }
            if flags & 16 == 0 {
                if let Some(mark) = crate::region::postprocess_on_write(state, pos) {
                    self.implicit_marks.push([mark.0, mark.1, mark.2]);
                }
            }
        }
        accepted
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        if (MIN_Y..=MAX_Y).contains(&pos.1) {
            self.explicit_marks
                .entry((pos.0 >> 4, pos.2 >> 4, pos.1 >> 4))
                .or_default()
                .push([pos.0, pos.1, pos.2]);
        }
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        self.ticks.push(Self::tick(request));
        true
    }
    fn has_feature_tick(&self, pos: Pos, target: TickTarget) -> FeatureResult<bool> {
        let row = Self::tick(TickRequest {
            block_pos: [pos.0, pos.1, pos.2],
            target,
            delay: 0,
        });
        Ok(self
            .ticks
            .iter()
            .any(|tick| tick[..4] == row[..4] && tick[5] == row[5]))
    }
    fn apply_feature_template_effect(&mut self, effect: TemplateEffect) -> FeatureResult<()> {
        match effect {
            TemplateEffect::ClearBlockEntity(pos) => {
                self.entities.remove(&pos);
            }
            TemplateEffect::BlockEntity(entity) => {
                self.entities
                    .insert(entity.pos, FeatureBlockEntity::from_template(&entity)?);
            }
            TemplateEffect::Entity(_) => {
                return Err(FeatureError::Unsupported(
                    "entity-bearing native spring template".into(),
                ))
            }
        }
        Ok(())
    }
    fn update_feature_neighbours(&mut self, pos: Pos, state: u32) -> FeatureResult<()> {
        let registry = &StructureAssets::bundled().blocks;
        let default = registry.default_state(&registry.state(state)?.name)?;
        self.neighbours
            .push(("updateNeighborsAt".into(), [pos.0, pos.1, pos.2], default));
        Ok(())
    }
    fn reset_feature_potent_sulfur(&mut self, pos: Pos) -> FeatureResult<bool> {
        let state = self.read(pos);
        let Some(entity) = self.entities.get_mut(&pos) else {
            return Ok(false);
        };
        *entity = entity.reset_potent_sulfur(state, pos)?;
        Ok(true)
    }
}

#[test]
fn native_template_selection_shapes_sulfur_sequences_and_rng() {
    let samples = native()["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 153);
    for (index, sample) in samples.iter().enumerate() {
        let captured_effects = &native_effects()["samples"][index];
        for key in ["name", "seed", "terrain", "origin"] {
            assert_eq!(
                captured_effects[key], sample[key],
                "effect capture identity {index}/{key}"
            );
        }
        let mut world = World::from_sample(sample);
        let seed = sample["seed"].as_i64().unwrap();
        let mut random = WorldgenRandom::new(seed);
        let mut cave = CaveRandomState::default();
        let effects = Rc::new(RefCell::new(tree_world::TreeEffects::default()));
        let mut dispatcher =
            super::super::placed::dispatcher(&mut cave, effects.clone(), 846692123413862008);
        let [x, y, z]: [i32; 3] = serde_json::from_value(sample["origin"].clone()).unwrap();
        let label = format!(
            "sample {index}: {} seed {seed} {}",
            sample["name"], sample["terrain"]
        );
        let placed = dispatcher
            .place_configured(
                &sample["document"],
                None,
                &mut world,
                &mut random,
                (x, y, z),
                &super::super::placed::GenerationEnvironment,
            )
            .unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(
            placed,
            sample["placed"].as_bool().unwrap(),
            "result {label}"
        );
        let mut native_source = crate::simplex::Xoroshiro128::new(seed);
        for _ in 0..sample["rng_count"].as_u64().unwrap() {
            native_source.next_long();
        }
        let mut source = random.source.clone();
        for _ in 0..2 {
            assert_eq!(
                source.next_long(),
                native_source.next_long(),
                "native RNG draw count {label}"
            );
        }
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "RNG {label}"
        );
        assert_rows(
            &world.calls,
            &sample["write_calls"],
            &format!("write order/flags/acceptance {label}"),
        );
        let states: Vec<_> = world
            .writes
            .iter()
            .map(|(&(x, y, z), &state)| [x, y, z, state as i32])
            .collect();
        assert_rows(&states, &sample["states"], &format!("states {label}"));
        world.ticks.extend(tree_world::captured_ticks(&effects));
        assert_rows(
            &world.ticks,
            &sample["ticks"],
            &format!("ordered ticks {label}"),
        );
        assert_rows(
            &world.implicit_marks,
            &sample["implicit_marks"],
            &format!("implicit marks {label}"),
        );
        let explicit: Vec<_> = world.explicit_marks.values().flatten().copied().collect();
        assert_rows(
            &explicit,
            &captured_effects["explicit_section_marks"],
            &format!("explicit section marks {label}"),
        );
        assert_rows(
            &world.neighbours,
            &sample["neighbour_updates"],
            &format!("neighbour notifications {label}"),
        );
        assert_eq!(
            world.entities.len(),
            sample["block_entities"].as_array().unwrap().len(),
            "block entity count {label}"
        );
        for entity in sample["block_entities"].as_array().unwrap() {
            let [x, y, z]: [i32; 3] = serde_json::from_value(entity["pos"].clone()).unwrap();
            let observed = &world.entities[&(x, y, z)];
            let full: Nbt = serde_json::from_value(entity["full"]["nbt"].clone()).unwrap();
            let update: Nbt = serde_json::from_value(entity["update"]["nbt"].clone()).unwrap();
            assert_eq!(observed.full_nbt().unwrap(), full, "full NBT {label}");
            assert_eq!(observed.update_nbt().unwrap(), update, "update NBT {label}");
            assert!(
                observed.valid_for(world.read((x, y, z)), (x, y, z)),
                "current BE admission {label}"
            );
        }
    }
}

fn assert_rows<T: serde::Serialize>(actual: &[T], expected: &Value, label: &str) {
    let actual = serde_json::to_value(actual).unwrap();
    let actual = actual.as_array().unwrap();
    let expected = expected.as_array().unwrap();
    let difference = actual
        .iter()
        .zip(expected)
        .position(|(a, b)| a != b)
        .or_else(|| (actual.len() != expected.len()).then_some(actual.len().min(expected.len())));
    if let Some(index) = difference {
        panic!(
            "{label}: first difference {index}: actual {:?}, native {:?}; counts {}/{}",
            actual.get(index),
            expected.get(index),
            actual.len(),
            expected.len()
        );
    }
}

#[test]
fn native_template_codec_defaults_numeric_weights_and_rotation_lists() {
    let cases = native()["admission"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let parsed = Config::parse(&case["document"]["config"]);
        assert_eq!(
            parsed.is_ok(),
            case["admitted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if parsed.is_ok() {
            Config::parse(&case["canonical_document"]["config"]).unwrap();
        }
    }
}

#[test]
fn native_potent_sulfur_load_save_and_update_preserve_numeric_tag_semantics() {
    let cases = native()["potent_sulfur_loads"].as_array().unwrap();
    assert_eq!(cases.len(), 55);
    for case in cases {
        let [x, y, z]: [i32; 3] = serde_json::from_value(case["pos"].clone()).unwrap();
        let state = case["state"].as_u64().unwrap() as u32;
        let load: Nbt = serde_json::from_value(case["load"]["nbt"].clone()).unwrap();
        let expected: Nbt = serde_json::from_value(case["full"]["nbt"].clone()).unwrap();
        let update: Nbt = serde_json::from_value(case["update"]["nbt"].clone()).unwrap();
        let template = TemplateBlockEntity::from_load(
            &StructureAssets::bundled().blocks,
            state,
            (x, y, z),
            load,
        )
        .unwrap();
        let entity = FeatureBlockEntity::from_template(&template).unwrap();
        assert_eq!(entity.type_id, 48);
        assert_eq!(
            entity.full_nbt().unwrap(),
            expected,
            "{} state {state}",
            case["snbt"]
        );
        assert_eq!(entity.update_nbt().unwrap(), update);
        assert!(entity.valid_for(state, (x, y, z)));
        let reset = entity.reset_potent_sulfur(state, (x, y, z)).unwrap();
        assert_eq!(
            reset.full_nbt().unwrap().get("countdown"),
            Some(&Nbt::Int(-1))
        );
        assert!(reset.valid_for(state, (x, y, z)));
    }
}

#[test]
fn manager_feature_template_assets_keep_the_published_identity_and_inventory() {
    let bytes = include_bytes!("../../data/feature_template_assets_26_2.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "ec03080755ce66e15b0f1acdf80278bf2d412638f901e1eb891ba57cecc0ebbf"
    );
    let assets = StructureAssets::bundled();
    let data: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(data["templates"].as_object().unwrap().len(), 10);
    for name in data["templates"].as_object().unwrap().keys() {
        let template = assets.template(name).unwrap();
        assert!(!template.palettes().is_empty());
        assert!(template.entities.is_empty());
    }
    let empty = json!({"templates": []});
    assert!(check_configured(&empty).is_ok());
}
