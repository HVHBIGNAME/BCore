use super::{update, Pos, StandingTreeWorld};
use crate::tree::fallen::FallenTreeWorld;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

struct World {
    blocks: BTreeMap<Pos, u32>,
    events: RefCell<Vec<Value>>,
}

impl FallenTreeWorld for World {
    fn get_block(&self, pos: Pos) -> u32 {
        let state = self.blocks.get(&pos).copied().unwrap_or(crate::block::AIR);
        self.events
            .borrow_mut()
            .push(json!(["get", pos.0, pos.1, pos.2, state]));
        state
    }
    fn set_block(&mut self, _: Pos, _: u32, _: i32) -> bool {
        panic!("stair update writes no blocks");
    }
    fn is_face_sturdy_up(&self, _: u32, _: Pos) -> bool {
        panic!("stair update makes no support queries");
    }
    fn mark_for_postprocessing(&mut self, _: Pos) {
        panic!("stair update adds no marks");
    }
}

impl StandingTreeWorld for World {
    fn has_beehive(&mut self, _: Pos) -> bool {
        panic!("no hive query");
    }
    fn store_bee(&mut self, _: Pos, _: i32) {
        panic!("no hive write");
    }
    fn schedule_tree_tick(&mut self, [x, y, z, fluid, delay, kind]: [i32; 6]) {
        assert_eq!(kind, 1);
        self.events
            .borrow_mut()
            .push(json!(["tick", x, y, z, fluid, delay]));
    }
}

#[test]
fn native_stair_corners_keep_ordered_world_reads_and_water_ticks() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../data/stair_block_shapes_26_1.json")).unwrap();
    assert_eq!(
        fixture["jar_sha256"],
        crate::structure::template_pool::StructureAssets::JAR_SHA256
    );
    assert_eq!(fixture["state_count"], 29873);
    assert_eq!(fixture["random_untouched"], true);
    assert_eq!(fixture["blocks"].as_array().unwrap().len(), 58);
    for block in fixture["blocks"].as_array().unwrap() {
        let first = block["first"].as_u64().unwrap() as u32;
        let last = first + block["count"].as_u64().unwrap() as u32 - 1;
        for state in [first, last] {
            assert_eq!(
                crate::tree::extra_data::block(state).update_shape,
                block["owner"].as_str().unwrap()
            );
        }
    }
    let cases = fixture["samples"].as_array().unwrap();
    assert_eq!(cases.len(), 10212);
    let mut shapes = BTreeSet::new();
    for row in cases {
        let [x, y, z]: [i32; 3] = serde_json::from_value(row["pos"].clone()).unwrap();
        let state = row["state"].as_u64().unwrap() as u32;
        let direction = row["direction"].as_u64().unwrap() as usize;
        let mut world = World {
            blocks: row["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        (
                            p[0].as_i64().unwrap() as i32,
                            p[1].as_i64().unwrap() as i32,
                            p[2].as_i64().unwrap() as i32,
                        ),
                        p[3].as_u64().unwrap() as u32,
                    )
                })
                .collect(),
            events: RefCell::new(Vec::new()),
        };
        let result = update(
            &mut world,
            (x, y, z),
            state,
            direction,
            row["neighbor"].as_u64().unwrap() as u32,
        )
        .unwrap();
        assert_eq!(
            result,
            row["result"].as_u64().unwrap() as u32,
            "state {state}, direction {direction}, {}",
            row["profile"]
        );
        assert_eq!(
            json!(world.events.into_inner()),
            row["events"],
            "state {state}, direction {direction}, {}",
            row["profile"]
        );
        shapes.insert(crate::tree::extra_data::property(result, "shape").unwrap());
    }
    assert_eq!(
        shapes,
        [
            "straight",
            "inner_left",
            "inner_right",
            "outer_left",
            "outer_right"
        ]
        .into()
    );
}
