//! Native configured-well observations, plus the real production dispatcher.
use super::*;
use crate::block_entity::PendingBlockEntity;
use crate::generation::FeatureBlockEntity;
use crate::region::FeatureRegion;
use crate::structure::template::{Nbt, TemplateBlockEntity};
use crate::structure::template_pool::StructureAssets;
use crate::{ChunkPos, WorldGenerator};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::BTreeSet;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../data/desert_well_26_1.json")).unwrap()
}

fn pos(v: &Value) -> Pos {
    (
        v[0].as_i64().unwrap() as i32,
        v[1].as_i64().unwrap() as i32,
        v[2].as_i64().unwrap() as i32,
    )
}

struct ObservedWorld {
    region: FeatureRegion,
    source: ChunkPos,
    radius: i32,
    reject: String,
    events: RefCell<Vec<Value>>,
    touched: BTreeSet<Pos>,
}

impl ObservedWorld {
    fn new(row: &Value) -> Self {
        let origin = pos(&row["origin"]);
        let source = ChunkPos::new(origin.0 >> 4, origin.2 >> 4);
        let mut region = FeatureRegion::shared(WorldGenerator::new(42));
        let mut available = BTreeMap::new();
        for x in source.x - 2..=source.x + 2 {
            for z in source.z - 2..=source.z + 2 {
                region.owned_chunk_mut(ChunkPos::new(x, z));
                available.insert((x, z), crate::generation::ChunkStatus::Carvers);
            }
        }
        for state in row["initial"].as_array().unwrap() {
            let p = pos(state);
            region
                .owned_chunk_mut(ChunkPos::new(p.0 >> 4, p.2 >> 4))
                .set(
                    (p.0 & 15) as usize,
                    p.1,
                    (p.2 & 15) as usize,
                    state[3].as_u64().unwrap() as u32,
                );
        }
        for entry in row["initial_entities"].as_array().unwrap() {
            let p = pos(&entry["pos"]);
            let chunk = region.owned_chunk_mut(ChunkPos::new(p.0 >> 4, p.2 >> 4));
            let local = ((p.0 & 15) as usize, p.1, (p.2 & 15) as usize);
            let data = &entry["data"];
            if !data["pending"].is_null() {
                assert!(chunk.set_pending_block_entity(
                    local.0,
                    p.1,
                    local.2,
                    PendingBlockEntity {
                        typed_data: data["pending"].clone()
                    }
                ));
            }
            if !data["full"].is_null() {
                let native = Nbt::from_typed_json(&data["full"]).unwrap();
                let entity = TemplateBlockEntity::from_load(
                    &StructureAssets::bundled().blocks,
                    chunk.get(local.0, p.1, local.2).unwrap(),
                    p,
                    native,
                )
                .unwrap();
                chunk
                    .feature_block_entities
                    .insert(local, FeatureBlockEntity::from_template(&entity).unwrap());
            }
        }
        region.begin_source(source, crate::generation::ChunkStatus::Features, available);
        Self {
            region,
            source,
            radius: row["write_radius"].as_i64().unwrap() as i32,
            reject: row["reject"].as_str().unwrap().into(),
            events: RefCell::new(Vec::new()),
            touched: BTreeSet::new(),
        }
    }

    fn inside_radius(&self, p: Pos) -> bool {
        ((p.0 >> 4) - self.source.x)
            .abs()
            .max(((p.2 >> 4) - self.source.z).abs())
            <= self.radius
    }

    fn raw(&self, p: Pos) -> u32 {
        // Component adapter follows the observed native ProtoChunk read contract.
        // Production's pre-existing out-of-height AIR representation is separate.
        if !(crate::MIN_Y..=crate::MAX_Y).contains(&p.1) {
            return catalog().default_state("void_air").unwrap();
        }
        self.region.get_block(p).unwrap()
    }

    fn entity(&self, p: Pos) -> Value {
        let chunk = self
            .region
            .owned_chunk(ChunkPos::new(p.0 >> 4, p.2 >> 4))
            .unwrap();
        let local = ((p.0 & 15) as usize, p.1, (p.2 & 15) as usize);
        let entity = chunk.feature_block_entities().get(&local);
        json!({"pending": chunk.pending_block_entities().get(&local).map(|e| &e.typed_data),
            "full": entity.map(|e| &e.typed_data), "update": entity.map(|e| e.update_nbt().unwrap().typed_json())})
    }

    fn assert_final(&self, row: &Value) {
        let mut hash = Sha256::new();
        let mut counts = BTreeMap::<String, usize>::new();
        let mut entities = Vec::new();
        for &p in &self.touched {
            let state = self.raw(p);
            for value in [p.0, p.1, p.2, state as i32] {
                hash.update(value.to_le_bytes());
            }
            *counts.entry(state.to_string()).or_default() += 1;
            let data = self.entity(p);
            if !data["pending"].is_null() || !data["full"].is_null() {
                entities.push(json!({"pos": [p.0,p.1,p.2], "data": data}));
            }
        }
        assert_eq!(
            format!("{:x}", hash.finalize()),
            row["world"]["final_sha256"],
            "{} states",
            row["name"]
        );
        assert_eq!(
            json!(counts),
            row["world"]["final_state_counts"],
            "{} histogram",
            row["name"]
        );
        assert_eq!(
            json!(entities),
            row["entities"],
            "{} typed entities",
            row["name"]
        );
        let mut marks = Vec::new();
        for x in self.source.x - 2..=self.source.x + 2 {
            for z in self.source.z - 2..=self.source.z + 2 {
                let chunk = self.region.owned_chunk(ChunkPos::new(x, z)).unwrap();
                assert!(chunk.tick_requests().is_empty(), "well schedules no ticks");
                for section in -4..20 {
                    marks.extend(
                        chunk
                            .postprocessing_positions()
                            .iter()
                            .filter(|p| p.1 >> 4 == section)
                            .map(|p| [x * 16 + p.0 as i32, p.1, z * 16 + p.2 as i32]),
                    );
                }
            }
        }
        assert_eq!(
            json!(marks),
            row["world"]["marks"],
            "{} ordered postprocessing",
            row["name"]
        );
        assert_eq!(row["world"]["native_ticks"], json!([]));
    }
}

impl OreWorld for ObservedWorld {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.region.ocean_floor_wg(x, z)
    }
    fn get_block(&self, p: Pos) -> Option<u32> {
        let state = self.raw(p);
        self.events
            .borrow_mut()
            .push(json!({"op":"read", "pos":[p.0,p.1,p.2], "state":state}));
        Some(state)
    }
    fn set_block(&mut self, p: Pos, state: u32) -> bool {
        self.set_feature_block(p, state, 2)
    }
}

impl FeatureWorld for ObservedWorld {
    fn feature_biome(&self, p: Pos) -> u32 {
        self.region.feature_biome(p)
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.region.feature_height(kind, x, z)
    }
    fn can_write_feature(&self, p: Pos) -> bool {
        let accepted = self.inside_radius(p);
        self.events
            .borrow_mut()
            .push(json!({"op":"guard", "pos":[p.0,p.1,p.2], "accepted":accepted}));
        accepted
    }
    fn set_feature_block(&mut self, p: Pos, state: u32, flags: i32) -> bool {
        let denied = self.reject == "all"
            || (self.reject == "archaeology"
                && state == catalog().default_state("suspicious_sand").unwrap());
        let accepted = !denied && self.inside_radius(p);
        if accepted {
            self.region.set_feature_block(p, state, flags);
        }
        // The native non-upgrading region reports true even when ProtoChunk drops
        // an out-of-build-height write. The kernel must ignore that return value.
        self.events.borrow_mut().push(json!({"op":"write", "pos":[p.0,p.1,p.2], "state":state,"flags":flags,"accepted":accepted}));
        self.touched.insert(p);
        accepted
    }
    fn mark_feature_postprocessing(&mut self, p: Pos) {
        self.region.mark_feature_postprocessing(p);
    }
    fn schedule_feature_tick(&mut self, tick: TickRequest) -> bool {
        self.region.schedule_feature_tick(tick)
    }
    fn set_feature_brushable_loot(
        &mut self,
        p: Pos,
        table: &str,
        seed: i64,
    ) -> FeatureResult<bool> {
        let before = self.entity(p);
        let present = self.region.set_feature_brushable_loot(p, table, seed)?;
        self.events
            .borrow_mut()
            .push(json!({"op":"lookup","pos":[p.0,p.1,p.2],"present":present,"before":before}));
        Ok(present)
    }
}

#[test]
fn desert_well_native_components_preserve_order_rng_rejections_and_typed_loot() {
    let fixture = fixture();
    assert_eq!(
        fixture["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    for row in fixture["samples"].as_array().unwrap() {
        let mut world = ObservedWorld::new(row);
        let mut random = WorldgenRandom::new(row["seed"].as_i64().unwrap());
        let origin = pos(&row["origin"]);
        let mut placed = Vec::new();
        for _ in 0..row["repeats"].as_u64().unwrap() {
            placed.push(
                world.can_write_feature(origin)
                    && crate::desert_well::place(&mut world, &mut random, origin).unwrap(),
            );
        }
        assert_eq!(json!(placed), row["placed"], "{} result", row["name"]);
        assert_eq!(
            random.next_long(),
            row["next_i64"].as_i64().unwrap(),
            "{} RNG",
            row["name"]
        );
        let expected: Vec<Value> = row["events"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(|mut e| {
                // Native also snapshots immediately after materialization, before the
                // lambda mutates loot. Our atomic callback is checked by its input
                // pending/live snapshot and the final full/update tags instead.
                if e["op"] == "lookup" {
                    e.as_object_mut().unwrap().remove("after");
                }
                e
            })
            .collect();
        assert_eq!(
            *world.events.borrow(),
            expected,
            "{} ordered operations",
            row["name"]
        );
        world.assert_final(row);
    }
}

#[test]
fn desert_well_native_cases_through_production_dispatcher_and_lazy_region() {
    let fixture = fixture();
    let mut checked = 0;
    for row in fixture["samples"].as_array().unwrap().iter().filter(|r| {
        r["reject"] == "none" && r["write_radius"] == 1 && pos(&r["origin"]).1 <= crate::MAX_Y
    }) {
        let mut world = ObservedWorld::new(row);
        let mut random = WorldgenRandom::new(row["seed"].as_i64().unwrap());
        let mut cave_random = CaveRandomState::default();
        let mut results = Vec::new();
        for _ in 0..row["repeats"].as_u64().unwrap() {
            results.push(
                place_configured(
                    catalog().configured("desert_well").unwrap(),
                    Some("desert_well"),
                    &mut world.region,
                    &mut random,
                    pos(&row["origin"]),
                    &mut cave_random,
                    &GenerationEnvironment,
                )
                .unwrap(),
            );
        }
        assert_eq!(
            json!(results),
            row["placed"],
            "{} integrated result",
            row["name"]
        );
        assert_eq!(
            random.next_long(),
            row["next_i64"].as_i64().unwrap(),
            "{} integrated RNG",
            row["name"]
        );
        world.touched = row["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["op"] == "write")
            .map(|e| pos(&e["pos"]))
            .collect();
        world.assert_final(row);
        checked += 1;
    }
    assert!(checked >= 20);
}
