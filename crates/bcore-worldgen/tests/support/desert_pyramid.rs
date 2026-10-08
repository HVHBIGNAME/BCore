#![allow(dead_code)]
use bcore_core::ChunkPos;
use bcore_worldgen::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::structure::scattered::{ScatteredCatalog, ScatteredEffect, ScatteredWorld};
use bcore_worldgen::structure::template::{BoundingBox, Nbt, TemplateBlockEntity};
use bcore_worldgen::structure::template_pool::StructureAssets;
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/desert_pyramid_26_1.json")).unwrap()
    })
}
pub fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
pub fn pos(v: &Value) -> Pos {
    (n(&v[0]), n(&v[1]), n(&v[2]))
}
pub fn chunk(v: &Value) -> ChunkPos {
    ChunkPos::new(n(&v[0]), n(&v[1]))
}
pub fn nbt(v: &Value) -> Nbt {
    serde_json::from_value(v["nbt"].clone()).unwrap()
}
pub fn bounds(v: &Value) -> BoundingBox {
    BoundingBox {
        min: pos(v),
        max: (n(&v[3]), n(&v[4]), n(&v[5])),
    }
}
pub fn block(name: &str) -> u32 {
    StructureAssets::bundled()
        .blocks
        .default_state(name)
        .unwrap()
}

pub fn terrain(mode: &str, p: Pos) -> u32 {
    let name = match mode {
        "flat" => {
            if p.1 < 64 {
                "stone"
            } else {
                "air"
            }
        }
        "slope" => {
            if p.1 < 60 + (p.0 + 2 * p.2).rem_euclid(9) {
                "stone"
            } else {
                "air"
            }
        }
        "water" => {
            if p.1 < 58 {
                "stone"
            } else if p.1 < 64 {
                "water"
            } else {
                "air"
            }
        }
        "void" => "air",
        other => panic!("unknown native pyramid terrain {other}"),
    };
    block(name)
}

pub struct World {
    pub terrain: String,
    pub source: ChunkPos,
    pub overrides: BTreeMap<Pos, u32>,
    pub denied: BTreeSet<Pos>,
    pub states: BTreeMap<Pos, u32>,
    pub entities: BTreeMap<Pos, TemplateBlockEntity>,
    pub effects: Vec<Value>,
    pub marks: BTreeMap<Pos, usize>,
    pub ticks: Vec<Value>,
    pub heights: RefCell<Vec<Value>>,
    pub suppress_entities: bool,
    observed_loot: BTreeMap<Pos, (String, i64)>,
}

impl World {
    pub fn new(row: &Value) -> Self {
        Self {
            terrain: row["terrain"].as_str().unwrap().into(),
            source: ChunkPos::new(0, 0),
            overrides: row["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (pos(p), n(&p[3]) as u32))
                .collect(),
            denied: row["denied"].as_array().unwrap().iter().map(pos).collect(),
            states: BTreeMap::new(),
            entities: BTreeMap::new(),
            effects: Vec::new(),
            marks: BTreeMap::new(),
            ticks: Vec::new(),
            heights: RefCell::new(Vec::new()),
            suppress_entities: row["suppress_block_entities"] == true,
            observed_loot: BTreeMap::new(),
        }
    }
    pub fn reset(&mut self, source: ChunkPos) {
        self.source = source;
        self.effects.clear();
        self.marks.clear();
        self.ticks.clear();
        self.heights.borrow_mut().clear();
    }
    pub fn at(&self, p: Pos) -> u32 {
        if !(-64..=319).contains(&p.1) {
            return block("air");
        }
        self.states
            .get(&p)
            .or_else(|| self.overrides.get(&p))
            .copied()
            .unwrap_or_else(|| terrain(&self.terrain, p))
    }
}
impl OreWorld for World {
    fn get_block(&self, p: Pos) -> Option<u32> {
        Some(self.at(p))
    }
    fn ocean_floor_wg(&self, _: i32, _: i32) -> i32 {
        panic!("unexpected ocean-floor query")
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("structure write must retain flags")
    }
}
impl FeatureWorld for World {
    fn feature_biome(&self, _: Pos) -> u32 {
        panic!("unexpected block biome query")
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        assert_eq!(kind, FeatureHeightmap::MotionBlockingNoLeaves);
        let y = (-64..=319)
            .rev()
            .find(|&y| {
                ScatteredCatalog::bundled()
                    .block_properties(self.at((x, y, z)))
                    .unwrap()
                    .motion_blocking_no_leaves
            })
            .map_or(-64, |y| y + 1);
        self.heights
            .borrow_mut()
            .push(json!(["MOTION_BLOCKING_NO_LEAVES", x, z, y]));
        y
    }
    fn can_write_feature(&self, p: Pos) -> bool {
        (-64..=319).contains(&p.1)
            && ((p.0 >> 4) - self.source.x).abs() <= 1
            && ((p.2 >> 4) - self.source.z).abs() <= 1
            && !self.denied.contains(&p)
    }
    fn set_feature_block(&mut self, p: Pos, state: u32, flags: i32) -> bool {
        let accepted = self.can_write_feature(p);
        self.effects
            .push(json!(["block", p.0, p.1, p.2, state, flags, accepted]));
        if !accepted {
            return false;
        }
        let registry = &StructureAssets::bundled().blocks;
        let previous = self.at(p);
        self.states.insert(p, state);
        if registry.flags(state).unwrap() & 8 == 0 {
            self.entities.remove(&p);
            self.observed_loot.remove(&p);
        } else if !self.suppress_entities
            && (!self.entities.contains_key(&p)
                || registry.state(previous).unwrap().name != registry.state(state).unwrap().name)
        {
            self.entities
                .insert(p, registry.default_block_entity(state, p).unwrap().unwrap());
            self.observed_loot.remove(&p);
        }
        true
    }
    fn mark_feature_postprocessing(&mut self, p: Pos) {
        self.effects.push(json!(["mark", p.0, p.1, p.2]));
        *self.marks.entry(p).or_default() += 1;
    }
    fn schedule_feature_tick(&mut self, tick: TickRequest) -> bool {
        let (kind, id) = match tick.target {
            TickTarget::Fluid(id) => ("fluid", id),
            TickTarget::Block(id) => ("block", id),
        };
        let [x, y, z] = tick.block_pos;
        self.effects
            .push(json!(["tick", x, y, z, kind, id, tick.delay]));
        self.ticks.push(json!([[x, y, z], kind, id, tick.delay]));
        true
    }
}
impl ScatteredWorld for World {
    fn structure_min_y(&self) -> i32 {
        -64
    }
    fn has_structure_block_entity(&self, p: Pos, id: &str) -> bool {
        self.entities.get(&p).is_some_and(|e| e.id == id)
    }
    fn apply_scattered_effect(&mut self, effect: ScatteredEffect) -> Result<(), FeatureError> {
        let ScatteredEffect::LootTable {
            pos: p,
            block_entity,
            table,
            seed,
        } = effect
        else {
            panic!("desert pyramid has no mob request")
        };
        let registry = &StructureAssets::bundled().blocks;
        let be = self.entities.get_mut(&p).unwrap();
        assert_eq!(be.id, block_entity);
        let mut load = be.full_data();
        let fields = load.compound_mut().unwrap();
        fields.insert("LootTable".into(), Nbt::String(table.clone()));
        fields.insert("LootTableSeed".into(), Nbt::Long(seed));
        *be = TemplateBlockEntity::from_load(registry, be.state, p, load)?;
        // The reference observes actual field mutations, suppressing unchanged
        // reassignments to an existing entity. All write requests remain ordered.
        let previous = self.observed_loot.insert(p, (table.clone(), seed));
        if previous.as_ref() != Some(&(table.clone(), seed)) {
            self.effects
                .push(json!(["loot", p.0, p.1, p.2, block_entity, table, seed]));
        }
        Ok(())
    }
}
