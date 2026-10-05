//! Exact configured geodes versus the pinned JAR running on native ProtoChunks.
//! The world below models storage/guards and uses the existing tick queues;
//! feature geometry, state selection and RNG consumption come from the kernel.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use bcore_worldgen::block_predicate::{
    catalog, FeatureEnvironment, FeatureResult, RegistryEnvironment,
};
use bcore_worldgen::dripstone::CaveRandomState;
use bcore_worldgen::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::geode;
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::placement::ConfiguredFeatureDispatcher;
use bcore_worldgen::simplex::{WorldgenRandom, Xoroshiro128};
use bcore_worldgen::tick_queue::PreparedTickQueues;
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("../data/geode_26_1.json")).unwrap())
}

fn pos(value: &Value) -> Pos {
    (
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
        value[2].as_i64().unwrap() as i32,
    )
}

fn is_air(state: u32) -> bool {
    catalog().info(state).unwrap().is_air()
}

fn add_hash(hash: &mut Sha256, row: &[i32]) {
    for &value in row {
        hash.update(value.to_le_bytes());
    }
}

struct World {
    source: (i32, i32),
    background: u32,
    initial: BTreeMap<Pos, u32>,
    storage: BTreeMap<Pos, u32>,
    non_air: BTreeMap<Pos, i32>,
    read_radius: i32,
    write_radius: i32,
    reject: bool,
    calls: Vec<[i32; 6]>,
    origins: RefCell<Vec<[i32; 3]>>,
    reads: Cell<usize>,
    read_hash: RefCell<Sha256>,
    first_reads: RefCell<Vec<[i32; 4]>>,
    failed_read: Cell<Option<Pos>>,
    marks: BTreeMap<(i32, i32, i32), Vec<[i32; 3]>>,
    explicit_marks: Vec<Pos>,
    ticks: Vec<TickRequest>,
    queues: BTreeMap<(i32, i32), PreparedTickQueues>,
    // This is independent state metadata, not expected feature output. Native
    // BlockState.getPostProcessPos is captured for every registry state.
    postprocess: BTreeMap<u32, Pos>,
}

impl World {
    fn new(sample: &Value) -> Self {
        let origin = pos(&sample["origin"]);
        let background = sample["background"].as_u64().unwrap() as u32;
        let initial: BTreeMap<_, _> = sample["initial"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (pos(v), v[3].as_u64().unwrap() as u32))
            .collect();
        let mut non_air = BTreeMap::new();
        let default_count = if is_air(background) { 0 } else { 4096 };
        for (&(x, y, z), &state) in &initial {
            if !(-64..=319).contains(&y) {
                continue;
            }
            let count = non_air
                .entry((x >> 4, y >> 4, z >> 4))
                .or_insert(default_count);
            *count += i32::from(!is_air(state)) - i32::from(!is_air(background));
        }
        let postprocess = fixture()["predicates"]["postprocess"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row[0].as_u64().unwrap() as u32,
                    (
                        row[1].as_i64().unwrap() as i32,
                        row[2].as_i64().unwrap() as i32,
                        row[3].as_i64().unwrap() as i32,
                    ),
                )
            })
            .collect();
        Self {
            source: (origin.0 >> 4, origin.2 >> 4),
            background,
            initial,
            non_air,
            storage: BTreeMap::new(),
            read_radius: sample["read_radius"].as_i64().unwrap() as i32,
            write_radius: sample["write_radius"].as_i64().unwrap() as i32,
            reject: sample["reject_writes"].as_bool().unwrap(),
            calls: Vec::new(),
            origins: RefCell::new(Vec::new()),
            reads: Cell::new(0),
            read_hash: RefCell::new(Sha256::new()),
            first_reads: RefCell::new(Vec::new()),
            failed_read: Cell::new(None),
            marks: BTreeMap::new(),
            explicit_marks: Vec::new(),
            ticks: Vec::new(),
            queues: BTreeMap::new(),
            postprocess,
        }
    }

    fn distance(&self, p: Pos) -> i32 {
        ((p.0 >> 4) - self.source.0)
            .abs()
            .max(((p.2 >> 4) - self.source.1).abs())
    }

    fn raw(&self, pos: Pos) -> u32 {
        *self
            .storage
            .get(&pos)
            .or_else(|| self.initial.get(&pos))
            .unwrap_or(&self.background)
    }

    fn view(&self, p: Pos) -> Option<u32> {
        if self.distance(p) > self.read_radius {
            return None;
        }
        if !(-64..=319).contains(&p.1) {
            return Some(catalog().default_state("void_air").unwrap());
        }
        let default_count = if is_air(self.background) { 0 } else { 4096 };
        let count = *self
            .non_air
            .get(&(p.0 >> 4, p.1 >> 4, p.2 >> 4))
            .unwrap_or(&default_count);
        // ProtoChunk returns canonical AIR for any section with only air states.
        Some(if count == 0 {
            catalog().default_state("air").unwrap()
        } else {
            self.raw(p)
        })
    }

    fn mark(&mut self, (x, y, z): Pos) {
        if (-64..=319).contains(&y) {
            self.marks
                .entry((x >> 4, z >> 4, y >> 4))
                .or_default()
                .push([x, y, z]);
        }
    }

    fn assert_snapshot(&self, sample: &Value) {
        let name = sample["name"].as_str().unwrap();
        let expected: Vec<[i32; 6]> =
            serde_json::from_value(sample["write_calls"].clone()).unwrap();
        assert_trace(&self.calls, &expected, name);
        let mut hash = Sha256::new();
        let touched: BTreeSet<_> = self
            .calls
            .iter()
            .map(|row| (row[0], row[1], row[2]))
            .collect();
        let mut counts = BTreeMap::<String, usize>::new();
        for p in touched {
            let state = self.view(p).unwrap();
            add_hash(&mut hash, &[p.0, p.1, p.2, state as i32]);
            *counts.entry(state.to_string()).or_default() += 1;
        }
        assert_eq!(
            format!("{:x}", hash.finalize()),
            sample["final_sha256"].as_str().unwrap(),
            "{name}: native final blocks"
        );
        assert_eq!(
            serde_json::to_value(counts).unwrap(),
            sample["final_state_counts"],
            "{name}: native state histogram"
        );
        assert_eq!(
            self.reads.get(),
            sample["read_count"].as_u64().unwrap() as usize,
            "{name}: read count"
        );
        assert_eq!(
            format!("{:x}", self.read_hash.borrow().clone().finalize()),
            sample["read_sha256"].as_str().unwrap(),
            "{name}: ordered native reads"
        );
        assert_eq!(
            serde_json::to_value(&*self.first_reads.borrow()).unwrap(),
            sample["first_reads"],
            "{name}: initial reads"
        );
        assert_eq!(
            serde_json::to_value(&*self.origins.borrow()).unwrap(),
            sample["origins"],
            "{name}: origin guard calls"
        );
        assert_eq!(
            self.failed_read.get(),
            sample.get("failed_read").filter(|v| !v.is_null()).map(pos),
            "{name}: unavailable read"
        );
        assert!(
            self.explicit_marks.is_empty(),
            "GeodeFeature never explicitly marks postprocessing"
        );
        let marks: Vec<_> = self.marks.values().flatten().collect();
        assert_eq!(
            serde_json::to_value(marks).unwrap(),
            sample["marks"],
            "{name}: native ProtoChunk section marks"
        );
        let ticks: Vec<_> = self
            .ticks
            .iter()
            .map(|request| {
                let (id, fluid) = match request.target {
                    TickTarget::Block(id) => (id, 0),
                    TickTarget::Fluid(id) => (id, 1),
                };
                let [x, y, z] = request.block_pos;
                [x, y, z, id as i32, request.delay, fluid]
            })
            .collect();
        assert_eq!(
            serde_json::to_value(ticks).unwrap(),
            sample["tick_requests"],
            "{name}: ordered tick requests (including duplicates)"
        );
        let mut ticks = Vec::new();
        for queues in self.queues.values() {
            for (fluid, queue) in [(0, &queues.blocks), (1, &queues.fluids)] {
                for tick in queue.scheduled_ticks() {
                    let [x, y, z] = tick.block_pos;
                    ticks.push([
                        x,
                        y,
                        z,
                        tick.target as i32,
                        tick.delay,
                        fluid,
                        tick.priority.value(),
                    ]);
                }
            }
        }
        assert_eq!(
            serde_json::to_value(ticks).unwrap(),
            sample["native_ticks"],
            "{name}: native tick queue retention/order"
        );
    }
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, _: i32, _: i32) -> i32 {
        panic!("geode does not query heights")
    }
    fn get_block(&self, p: Pos) -> Option<u32> {
        let Some(state) = self.view(p) else {
            self.failed_read.set(Some(p));
            return None;
        };
        self.reads.set(self.reads.get() + 1);
        let row = [p.0, p.1, p.2, state as i32];
        add_hash(&mut self.read_hash.borrow_mut(), &row);
        if self.first_reads.borrow().len() < 24 {
            self.first_reads.borrow_mut().push(row);
        }
        Some(state)
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("geode must preserve explicit write flags")
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, _: Pos) -> u32 {
        panic!("geode does not query biomes")
    }
    fn feature_height(&self, _: FeatureHeightmap, _: i32, _: i32) -> i32 {
        panic!("geode does not query heights")
    }
    fn can_write_feature(&self, p: Pos) -> bool {
        self.origins.borrow_mut().push([p.0, p.1, p.2]);
        self.write_radius >= 0 && self.distance(p) <= self.write_radius
    }
    fn set_feature_block(&mut self, p: Pos, state: u32, flags: i32) -> bool {
        // Actual WorldGenRegion does not apply a height guard for fresh chunks.
        let accepted =
            !self.reject && self.write_radius >= 0 && self.distance(p) <= self.write_radius;
        self.calls
            .push([p.0, p.1, p.2, state as i32, flags, i32::from(accepted)]);
        if accepted {
            if (-64..=319).contains(&p.1) {
                let difference = i32::from(!is_air(state)) - i32::from(!is_air(self.raw(p)));
                let default_count = if is_air(self.background) { 0 } else { 4096 };
                *self
                    .non_air
                    .entry((p.0 >> 4, p.1 >> 4, p.2 >> 4))
                    .or_insert(default_count) += difference;
                self.storage.insert(p, state);
            }
            if flags & 16 == 0 {
                if let Some(&(x, y, z)) = self.postprocess.get(&state) {
                    self.mark((p.0 + x, p.1 + y, p.2 + z));
                }
            }
        }
        accepted
    }
    fn mark_feature_postprocessing(&mut self, p: Pos) {
        self.explicit_marks.push(p);
        self.mark(p);
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        self.ticks.push(request);
        let [x, _, z] = request.block_pos;
        self.queues
            .entry((x >> 4, z >> 4))
            .or_default()
            .schedule_request(request)
    }
}

#[derive(Default)]
struct Providers(CaveRandomState);
impl ConfiguredFeatureDispatcher for Providers {
    fn next_gaussian(&mut self, random: &mut WorldgenRandom) -> FeatureResult<f64> {
        Ok(self.0.next_gaussian(random))
    }
    fn place_configured(
        &mut self,
        _: &Value,
        _: Option<&str>,
        _: &mut dyn FeatureWorld,
        _: &mut WorldgenRandom,
        _: Pos,
        _: &dyn FeatureEnvironment,
    ) -> FeatureResult<bool> {
        panic!("geode must not dispatch child features")
    }
}

fn assert_trace(actual: &[[i32; 6]], expected: &[[i32; 6]], name: &str) {
    if actual != expected {
        let index = actual
            .iter()
            .zip(expected)
            .position(|(a, b)| a != b)
            .unwrap_or(actual.len().min(expected.len()));
        panic!(
            "{name}: first differing write {index}: actual {:?}, native {:?}; counts {}/{}",
            actual.get(index),
            expected.get(index),
            actual.len(),
            expected.len()
        );
    }
}

fn check(sample: &Value) {
    let name = sample["name"].as_str().unwrap();
    let document = &fixture()["configs"][sample["config"].as_str().unwrap()];
    let feature_type = document["type"].as_str().unwrap();
    geode::check_configured(feature_type, &document["config"]).unwrap();
    let mut world = World::new(sample);
    let mut providers = Providers::default();
    let seed = sample["feature_seed"].as_i64().unwrap();
    let mut random = WorldgenRandom::new(seed);
    let mut native_source = Xoroshiro128::new(seed);
    if sample["advance"].as_bool().unwrap() {
        random.next_int(17);
        random.next_float();
        random.next_int(1073741825);
    }
    let expected_calls: Vec<[i32; 6]> =
        serde_json::from_value(sample["write_calls"].clone()).unwrap();
    let mut count = 0;
    for (index, result) in sample["results"].as_array().unwrap().iter().enumerate() {
        if index > 0 && sample["reseed"].as_bool().unwrap() {
            random.set_seed(seed.wrapping_add(index as i64));
            native_source = Xoroshiro128::new(seed.wrapping_add(index as i64));
        }
        let placed = geode::place_configured_with(
            feature_type,
            &document["config"],
            &mut world,
            &mut random,
            pos(&sample["origin"]),
            sample["world_seed"].as_i64().unwrap(),
            &RegistryEnvironment,
            &mut providers,
        );
        if result.is_null() {
            assert!(
                matches!(placed, Err(FeatureError::MissingData(_))),
                "{name}: expected native missing-read failure, got {placed:?}"
            );
        } else {
            assert_eq!(
                placed,
                Ok(result.as_bool().unwrap()),
                "{name}: placement result {index}"
            );
        }
        let checkpoint = &sample["checkpoints"][index];
        let writes = checkpoint[1].as_u64().unwrap() as usize;
        assert_trace(&world.calls, &expected_calls[..writes], name);
        assert_eq!(
            world.ticks.len(),
            checkpoint[2].as_u64().unwrap() as usize,
            "{name}: tick count at attempt {index}"
        );
        assert_eq!(
            world.reads.get(),
            checkpoint[3].as_u64().unwrap() as usize,
            "{name}: read count at attempt {index}"
        );
        let next_count = checkpoint[0].as_u64().unwrap();
        for _ in count..next_count {
            native_source.next_long();
        }
        count = next_count;
        let mut actual = random.source.clone();
        let mut expected = native_source.clone();
        for _ in 0..2 {
            assert_eq!(
                actual.next_long(),
                expected.next_long(),
                "{name}: exact native RNG draw count after attempt {index}"
            );
        }
    }
    world.assert_snapshot(sample);
    assert_eq!(
        random.next_long(),
        sample["next_i64"].as_i64().unwrap(),
        "{name}: RNG continuation"
    );
    assert_eq!(
        count,
        sample["rng_count"].as_u64().unwrap(),
        "{name}: total draws"
    );
}

#[test]
fn native_geodes_every_write_read_rng_and_side_effect() {
    let samples = fixture()["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 116);
    for sample in samples {
        check(sample);
    }
}

#[test]
fn native_geode_registry_predicates_cover_every_state() {
    let mut total = 0;
    for row in fixture()["predicates"]["ranges"].as_array().unwrap() {
        let first = row[0].as_u64().unwrap() as u32;
        let end = row[1].as_u64().unwrap() as u32;
        assert_eq!(first, total);
        for state in first..end {
            let info = catalog().info(state).unwrap();
            let block = catalog().block(state).unwrap().1;
            let full = info.fluid_amount == 8;
            let source = info.fluid == catalog().fluids["minecraft:water"]
                || info.fluid == catalog().fluids["minecraft:lava"];
            let grow = info.is_air() || (catalog().is_block(state, "water").unwrap() && full);
            let facing = block.properties.iter().any(|(name, values)| {
                name == "facing" && values.len() == 6 && values.iter().any(|v| v == "up")
            });
            let flags = u32::from(info.is_air())
                | u32::from(
                    catalog()
                        .in_block_tag(state, "geode_invalid_blocks")
                        .unwrap(),
                ) << 1
                | u32::from(
                    catalog()
                        .in_block_tag(state, "features_cannot_replace")
                        .unwrap(),
                ) << 2
                | u32::from(grow) << 3
                | u32::from(source) << 4
                | u32::from(full) << 5
                | u32::from(facing) << 6
                | u32::from(block.property(state, "waterlogged").is_some()) << 7;
            assert_eq!(
                flags,
                row[2].as_u64().unwrap() as u32,
                "native geode predicate flags for state {state}"
            );
            assert_eq!(
                info.fluid,
                row[3].as_u64().unwrap() as u32,
                "fluid type for {state}"
            );
            assert_eq!(
                info.fluid_amount as u64,
                row[4].as_u64().unwrap(),
                "fluid amount for {state}"
            );
        }
        total = end;
    }
    assert_eq!(total, 29_873);
}

#[test]
fn geode_fixture_provenance_and_builtin_config_are_pinned() {
    let data = fixture();
    assert_eq!(data["minecraft"], "26.1");
    assert_eq!(data["protocol"], 775);
    assert_eq!(
        data["jar_sha256"],
        "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
    );
    assert_eq!(
        data["configs"]["amethyst_geode"],
        *catalog().configured("amethyst_geode").unwrap()
    );
    let sources: [(&str, &[u8]); 6] = [
        (
            "scripts/TreeReference.java",
            include_bytes!("../../../scripts/TreeReference.java"),
        ),
        (
            "scripts/NativeEntityLevel.java",
            include_bytes!("../../../scripts/NativeEntityLevel.java"),
        ),
        (
            "scripts/NativeWorldgenRegistries.java",
            include_bytes!("../../../scripts/NativeWorldgenRegistries.java"),
        ),
        (
            "scripts/TreeEffectReference.java",
            include_bytes!("../../../scripts/TreeEffectReference.java"),
        ),
        (
            "scripts/geode-reference/GeodeReference.java",
            include_bytes!("../../../scripts/geode-reference/GeodeReference.java"),
        ),
        (
            "scripts/geode-reference/capture.py",
            include_bytes!("../../../scripts/geode-reference/capture.py"),
        ),
    ];
    for (path, bytes) in sources {
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            data["source_sha256"][path].as_str().unwrap(),
            "source provenance for {path}"
        );
    }
}

#[test]
fn native_geode_corpus_exercises_crystals_ticks_marks_and_precision() {
    let samples = fixture()["samples"].as_array().unwrap();
    let find = |name: &str| samples.iter().find(|s| s["name"] == name).unwrap();
    let crystal = |state| catalog().block(state).unwrap().1.class == "AmethystClusterBlock";
    let mut directions = BTreeSet::new();
    for sample in samples.iter().filter(|s| s["config"] == "all_placements") {
        for row in sample["write_calls"].as_array().unwrap() {
            let state = row[3].as_u64().unwrap() as u32;
            if crystal(state) {
                directions.insert(
                    catalog()
                        .block(state)
                        .unwrap()
                        .1
                        .property(state, "facing")
                        .unwrap(),
                );
            }
        }
    }
    assert_eq!(
        directions,
        BTreeSet::from(["down", "up", "north", "south", "west", "east"])
    );
    for (name, wet) in [
        ("fill_water", "true"),
        ("fill_falling_water", "false"),
        ("fill_falling_flowing_water", "false"),
    ] {
        let states: Vec<_> = find(name)["write_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row[3].as_u64().unwrap() as u32)
            .filter(|&s| crystal(s))
            .collect();
        assert!(!states.is_empty(), "{name}: exercise actual crystals");
        for state in states {
            assert_eq!(
                catalog()
                    .block(state)
                    .unwrap()
                    .1
                    .property(state, "waterlogged"),
                Some(wet),
                "{name}"
            );
        }
    }
    for sample in samples {
        let ticks = sample["tick_requests"].as_array().unwrap().len();
        let marks = sample["marks"].as_array().unwrap().len();
        if ticks > 0 || marks > 0 {
            println!(
                "{}: {ticks} requests, {} native ticks, {marks} marks",
                sample["name"],
                sample["native_ticks"].as_array().unwrap().len()
            );
        }
    }
    assert!(
        samples.iter().any(|sample| {
            sample["tick_requests"].as_array().unwrap().len()
                > sample["native_ticks"].as_array().unwrap().len()
        }),
        "corpus must exercise duplicate fluid tick requests"
    );
    assert!(!find("placement_brown_mushroom")["marks"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(find("denied_origin")["rng_count"], 0);
    assert_eq!(find("allowed_boundary_320")["results"][0], true);
    assert!(find("allowed_boundary_320")["rng_count"].as_u64().unwrap() > 0);
    for field in 0..3 {
        assert_ne!(
            find(&format!("double_threshold_{field}_0"))["final_sha256"],
            find(&format!("double_threshold_{field}_1"))["final_sha256"],
            "double probability boundary {field} must exercise the strict comparison"
        );
    }
    let writes: usize = samples
        .iter()
        .map(|s| s["write_calls"].as_array().unwrap().len())
        .sum();
    let ticks: usize = samples
        .iter()
        .map(|s| s["tick_requests"].as_array().unwrap().len())
        .sum();
    let marks: usize = samples
        .iter()
        .map(|s| s["marks"].as_array().unwrap().len())
        .sum();
    println!("native geode corpus: {} cases, {writes} write attempts, {ticks} tick requests, {marks} retained marks", samples.len());
}

#[test]
fn unsupported_and_invalid_geodes_fail_before_random_or_world_effects() {
    let sample = &fixture()["samples"][0];
    let mut world = World::new(sample);
    let mut random = WorldgenRandom::new(17);
    let document = &fixture()["configs"]["amethyst_geode"];
    assert!(geode::supports("geode"));
    assert!(geode::supports("minecraft:geode"));
    assert!(!geode::supports("custom:geode"));
    assert!(matches!(
        geode::place_configured(
            "custom:geode",
            &document["config"],
            &mut world,
            &mut random,
            pos(&sample["origin"]),
            42,
            &RegistryEnvironment
        ),
        Err(FeatureError::Unsupported(_))
    ));
    let mut config = document["config"].clone();
    config["blocks"]["inner_placements"] = serde_json::json!([]);
    assert!(matches!(
        geode::place_configured(
            "geode",
            &config,
            &mut world,
            &mut random,
            pos(&sample["origin"]),
            42,
            &RegistryEnvironment
        ),
        Err(FeatureError::InvalidConfig(_))
    ));
    let mut config = document["config"].clone();
    config["blocks"]["filling_provider"]["type"] = serde_json::json!("unsupported_geode_provider");
    assert!(matches!(
        geode::check_configured("geode", &config),
        Err(FeatureError::Unsupported(_))
    ));
    assert!(world.calls.is_empty());
    assert!(world.origins.borrow().is_empty());
    assert_eq!(world.reads.get(), 0);
    assert_eq!(random.next_long(), WorldgenRandom::new(17).next_long());
}
