//! Native trial chambers through the public generic assembly/template APIs.
use bcore_core::ChunkPos;
use bcore_worldgen::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use bcore_worldgen::ore::OreWorld;
use bcore_worldgen::simplex::WorldgenRandom;
use bcore_worldgen::structure::pool_alias::PoolAliasBindings;
use bcore_worldgen::structure::template::{BoundingBox, Nbt, PlacementResult, TemplateBlockEntity};
use bcore_worldgen::structure::template_pool::StructureAssets;
use bcore_worldgen::structure::{jigsaw, placement};
use bcore_worldgen::tick_request::{TickRequest, TickTarget};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

fn assets() -> &'static StructureAssets {
    static ASSETS: OnceLock<StructureAssets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        StructureAssets::from_json(include_str!(
            "../data/pool_alias_trial_combined_assets_26_1_v2.json"
        ))
        .unwrap()
    })
}

fn fixture() -> &'static Value {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../data/pool_alias_trial_reference_26_1_v2.json"
        ))
        .unwrap()
    })
}

fn pos(value: &Value) -> Pos {
    let [x, y, z]: [i32; 3] = serde_json::from_value(value.clone()).unwrap();
    (x, y, z)
}

fn bounds(value: &Value) -> BoundingBox {
    let [x0, y0, z0, x1, y1, z1]: [i32; 6] = serde_json::from_value(value.clone()).unwrap();
    BoundingBox::new((x0, y0, z0), (x1, y1, z1))
}

fn config(value: &Value) -> (jigsaw::JigsawConfig, PoolAliasBindings) {
    let aliases = PoolAliasBindings::from_structure_json(value).unwrap();
    let config = jigsaw::JigsawConfig::from_json(value).unwrap();
    assert_eq!(config.pool_aliases, aliases);
    (config, aliases)
}

fn heights() -> jigsaw::HeightContext<'static> {
    jigsaw::HeightContext {
        min_y: -64,
        max_y: 319,
        first_free: &|_, _, _| 65,
    }
}

fn generate(seed: i64, chunk: ChunkPos) -> jigsaw::JigsawStart {
    let assets = assets();
    let (config, aliases) = config(&assets.structure_configs["minecraft:trial_chambers"]);
    let heights = heights();
    let mut random = placement::large_feature_random(seed, chunk);
    let y = config.start_height.sample(&heights, &mut random).unwrap();
    let origin = (chunk.x.wrapping_mul(16), y, chunk.z.wrapping_mul(16));
    jigsaw::assemble(
        assets,
        &config,
        origin,
        &heights,
        &mut random,
        &aliases.resolve(seed, origin).unwrap(),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn trial_assemblies_match_native_complete_piece_order_junctions_nbt_limits_and_rng() {
    let reference = fixture();
    assert_eq!(reference["jar_sha256"], StructureAssets::JAR_SHA256);
    let samples = reference["assembly"].as_array().unwrap();
    assert_eq!(samples.len(), 13);
    assert_eq!(
        samples
            .iter()
            .map(|s| s["pieces"].as_array().unwrap().len())
            .sum::<usize>(),
        1950
    );
    for sample in samples {
        let (config, aliases) = config(&sample["config"]);
        let seed = sample["seed"].as_i64().unwrap();
        let [x, z]: [i32; 2] = serde_json::from_value(sample["chunk"].clone()).unwrap();
        let label = format!("{} seed={seed} chunk={x},{z}", sample["label"]);
        if sample["label"] == "registered" {
            assert_eq!(
                (
                    config.max_depth,
                    config.max_horizontal_distance,
                    config.max_vertical_distance
                ),
                (20, 116, 116)
            );
            assert_eq!(
                (
                    config.padding_bottom,
                    config.padding_top,
                    config.waterlogging
                ),
                (10, 10, false)
            );
        }
        let mut random = placement::large_feature_random(seed, ChunkPos::new(x, z));
        let heights = heights();
        let y = config.start_height.sample(&heights, &mut random).unwrap();
        let origin = (x.wrapping_mul(16), y, z.wrapping_mul(16));
        assert_eq!(
            origin,
            pos(&sample["start_origin"]),
            "{label} sampled alias origin"
        );
        let resolved = aliases.resolve(seed, origin).unwrap();
        for (query, expected) in sample["aliases"]["lookup"].as_object().unwrap() {
            assert_eq!(
                resolved.get(query).unwrap_or(query),
                expected.as_str().unwrap(),
                "{label} alias {query}"
            );
        }
        let start =
            jigsaw::assemble(assets(), &config, origin, &heights, &mut random, &resolved).unwrap();
        assert_eq!(
            jigsaw::generate_start(
                assets(),
                &config,
                seed,
                ChunkPos::new(x, z),
                &heights,
                |_| true
            )
            .unwrap(),
            start,
            "{label} production alias resolution"
        );
        assert_eq!(
            start.is_some(),
            sample["admitted"].as_bool().unwrap(),
            "{label} padding admission"
        );
        assert_eq!(
            random.next_long(),
            sample["next_i64"].as_i64().unwrap(),
            "{label} assembly RNG"
        );
        let Some(start) = start else { continue };
        assert_eq!(
            start.generation_point,
            pos(&sample["generation_point"]),
            "{label} generation point"
        );
        let expected = sample["pieces"].as_array().unwrap();
        assert_eq!(start.pieces.len(), expected.len(), "{label} piece count");
        if let Some(reference_bounds) = sample.get("reference_bounds") {
            assert_eq!(
                start.reference_bounds(),
                Some(bounds(reference_bounds)),
                "{label} reference bounds"
            );
        } else {
            assert!(start.reference_bounds().is_none());
        }
        for (index, (piece, expected)) in start.pieces.iter().zip(expected).enumerate() {
            assert_eq!(
                piece.origin,
                pos(&expected["pos"]),
                "{label} piece {index} origin"
            );
            assert_eq!(
                piece.bounds,
                bounds(&expected["bounds"]),
                "{label} piece {index} bounds"
            );
            assert_eq!(
                piece.rotation.name(),
                expected["rotation"].as_str().unwrap(),
                "{label} piece {index} rotation"
            );
            assert_eq!(
                piece.ground_level_delta,
                expected["ground_level_delta"].as_i64().unwrap() as i32,
                "{label} piece {index} ground delta"
            );
            assert!(
                piece.depth <= config.max_depth + 1,
                "{label} piece {index} depth"
            );
            assert_eq!(
                piece.to_nbt(),
                serde_json::from_value::<Nbt>(expected["nbt"].clone()).unwrap(),
                "{label} piece {index} typed NBT / ordered junctions"
            );
        }
    }
}

#[test]
fn captured_trial_assets_and_all_alias_targets_are_complete_and_typed() {
    let trial =
        StructureAssets::from_json(include_str!("../data/pool_alias_trial_assets_26_1.json"))
            .unwrap();
    let combined = assets();
    assert_eq!(trial.templates.len(), 191);
    assert_eq!(combined.templates.len(), 817);
    assert_eq!(trial.blocks.len(), 29873);
    for (name, source) in &trial.templates {
        let target = combined.template(name).unwrap();
        assert_eq!(source.size, target.size, "{name}");
        assert_eq!(
            source.palettes(),
            target.palettes(),
            "{name} palettes / typed NBT"
        );
        assert_eq!(source.entities, target.entities, "{name} entities");
        assert!(
            source.entities.is_empty(),
            "trial fixture must not hide an entity callback"
        );
    }
    for (name, source) in &trial.pools {
        let target = combined.pool(name).unwrap();
        assert_eq!(source.fallback, target.fallback, "{name} fallback");
        assert_eq!(source.weight(), target.weight(), "{name} weights");
        assert_eq!(
            source
                .elements
                .iter()
                .map(|e| (&e.element, e.weight))
                .collect::<Vec<_>>(),
            target
                .elements
                .iter()
                .map(|e| (&e.element, e.weight))
                .collect::<Vec<_>>(),
            "{name} elements"
        );
        assert!(
            source
                .elements
                .iter()
                .all(|e| !e.element.contains_features()),
            "{name} unsupported placed-feature callback"
        );
    }
    let aliases = PoolAliasBindings::from_structure_json(
        &trial.structure_configs["minecraft:trial_chambers"],
    )
    .unwrap();
    assert_eq!(aliases.all_targets().len(), 13);
    for name in aliases.all_targets() {
        trial.pool(name).unwrap();
    }
    let configs = fixture()["trial_spawner_configs"].as_object().unwrap();
    assert_eq!(configs.len(), 28);
    let mut references = BTreeSet::new();
    for sample in fixture()["block_entity_loads"].as_array().unwrap() {
        let template = trial
            .template(sample["template"].as_str().unwrap())
            .unwrap();
        let input: Nbt = serde_json::from_value(sample["input"]["nbt"].clone()).unwrap();
        let position = pos(&sample["local_pos"]);
        assert!(
            template
                .palettes()
                .iter()
                .flatten()
                .any(|info| info.pos == position
                    && info.state == sample["state"].as_u64().unwrap() as u32
                    && info.nbt.as_ref() == Some(&input)),
            "native data-fixed template payload {}",
            sample["template"]
        );
        let kind = &trial.blocks.block_entities[sample["block"].as_str().unwrap()];
        assert!(
            !kind.randomizable,
            "trial spawners/vaults must not receive container loot RNG"
        );
        assert_eq!(kind.type_id, sample["type_id"].as_u64().unwrap() as u32);
        if sample["block"] == "minecraft:trial_spawner" {
            for key in ["normal_config", "ominous_config"] {
                let reference = input.get(key).and_then(Nbt::string).unwrap();
                assert!(
                    configs.contains_key(reference),
                    "missing native config {reference}"
                );
                references.insert(reference.to_owned());
                let typed: Nbt = serde_json::from_value(configs[reference]["nbt"].clone()).unwrap();
                assert_eq!(
                    typed.to_json(),
                    configs[reference]["json"],
                    "{reference} typed codec"
                );
            }
        }
    }
    assert_eq!(references.len(), 28);
}

struct World {
    terrain: String,
    fill: u32,
    states: BTreeMap<Pos, u32>,
    writes: Vec<[i32; 5]>,
    ticks: Vec<TickRequest>,
    marks: BTreeMap<Pos, usize>,
}

impl World {
    fn new(terrain: &str) -> Self {
        let fill = assets()
            .blocks
            .default_state(if terrain == "water" { "water" } else { "stone" })
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
}

impl OreWorld for World {
    fn ocean_floor_wg(&self, _: i32, _: i32) -> i32 {
        65
    }
    fn get_block(&self, p: Pos) -> Option<u32> {
        let blocks = &assets().blocks;
        if !(-64..=319).contains(&p.1) {
            return Some(blocks.default_state("void_air").unwrap());
        }
        if let Some(state) = self.states.get(&p) {
            return Some(*state);
        }
        if self.terrain == "protected" && (p.0 * 3 + p.1 + p.2 * 5).rem_euclid(17) == 0 {
            return Some(blocks.default_state("bedrock").unwrap());
        }
        Some(self.fill)
    }
    fn set_block(&mut self, p: Pos, state: u32) -> bool {
        self.set_feature_block(p, state, 2)
    }
}

impl FeatureWorld for World {
    fn feature_biome(&self, _: Pos) -> u32 {
        0
    }
    fn feature_height(&self, _: FeatureHeightmap, _: i32, _: i32) -> i32 {
        65
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
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        self.ticks.push(request);
        true
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        *self.marks.entry(pos).or_default() += 1;
    }
}

fn digest<const N: usize>(rows: impl IntoIterator<Item = [i32; N]>) -> String {
    let mut md5 = md5::Context::new();
    for row in rows {
        for value in row {
            md5.consume(value.to_le_bytes());
        }
    }
    format!("{:x}", md5.compute())
}

fn place(sample: &Value) -> (World, PlacementResult, i64) {
    assert_eq!(
        sample["status"], "complete",
        "unsupported native callback: {}",
        sample["error"]
    );
    let seed = sample["seed"].as_i64().unwrap();
    let start = generate(seed, ChunkPos::new(0, 0));
    let clip = bounds(&sample["clip"]);
    if sample["entire"] == true {
        assert_eq!(start.reference_bounds(), Some(clip));
    }
    let mut world = World::new(sample["terrain"].as_str().unwrap());
    let mut random = WorldgenRandom::new(sample["placement_seed"].as_i64().unwrap());
    let result = jigsaw::place_in_chunk_with_features(
        assets(),
        &start,
        &mut world,
        &mut random,
        clip,
        seed,
        &mut |name, _, _, _| {
            Err(FeatureError::Unsupported(format!(
                "trial placed-feature callback {name}"
            )))
        },
    )
    .unwrap();
    assert!(
        result.entities.is_empty(),
        "trial entity factory/finalization is not implemented by this fixture"
    );
    (world, result, random.next_long())
}

#[test]
fn native_trial_full_and_clipped_placement_preserves_blocks_ordered_writes_ticks_and_rng() {
    let cases = fixture()["placement"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for sample in cases {
        let label = format!(
            "seed={} terrain={} entire={} chunk={}",
            sample["seed"], sample["terrain"], sample["entire"], sample["chunk"]
        );
        let (world, result, tail) = place(sample);
        assert_eq!(
            world.writes.len(),
            sample["write_count"].as_u64().unwrap() as usize,
            "{label} writes"
        );
        assert_eq!(
            digest(world.writes),
            sample["writes_md5"].as_str().unwrap(),
            "{label} write order"
        );
        assert_eq!(
            world.states.len(),
            sample["state_count"].as_u64().unwrap() as usize,
            "{label} states"
        );
        assert_eq!(
            digest(
                world
                    .states
                    .iter()
                    .map(|(p, state)| [p.0, p.1, p.2, *state as i32])
            ),
            sample["states_md5"].as_str().unwrap(),
            "{label} final blocks"
        );
        assert_eq!(
            tail,
            sample["next_i64"].as_i64().unwrap(),
            "{label} placement RNG"
        );
        let ticks: Vec<_> = world
            .ticks
            .iter()
            .map(|tick| match tick.target {
                TickTarget::Fluid(id) => serde_json::json!([tick.block_pos, id, tick.delay]),
                _ => panic!("unexpected block tick"),
            })
            .collect();
        assert_eq!(ticks, *sample["ticks"].as_array().unwrap(), "{label} ticks");
        let marks: Vec<_> = world
            .marks
            .iter()
            .map(|(p, count)| serde_json::json!([p.0, p.1, p.2, count]))
            .collect();
        assert_eq!(marks, *sample["marks"].as_array().unwrap(), "{label} marks");
        assert_eq!(
            result.block_entities.len(),
            sample["block_entities"].as_array().unwrap().len(),
            "{label} block-entity count"
        );
        for entity in sample["block_entities"].as_array().unwrap() {
            let p = pos(&entity["pos"]);
            let actual = &result.block_entities[&p];
            let expected: Nbt = serde_json::from_value(entity["nbt"].clone()).unwrap();
            assert_eq!(
                actual.state,
                entity["state"].as_u64().unwrap() as u32,
                "{label} BE state {p:?}"
            );
            assert_eq!(
                Some(actual.id.as_str()),
                expected.get("id").and_then(Nbt::string),
                "{label} BE ID {p:?}"
            );
            if actual.id == "minecraft:trial_spawner" {
                assert_eq!(
                    actual.full_data(),
                    expected,
                    "{label} typed trial spawner at {p:?}"
                );
                assert!(
                    actual.loot_seed.is_none(),
                    "spawners do not consume container loot RNG"
                );
            }
        }
    }
}

#[test]
fn native_trial_spawner_and_vault_load_save_nbt() {
    let cases = fixture()["block_entity_loads"].as_array().unwrap();
    assert_eq!(cases.len(), 16);
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for sample in cases {
        checked += 1;
        let input = serde_json::from_value(sample["input"]["nbt"].clone()).unwrap();
        let entity = TemplateBlockEntity::from_load(
            &assets().blocks,
            sample["state"].as_u64().unwrap() as u32,
            pos(&sample["pos"]),
            input,
        )
        .unwrap();
        let expected: Nbt = serde_json::from_value(sample["nbt"].clone()).unwrap();
        if entity.full_data() != expected {
            mismatches.push(format!(
                "{}: actual={:?}, native={expected:?}",
                sample["template"],
                entity.full_data()
            ));
        }
    }
    assert_eq!(checked, 16);
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

fn differing_paths(
    actual: Option<&Nbt>,
    expected: Option<&Nbt>,
    prefix: &str,
    paths: &mut Vec<String>,
) {
    if actual == expected {
        return;
    }
    if let (Some(Nbt::Compound(a)), Some(Nbt::Compound(b))) = (actual, expected) {
        for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
            differing_paths(a.get(key), b.get(key), &format!("{prefix}/{key}"), paths);
        }
    } else {
        paths.push(prefix.to_owned());
    }
}

#[test]
fn native_trial_full_placement_saved_block_entity_nbt() {
    let mut differences = BTreeMap::<String, usize>::new();
    for sample in fixture()["placement"].as_array().unwrap() {
        let (_, result, _) = place(sample);
        for entity in sample["block_entities"].as_array().unwrap() {
            let actual = result.block_entities[&pos(&entity["pos"])].full_data();
            let expected: Nbt = serde_json::from_value(entity["nbt"].clone()).unwrap();
            let mut paths = Vec::new();
            differing_paths(
                Some(&actual),
                Some(&expected),
                expected.get("id").and_then(Nbt::string).unwrap(),
                &mut paths,
            );
            for path in paths {
                *differences.entry(path).or_default() += 1;
            }
        }
    }
    assert!(
        differences.is_empty(),
        "native BE serialization differences: {differences:#?}"
    );
}
