use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

use bcore_worldgen::{
    block,
    random::WorldgenRandom,
    tree::{
        fallen::{self, Direction, FallenTreeDecorator, FallenTreeWorld, Pos},
        IntProvider,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const JAR_SHA256: &str = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeReference {
    minecraft: String,
    jar_sha256: String,
    probe_sha256: String,
    samples: Vec<NativeSample>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeSample {
    kind: String,
    seed: i64,
    flat_ground: bool,
    placed: bool,
    states_md5: String,
    next_i64: i64,
    origin: [i32; 3],
    chunk: [i32; 2],
    floor_y: i32,
    soil: u32,
    soil_name: String,
    terrain: String,
    initial_blocks: Vec<[i32; 4]>,
    writes: Vec<[i32; 4]>,
    read_count: usize,
    reads_md5: String,
    write_count: usize,
    write_calls_md5: String,
    postprocessing: Vec<[i32; 3]>,
    cached_up_support: Vec<(u32, bool)>,
}

fn native_reference() -> NativeReference {
    serde_json::from_str(include_str!("../data/fallen_trees_26_1.json")).unwrap()
}

fn block_map(blocks: &[[i32; 4]]) -> BTreeMap<Pos, u32> {
    let result: BTreeMap<_, _> = blocks
        .iter()
        .map(|&[x, y, z, state]| ((x, y, z), u32::try_from(state).unwrap()))
        .collect();
    assert_eq!(result.len(), blocks.len(), "duplicate fixture position");
    result
}

fn trace_md5<const N: usize>(rows: impl IntoIterator<Item = [i32; N]>) -> String {
    let mut digest = md5::Context::new();
    for row in rows {
        for value in row {
            digest.consume(value.to_le_bytes());
        }
    }
    format!("{:x}", digest.compute())
}

// Synthetic terrain with absolute-coordinate overrides and native cached support
// predicates. Block reads beyond build height are air; writes there are rejected.
struct World {
    flat_ground: bool,
    floor_y: i32,
    soil: u32,
    initial: BTreeMap<Pos, u32>,
    writes: BTreeMap<Pos, u32>,
    attempts: Vec<(Pos, u32, i32, bool)>,
    reads: RefCell<Vec<[i32; 4]>>,
    support_up: BTreeMap<u32, bool>,
    support_queries: RefCell<Vec<(u32, Pos)>>,
    marked: Vec<Pos>,
    reject_writes: bool,
}

impl World {
    fn new(flat_ground: bool) -> Self {
        Self {
            flat_ground,
            floor_y: 64,
            soil: block::GRASS_BLOCK,
            initial: BTreeMap::new(),
            writes: BTreeMap::new(),
            attempts: Vec::new(),
            reads: RefCell::new(Vec::new()),
            support_up: (136..=147)
                .map(|state| (state, true))
                .chain([
                    (block::AIR, false),
                    (block::WATER, false),
                    (block::GRASS_BLOCK, true),
                    (block::STONE, true),
                ])
                .collect(),
            support_queries: RefCell::new(Vec::new()),
            marked: Vec::new(),
            reject_writes: false,
        }
    }

    fn from_sample(sample: &NativeSample) -> Self {
        Self {
            floor_y: sample.floor_y,
            soil: sample.soil,
            initial: block_map(&sample.initial_blocks),
            support_up: sample.cached_up_support.iter().copied().collect(),
            reject_writes: sample.terrain == "reject_writes",
            ..Self::new(sample.flat_ground)
        }
    }

    fn block_at(&self, pos: Pos) -> u32 {
        if !(-64..=319).contains(&pos.1) {
            return block::AIR;
        }
        self.writes
            .get(&pos)
            .or_else(|| self.initial.get(&pos))
            .copied()
            .unwrap_or(if self.flat_ground && pos.1 == self.floor_y {
                self.soil
            } else {
                block::AIR
            })
    }

    fn states_md5(&self, [cx, cz]: [i32; 2]) -> String {
        let mut digest = md5::Context::new();
        for y in -64..320 {
            for z in 0..16 {
                for x in 0..16 {
                    digest.consume(self.block_at((cx * 16 + x, y, cz * 16 + z)).to_le_bytes());
                }
            }
        }
        format!("{:x}", digest.compute())
    }
}

impl FallenTreeWorld for World {
    fn get_block(&self, pos: Pos) -> u32 {
        let state = self.block_at(pos);
        self.reads
            .borrow_mut()
            .push([pos.0, pos.1, pos.2, state as i32]);
        state
    }

    fn set_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        let accepted = !self.reject_writes && (-64..=319).contains(&pos.1);
        self.attempts.push((pos, state, flags, accepted));
        if accepted {
            self.writes.insert(pos, state);
        }
        accepted
    }

    fn is_face_sturdy_up(&self, state: u32, at: Pos) -> bool {
        self.support_queries.borrow_mut().push((state, at));
        *self
            .support_up
            .get(&state)
            .unwrap_or_else(|| panic!("unhandled support shape in test world: {state} at {at:?}"))
    }

    fn mark_for_postprocessing(&mut self, pos: Pos) {
        self.marked.push(pos);
    }
}

// Complete final write maps from the unchanged FallenTreeReference.probe calls,
// inspected in an isolated Java build with FallenTreeInspector's `writes` mode.
// Inspector SHA-256: 3cc9728bb1a87a23dc6cbc58c83d4d6ab8d16ffa93e8333dd6c6411604110d5e.
// These strengthen the existing chunk MD5 fixture by also rejecting extra writes.
const NATIVE_WRITES: &[(&str, u64, bool, &[[i32; 4]])] = &[
    (
        "fallen_oak_tree",
        0,
        true,
        &[
            [7, 65, 8, 8373],
            [8, 65, 1, 138],
            [8, 65, 2, 138],
            [8, 65, 3, 138],
            [8, 65, 4, 138],
            [8, 65, 5, 138],
            [8, 65, 8, 137],
            [8, 65, 9, 8381],
            [8, 66, 3, 2336],
            [9, 65, 8, 8388],
        ],
    ),
    (
        "fallen_oak_tree",
        1,
        true,
        &[
            [7, 65, 8, 8373],
            [8, 65, 7, 8385],
            [8, 65, 8, 137],
            [9, 65, 8, 8388],
            [11, 65, 8, 136],
            [12, 65, 8, 136],
        ],
    ),
    (
        "fallen_oak_tree",
        17,
        true,
        &[
            [2, 65, 8, 136],
            [2, 66, 8, 2337],
            [3, 65, 8, 136],
            [4, 65, 8, 136],
            [5, 65, 8, 136],
            [6, 65, 8, 136],
            [8, 65, 7, 8385],
            [8, 65, 8, 137],
            [8, 65, 9, 8381],
            [9, 65, 8, 8388],
        ],
    ),
    (
        "fallen_oak_tree",
        42,
        true,
        &[
            [8, 65, 7, 8385],
            [8, 65, 8, 137],
            [8, 65, 9, 8381],
            [10, 65, 8, 136],
            [11, 65, 8, 136],
            [11, 66, 8, 2337],
            [12, 65, 8, 136],
            [13, 65, 8, 136],
        ],
    ),
    (
        "fallen_oak_tree",
        0,
        false,
        &[
            [7, 65, 8, 8373],
            [8, 65, 8, 137],
            [8, 65, 9, 8381],
            [9, 65, 8, 8388],
        ],
    ),
    (
        "fallen_birch_tree",
        0,
        true,
        &[
            [8, 65, 1, 144],
            [8, 65, 2, 144],
            [8, 65, 3, 144],
            [8, 65, 4, 144],
            [8, 65, 5, 144],
            [8, 65, 6, 144],
            [8, 65, 8, 143],
        ],
    ),
    (
        "fallen_birch_tree",
        1,
        true,
        &[
            [2, 65, 8, 142],
            [3, 65, 8, 142],
            [4, 65, 8, 142],
            [5, 65, 8, 142],
            [8, 65, 8, 143],
        ],
    ),
    (
        "fallen_birch_tree",
        17,
        true,
        &[
            [8, 65, 8, 143],
            [8, 65, 11, 144],
            [8, 65, 12, 144],
            [8, 65, 13, 144],
        ],
    ),
    (
        "fallen_birch_tree",
        42,
        true,
        &[
            [8, 65, 8, 143],
            [8, 65, 11, 144],
            [8, 65, 12, 144],
            [8, 65, 13, 144],
            [8, 65, 14, 144],
        ],
    ),
    ("fallen_birch_tree", 0, false, &[[8, 65, 8, 143]]),
];

#[test]
fn fallen_tree_blocks_decorators_return_and_rng_match_all_native_cases() {
    let fixture = native_reference();
    assert_eq!(fixture.minecraft, "26.1");
    assert_eq!(fixture.jar_sha256, JAR_SHA256);
    let mut counts = BTreeMap::new();
    for sample in &fixture.samples {
        let label = format!(
            "{} seed={} ground={} soil={} terrain={} origin={:?} floor={}",
            sample.kind,
            sample.seed,
            sample.flat_ground,
            sample.soil_name,
            sample.terrain,
            sample.origin,
            sample.floor_y
        );
        let config = fallen::config_for(&sample.kind).unwrap();
        let [x, y, z] = sample.origin;
        assert_eq!(
            sample.chunk,
            [x.div_euclid(16), z.div_euclid(16)],
            "{label}"
        );
        let mut world = World::from_sample(sample);
        let mut random = WorldgenRandom::from_seed(sample.seed as u64);
        assert_eq!(
            fallen::place(&mut world, &mut random, &config, (x, y, z)),
            sample.placed,
            "return: {label}"
        );
        assert_eq!(random.next_i64(), sample.next_i64, "RNG: {label}");
        assert_eq!(
            world.states_md5(sample.chunk),
            sample.states_md5,
            "blocks: {label}"
        );
        assert_eq!(
            world.writes,
            block_map(&sample.writes),
            "all world-space writes: {label}"
        );
        let postprocessing: Vec<_> = sample
            .postprocessing
            .iter()
            .map(|&[x, y, z]| (x, y, z))
            .collect();
        assert_eq!(world.marked, postprocessing, "postprocessing: {label}");
        assert_eq!(
            world.reads.borrow().len(),
            sample.read_count,
            "read count: {label}"
        );
        assert_eq!(
            trace_md5(world.reads.borrow().iter().copied()),
            sample.reads_md5,
            "read order/states: {label}"
        );
        assert_eq!(
            world.attempts.len(),
            sample.write_count,
            "write count: {label}"
        );
        let calls = world
            .attempts
            .iter()
            .map(|&((x, y, z), state, flags, accepted)| {
                [x, y, z, state as i32, flags, i32::from(accepted)]
            });
        assert_eq!(
            trace_md5(calls),
            sample.write_calls_md5,
            "write order/flags/result: {label}"
        );
        for &(_, state, flags, _) in &world.attempts {
            let is_log = config.trunk_states.contains(&state);
            assert_eq!(flags, if is_log { 3 } else { 19 }, "update flags: {label}");
        }
        *counts.entry(sample.kind.as_str()).or_insert(0) += 1;
    }
    for (kind, count) in counts {
        eprintln!("Verified {count} native cases for {kind}");
    }
    eprintln!("Verified {} native fallen-tree cases: complete writes, read/write traces, postprocessing, return and RNG continuation", fixture.samples.len());
}

#[test]
fn fallen_tree_fixture_preserves_original_ten_cases_and_source_provenance() {
    let fixture = native_reference();
    let mut hash = Sha256::new();
    hash.update(include_bytes!("../../../scripts/TreeReference.java"));
    hash.update(include_bytes!("../../../scripts/FallenTreeReference.java"));
    assert_eq!(format!("{:x}", hash.finalize()), fixture.probe_sha256);

    let fingerprints = [
        ("7e4867491aa36ed293907b98ea0e5353", -1341736375228859770),
        ("cb4eb15420f4582fbeafc9e04bd532e3", -860648294710421660),
        ("a7c314b6d35a85b400e348860de7fdbd", 7799626953667144776),
        ("4341a4917772963b38bf6ca5d38c1114", 8296740667292563995),
        ("9f348e6e91292062f7f68fc2f99b8878", -1069045159233708826),
        ("958fdb84a23fabb817188de2fdd8b868", -3770814357236379989),
        ("25bf869ac24a9460319eae6edc9c6796", -4439334553339746772),
        ("dd1a02fdf5f60b8a033012a39f5624e5", -1033142044215729552),
        ("0e521694a7be33ec3da5d347cb1646fe", -7688159163963057199),
        ("a2cda4e75495739dd74a95f4dfe80019", 2160572956399813066),
    ];
    for (index, &(kind, seed, ground, writes)) in NATIVE_WRITES.iter().enumerate() {
        let sample = &fixture.samples[index];
        assert_eq!(
            (sample.kind.as_str(), sample.seed, sample.flat_ground),
            (kind, seed as i64, ground)
        );
        assert_eq!(sample.origin, [8, 65, 8]);
        assert_eq!(sample.chunk, [0, 0]);
        assert_eq!(sample.floor_y, 64);
        assert_eq!(sample.soil, block::GRASS_BLOCK);
        assert_eq!(sample.terrain, "flat");
        assert!(sample.initial_blocks.is_empty());
        assert!(sample.placed);
        assert_eq!(
            (sample.states_md5.as_str(), sample.next_i64),
            fingerprints[index]
        );
        assert_eq!(sample.writes, writes);
        assert!(sample.postprocessing.is_empty());
    }
}

#[test]
fn fallen_tree_native_matrix_covers_all_presets_lengths_directions_and_boundaries() {
    let fixture = native_reference();
    let kinds = [
        "fallen_oak_tree",
        "fallen_birch_tree",
        "fallen_super_birch_tree",
        "fallen_jungle_tree",
        "fallen_spruce_tree",
    ];
    assert_eq!(fixture.samples.len(), 655);
    let mut unique = BTreeSet::new();
    for sample in &fixture.samples {
        assert!(
            unique.insert((
                &sample.kind,
                sample.seed,
                sample.flat_ground,
                sample.origin,
                sample.floor_y,
                sample.soil,
                &sample.terrain,
            )),
            "duplicate native input"
        );
    }
    for kind in kinds {
        let samples: Vec<_> = fixture.samples.iter().filter(|s| s.kind == kind).collect();
        assert_eq!(samples.len(), 131, "{kind}");
        let config = fallen::config_for(kind).unwrap();
        let baseline: Vec<_> = samples
            .iter()
            .filter(|s| {
                s.origin == [8, 65, 8]
                    && s.floor_y == 64
                    && s.soil == block::GRASS_BLOCK
                    && s.flat_ground
                    && s.terrain == "flat"
            })
            .collect();
        let seeds: BTreeSet<_> = baseline.iter().map(|s| s.seed).collect();
        assert_eq!(
            seeds,
            (0..64).chain([-1, i64::MIN, i64::MAX]).collect(),
            "{kind}"
        );
        let mut lengths = BTreeSet::new();
        let mut directions = BTreeSet::new();
        let mut mushrooms = BTreeSet::new();
        for sample in baseline {
            let logs: Vec<_> = sample
                .writes
                .iter()
                .filter(|p| {
                    p[3] as u32 == config.trunk_states[0] || p[3] as u32 == config.trunk_states[2]
                })
                .collect();
            lengths.insert(logs.len() as i32);
            let first = logs.first().unwrap();
            directions.insert(((first[0] - 8).signum(), (first[2] - 8).signum()));
            mushrooms.extend(
                sample
                    .writes
                    .iter()
                    .filter_map(|p| matches!(p[3], 2336 | 2337).then_some(p[3])),
            );
        }
        let IntProvider::Uniform { min, max } = config.log_length else {
            panic!("unexpected native provider");
        };
        assert_eq!(
            lengths,
            (min - 2..=max - 2).collect(),
            "native length coverage: {kind}"
        );
        assert_eq!(
            directions,
            BTreeSet::from([(-1, 0), (1, 0), (0, -1), (0, 1)]),
            "{kind}"
        );
        assert_eq!(
            mushrooms,
            BTreeSet::from([2336, 2337]),
            "native decorator coverage: {kind}"
        );
        let soils: BTreeSet<_> = samples.iter().map(|s| s.soil_name.as_str()).collect();
        assert_eq!(soils.len(), 14, "{kind}");
        assert!(samples.iter().any(|s| s.origin[1] == -64));
        assert!(samples.iter().any(|s| s.origin[1] == -63));
        assert!(samples.iter().any(|s| s.origin[1] == 319));
        assert!(samples.iter().any(|s| s.origin[0] < 0 && s.origin[2] < 0));
        assert!(samples.iter().any(|s| s.origin[0] > 29_000_000));
        assert!(
            samples.iter().any(|s| s
                .writes
                .iter()
                .any(|p| { [p[0].div_euclid(16), p[2].div_euclid(16)] != s.chunk })),
            "native cross-chunk writes: {kind}"
        );
        assert!(
            samples.iter().any(|s| !s.postprocessing.is_empty()),
            "{kind}"
        );
    }
    assert!(
        fixture
            .samples
            .iter()
            .any(|s| s.terrain != "reject_writes" && s.write_count > s.writes.len()),
        "native build-height write rejection coverage"
    );
}

fn decorator_json(decorator: &FallenTreeDecorator, ids: &Value) -> Value {
    match decorator {
        FallenTreeDecorator::TrunkVine => json!({"type": "minecraft:trunk_vine"}),
        FallenTreeDecorator::AttachedToLogs {
            probability,
            entries,
            directions,
        } => {
            let entries: Vec<_> = entries
                .iter()
                .map(|&(state, weight)| {
                    let name = ids
                        .as_object()
                        .unwrap()
                        .iter()
                        .find(|(_, id)| id.as_u64() == Some(state as u64))
                        .unwrap()
                        .0;
                    json!({"data": {"Name": name}, "weight": weight})
                })
                .collect();
            let directions: Vec<_> = directions
                .iter()
                .map(|d| match d {
                    Direction::Down => "down",
                    Direction::Up => "up",
                    Direction::North => "north",
                    Direction::South => "south",
                    Direction::West => "west",
                    Direction::East => "east",
                })
                .collect();
            json!({
                "type": "minecraft:attached_to_logs",
                "block_provider": {"type": "minecraft:weighted_state_provider", "entries": entries},
                "directions": directions,
                // Compare the float decoded by the native float codec, not f64.
                "probability": probability,
            })
        }
    }
}

#[test]
fn fallen_tree_presets_and_replaceable_tag_match_pinned_jar_resources() {
    let oracle: Value =
        serde_json::from_str(include_str!("../data/fallen_tree_configs_26_1.json")).unwrap();
    assert_eq!(oracle["jar_sha256"], JAR_SHA256);
    assert_eq!(oracle["protocol"], 775);
    let configs = oracle["configured_features"].as_object().unwrap();
    assert_eq!(configs.len(), 5);
    for (name, feature) in configs {
        let config = fallen::config_for(name).unwrap();
        assert_eq!(
            fallen::config_for(&format!("minecraft:{name}")),
            Some(config)
        );
        let mut expected = feature["config"].clone();
        let trunk = &expected["trunk_provider"]["state"]["Name"];
        assert_eq!(
            json!(config.trunk_states),
            oracle["pillar_state_ids_xyz"][trunk.as_str().unwrap()]
        );
        assert_eq!(
            expected["trunk_provider"]["type"],
            "minecraft:simple_state_provider"
        );
        assert_eq!(
            expected["trunk_provider"]["state"]["Properties"],
            json!({"axis":"y"})
        );
        let length = match config.log_length {
            IntProvider::Constant(value) => json!(value),
            IntProvider::Uniform { min, max } => {
                json!({"type":"minecraft:uniform", "min_inclusive":min, "max_inclusive":max})
            }
        };
        assert_eq!(length, expected["log_length"], "{name}");
        for (field, decorators) in [
            ("stump_decorators", config.stump_decorators),
            ("log_decorators", config.log_decorators),
        ] {
            for decorator in expected[field].as_array_mut().unwrap() {
                if let Some(probability) = decorator.get_mut("probability") {
                    *probability = json!(probability.as_f64().unwrap() as f32);
                }
            }
            let actual: Vec<_> = decorators
                .iter()
                .map(|d| decorator_json(d, &oracle["state_ids"]))
                .collect();
            assert_eq!(json!(actual), expected[field], "{name} {field}");
        }
    }
    assert!(fallen::config_for("fallen_acacia_tree").is_none());
    assert!(fallen::config_for("other:fallen_oak_tree").is_none());
    let ranges = oracle["replaceable_by_trees_ranges_exclusive_end"]
        .as_array()
        .unwrap();
    let heightmap: Value =
        serde_json::from_str(include_str!("../data/heightmaps_26_1.json")).unwrap();
    assert_eq!(heightmap["jar_sha256"], JAR_SHA256);
    for state in 0..heightmap["state_count"].as_u64().unwrap() {
        let expected = matches!(state, 0 | 15292 | 15293)
            || ranges.iter().any(|range| {
                (range[0].as_u64().unwrap()..range[1].as_u64().unwrap()).contains(&state)
            });
        assert_eq!(
            fallen::valid_tree_state(state as u32),
            expected,
            "state {state}"
        );
    }
}

// Focused contract tests complement the native matrix; they do not add to its
// native case count.
#[test]
fn fallen_tree_reads_and_writes_real_neighbours_across_negative_chunk_edges() {
    for origin in [(0, 65, 0), (31, 65, -16), (-17, 65, 15)] {
        let mut clear = World::new(true);
        assert!(fallen::place(
            &mut clear,
            &mut WorldgenRandom::from_seed(0),
            &fallen::BIRCH,
            origin
        ));
        let mut expected = BTreeMap::from([(origin, block::BIRCH_LOG)]);
        for distance in 2..=7 {
            expected.insert((origin.0, 65, origin.2 - distance), block::BIRCH_LOG + 1);
        }
        assert_eq!(clear.writes, expected);

        let obstacle = (origin.0, 65, origin.2 - 4);
        let mut blocked = World::new(true);
        blocked.initial.insert(obstacle, block::STONE);
        let mut random = WorldgenRandom::from_seed(0);
        assert!(fallen::place(
            &mut blocked,
            &mut random,
            &fallen::BIRCH,
            origin
        ));
        assert_eq!(blocked.writes, BTreeMap::from([(origin, block::BIRCH_LOG)]));
        assert!(blocked
            .reads
            .borrow()
            .iter()
            .any(|p| (p[0], p[1], p[2]) == obstacle));
        assert_eq!(random.next_i64(), 2160572956399813066); // Native continuation after the three geometry draws.
    }
}

#[test]
fn fallen_tree_ground_gaps_reset_and_reject_the_whole_horizontal_log() {
    for holes in [&[-3, -4, -6, -7][..], &[-3, -4, -5][..]] {
        let mut world = World::new(true);
        for &z in holes {
            world.initial.insert((0, 64, z), block::AIR);
        }
        let mut random = WorldgenRandom::from_seed(0);
        assert!(fallen::place(
            &mut world,
            &mut random,
            &fallen::BIRCH,
            (0, 65, 0)
        ));
        if holes.len() == 4 {
            assert_eq!(world.writes.len(), 7); // Two gaps of two, separated by support.
        } else {
            assert_eq!(
                world.writes,
                BTreeMap::from([((0, 65, 0), block::BIRCH_LOG)])
            );
            assert_eq!(random.next_i64(), 2160572956399813066);
        }
        assert!(world
            .support_queries
            .borrow()
            .contains(&(block::GRASS_BLOCK, (0, 65, -2))));
        assert!(world
            .support_queries
            .borrow()
            .contains(&(block::AIR, (0, 65, -3))));
    }
}

#[test]
fn fallen_tree_two_log_air_gap_uses_the_final_unclamped_search_position() {
    let mut world = World::new(false);
    let mut random = WorldgenRandom::from_seed(1);
    assert!(fallen::place(
        &mut world,
        &mut random,
        &fallen::OAK,
        (8, 65, 8)
    ));
    assert_eq!(world.writes.get(&(11, 60, 8)), Some(&(block::OAK_LOG - 1)));
    assert_eq!(world.writes.get(&(12, 60, 8)), Some(&(block::OAK_LOG - 1)));
    assert_eq!(world.writes.get(&(8, 65, 8)), Some(&block::OAK_LOG));
    assert_eq!(world.writes.len(), 6);
}

#[test]
fn fallen_tree_postprocessing_marks_two_non_air_positions_and_stops_at_air() {
    for first in [block::AIR, block::SHORT_GRASS] {
        let origin = (-1, 65, 16);
        let mut world = World::new(false);
        world.initial.insert(origin, block::BEDROCK);
        world.initial.insert((-1, 66, 16), first);
        world.initial.insert((-1, 67, 16), block::STONE);
        world.initial.insert((-1, 68, 16), block::STONE);
        assert!(fallen::place(
            &mut world,
            &mut WorldgenRandom::from_seed(0),
            &fallen::BIRCH,
            origin
        ));
        assert_eq!(world.writes, BTreeMap::from([(origin, block::BIRCH_LOG)]));
        let expected = if first == block::AIR {
            vec![]
        } else {
            vec![(-1, 66, 16), (-1, 67, 16)]
        };
        assert_eq!(world.marked, expected);
    }
}

#[test]
fn fallen_tree_rejected_writes_do_not_change_native_return_or_rng() {
    let mut world = World::new(true);
    world.reject_writes = true;
    let mut random = WorldgenRandom::from_seed(0);
    assert!(fallen::place(
        &mut world,
        &mut random,
        &fallen::BIRCH,
        (8, 65, 8)
    ));
    assert!(world.writes.is_empty());
    assert_eq!(world.attempts.len(), 7);
    assert_eq!(random.next_i64(), -3770814357236379989);
}
