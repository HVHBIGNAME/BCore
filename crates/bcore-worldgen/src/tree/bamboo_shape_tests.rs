use super::{update, Pos, StandingTreeWorld};
use crate::tree::fallen::FallenTreeWorld;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeSet;

struct World {
    pos: Pos,
    below: u32,
    events: RefCell<Vec<Value>>,
}

impl FallenTreeWorld for World {
    fn get_block(&self, pos: Pos) -> u32 {
        assert_eq!(pos, (self.pos.0, self.pos.1 - 1, self.pos.2));
        self.events
            .borrow_mut()
            .push(json!(["get", pos.0, pos.1, pos.2, self.below]));
        self.below
    }
    fn set_block(&mut self, _: Pos, _: u32, _: i32) -> bool {
        panic!("bamboo callback writes no world blocks");
    }
    fn is_face_sturdy_up(&self, _: u32, _: Pos) -> bool {
        panic!("bamboo uses its block tag, not support faces");
    }
    fn mark_for_postprocessing(&mut self, _: Pos) {
        panic!("no bamboo callback mark");
    }
}

impl StandingTreeWorld for World {
    fn has_beehive(&mut self, _: Pos) -> bool {
        panic!("no hive query");
    }
    fn store_bee(&mut self, _: Pos, _: i32) {
        panic!("no hive write");
    }
    fn schedule_tree_tick(&mut self, [x, y, z, block, delay, kind]: [i32; 6]) {
        assert_eq!(kind, 0);
        self.events
            .borrow_mut()
            .push(json!(["tick", x, y, z, block, delay]));
    }
}

#[test]
fn native_bamboo_edges_preserve_live_support_age_propagation_and_tick_order() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../data/bamboo_block_shapes_26_1.json")).unwrap();
    assert_eq!(
        fixture["jar_sha256"],
        crate::structure::template_pool::StructureAssets::JAR_SHA256
    );
    assert_eq!(fixture["state_count"], 29873);
    assert_eq!(fixture["random_untouched"], true);
    let supports: BTreeSet<u32> =
        serde_json::from_value(fixture["supports_bamboo"].clone()).unwrap();
    assert_eq!(supports.len(), 39);
    for state in 0..29873 {
        assert_eq!(
            crate::block_predicate::catalog()
                .in_block_tag(state, "supports_bamboo")
                .unwrap(),
            supports.contains(&state),
            "native bamboo support {state}"
        );
    }
    let rows = fixture["samples"].as_array().unwrap();
    assert_eq!(rows.len(), 9360);
    for row in rows {
        let [x, y, z]: [i32; 3] = serde_json::from_value(row["pos"].clone()).unwrap();
        let mut world = World {
            pos: (x, y, z),
            below: row["below"].as_u64().unwrap() as u32,
            events: RefCell::new(Vec::new()),
        };
        let state = row["state"].as_u64().unwrap() as u32;
        let dir = row["direction"].as_u64().unwrap() as usize;
        let neighbor = row["neighbor"].as_u64().unwrap() as u32;
        let result = update(&mut world, (x, y, z), state, dir, neighbor).unwrap();
        assert_eq!(result, row["result"].as_u64().unwrap() as u32, "{row}");
        assert_eq!(json!(world.events.into_inner()), row["events"], "{row}");
    }
}
