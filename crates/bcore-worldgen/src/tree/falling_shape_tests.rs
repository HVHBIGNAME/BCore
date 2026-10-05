use super::{update, Pos, StandingTreeWorld};
use crate::tree::fallen::FallenTreeWorld;

#[derive(Default)]
struct RecordingWorld {
    ticks: Vec<[i32; 6]>,
}

impl FallenTreeWorld for RecordingWorld {
    fn get_block(&self, _: Pos) -> u32 {
        panic!("native falling update reads no world blocks")
    }
    fn set_block(&mut self, _: Pos, _: u32, _: i32) -> bool {
        panic!("native falling update writes no world blocks")
    }
    fn is_face_sturdy_up(&self, _: u32, _: Pos) -> bool {
        panic!("native falling update checks no support")
    }
    fn mark_for_postprocessing(&mut self, _: Pos) {
        panic!("native falling update adds no postprocess mark")
    }
}

impl StandingTreeWorld for RecordingWorld {
    fn has_beehive(&mut self, _: Pos) -> bool {
        panic!("no hive query")
    }
    fn store_bee(&mut self, _: Pos, _: i32) {
        panic!("no bee effect")
    }
    fn schedule_tree_tick(&mut self, request: [i32; 6]) {
        self.ticks.push(request);
    }
}

#[test]
fn all_native_falling_edge_updates_preserve_states_and_virtual_tick_delays() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../data/falling_block_shapes_26_1.json")).unwrap();
    assert_eq!(fixture["minecraft"], "26.1");
    assert_eq!(
        fixture["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(fixture["state_count"], 29873);
    assert_eq!(fixture["random_untouched"], true);
    assert_eq!(fixture["blocks"].as_array().unwrap().len(), 7);
    let cases = fixture["samples"].as_array().unwrap();
    assert_eq!(cases.len(), 1440);
    let mut delays = std::collections::BTreeSet::new();
    for case in cases {
        let [x, y, z]: [i32; 3] = serde_json::from_value(case["pos"].clone()).unwrap();
        let mut world = RecordingWorld::default();
        let state = case["state"].as_u64().unwrap() as u32;
        let direction = case["direction"].as_u64().unwrap() as usize;
        let neighbor = case["neighbor"].as_u64().unwrap() as u32;
        assert_eq!(
            update(&mut world, (x, y, z), state, direction, neighbor).unwrap(),
            case["result"].as_u64().unwrap() as u32,
            "{case}"
        );
        assert_eq!(serde_json::json!(world.ticks), case["ticks"], "{case}");
        delays.extend(world.ticks.iter().map(|tick| tick[4]));
    }
    assert_eq!(delays, [2, 5].into());
}
