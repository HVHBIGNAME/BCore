//! Native fixtures exercise the exported library implementation.
use bcore_worldgen::structure::{jigsaw, placement, processors, template, template_pool};
use bcore_worldgen::{feature_world, simplex, tick_request};

use bcore_core::ChunkPos;
use bcore_worldgen::ore::OreWorld;
use feature_world::{FeatureHeightmap, FeatureWorld, Pos};
use serde_json::Value;
use std::collections::BTreeMap;
use template::{BoundingBox, Mirror, Nbt, PlacementSettings, Rotation};
use template_pool::StructureAssets;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../data/jigsaw_reference_26_1.json")).unwrap()
}

fn pos(value: &Value) -> Pos {
    let v: [i32; 3] = serde_json::from_value(value.clone()).unwrap();
    (v[0], v[1], v[2])
}

fn rotation(value: &Value) -> Rotation {
    Rotation::ALL
        .into_iter()
        .find(|r| r.name() == value.as_str().unwrap())
        .unwrap()
}

fn bounds(value: &Value) -> Option<BoundingBox> {
    let v = value.as_array().unwrap();
    if v.is_empty() {
        return None;
    }
    Some(BoundingBox::new(
        (
            v[0].as_i64().unwrap() as i32,
            v[1].as_i64().unwrap() as i32,
            v[2].as_i64().unwrap() as i32,
        ),
        (
            v[3].as_i64().unwrap() as i32,
            v[4].as_i64().unwrap() as i32,
            v[5].as_i64().unwrap() as i32,
        ),
    ))
}

#[test]
fn complete_assemblies_match_native_pieces_junctions_nbt_and_rng() {
    let assets = StructureAssets::bundled();
    let reference = fixture();
    assert_eq!(reference["jar_sha256"], StructureAssets::JAR_SHA256);
    let cases = reference["assembly"].as_array().unwrap();
    assert_eq!(cases.len(), 24);
    assert_eq!(
        cases
            .iter()
            .map(|case| case["pieces"].as_array().unwrap().len())
            .sum::<usize>(),
        2527
    );
    for sample in cases {
        let name = sample["structure"].as_str().unwrap();
        let seed = sample["seed"].as_i64().unwrap();
        let chunk = ChunkPos::new(
            sample["chunk"][0].as_i64().unwrap() as i32,
            sample["chunk"][1].as_i64().unwrap() as i32,
        );
        let first_free = sample["first_free_height"].as_i64().unwrap() as i32;
        let height = |_, _, _| first_free;
        let heights = jigsaw::HeightContext {
            min_y: -64,
            max_y: 319,
            first_free: &height,
        };
        let config = jigsaw::JigsawConfig::from_assets(assets, name).unwrap();
        let mut random = placement::large_feature_random(seed, chunk);
        let y = config.start_height.sample(&heights, &mut random).unwrap();
        let result = jigsaw::assemble(
            assets,
            &config,
            (chunk.x * 16, y, chunk.z * 16),
            &heights,
            &mut random,
            &BTreeMap::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            result.generation_point,
            pos(&sample["generation_point"]),
            "{name} seed={seed} generation point"
        );
        let expected = sample["pieces"].as_array().unwrap();
        assert_eq!(
            result.pieces.len(),
            expected.len(),
            "{name} seed={seed} piece count"
        );
        for (index, (piece, expected)) in result.pieces.iter().zip(expected).enumerate() {
            assert_eq!(
                piece.origin,
                pos(&expected["pos"]),
                "{name} seed={seed} piece={index} origin"
            );
            assert_eq!(
                piece.bounds,
                bounds(&expected["bounds"]).unwrap(),
                "{name} seed={seed} piece={index} bounds"
            );
            assert_eq!(
                piece.rotation,
                rotation(&expected["rotation"]),
                "{name} seed={seed} piece={index} rotation"
            );
            let expected_nbt: Nbt = serde_json::from_value(expected["nbt"].clone()).unwrap();
            assert_eq!(
                piece.to_nbt(),
                expected_nbt,
                "{name} seed={seed} piece={index} NBT"
            );
        }
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{name} seed={seed} RNG"
        );
    }
}

struct World {
    terrain: String,
    fill: u32,
    states: BTreeMap<Pos, u32>,
    writes: Vec<[i32; 5]>,
    ticks: Vec<tick_request::TickRequest>,
    marks: BTreeMap<Pos, usize>,
}

impl World {
    fn new(terrain: &str) -> Self {
        let registry = &StructureAssets::bundled().blocks;
        let fill = registry
            .default_state(match terrain {
                "water" => "water",
                "air" => "air",
                _ => "stone",
            })
            .unwrap();
        Self {
            terrain: terrain.to_owned(),
            fill,
            states: BTreeMap::new(),
            writes: Vec::new(),
            ticks: Vec::new(),
            marks: BTreeMap::new(),
        }
    }

    fn surface(&self, x: i32, z: i32) -> i32 {
        if self.terrain == "slope" {
            64 + (x + 2 * z).rem_euclid(5)
        } else {
            65
        }
    }
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.surface(x, z)
    }
    fn get_block(&self, p: Pos) -> Option<u32> {
        let registry = &StructureAssets::bundled().blocks;
        if !(-64..=319).contains(&p.1) {
            return Some(registry.default_state("void_air").unwrap());
        }
        if let Some(state) = self.states.get(&p) {
            return Some(*state);
        }
        if self.terrain == "protected" && (p.0 * 3 + p.1 + p.2 * 5).rem_euclid(17) == 0 {
            return Some(registry.default_state("bedrock").unwrap());
        }
        if self.terrain == "slope" {
            return Some(if p.1 < self.surface(p.0, p.2) {
                registry.default_state("stone").unwrap()
            } else {
                registry.default_state("air").unwrap()
            });
        }
        if self.terrain == "city_cave" {
            return Some(
                registry
                    .default_state(if p.1 < -50 || p.1 >= -20 {
                        "stone"
                    } else {
                        "air"
                    })
                    .unwrap(),
            );
        }
        Some(self.fill)
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.set_feature_block(pos, state, 2)
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, _: Pos) -> u32 {
        0
    }
    fn feature_height(&self, _: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.surface(x, z)
    }
    fn can_write_feature(&self, p: Pos) -> bool {
        (-64..=319).contains(&p.1)
    }
    fn set_feature_block(&mut self, p: Pos, state: u32, flags: i32) -> bool {
        if !self.can_write_feature(p) {
            return false;
        }
        self.states.insert(p, state);
        self.writes.push([p.0, p.1, p.2, state as i32, flags]);
        true
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        *self.marks.entry(pos).or_default() += 1;
    }
    fn schedule_feature_tick(&mut self, request: tick_request::TickRequest) -> bool {
        self.ticks.push(request);
        true
    }
}

fn digest<const N: usize>(rows: impl IntoIterator<Item = [i32; N]>) -> String {
    let mut context = md5::Context::new();
    for row in rows {
        for value in row {
            context.consume(value.to_le_bytes());
        }
    }
    format!("{:x}", context.compute())
}

#[test]
fn actual_template_placement_matches_native_states_writes_loot_nbt_ticks_and_rng() {
    let assets = StructureAssets::bundled();
    let reference = fixture();
    let cases = reference["placement"].as_array().unwrap();
    assert_eq!(cases.len(), 28);
    for sample in cases {
        let name = sample["template"].as_str().unwrap();
        let mut world = World::new(sample["terrain"].as_str().unwrap());
        let mut random = simplex::WorldgenRandom::new(sample["seed"].as_i64().unwrap());
        let mut processors = vec![
            processors::Processor::ignore_structure(),
            processors::Processor::JigsawReplacement,
        ];
        if sample["processors"] != "" {
            processors.extend(assets.resolve_processors(&sample["processors"]).unwrap());
        }
        if sample["projection"] == "terrain_matching" {
            processors.push(processors::Processor::Gravity {
                heightmap: FeatureHeightmap::WorldSurfaceWg,
                offset: -1,
            });
        }
        let settings = PlacementSettings {
            rotation: rotation(&sample["rotation"]),
            mirror: serde_json::from_value(sample["mirror"].clone()).unwrap(),
            clip: bounds(&sample["clip"]),
            processors,
            ignore_entities: true,
            ..PlacementSettings::default()
        };
        let description = format!(
            "{name} {:?}/{:?} {}",
            settings.rotation, settings.mirror, sample["terrain"]
        );
        let result = assets
            .template(name)
            .unwrap()
            .place_in_world(
                &mut world,
                &mut random,
                &assets.blocks,
                pos(&sample["origin"]),
                pos(&sample["reference"]),
                &settings,
            )
            .unwrap();
        assert_eq!(
            result.placed,
            sample["placed"].as_bool().unwrap(),
            "{description}"
        );
        assert_eq!(
            world.writes.len(),
            sample["write_count"].as_u64().unwrap() as usize,
            "{description} write count"
        );
        assert_eq!(
            world.states.len(),
            sample["state_count"].as_u64().unwrap() as usize,
            "{description} state count"
        );
        assert_eq!(
            digest(world.writes.iter().copied()),
            sample["writes_md5"].as_str().unwrap(),
            "{description} write order"
        );
        assert_eq!(
            digest(
                world
                    .states
                    .iter()
                    .map(|(p, state)| [p.0, p.1, p.2, *state as i32])
            ),
            sample["states_md5"].as_str().unwrap(),
            "{description} final states"
        );
        let expected = sample["block_entities"].as_array().unwrap();
        assert_eq!(
            result.block_entities.len(),
            expected.len(),
            "{description} block entities"
        );
        for entity in expected {
            let pos = pos(&entity["pos"]);
            let expected: Nbt = serde_json::from_value(entity["nbt"].clone()).unwrap();
            assert_eq!(
                result.block_entities[&pos].full_data(),
                expected,
                "{description} block entity at {pos:?}"
            );
        }
        let ticks: Vec<_> = world
            .ticks
            .iter()
            .map(|t| match t.target {
                tick_request::TickTarget::Fluid(id) => {
                    serde_json::json!([t.block_pos, id, t.delay])
                }
                _ => panic!("unexpected block tick"),
            })
            .collect();
        assert_eq!(
            ticks,
            *sample["ticks"].as_array().unwrap(),
            "{description} ticks"
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{description} RNG"
        );
        assert!(
            world.marks.is_empty(),
            "known-shape template without sculk: {description}"
        );
    }
}

#[test]
fn full_ancient_city_block_path_matches_native_structure_start_including_sculk() {
    let assets = StructureAssets::bundled();
    let reference = fixture();
    let cases = reference["cities"]
        .as_array()
        .expect("native full-city captures");
    assert_eq!(cases.len(), 6);
    for sample in cases {
        let seed = sample["seed"].as_i64().unwrap();
        let config = jigsaw::JigsawConfig::from_assets(assets, "ancient_city").unwrap();
        let heights = jigsaw::HeightContext {
            min_y: -64,
            max_y: 319,
            first_free: &|_, _, _| 65,
        };
        let start =
            jigsaw::generate_start(assets, &config, seed, ChunkPos::new(0, 0), &heights, |_| {
                true
            })
            .unwrap()
            .unwrap();
        let clip = bounds(&sample["clip"]).unwrap();
        let description = format!("city seed={seed} clip={clip:?}");
        if sample["entire"] == true {
            assert_eq!(
                start.reference_bounds(),
                Some(clip),
                "{description} reference bounds"
            );
        }
        let mut world = World::new(sample["terrain"].as_str().unwrap());
        let mut random = simplex::WorldgenRandom::new(sample["placement_seed"].as_i64().unwrap());
        let result = jigsaw::place_ancient_city_in_chunk(
            assets,
            &start,
            &mut world,
            &mut random,
            clip,
            seed,
        )
        .unwrap();
        assert_eq!(
            world.writes.len(),
            sample["write_count"].as_u64().unwrap() as usize,
            "{description} write count"
        );
        assert_eq!(
            world.states.len(),
            sample["state_count"].as_u64().unwrap() as usize,
            "{description} state count"
        );
        assert_eq!(
            digest(world.writes.iter().copied()),
            sample["writes_md5"].as_str().unwrap(),
            "{description} write order"
        );
        assert_eq!(
            digest(
                world
                    .states
                    .iter()
                    .map(|(p, state)| [p.0, p.1, p.2, *state as i32])
            ),
            sample["states_md5"].as_str().unwrap(),
            "{description} final states"
        );
        let expected = sample["block_entities"].as_array().unwrap();
        assert_eq!(
            result.block_entities.len(),
            expected.len(),
            "{description} block entities"
        );
        for entity in expected {
            let pos = pos(&entity["pos"]);
            let expected: Nbt = serde_json::from_value(entity["nbt"].clone()).unwrap();
            assert_eq!(
                result.block_entities[&pos].full_data(),
                expected,
                "{description} block entity at {pos:?}"
            );
        }
        let ticks: Vec<_> = world
            .ticks
            .iter()
            .map(|t| match t.target {
                tick_request::TickTarget::Fluid(id) => {
                    serde_json::json!([t.block_pos, id, t.delay])
                }
                _ => panic!("unexpected block tick"),
            })
            .collect();
        assert_eq!(
            ticks,
            *sample["ticks"].as_array().unwrap(),
            "{description} ticks"
        );
        let marks: Vec<_> = world
            .marks
            .iter()
            .map(|(p, count)| serde_json::json!([p.0, p.1, p.2, count]))
            .collect();
        assert_eq!(
            marks,
            *sample["marks"].as_array().unwrap(),
            "{description} postprocessing"
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{description} RNG"
        );
    }
}

#[test]
fn block_and_entity_transforms_match_native_mirrors_and_nonzero_pivots() {
    let reference = fixture();
    let cases = reference["transforms"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for sample in cases {
        let r = rotation(&sample["rotation"]);
        let m: Mirror = serde_json::from_value(sample["mirror"].clone()).unwrap();
        assert_eq!(
            template::transform_block((-5, 4, 7), m, r, (2, 9, -3)),
            pos(&sample["block"])
        );
        let expected: [f64; 3] = serde_json::from_value(sample["entity"].clone()).unwrap();
        assert_eq!(
            template::transform_entity([-4.75, 4.125, 7.75], m, r, (2, 9, -3)),
            expected
        );
    }
}

#[test]
fn ancient_city_anchor_moves_the_template_down_to_its_native_floor() {
    let assets = StructureAssets::bundled();
    let config = jigsaw::JigsawConfig::from_assets(assets, "ancient_city").unwrap();
    let heights = jigsaw::HeightContext {
        min_y: -64,
        max_y: 319,
        first_free: &|_, _, _| 65,
    };
    let start = jigsaw::generate_start(assets, &config, 0, ChunkPos::new(0, 0), &heights, |_| true)
        .unwrap()
        .unwrap();
    assert_eq!(start.generation_point.1, -27);
    assert_eq!(start.pieces[0].bounds.min.1, -52);
    assert!(
        jigsaw::generate_start(assets, &config, 0, ChunkPos::new(0, 0), &heights, |_| false)
            .unwrap()
            .is_none()
    );
}

fn trail_assets() -> &'static StructureAssets {
    static ASSETS: std::sync::OnceLock<StructureAssets> = std::sync::OnceLock::new();
    ASSETS.get_or_init(|| {
        StructureAssets::from_json(include_str!("../data/jigsaw_trail_assets_26_1_v2.json"))
            .unwrap()
    })
}

#[test]
fn trail_ruins_native_assemblies_preserve_pieces_and_random_continuation() {
    let assets = trail_assets();
    let fixture: Value =
        serde_json::from_str(include_str!("../data/jigsaw_trail_reference_26_1.json")).unwrap();
    assert_eq!(fixture["jar_sha256"], StructureAssets::JAR_SHA256);
    let heights = jigsaw::HeightContext {
        min_y: -64,
        max_y: 319,
        first_free: &|_, _, _| 65,
    };
    let config = jigsaw::JigsawConfig::from_assets(assets, "trail_ruins").unwrap();
    for sample in fixture["assembly"].as_array().unwrap() {
        let seed = sample["seed"].as_i64().unwrap();
        let chunk = ChunkPos::new(
            sample["chunk"][0].as_i64().unwrap() as i32,
            sample["chunk"][1].as_i64().unwrap() as i32,
        );
        let mut random = placement::large_feature_random(seed, chunk);
        let y = config.start_height.sample(&heights, &mut random).unwrap();
        let start = jigsaw::assemble(
            assets,
            &config,
            (chunk.x * 16, y, chunk.z * 16),
            &heights,
            &mut random,
            &BTreeMap::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            start.generation_point,
            pos(&sample["generation_point"]),
            "seed {seed}"
        );
        let pieces: Vec<Nbt> = sample["pieces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| serde_json::from_value(p["nbt"].clone()).unwrap())
            .collect();
        assert_eq!(
            start.pieces.iter().map(|p| p.to_nbt()).collect::<Vec<_>>(),
            pieces,
            "seed {seed}"
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "seed {seed}"
        );
    }
}

#[test]
fn trail_ruins_native_archaeology_matches_full_and_clipped_placement() {
    let assets = trail_assets();
    let fixture: Value =
        serde_json::from_str(include_str!("../data/jigsaw_trail_reference_26_1.json")).unwrap();
    let heights = jigsaw::HeightContext {
        min_y: -64,
        max_y: 319,
        first_free: &|_, _, _| 65,
    };
    let config = jigsaw::JigsawConfig::from_assets(assets, "trail_ruins").unwrap();
    for sample in fixture["placement"].as_array().unwrap() {
        let seed = sample["seed"].as_i64().unwrap();
        let start =
            jigsaw::generate_start(assets, &config, seed, ChunkPos::new(0, 0), &heights, |_| {
                true
            })
            .unwrap()
            .unwrap();
        let clip = bounds(&sample["clip"]).unwrap();
        let label = format!("trail ruins seed={seed} clip={clip:?}");
        if sample["entire"] == true {
            assert_eq!(start.reference_bounds(), Some(clip), "{label}");
        }
        let mut world = World::new(sample["terrain"].as_str().unwrap());
        let mut random = simplex::WorldgenRandom::new(sample["placement_seed"].as_i64().unwrap());
        let result = jigsaw::place_in_chunk_with_features(
            assets,
            &start,
            &mut world,
            &mut random,
            clip,
            seed,
            &mut |name, _, _, _| {
                Err(feature_world::FeatureError::Unsupported(format!(
                    "unexpected trail feature {name}"
                )))
            },
        )
        .unwrap();
        assert_eq!(
            world.writes.len(),
            sample["write_count"].as_u64().unwrap() as usize,
            "{label} write count"
        );
        assert_eq!(
            digest(world.writes.iter().copied()),
            sample["writes_md5"].as_str().unwrap(),
            "{label} write order"
        );
        assert_eq!(
            world.states.len(),
            sample["state_count"].as_u64().unwrap() as usize,
            "{label} state count"
        );
        assert_eq!(
            digest(world.states.iter().map(|(p, s)| [p.0, p.1, p.2, *s as i32])),
            sample["states_md5"].as_str().unwrap(),
            "{label} final states"
        );
        assert_eq!(
            result.block_entities.len(),
            sample["block_entities"].as_array().unwrap().len(),
            "{label} block entities"
        );
        for entity in sample["block_entities"].as_array().unwrap() {
            let p = pos(&entity["pos"]);
            let expected: Nbt = serde_json::from_value(entity["nbt"].clone()).unwrap();
            assert_eq!(
                result.block_entities[&p].full_data(),
                expected,
                "{label} entity at {p:?}"
            );
        }
        let ticks: Vec<_> = world
            .ticks
            .iter()
            .map(|t| match t.target {
                tick_request::TickTarget::Fluid(id) => {
                    serde_json::json!([t.block_pos, id, t.delay])
                }
                _ => panic!("unexpected block tick"),
            })
            .collect();
        assert_eq!(ticks, *sample["ticks"].as_array().unwrap(), "{label} ticks");
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{label} RNG"
        );
    }
}
