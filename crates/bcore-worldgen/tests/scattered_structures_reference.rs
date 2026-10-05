use bcore_core::ChunkPos;
use bcore_worldgen::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::simplex::WorldgenRandom;
use bcore_worldgen::structure::jigsaw::HeightContext;
use bcore_worldgen::structure::placement::large_feature_random;
use bcore_worldgen::structure::scattered::{
    assemble, assemble_with_sea_level, for_chunk, for_chunk_with_sea_level, place_in_chunk,
    reorient_chest, ScatteredCatalog, ScatteredContainer, ScatteredEffect, ScatteredKind,
    ScatteredPieceData, ScatteredStart, ScatteredStatus, ScatteredWorld,
};
use bcore_worldgen::structure::template::{BoundingBox, Nbt};
use bcore_worldgen::structure::template_pool::StructureAssets;
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/scattered_structures_26_1.json")).unwrap()
    })
}

fn jungle_fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/scattered_jungle_temple_26_1.json")).unwrap()
    })
}

fn native_nbt(value: &Value) -> Nbt {
    serde_json::from_value(value["nbt"].clone()).unwrap()
}
fn n(value: &Value) -> i32 {
    value.as_i64().unwrap() as i32
}
fn pos(value: &Value) -> Pos {
    (n(&value[0]), n(&value[1]), n(&value[2]))
}
fn bounds(value: &Value) -> BoundingBox {
    BoundingBox {
        min: (n(&value[0]), n(&value[1]), n(&value[2])),
        max: (n(&value[3]), n(&value[4]), n(&value[5])),
    }
}
fn block(name: &str) -> u32 {
    StructureAssets::bundled()
        .blocks
        .default_state(name)
        .unwrap()
}

struct World {
    terrain: String,
    min_y: i32,
    source: ChunkPos,
    overrides: BTreeMap<Pos, u32>,
    denied: BTreeSet<Pos>,
    states: BTreeMap<Pos, u32>,
    entities: BTreeMap<Pos, ScatteredContainer>,
    writes: Vec<Value>,
    effects: Vec<Value>,
    ticks: Vec<Value>,
    marks: BTreeMap<Pos, usize>,
    heights: RefCell<Vec<Value>>,
    suppress_entities: bool,
    reject_effects: bool,
    unavailable: Option<Pos>,
}

impl World {
    fn new(row: &Value) -> Self {
        Self {
            terrain: row["terrain"].as_str().unwrap_or("flat").into(),
            min_y: row["min_y"].as_i64().unwrap_or(-64) as i32,
            source: ChunkPos::new(0, 0),
            overrides: row["overrides"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| (pos(a), n(&a[3]) as u32))
                .collect(),
            denied: row["denied"]
                .as_array()
                .into_iter()
                .flatten()
                .map(pos)
                .collect(),
            states: BTreeMap::new(),
            entities: BTreeMap::new(),
            writes: Vec::new(),
            effects: Vec::new(),
            ticks: Vec::new(),
            marks: BTreeMap::new(),
            heights: RefCell::new(Vec::new()),
            suppress_entities: row["suppress_block_entities"] == true,
            reject_effects: false,
            unavailable: None,
        }
    }

    fn reset_pass(&mut self, source: ChunkPos) {
        self.source = source;
        self.writes.clear();
        self.effects.clear();
        self.ticks.clear();
        self.marks.clear();
        self.heights.borrow_mut().clear();
    }

    fn at(&self, p: Pos) -> u32 {
        if p.1 < self.min_y || p.1 > 319 {
            return block("air");
        }
        if let Some(state) = self.states.get(&p).or_else(|| self.overrides.get(&p)) {
            return *state;
        }
        let name = match self.terrain.as_str() {
            "beach" => {
                if p.1 < 53 {
                    "stone"
                } else if p.1 < 63 {
                    "sand"
                } else if p.1 < 65 {
                    "water"
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
            "slope" => {
                if p.1 < 60 + (p.0 + 2 * p.2).rem_euclid(9) {
                    "stone"
                } else {
                    "air"
                }
            }
            "dirt" => {
                if p.1 < 64 {
                    "dirt"
                } else {
                    "air"
                }
            }
            "deepslate" => {
                if p.1 < 64 {
                    "deepslate"
                } else {
                    "air"
                }
            }
            "void" => "air",
            "flat" => {
                if p.1 < 64 {
                    "stone"
                } else {
                    "air"
                }
            }
            other => panic!("unknown fixture terrain {other}"),
        };
        block(name)
    }

    fn height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        let data = ScatteredCatalog::bundled();
        for y in (self.min_y..=319).rev() {
            let props = data.block_properties(self.at((x, y, z))).unwrap();
            let matches = match kind {
                FeatureHeightmap::MotionBlockingNoLeaves => props.motion_blocking_no_leaves,
                FeatureHeightmap::OceanFloorWg | FeatureHeightmap::OceanFloor => props.ocean_floor,
                FeatureHeightmap::WorldSurfaceWg | FeatureHeightmap::WorldSurface => !props.air,
                _ => panic!("unexpected scattered heightmap {kind:?}"),
            };
            if matches {
                return y + 1;
            }
        }
        self.min_y
    }
}

impl OreWorld for World {
    fn get_block(&self, p: Pos) -> Option<u32> {
        (self.unavailable != Some(p)).then(|| self.at(p))
    }
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.height(FeatureHeightmap::OceanFloorWg, x, z)
    }
    fn set_block(&mut self, _: Pos, _: u32) -> bool {
        panic!("structure placement must retain setBlock flags")
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, _: Pos) -> u32 {
        panic!("scattered blocks must not query zoomed biomes")
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        let result = self.height(kind, x, z);
        let name = match kind {
            FeatureHeightmap::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
            FeatureHeightmap::OceanFloorWg => "OCEAN_FLOOR_WG",
            FeatureHeightmap::WorldSurfaceWg => "WORLD_SURFACE_WG",
            _ => panic!("unexpected structure height request"),
        };
        self.heights.borrow_mut().push(json!([name, x, z, result]));
        result
    }
    fn can_write_feature(&self, p: Pos) -> bool {
        (self.min_y..=319).contains(&p.1)
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
        self.writes.push(json!([p.0, p.1, p.2, state, flags]));
        if registry.flags(state).unwrap() & 8 == 0 {
            self.entities.remove(&p);
        } else if !self.suppress_entities
            && (!self.entities.contains_key(&p)
                || registry.state(previous).unwrap().name != registry.state(state).unwrap().name)
        {
            self.entities
                .insert(p, ScatteredContainer::for_state(state, p).unwrap().unwrap());
        }
        true
    }
    fn mark_feature_postprocessing(&mut self, p: Pos) {
        self.effects.push(json!(["mark", p.0, p.1, p.2]));
        *self.marks.entry(p).or_default() += 1;
    }
    fn schedule_feature_tick(&mut self, tick: TickRequest) -> bool {
        let (kind, id) = match tick.target {
            TickTarget::Block(id) => ("block", id),
            TickTarget::Fluid(id) => ("fluid", id),
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
        self.min_y
    }
    fn has_structure_block_entity(&self, p: Pos, id: &str) -> bool {
        self.entities.get(&p).is_some_and(|be| be.id == id)
    }
    fn apply_scattered_effect(&mut self, effect: ScatteredEffect) -> Result<(), FeatureError> {
        if self.reject_effects {
            return Err(FeatureError::Unsupported(
                "test consumer has no mob/finalization queue".into(),
            ));
        }
        match effect {
            ScatteredEffect::LootTable {
                pos: p,
                block_entity,
                table,
                seed,
            } => {
                let be = self
                    .entities
                    .get_mut(&p)
                    .expect("native container exists before loot draw");
                assert_eq!(be.id, block_entity);
                be.set_loot_table(table.clone(), seed);
                self.effects
                    .push(json!(["loot", p.0, p.1, p.2, block_entity, table, seed]));
            }
            ScatteredEffect::MobRequest(request) => {
                let p = request.block_pos;
                assert_eq!(
                    request.position,
                    [f64::from(p.0) + 0.5, f64::from(p.1), f64::from(p.2) + 0.5]
                );
                assert_eq!(request.rotation, [0.0, 0.0]);
                assert!(request.finalize && request.persistence_required);
                assert_eq!(request.spawn_reason, "STRUCTURE");
                self.effects
                    .push(json!(["mob_request", request.mob.name(), [p.0, p.1, p.2]]));
            }
        }
        Ok(())
    }
}

#[test]
fn scattered_native_admission_configuration_and_legacy_rng() {
    let fixture = fixture();
    assert_eq!(fixture["jar_sha256"], StructureAssets::JAR_SHA256);
    let catalog = ScatteredCatalog::bundled();
    let mut actual_overworld_starts = BTreeMap::<_, usize>::new();
    for row in fixture["admission"].as_array().unwrap() {
        let kind = ScatteredKind::from_name(row["kind"].as_str().unwrap()).unwrap();
        let config = catalog.config(kind);
        let seed = row["seed"].as_i64().unwrap();
        let chunk = ChunkPos::new(n(&row["chunk"][0]), n(&row["chunk"][1]));
        let potential = config.placement.potential_chunk(seed, chunk);
        assert_eq!(json!([potential.x, potential.z]), row["potential"], "{row}");
        assert_eq!(
            json!(config.placement.is_candidate(seed, chunk)),
            row["candidate"],
            "{row}"
        );
        let first_free = |heightmap, x, z| {
            assert_eq!(heightmap, kind.admission_heightmap());
            assert_eq!((x, z), (chunk.x * 16 + 8, chunk.z * 16 + 8));
            n(&row["first_free"])
        };
        let heights = HeightContext {
            min_y: -64,
            max_y: 319,
            first_free: &first_free,
        };
        let noise_biome = n(&row["noise_biome"]) as u32;
        let mut random = large_feature_random(seed, chunk);
        let built = assemble(kind, chunk, &heights, &mut random, |p| {
            assert_eq!(
                p,
                (
                    chunk.x * 16 + 8,
                    n(&row["first_free"]) - 1,
                    chunk.z * 16 + 8
                )
            );
            config.biomes.contains(&noise_biome)
        })
        .unwrap();
        assert_eq!(
            json!(random.next_long()),
            row["next_i64"],
            "assembly RNG {row}"
        );
        assert_eq!(json!(built.is_some()), row["biome_admitted"]);
        if let Some(start) = built {
            assert_eq!(
                start.to_nbt(),
                native_nbt(&row["assembled"]),
                "assembly {row}"
            );
            assert_eq!(
                json!(start.generation_point.unwrap()),
                row["generation_point"]
            );
            assert!(start.valid_for(kind.name(), chunk));
        }
        let admitted = for_chunk(kind, seed, chunk, &heights, |_| noise_biome).unwrap();
        let starts = row["starts"].as_array().unwrap();
        assert_eq!(
            usize::from(admitted.is_some()),
            starts.len(),
            "native createStructures {row}"
        );
        if let Some(mut start) = admitted {
            assert_eq!(start.to_nbt(), native_nbt(&starts[0]));
            assert_eq!(
                json!(start.reference_bounds().as_array()),
                starts[0]["reference_bounds"]
            );
            if row["biome"] == "overworld" {
                *actual_overworld_starts.entry(kind).or_default() += 1;
            }
        }
        assert_eq!(row["retained"], true);
    }
    assert_eq!(actual_overworld_starts.values().sum::<usize>(), 4);
    assert_eq!(actual_overworld_starts.len(), 2);
    assert_eq!(
        (
            catalog
                .config(ScatteredKind::BuriedTreasure)
                .decoration_step,
            catalog
                .config(ScatteredKind::BuriedTreasure)
                .structure_index
        ),
        (3, 0)
    );
    assert_eq!(
        (
            catalog.config(ScatteredKind::SwampHut).decoration_step,
            catalog.config(ScatteredKind::SwampHut).structure_index
        ),
        (4, 20)
    );
    assert_eq!(
        catalog.config(ScatteredKind::BuriedTreasure).locate_offset,
        (9, 0, 9)
    );
    assert_eq!(
        catalog.config(ScatteredKind::SwampHut).native_config["spawn_overrides"]["monster"]
            ["bounding_box"],
        "piece"
    );
}

#[test]
fn scattered_native_blocks_ordered_effects_clips_nbt_and_rng() {
    let (writes, loot, requests, no_substrates) =
        verify_placements(fixture()["placements"].as_array().unwrap());
    assert!(writes > 2500 && loot >= 10 && requests >= 20);
    assert_eq!(no_substrates, 6);
}

fn verify_placements(rows: &[Value]) -> (usize, usize, usize, usize) {
    let mut total_writes = 0;
    let mut total_loot = 0;
    let mut total_requests = 0;
    let mut orientations = BTreeSet::new();
    let mut no_substrates = 0;
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let seed = row["seed"].as_i64().unwrap();
        let mut start = ScatteredStart::from_nbt(&native_nbt(&row["initial"])).unwrap();
        assert_eq!(start.to_nbt(), native_nbt(&row["initial"]));
        assert_eq!(
            json!(start.reference_bounds().as_array()),
            row["reference_bounds"]
        );
        if let Some(orientation) = start.piece.orientation {
            orientations.insert(orientation.name());
        }
        let mut world = World::new(row);
        for (index, pass) in row["passes"].as_array().unwrap().iter().enumerate() {
            let source = ChunkPos::new(n(&pass["source"][0]), n(&pass["source"][1]));
            world.reset_pass(source);
            let config = ScatteredCatalog::bundled().config(start.kind);
            let mut random = WorldgenRandom::new(0);
            let decoration = random.set_decoration_seed(seed, source.x * 16, source.z * 16);
            assert_eq!(json!(decoration), pass["decoration_seed"]);
            random.set_feature_seed(decoration, config.structure_index, config.decoration_step);
            let report =
                place_in_chunk(&mut start, &mut world, &mut random, bounds(&pass["clip"])).unwrap();
            assert_eq!(
                json!(random.next_long()),
                pass["next_i64"],
                "{name} pass {index} RNG"
            );
            let effects = pass["effects"].as_array().unwrap();
            for (event, (actual, expected)) in world.effects.iter().zip(effects).enumerate() {
                assert_eq!(
                    actual, expected,
                    "{name} pass {index} ordered effect {event}"
                );
            }
            assert_eq!(
                world.effects.len(),
                effects.len(),
                "{name} pass {index} effect count"
            );
            assert_eq!(
                json!(world.heights.borrow().as_slice()),
                pass["height_queries"],
                "{name} pass {index} heights"
            );
            assert_eq!(
                report.blocks_written,
                pass["write_count"].as_u64().unwrap() as usize
            );
            assert_eq!(
                report.block_attempts,
                world.effects.iter().filter(|e| e[0] == "block").count()
            );
            assert_eq!(
                report.loot_assignments,
                world.effects.iter().filter(|e| e[0] == "loot").count()
            );
            assert_eq!(
                report.mob_requests,
                world
                    .effects
                    .iter()
                    .filter(|e| e[0] == "mob_request")
                    .count()
            );
            let states = pass["states"].as_array().unwrap();
            assert_eq!(
                world.states.len(),
                states.len(),
                "{name} pass {index} state count"
            );
            for (((x, y, z), state), expected) in world.states.iter().zip(states) {
                assert_eq!(
                    json!([x, y, z, state]),
                    *expected,
                    "{name} pass {index} final blocks"
                );
            }
            assert_eq!(
                json!(world
                    .marks
                    .iter()
                    .map(|(p, count)| json!([p.0, p.1, p.2, count]))
                    .collect::<Vec<_>>()),
                pass["marks"],
                "{name} pass {index} marks"
            );
            assert_eq!(
                json!(world.ticks),
                pass["ticks"],
                "{name} pass {index} ticks"
            );
            assert_eq!(
                world.entities.len(),
                pass["block_entities"].as_array().unwrap().len()
            );
            for entry in pass["block_entities"].as_array().unwrap() {
                let entity = &world.entities[&pos(&entry["pos"])];
                assert_eq!(
                    entity.full_data(),
                    native_nbt(&entry["full"]),
                    "{name} block entity"
                );
                assert_eq!(entity.update_data(), native_nbt(&entry["update"]));
            }
            assert_eq!(
                start.to_nbt(),
                native_nbt(&pass["after"]),
                "{name} pass {index} retained piece"
            );
            assert_eq!(
                json!(start.reference_bounds().as_array()),
                pass["cached_reference_bounds"]
            );
            let mut loaded = ScatteredStart::from_nbt(&start.to_nbt()).unwrap();
            assert_eq!(
                json!(loaded.reference_bounds().as_array()),
                pass["reloaded_reference_bounds"],
                "{name} reloaded bounds"
            );
            let retained: ScatteredStart =
                serde_json::from_str(&serde_json::to_string(&start).unwrap()).unwrap();
            assert_eq!(retained, start, "saved live-holder snapshot");
            if report.status == ScatteredStatus::NoTreasureSubstrate {
                assert_eq!(report.blocks_written, 0);
                no_substrates += 1;
            }
            total_writes += report.blocks_written;
            total_loot += report.loot_assignments;
            total_requests += report.mob_requests;
        }
    }
    assert_eq!(
        orientations.len(),
        4,
        "all four native horizontal orientations"
    );
    (total_writes, total_loot, total_requests, no_substrates)
}

#[test]
fn scattered_jungle_native_admission_corners_sea_level_and_rng() {
    let kind = ScatteredKind::JungleTemple;
    let config = ScatteredCatalog::bundled().config(kind);
    assert_eq!(
        (
            config.structure_id,
            config.decoration_step,
            config.structure_index
        ),
        (7, 4, 4)
    );
    let mut real_starts = 0;
    let mut height_rejections = 0;
    for row in jungle_fixture()["admission"].as_array().unwrap() {
        let chunk = ChunkPos::new(n(&row["chunk"][0]), n(&row["chunk"][1]));
        let seed = row["seed"].as_i64().unwrap();
        let sea_level = n(&row["sea_level"]);
        let potential = config.placement.potential_chunk(seed, chunk);
        assert_eq!(json!([potential.x, potential.z]), row["potential"]);
        assert_eq!(
            json!(config.placement.is_candidate(seed, chunk)),
            row["candidate"]
        );
        let mut input_heights: BTreeMap<_, _> = row["corner_first_free"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| ((n(&r[0]), n(&r[1])), n(&r[2])))
            .collect();
        let lowest = input_heights.values().min().unwrap() - 1;
        assert_eq!(json!(lowest), row["native_lowest_y"]);
        let center = (chunk.x * 16 + 8, chunk.z * 16 + 8);
        input_heights.insert(center, n(&row["first_free"]));
        let calls = RefCell::new(Vec::new());
        let first_free = |map, x, z| {
            assert_eq!(map, FeatureHeightmap::WorldSurfaceWg);
            calls.borrow_mut().push((x, z));
            input_heights[&(x, z)]
        };
        let heights = HeightContext {
            min_y: -64,
            max_y: 319,
            first_free: &first_free,
        };
        let mut random = large_feature_random(seed, chunk);
        let valid = |_: Pos| config.biomes.contains(&(n(&row["noise_biome"]) as u32));
        let assembled =
            assemble_with_sea_level(kind, chunk, &heights, sea_level, &mut random, valid).unwrap();
        assert_eq!(
            json!(random.next_long()),
            row["next_i64"],
            "jungle admission RNG {row}"
        );
        assert_eq!(
            json!(assembled.is_some()),
            row["biome_admitted"],
            "jungle native admission {row}"
        );
        let mut expected_queries = vec![
            (chunk.x * 16, chunk.z * 16),
            (chunk.x * 16, chunk.z * 16 + 15),
            (chunk.x * 16 + 12, chunk.z * 16),
            (chunk.x * 16 + 12, chunk.z * 16 + 15),
        ];
        if lowest >= sea_level {
            expected_queries.push(center);
        } else {
            height_rejections += 1;
        }
        assert_eq!(*calls.borrow(), expected_queries);
        if let Some(start) = assembled {
            assert_eq!(start.to_nbt(), native_nbt(&row["assembled"]));
            assert_eq!(
                json!(start.generation_point.unwrap()),
                row["generation_point"]
            );
        }
        let admitted = for_chunk_with_sea_level(kind, seed, chunk, &heights, sea_level, |_| {
            n(&row["noise_biome"]) as u32
        })
        .unwrap();
        let native_starts = row["starts"].as_array().unwrap();
        assert_eq!(usize::from(admitted.is_some()), native_starts.len());
        if let Some(mut start) = admitted {
            assert_eq!(start.to_nbt(), native_nbt(&native_starts[0]));
            assert_eq!(
                json!(start.reference_bounds().as_array()),
                native_starts[0]["reference_bounds"]
            );
            if row["biome"] == "overworld" {
                real_starts += 1;
            }
        }
        assert_eq!(row["retained"], true);
    }
    assert_eq!(real_starts, 2);
    assert!(height_rejections >= 2);
}

#[test]
fn scattered_jungle_native_masonry_traps_loot_flags_and_rng() {
    let (writes, loot, requests, no_substrates) =
        verify_placements(jungle_fixture()["placements"].as_array().unwrap());
    assert!(writes > 35_000 && loot >= 40);
    assert_eq!((requests, no_substrates), (0, 0));
}

#[test]
fn scattered_reference_membership_matches_native_create_references() {
    let rows = jungle_fixture()["references"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    let mut comparisons = 0;
    for row in rows {
        let mut start = ScatteredStart::from_nbt(&native_nbt(&row["start"])).unwrap();
        assert_eq!(
            json!(start.reference_bounds().as_array()),
            row["reference_bounds"]
        );
        for target in row["targets"].as_array().unwrap() {
            let chunk = ChunkPos::new(n(&target["chunk"][0]), n(&target["chunk"][1]));
            let expected = target["sources"].as_array().unwrap();
            assert_eq!(
                start.references_chunk(chunk),
                !expected.is_empty(),
                "{row} {target}"
            );
            if !expected.is_empty() {
                assert_eq!(expected, &vec![json!(start.source)]);
            }
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 216);
}

#[test]
fn scattered_native_chest_reorientation_including_early_exit() {
    let rows = fixture()["chest_orientations"].as_array().unwrap();
    assert_eq!(rows.len(), 80);
    for row in rows {
        let world = World::new(row);
        assert_eq!(
            json!(reorient_chest(&world, pos(&row["position"])).unwrap()),
            row["state"],
            "{row}"
        );
        assert!(world.effects.is_empty());
    }
}

#[test]
fn scattered_requires_available_world_reads_and_an_effect_consumer() {
    let row = &fixture()["placements"][0];
    let mut start = ScatteredStart::from_nbt(&native_nbt(&row["initial"])).unwrap();
    let pass = &row["passes"][0];
    let mut world = World::new(row);
    world.source = ChunkPos::new(n(&pass["source"][0]), n(&pass["source"][1]));
    world.reject_effects = true;
    assert!(matches!(
        place_in_chunk(
            &mut start,
            &mut world,
            &mut WorldgenRandom::new(0),
            bounds(&pass["clip"])
        ),
        Err(FeatureError::Unsupported(_))
    ));
    assert!(matches!(
        start.piece.data,
        ScatteredPieceData::SwampHut {
            spawned_witch: true,
            spawned_cat: false,
            ..
        }
    ));
    let mut world = World::new(&json!({"overrides":[]}));
    world.unavailable = Some((0, 64, -1));
    assert!(matches!(
        reorient_chest(&world, (0, 64, 0)),
        Err(FeatureError::MissingData(_))
    ));
    assert!(matches!(
        ScatteredKind::from_name("minecraft:igloo"),
        Err(FeatureError::Unsupported(_))
    ));
}

#[test]
fn scattered_container_defaults_and_zero_seed_match_native_serialization() {
    let data: Value =
        serde_json::from_str(include_str!("../data/scattered_containers_26_1.json")).unwrap();
    assert_eq!(data["jar_sha256"], StructureAssets::JAR_SHA256);
    for (name, native) in data["containers"]["defaults"].as_object().unwrap() {
        let container = ScatteredContainer::for_state(block(name), (0, 0, 0))
            .unwrap()
            .unwrap();
        assert_eq!(container.full_data(), native_nbt(&native["full"]));
        assert_eq!(container.update_data(), native_nbt(&native["update"]));
        assert_eq!(json!(container.type_id), native["type_id"]);
    }
    let cases = data["containers"]["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 10);
    for row in cases {
        let mut container =
            ScatteredContainer::for_state(n(&row["state"]) as u32, pos(&row["pos"]))
                .unwrap()
                .unwrap();
        container.set_loot_table(
            row["table"].as_str().unwrap().into(),
            row["seed"].as_i64().unwrap(),
        );
        assert_eq!(container.full_data(), native_nbt(&row["full"]));
        assert_eq!(container.update_data(), native_nbt(&row["update"]));
        let round_trip: ScatteredContainer =
            serde_json::from_str(&serde_json::to_string(&container).unwrap()).unwrap();
        assert_eq!(round_trip, container);
    }
}
