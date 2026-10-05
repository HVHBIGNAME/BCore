//! Native container traces, including heap iteration, save/reload, and duplicate loads.
use bcore_core::ChunkPos;
use bcore_worldgen::tick_queue::{
    filter_ticks_for_chunk, LevelTickQueue, PreparedTickQueues, ProtoTickQueue, SavedTick,
    ScheduledTick, TickPriority,
};
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
enum Target {
    Block(u32),
    Fluid(u32),
}

impl Target {
    fn index(self) -> usize {
        match self {
            Self::Block(_) => 0,
            Self::Fluid(_) => 1,
        }
    }

    fn id(self) -> u32 {
        match self {
            Self::Block(id) | Self::Fluid(id) => id,
        }
    }

    fn in_container(index: usize, id: u32) -> Self {
        if index == 0 {
            Self::Block(id)
        } else {
            Self::Fluid(id)
        }
    }
}

fn fixture() -> &'static Value {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let fixture: Value = serde_json::from_str(include_str!("../data/ticks_26_1.json")).unwrap();
        assert_eq!(fixture["minecraft"], "26.1");
        assert_eq!(
            fixture["jar_sha256"],
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        let mut hash = Sha256::new();
        hash.update(include_bytes!("../../../scripts/TreeReference.java"));
        hash.update(include_bytes!("../../../scripts/TickReference.java"));
        assert_eq!(fixture["probe_sha256"], format!("{:x}", hash.finalize()));
        assert_eq!(fixture["samples"].as_array().unwrap().len(), 22);
        assert_eq!(fixture["preparations"].as_array().unwrap().len(), 4);
        assert_eq!(fixture["conversion_cases"].as_array().unwrap().len(), 24);
        assert_eq!(fixture["comparison_cases"].as_array().unwrap().len(), 81);
        assert_eq!(fixture["filter_cases"].as_array().unwrap().len(), 12);
        assert_eq!(fixture["priority_cases"].as_array().unwrap().len(), 11);
        let names: BTreeSet<_> = fixture["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| case["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 22);
        fixture
    })
}

fn target(row: &Value) -> Target {
    serde_json::from_value(row["target"].clone()).unwrap()
}

fn position(row: &Value) -> [i32; 3] {
    serde_json::from_value(row["block_pos"].clone()).unwrap()
}

fn priority(row: &Value) -> TickPriority {
    TickPriority::from_value(i32::try_from(row["priority"].as_i64().unwrap()).unwrap())
}

fn saved(row: &Value) -> SavedTick<Target> {
    SavedTick {
        block_pos: position(row),
        target: target(row),
        priority: priority(row),
        delay: i32::try_from(row["delay"].as_i64().unwrap()).unwrap(),
    }
}

fn saved_id(row: &Value) -> SavedTick<u32> {
    let tick = saved(row);
    SavedTick {
        block_pos: tick.block_pos,
        target: tick.target.id(),
        delay: tick.delay,
        priority: tick.priority,
    }
}

fn scheduled(row: &Value) -> ScheduledTick<Target> {
    ScheduledTick {
        block_pos: position(row),
        target: target(row),
        priority: priority(row),
        trigger_tick: row["trigger_tick"].as_i64().unwrap(),
        sub_tick_order: row["sub_tick_order"].as_i64().unwrap(),
    }
}

fn scheduled_id(row: &Value) -> ScheduledTick<u32> {
    let tick = scheduled(row);
    ScheduledTick {
        block_pos: tick.block_pos,
        target: tick.target.id(),
        priority: tick.priority,
        trigger_tick: tick.trigger_tick,
        sub_tick_order: tick.sub_tick_order,
    }
}

fn saved_row<T>(tick: &SavedTick<T>, target: Target) -> Value {
    json!({"block_pos": tick.block_pos, "target": target, "delay": tick.delay, "priority": tick.priority.value()})
}

fn scheduled_row<T>(tick: &ScheduledTick<T>, target: Target) -> Value {
    json!({"block_pos": tick.block_pos, "target": target, "trigger_tick": tick.trigger_tick,
        "priority": tick.priority.value(), "sub_tick_order": tick.sub_tick_order})
}

fn type_name(target: Target) -> &'static str {
    let target = serde_json::to_value(target).unwrap();
    fixture()["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["target"] == target)
        .unwrap()["name"]
        .as_str()
        .unwrap()
}

enum Queues {
    Proto([ProtoTickQueue<u32>; 2]),
    Level([LevelTickQueue<u32>; 2]),
}

impl Queues {
    fn new(mode: &str, initial: &Value) -> Self {
        if initial.is_null() {
            return match mode {
                "proto" => Self::Proto(std::array::from_fn(|_| ProtoTickQueue::default())),
                "level" => Self::Level(std::array::from_fn(|_| LevelTickQueue::default())),
                _ => panic!("unknown native container {mode}"),
            };
        }
        let ticks: [Vec<_>; 2] = std::array::from_fn(|index| {
            initial
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| target(row).index() == index)
                .map(saved_id)
                .collect()
        });
        match mode {
            "proto" => Self::Proto(ticks.map(ProtoTickQueue::load)),
            "level" => Self::Level(ticks.map(LevelTickQueue::from_saved)),
            _ => panic!("unknown native container {mode}"),
        }
    }

    fn pack(&self, index: usize, time: i64) -> Vec<SavedTick<u32>> {
        match self {
            Self::Proto(queues) => queues[index].pack(time),
            Self::Level(queues) => queues[index].pack(time),
        }
    }

    fn check(&self, expected: &Value, queries: &[Value], label: &str) {
        assert_eq!(
            expected["mode"],
            match self {
                Self::Proto(_) => "proto",
                Self::Level(_) => "level",
            }
        );
        let time = expected["pack_time"].as_i64().unwrap();
        for (index, name) in ["blocks", "fluids"].into_iter().enumerate() {
            let state = &expected[name];
            let count = match self {
                Self::Proto(queues) => {
                    let ticks: Vec<_> = queues[index]
                        .scheduled_ticks()
                        .iter()
                        .map(|tick| saved_row(tick, Target::in_container(index, tick.target)))
                        .collect();
                    assert_eq!(
                        json!(ticks),
                        state["scheduled_ticks"],
                        "{label}: {name} scheduledTicks"
                    );
                    queues[index].count()
                }
                Self::Level(queues) => {
                    let ticks: Vec<_> = queues[index]
                        .get_all()
                        .iter()
                        .map(|tick| scheduled_row(tick, Target::in_container(index, tick.target)))
                        .collect();
                    assert_eq!(
                        json!(ticks),
                        state["get_all"],
                        "{label}: {name} getAll heap order"
                    );
                    let peek = queues[index]
                        .peek()
                        .map(|tick| scheduled_row(tick, Target::in_container(index, tick.target)));
                    assert_eq!(json!(peek), state["peek"], "{label}: {name} peek");
                    queues[index].count()
                }
            };
            assert_eq!(json!(count), state["count"], "{label}: {name} count");
            let packed = self.pack(index, time);
            let rows: Vec<_> = packed
                .iter()
                .map(|tick| saved_row(tick, Target::in_container(index, tick.target)))
                .collect();
            assert_eq!(json!(rows), state["pack"], "{label}: {name} pack");
            let data: Vec<_> = packed
                .iter()
                .map(|tick| tick.save_data(type_name(Target::in_container(index, tick.target))))
                .collect();
            assert_eq!(
                json!(data),
                state["save_data"],
                "{label}: {name} native save codec"
            );
        }
        let membership: Vec<_> = queries
            .iter()
            .map(|query| {
                let target = target(query);
                match self {
                    Self::Proto(queues) => {
                        queues[target.index()].has_scheduled_tick(position(query), &target.id())
                    }
                    Self::Level(queues) => {
                        queues[target.index()].has_scheduled_tick(position(query), &target.id())
                    }
                }
            })
            .collect();
        assert_eq!(
            json!(membership),
            expected["has_scheduled"],
            "{label}: identity membership"
        );
    }

    fn step(&mut self, step: &Value, label: &str) {
        match step["op"].as_str().unwrap() {
            "schedule" => {
                let accepted: Vec<_> = step["ticks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| {
                        let index = target(row).index();
                        let tick = scheduled_id(row);
                        match self {
                            Self::Proto(queues) => queues[index].schedule(tick),
                            Self::Level(queues) => queues[index].schedule(tick),
                        }
                    })
                    .collect();
                assert_eq!(
                    json!(accepted),
                    step["accepted"],
                    "{label}: schedule acceptance"
                );
            }
            "poll" => {
                let index = match step["container"].as_str().unwrap() {
                    "blocks" => 0,
                    "fluids" => 1,
                    other => panic!("unknown container {other}"),
                };
                let Self::Level(queues) = self else {
                    panic!("poll on native proto")
                };
                let polled = queues[index]
                    .poll()
                    .map(|tick| scheduled_row(&tick, Target::in_container(index, tick.target)));
                assert_eq!(json!(polled), step["polled"], "{label}: poll");
            }
            "unpack" => {
                let Self::Level(queues) = self else {
                    panic!("unpack on native proto")
                };
                for queue in queues {
                    queue.unpack(step["game_time"].as_i64().unwrap());
                }
            }
            "reload" | "transfer" => {
                let time = step["game_time"].as_i64().unwrap();
                let saved = [self.pack(0, time), self.pack(1, time)];
                *self = if step["op"] == "transfer" || matches!(self, Self::Level(_)) {
                    Self::Level(saved.map(LevelTickQueue::from_saved))
                } else {
                    Self::Proto(saved.map(ProtoTickQueue::load))
                };
            }
            "pack_time" => {}
            "drain" => {
                let Self::Level(queues) = self else {
                    panic!("drain on native proto")
                };
                for (index, name) in ["blocks", "fluids"].into_iter().enumerate() {
                    let mut rows = Vec::new();
                    while let Some(tick) = queues[index].poll() {
                        rows.push(scheduled_row(
                            &tick,
                            Target::in_container(index, tick.target),
                        ));
                    }
                    assert_eq!(
                        json!(rows),
                        step["drained"][name],
                        "{label}: {name} drain order"
                    );
                }
            }
            other => panic!("unknown native operation {other}"),
        }
    }
}

fn run_case(case: &Value) {
    let name = case["name"].as_str().unwrap();
    let mut queues = Queues::new(
        case["initial_mode"].as_str().unwrap(),
        &case["initial_saved"],
    );
    let queries = case["queries"].as_array().unwrap();
    queues.check(&case["initial"], queries, name);
    for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
        let label = format!("{name} step {index} {}", step["op"]);
        queues.step(step, &label);
        queues.check(&step["after"], queries, &label);
    }
}

fn raw_request(row: &Value) -> TickRequest {
    TickRequest {
        block_pos: position(row),
        target: match target(row) {
            Target::Block(id) => TickTarget::Block(id),
            Target::Fluid(id) => TickTarget::Fluid(id),
        },
        delay: i32::try_from(row["delay"].as_i64().unwrap()).unwrap(),
    }
}

#[test]
fn raw_requests_prepare_as_separate_native_proto_containers() {
    for case in fixture()["preparations"].as_array().unwrap() {
        let requests: Vec<_> = case["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(raw_request)
            .collect();
        let original = requests.clone();
        let prepared = PreparedTickQueues::from_requests(requests.iter().copied());
        Queues::Proto([prepared.blocks, prepared.fluids]).check(
            &case["expected"],
            case["queries"].as_array().unwrap(),
            "raw preparation",
        );
        let mut incremental = PreparedTickQueues::default();
        let accepted: Vec<_> = requests
            .iter()
            .map(|request| incremental.schedule_request(*request))
            .collect();
        assert_eq!(json!(accepted), case["accepted"]);
        assert_eq!(requests, original);
    }
}

#[test]
fn explicit_priorities_on_prepared_requests_match_native_proto() {
    let case = fixture()["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "proto_identity_first_wins")
        .unwrap();
    let step = &case["steps"][0];
    let mut queues = PreparedTickQueues::default();
    let accepted: Vec<_> = step["ticks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let tick = scheduled(row);
            let request = TickRequest {
                block_pos: tick.block_pos,
                target: match tick.target {
                    Target::Block(id) => TickTarget::Block(id),
                    Target::Fluid(id) => TickTarget::Fluid(id),
                },
                delay: tick.trigger_tick as i32,
            };
            queues.schedule_request_with_priority(request, tick.priority)
        })
        .collect();
    assert_eq!(json!(accepted), step["accepted"]);
    Queues::Proto([queues.blocks, queues.fluids]).check(
        &step["after"],
        case["queries"].as_array().unwrap(),
        "explicit request priority",
    );
}

#[test]
fn proto_schedule_load_and_transfer_match_native_traces() {
    let cases: Vec<_> = fixture()["samples"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["initial_mode"] == "proto")
        .collect();
    assert_eq!(cases.len(), 3);
    for case in cases {
        run_case(case);
    }
}

#[test]
fn level_heap_poll_pending_and_reload_match_native_traces() {
    let cases: Vec<_> = fixture()["samples"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["initial_mode"] == "level")
        .collect();
    assert_eq!(cases.len(), 19);
    for case in cases {
        run_case(case);
    }
}

#[test]
fn saved_tick_conversions_match_native_signed_narrowing_and_overflow() {
    for case in fixture()["conversion_cases"].as_array().unwrap() {
        let tick = scheduled(&case["scheduled"]);
        let packed = tick.to_saved_tick(case["pack_time"].as_i64().unwrap());
        assert_eq!(saved_row(&packed, packed.target), case["saved"]);
        assert_eq!(
            json!([packed.save_data(type_name(packed.target))]),
            case["save_data"]
        );
        let unpacked = packed.unpack(
            case["unpack_time"].as_i64().unwrap(),
            case["sub_tick_order"].as_i64().unwrap(),
        );
        assert_eq!(scheduled_row(&unpacked, unpacked.target), case["unpacked"]);
    }
}

fn sign(order: Ordering) -> i32 {
    match order {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

#[test]
fn comparators_and_unique_identity_are_distinct_from_record_equality() {
    for case in fixture()["comparison_cases"].as_array().unwrap() {
        let left = scheduled(&case["left"]);
        let right = scheduled(&case["right"]);
        assert_eq!(json!(sign(left.cmp_drain_order(&right))), case["drain"]);
        assert_eq!(
            json!(sign(left.cmp_intra_tick_order(&right))),
            case["intra"]
        );
        assert_eq!(json!(left == right), case["record_equal"]);
        let mut queue = ProtoTickQueue::default();
        assert!(queue.schedule(left));
        assert_eq!(json!(!queue.schedule(right)), case["unique_equal"]);
    }
}

#[test]
fn saved_chunk_filter_retains_duplicates_and_ignores_y() {
    for case in fixture()["filter_cases"].as_array().unwrap() {
        let owner: [i32; 2] = serde_json::from_value(case["owner"].clone()).unwrap();
        let ticks: Vec<_> = case["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(saved)
            .collect();
        let filtered = filter_ticks_for_chunk(
            &ticks,
            ChunkPos {
                x: owner[0],
                z: owner[1],
            },
        );
        let rows: Vec<_> = filtered
            .iter()
            .map(|tick| saved_row(tick, tick.target))
            .collect();
        assert_eq!(json!(rows), case["filtered"]);
    }
}

#[test]
fn priorities_match_native_by_value_and_codec_clamping() {
    assert_eq!(TickPriority::default(), TickPriority::Normal);
    for case in fixture()["priority_cases"].as_array().unwrap() {
        let input = i32::try_from(case["input"].as_i64().unwrap()).unwrap();
        let actual = json!(TickPriority::from_value(input).value());
        assert_eq!(actual, case["by_value"]);
        assert_eq!(actual, case["codec_decoded"]);
        assert_eq!(actual, case["codec_encoded"]);
    }
}
