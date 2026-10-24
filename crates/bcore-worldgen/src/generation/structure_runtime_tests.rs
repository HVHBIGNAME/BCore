//! Differential checks against actual native admission/reference/decoration entry points.
use super::*;
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::ore::OreWorld;
use crate::structure::jigsaw::{self, HeightContext};
use crate::structure::template::{Nbt, TemplateBlockEntity, TemplateEffect};
use crate::structure::template_pool::StructureAssets;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../data/structure_runtime_fresh_light_26_1.json"
        ))
        .unwrap()
    })
}

fn chunk(value: &Value) -> ChunkPos {
    ChunkPos::new(
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
    )
}

fn point(value: &Value) -> (i32, i32, i32) {
    (
        value[0].as_i64().unwrap() as i32,
        value[1].as_i64().unwrap() as i32,
        value[2].as_i64().unwrap() as i32,
    )
}

fn flat_start(row: &Value) -> Option<jigsaw::NamedStart> {
    let biome = crate::reference_26_1::feature_documents()["biome_ids"]
        [format!("minecraft:{}", row["biome"].as_str().unwrap())]
    .as_u64()
    .unwrap() as u32;
    jigsaw::for_chunk(
        crate::registry_26_1::historical_assets(),
        row["set"].as_str().unwrap(),
        row["seed"].as_i64().unwrap(),
        chunk(&row["chunk"]),
        &HeightContext {
            min_y: crate::MIN_Y,
            max_y: crate::MAX_Y,
            first_free: &|kind, _, _| {
                assert_eq!(kind, FeatureHeightmap::WorldSurfaceWg);
                65
            },
        },
        |_| biome,
    )
    .unwrap()
}

#[test]
fn native_structure_runtime_admission_retains_all_variants_and_real_noise_starts() {
    assert_eq!(
        fixture()["jar_sha256"],
        StructureAssets::HISTORICAL_JAR_SHA256
    );
    for row in fixture()["admission"].as_array().unwrap() {
        let source = chunk(&row["chunk"]);
        let expected = row["starts"].as_array().unwrap();
        let actual = if row["biome"] == "overworld" {
            let world = GenerationWorld::new(row["seed"].as_i64().unwrap());
            let first = world
                .generate_to_status(source, ChunkStatus::StructureStarts)
                .unwrap();
            let repeated = world
                .generate_to_status(source, ChunkStatus::StructureStarts)
                .unwrap();
            assert_eq!(first.chunk, repeated.chunk);
            assert_eq!(
                repeated
                    .coverage
                    .target
                    .stage(ChunkStatus::StructureStarts)
                    .attempts,
                1
            );
            first
                .chunk
                .structures
                .jigsaw_starts
                .into_iter()
                .filter(|(name, _)| {
                    if row["set"] == "villages" {
                        name.starts_with("minecraft:village_")
                    } else {
                        name == "minecraft:ancient_city"
                    }
                })
                .collect::<BTreeMap<_, _>>()
        } else {
            flat_start(row)
                .into_iter()
                .map(|named| (named.structure, named.start))
                .collect()
        };
        assert_eq!(actual.len(), expected.len(), "{row}");
        for expected in expected {
            let name = expected["structure"].as_str().unwrap();
            let start = &actual[name];
            let expected_nbt: Nbt = serde_json::from_value(expected["nbt"].clone()).unwrap();
            assert_eq!(
                start.to_nbt(name, source),
                expected_nbt,
                "{} {:?} {}",
                row["seed"],
                source,
                name
            );
            assert_eq!(
                json!(start.reference_bounds().unwrap().as_array()),
                expected["reference_bounds"]
            );
            assert!(
                start.valid_for(name, source),
                "invalid retained {name} at {source:?}"
            );
            let encoded = serde_json::to_vec(start).unwrap();
            let restored: jigsaw::JigsawStart = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(*start, restored);
            assert_eq!(start.to_nbt(name, source), restored.to_nbt(name, source));
        }
        assert_eq!(row["retained"], true);
    }
}

#[test]
fn native_trail_ruins_admission_and_saved_starts_match_real_noise_and_biome_rejections() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../data/structure_runtime_trail_26_1.json")).unwrap();
    verify_jigsaw_admission(&fixture, "minecraft:trail_ruins", 60);
}

#[test]
fn native_trial_chambers_admission_and_saved_starts_match_real_noise_and_biome_rejections() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../data/structure_runtime_trial_26_1.json")).unwrap();
    verify_jigsaw_admission(&fixture, "minecraft:trial_chambers", 54);
}

fn verify_jigsaw_admission(fixture: &Value, structure: &str, count: usize) {
    assert_eq!(
        fixture["jar_sha256"],
        StructureAssets::HISTORICAL_JAR_SHA256
    );
    let rows = fixture["admission"].as_array().unwrap();
    assert_eq!(rows.len(), count);
    let mut actual_noise_starts = 0;
    for row in rows {
        let source = chunk(&row["chunk"]);
        let actual: BTreeMap<_, _> = if row["biome"] == "overworld" {
            let world = GenerationWorld::new(row["seed"].as_i64().unwrap());
            let first = world
                .generate_to_status(source, ChunkStatus::StructureStarts)
                .unwrap();
            let again = world
                .generate_to_status(source, ChunkStatus::StructureStarts)
                .unwrap();
            assert_eq!(first.chunk, again.chunk);
            assert_eq!(
                again
                    .coverage
                    .target
                    .stage(ChunkStatus::StructureStarts)
                    .attempts,
                1
            );
            let starts: BTreeMap<_, _> = first
                .chunk
                .structures
                .jigsaw_starts
                .into_iter()
                .filter(|(name, _)| name == structure)
                .collect();
            actual_noise_starts += starts.len();
            starts
        } else {
            flat_start(row)
                .into_iter()
                .map(|named| (named.structure, named.start))
                .collect()
        };
        let expected = row["starts"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "{row}");
        for expected in expected {
            let name = expected["structure"].as_str().unwrap();
            let start = &actual[name];
            let nbt: Nbt = serde_json::from_value(expected["nbt"].clone()).unwrap();
            assert_eq!(start.to_nbt(name, source), nbt, "{source:?}");
            assert_eq!(
                json!(start.reference_bounds().unwrap().as_array()),
                expected["reference_bounds"]
            );
            assert!(start.valid_for(name, source));
            let saved = serde_json::to_vec(start).unwrap();
            let restored: jigsaw::JigsawStart = serde_json::from_slice(&saved).unwrap();
            assert_eq!(*start, restored);
        }
        assert_eq!(row["retained"], true);
    }
    assert_eq!(actual_noise_starts, 3);
}

#[test]
fn native_structure_runtime_reference_bounds_and_order_match_450_targets() {
    let mut total = 0;
    for row in fixture()["references"].as_array().unwrap() {
        let mut state = GenerationState::new(WorldGenerator::new(row["seed"].as_i64().unwrap()));
        for target in row["targets"].as_array().unwrap() {
            for pos in layer_positions(chunk(&target["chunk"]), 8) {
                state
                    .holders
                    .entry((pos.x, pos.z))
                    .or_insert_with(|| ChunkHolder::new(pos));
            }
        }
        for source in row["sources"].as_array().unwrap() {
            let named = flat_start(source).unwrap();
            state
                .holders
                .get_mut(&(named.source.x, named.source.z))
                .unwrap()
                .structures
                .jigsaw_starts
                .insert(named.structure, named.start);
        }
        for target in row["targets"].as_array().unwrap() {
            let pos = chunk(&target["chunk"]);
            let references = state.jigsaw_references(pos);
            assert_eq!(
                json!(references),
                target["references"],
                "{} {pos:?}",
                row["set"]
            );
            state
                .holders
                .get_mut(&(pos.x, pos.z))
                .unwrap()
                .structures
                .jigsaw_references = references;
            assert!(state.holders[&(pos.x, pos.z)].structures.valid_for(pos));
            total += 1;
        }
    }
    assert_eq!(total, 450);
    let legacy: StructureData =
        serde_json::from_value(json!({"mineshaft_start":null,"references":[]})).unwrap();
    assert!(legacy.is_empty());
}

#[test]
fn native_structure_runtime_template_load_save_update_preserves_inventories_and_widths() {
    let assets = crate::registry_26_1::historical_assets();
    let cases = fixture()["template_block_entities"].as_array().unwrap();
    assert!(cases.len() > 100);
    for row in cases {
        let state = row["state"].as_u64().unwrap() as u32;
        let pos = point(&row["pos"]);
        let load: Nbt = serde_json::from_value(row["load"]["nbt"].clone()).unwrap();
        let entity = TemplateBlockEntity::from_load(&assets.blocks, state, pos, load)
            .unwrap_or_else(|error| {
                panic!(
                    "{} palette {} index {} state {state}: {error}",
                    row["template"], row["palette"], row["index"]
                )
            });
        let saved = entity.full_data();
        let expected: Nbt = serde_json::from_value(row["full"]["nbt"].clone()).unwrap();
        assert_eq!(
            saved, expected,
            "{} palette {} index {} state {state}",
            row["template"], row["palette"], row["index"]
        );
        let data = FeatureBlockEntity::from_template_in_registry(&entity, &assets.blocks).unwrap();
        let update: Nbt = serde_json::from_value(row["update"]["nbt"].clone()).unwrap();
        assert_eq!(
            data.update_nbt().unwrap(),
            update,
            "{} {pos:?}",
            row["template"]
        );
        assert!(data.valid_for_registry(&assets.blocks, state, pos));
        assert!(!data.valid_for_registry(&assets.blocks, state, (pos.0 + 1, pos.1, pos.2)));
        assert_eq!(Nbt::from_typed_json(&data.typed_data).unwrap(), saved);
        let decoded: FeatureBlockEntity =
            serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert_eq!(data, decoded);
    }
}

#[test]
fn native_structure_runtime_brushable_load_save_update_preserves_codecs() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../data/brushable_block_entities_26_1_v2.json"
    ))
    .unwrap();
    verify_template_block_entity_loads(&fixture, 106);
}

#[test]
fn native_structure_runtime_trial_spawner_and_vault_full_update_tags() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../data/trial_block_entities_26_1.json")).unwrap();
    verify_template_block_entity_loads(&fixture, 48);
}

#[test]
fn native_structure_runtime_decorated_pot_load_save_update_preserves_codecs() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../data/decorated_pot_block_entities_26_1.json"
    ))
    .unwrap();
    verify_template_block_entity_loads(&fixture, 80);
}

fn verify_template_block_entity_loads(fixture: &Value, count: usize) {
    assert_eq!(
        fixture["jar_sha256"],
        StructureAssets::HISTORICAL_JAR_SHA256
    );
    let rows = fixture["block_entity_loads"].as_array().unwrap();
    assert_eq!(rows.len(), count);
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let pos = point(&row["pos"]);
        let state = crate::registry_26_1::target_state(row["state"].as_u64().unwrap() as u32);
        let load: Nbt = serde_json::from_value(row["load"]["nbt"].clone()).unwrap();
        let entity = TemplateBlockEntity::from_load(
            &StructureAssets::bundled().blocks,
            state,
            pos,
            load.clone(),
        )
        .unwrap();
        let full: Nbt = serde_json::from_value(row["full"]["nbt"].clone()).unwrap();
        let update: Nbt = serde_json::from_value(row["update"]["nbt"].clone()).unwrap();
        assert_eq!(entity.full_data(), full, "{name} full save");
        assert_eq!(entity.load_data, load, "{name} original load retained");
        let generated = FeatureBlockEntity::from_template(&entity).unwrap();
        assert_eq!(generated.full_nbt().unwrap(), full, "{name} generated full");
        assert_eq!(generated.update_nbt().unwrap(), update, "{name} update");
        assert!(generated.valid_for(state, pos), "{name} storage validation");
        let restored: FeatureBlockEntity =
            serde_json::from_slice(&serde_json::to_vec(&generated).unwrap()).unwrap();
        assert_eq!(restored, generated, "{name} persistence");
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

fn runtime_state(row: &Value) -> GenerationState {
    let seed = row["admitted"]["seed"].as_i64().unwrap();
    let mut state = GenerationState::new(WorldGenerator::new(seed));
    let source = chunk(&row["source"]);
    for pos in layer_positions(source, 8) {
        let mut holder = ChunkHolder::new(pos);
        let status = if graph::chessboard_distance(source, pos) <= 1 {
            ChunkStatus::Carvers
        } else {
            ChunkStatus::StructureStarts
        };
        for s in ChunkStatus::ALL.into_iter().take(status.index() + 1) {
            holder.progress.stages[s.index()].state = StageState::Complete;
        }
        state.holders.insert((pos.x, pos.z), holder);
    }
    let named = flat_start(&row["admitted"]).unwrap();
    state
        .holders
        .get_mut(&(named.source.x, named.source.z))
        .unwrap()
        .structures
        .jigsaw_starts
        .insert(named.structure, named.start);
    let references = state.jigsaw_references(source);
    state
        .holders
        .get_mut(&(source.x, source.z))
        .unwrap()
        .structures
        .jigsaw_references = references;
    let biome = crate::biome::id(row["admitted"]["biome"].as_str().unwrap()).unwrap();
    for pos in layer_positions(source, 1) {
        let chunk = state.region.owned_chunk_mut(pos);
        for y in crate::MIN_Y..=crate::MAX_Y {
            let block = if row["terrain"] == "city_cave" {
                if y < -50 || y >= -20 {
                    crate::block::STONE
                } else {
                    crate::block::AIR
                }
            } else if y < 64 {
                crate::block::STONE
            } else if y == 64 {
                crate::block::GRASS_BLOCK
            } else {
                crate::block::AIR
            };
            chunk.states[(y - crate::MIN_Y) as usize * 256..(y - crate::MIN_Y + 1) as usize * 256]
                .fill(block);
        }
        chunk.noise_biomes = Some(vec![biome; 1536]);
    }
    state
}

type HistoricalEntities = Rc<RefCell<BTreeMap<Pos, TemplateBlockEntity>>>;

// The old runtime fixture includes bed factories. Target beds no longer have
// block entities, so their historical factory outputs remain in this replay
// context rather than entering current-registry chunk persistence.
struct HistoricalWorld<'a> {
    region: &'a mut dyn FeatureWorld,
    entities: HistoricalEntities,
}

impl OreWorld for HistoricalWorld<'_> {
    fn get_block(&self, pos: Pos) -> Option<u32> {
        OreWorld::get_block(self.region, pos).map(crate::registry_26_1::historical_state)
    }
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.region.ocean_floor_wg(x, z)
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.set_feature_block(pos, state, 2)
    }
}

impl FeatureWorld for HistoricalWorld<'_> {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.region.feature_biome(pos)
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.region.feature_height(kind, x, z)
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        self.region.can_write_feature(pos)
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        let accepted =
            self.region
                .set_feature_block(pos, crate::registry_26_1::target_state(state), flags);
        if accepted {
            let mut entities = self.entities.borrow_mut();
            match crate::registry_26_1::historical_assets()
                .blocks
                .default_block_entity(state, pos)
                .unwrap()
            {
                Some(entity) => {
                    entities.insert(pos, entity);
                }
                None => {
                    entities.remove(&pos);
                }
            }
        }
        accepted
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        self.region.mark_feature_postprocessing(pos);
    }
    fn schedule_feature_tick(&mut self, mut request: crate::tick_request::TickRequest) -> bool {
        if let crate::tick_request::TickTarget::Block(state) = &mut request.target {
            *state = crate::registry_26_1::target_state(*state);
        }
        self.region.schedule_feature_tick(request)
    }
    fn update_feature_neighbours(&mut self, pos: Pos, state: u32) -> Result<(), FeatureError> {
        self.region
            .update_feature_neighbours(pos, crate::registry_26_1::target_state(state))
    }
    fn has_feature_tick(
        &self,
        pos: Pos,
        mut target: crate::tick_request::TickTarget,
    ) -> Result<bool, FeatureError> {
        if let crate::tick_request::TickTarget::Block(state) = &mut target {
            *state = crate::registry_26_1::target_state(*state);
        }
        self.region.has_feature_tick(pos, target)
    }
    fn lookup_feature_beehive(&mut self, pos: Pos) -> Result<Option<Vec<i32>>, FeatureError> {
        self.region.lookup_feature_beehive(pos)
    }
    fn set_feature_brushable_loot(
        &mut self,
        pos: Pos,
        table: &str,
        seed: i64,
    ) -> Result<bool, FeatureError> {
        self.region.set_feature_brushable_loot(pos, table, seed)
    }
}

struct CurrentWorld<'a>(&'a mut dyn FeatureWorld);
impl OreWorld for CurrentWorld<'_> {
    fn get_block(&self, pos: Pos) -> Option<u32> {
        self.0
            .get_block(pos)
            .map(crate::registry_26_1::target_state)
    }
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.0.ocean_floor_wg(x, z)
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.0
            .set_block(pos, crate::registry_26_1::historical_state(state))
    }
}
impl FeatureWorld for CurrentWorld<'_> {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.0.feature_biome(pos)
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.0.feature_height(kind, x, z)
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        self.0.can_write_feature(pos)
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        self.0
            .set_feature_block(pos, crate::registry_26_1::historical_state(state), flags)
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        self.0.mark_feature_postprocessing(pos);
    }
    fn schedule_feature_tick(&mut self, mut request: crate::tick_request::TickRequest) -> bool {
        if let crate::tick_request::TickTarget::Block(state) = &mut request.target {
            *state = crate::registry_26_1::historical_state(*state);
        }
        self.0.schedule_feature_tick(request)
    }
    fn update_feature_neighbours(&mut self, pos: Pos, state: u32) -> Result<(), FeatureError> {
        self.0
            .update_feature_neighbours(pos, crate::registry_26_1::historical_state(state))
    }
    fn has_feature_tick(
        &self,
        pos: Pos,
        mut target: crate::tick_request::TickTarget,
    ) -> Result<bool, FeatureError> {
        if let crate::tick_request::TickTarget::Block(state) = &mut target {
            *state = crate::registry_26_1::historical_state(*state);
        }
        self.0.has_feature_tick(pos, target)
    }
    fn lookup_feature_beehive(&mut self, pos: Pos) -> Result<Option<Vec<i32>>, FeatureError> {
        self.0.lookup_feature_beehive(pos)
    }
    fn set_feature_brushable_loot(
        &mut self,
        pos: Pos,
        table: &str,
        seed: i64,
    ) -> Result<bool, FeatureError> {
        self.0.set_feature_brushable_loot(pos, table, seed)
    }
}

fn place_historical_jigsaw_source(
    state: &mut GenerationState,
    source: ChunkPos,
    name: &str,
    seed: i64,
    random: &mut crate::simplex::WorldgenRandom,
    gaussian: &mut crate::dripstone::CaveRandomState,
    entities: &HistoricalEntities,
) {
    let assets = crate::registry_26_1::historical_assets();
    let references = state.holders[&(source.x, source.z)]
        .structures
        .jigsaw_references
        .get(name)
        .cloned()
        .unwrap_or_default();
    let clip = crate::structure::template::BoundingBox {
        min: (source.x * 16, crate::MIN_Y + 1, source.z * 16),
        max: (source.x * 16 + 15, crate::MAX_Y, source.z * 16 + 15),
    };
    let mut spawned = Vec::new();
    {
        let mut region = tree_world::EffectWorld::new(&mut state.region);
        let effects = region.effects.clone();
        let mut world = HistoricalWorld {
            region: &mut region,
            entities: entities.clone(),
        };
        for [x, z] in references {
            let start = &state.holders[&(x, z)].structures.jigsaw_starts[name];
            jigsaw::place_in_chunk_with_callbacks(
                assets,
                start,
                &mut world,
                random,
                clip,
                seed,
                &mut |feature, world, random, origin| {
                    if let Some(document) = assets.placed_features.get(feature) {
                        assert_eq!(
                            document,
                            crate::block_predicate::catalog().placed(feature).unwrap(),
                            "nested historical configuration differs: {feature}"
                        );
                    }
                    let reference = Value::String(feature.into());
                    placed::check_nested(&reference, 0)?;
                    let mut dispatcher = placed::dispatcher(gaussian, effects.clone(), seed);
                    let placed = crate::placement::place_nested(
                        &reference,
                        &mut CurrentWorld(world),
                        random,
                        origin,
                        &placed::GenerationEnvironment,
                        &mut dispatcher,
                    )?;
                    Ok(crate::structure::template::PlacementResult {
                        placed,
                        ..Default::default()
                    })
                },
                &mut |_, effect| {
                    match effect {
                        TemplateEffect::ClearBlockEntity(pos) => {
                            entities.borrow_mut().remove(&pos);
                        }
                        TemplateEffect::BlockEntity(entity) => {
                            entities.borrow_mut().insert(entity.pos, entity);
                        }
                        TemplateEffect::Entity(entity) => spawned.push(entity),
                    }
                    Ok(())
                },
            )
            .unwrap();
        }
    }
    for entity in spawned {
        state
            .region
            .apply_template_effect(source, TemplateEffect::Entity(entity))
            .unwrap();
    }
}

#[test]
fn native_structure_runtime_clipped_decoration_callbacks_effects_and_rng() {
    let mut pending_entities = 0;
    for row in fixture()["runtime"].as_array().unwrap() {
        let mut state = runtime_state(row);
        let historical_entities = HistoricalEntities::default();
        let seed = row["admitted"]["seed"].as_i64().unwrap();
        let source = chunk(&row["source"]);
        let label = format!("{} {source:?}", row["admitted"]["biome"]);
        let available = state
            .holders
            .iter()
            .map(|(&key, h)| (key, h.progress.available_status().unwrap()))
            .collect();
        state
            .region
            .begin_source(source, ChunkStatus::Features, available);
        crate::region::begin_structure_write_trace();
        let mut random = crate::simplex::WorldgenRandom::new(0);
        let decoration = random.set_decoration_seed(seed, source.x * 16, source.z * 16);
        assert_eq!(json!(decoration), row["decoration_seed"]);
        assert_eq!(row["fresh_raw_brightness"], 15);
        let mut gaussian = crate::dripstone::CaveRandomState::default();
        // Native applyBiomeDecoration reseeds even types with no retained starts.
        // The fixture only admits the one tested unmodified structure set.
        for step in 0..11 {
            for ty in fixture()["metadata"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|ty| ty["step"] == step)
            {
                let name = ty["structure"].as_str().unwrap();
                random.set_feature_seed(decoration, ty["index"].as_i64().unwrap() as i32, step);
                if crate::registry_26_1::historical_assets()
                    .structure_metadata
                    .contains_key(name)
                {
                    place_historical_jigsaw_source(
                        &mut state,
                        source,
                        name,
                        seed,
                        &mut random,
                        &mut gaussian,
                        &historical_entities,
                    );
                }
                let mut continuation = crate::simplex::WorldgenRandom::new(0);
                continuation.source = random.source.clone();
                assert_eq!(
                    continuation.next_long(),
                    row["structure_rng_tails"][name].as_i64().unwrap(),
                    "{label} {name} RNG tail"
                );
            }
        }
        state.region.transfer_tree_effects().unwrap();
        let writes: Vec<_> = crate::region::take_structure_write_trace()
            .into_iter()
            .map(|mut row| {
                row[3] = crate::registry_26_1::historical_state(row[3] as u32) as i32;
                row
            })
            .collect();
        assert_eq!(
            writes.len(),
            row["write_count"].as_u64().unwrap() as usize,
            "{label} writes"
        );
        assert_eq!(
            digest(writes.iter().copied()),
            row["writes_md5"].as_str().unwrap(),
            "{label} write ordering"
        );
        assert_eq!(
            random.next_long(),
            row["next_i64"].as_i64().unwrap(),
            "{label} RNG"
        );
        let expected: BTreeMap<_, _> = row["states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    point(r),
                    crate::registry_26_1::target_state(r[3].as_u64().unwrap() as u32),
                )
            })
            .collect();
        let mut actual_entities = BTreeMap::new();
        let mut ticks: BTreeMap<_, Vec<Value>> = BTreeMap::new();
        let mut marks = BTreeMap::new();
        for pos in layer_positions(source, 1) {
            // The original isolated native fixture saved factory-created BEs.
            // Compare that boundary explicitly without changing the proto data.
            let mut chunk = (*state.region.owned_chunk(pos).unwrap()).clone();
            chunk.materialize_block_entities().unwrap();
            for y in crate::MIN_Y..=crate::MAX_Y {
                for z in 0..16 {
                    for x in 0..16 {
                        let p = (pos.x * 16 + x as i32, y, pos.z * 16 + z as i32);
                        let base = if row["terrain"] == "city_cave" {
                            if y < -50 || y >= -20 {
                                crate::block::STONE
                            } else {
                                crate::block::AIR
                            }
                        } else if y < 64 {
                            crate::block::STONE
                        } else if y == 64 {
                            crate::block::GRASS_BLOCK
                        } else {
                            crate::block::AIR
                        };
                        assert_eq!(
                            chunk.get(x, y, z),
                            Some(expected.get(&p).copied().unwrap_or(base)),
                            "{label} {p:?}"
                        );
                    }
                }
            }
            for (&p, entity) in historical_entities
                .borrow()
                .iter()
                .filter(|(p, _)| p.0 >> 4 == pos.x && p.2 >> 4 == pos.z)
            {
                let entity = FeatureBlockEntity::from_template_in_registry(
                    entity,
                    &crate::registry_26_1::historical_assets().blocks,
                )
                .unwrap();
                assert!(
                    entity.valid_for_registry(
                        &crate::registry_26_1::historical_assets().blocks,
                        crate::registry_26_1::historical_state(
                            chunk
                                .get((p.0 & 15) as usize, p.1, (p.2 & 15) as usize)
                                .unwrap()
                        ),
                        p
                    ),
                    "{label} invalid {p:?}"
                );
                actual_entities.insert(
                    p,
                    (entity.full_nbt().unwrap(), entity.update_nbt().unwrap()),
                );
            }
            assert!(
                chunk.block_entities().is_empty(),
                "fixture has no beehive/spawner creation"
            );
            for request in chunk.structure_entities() {
                assert!(request.valid_for(pos));
                assert!(request.finalize);
                pending_entities += 1;
            }
            for tick in chunk.tick_requests() {
                let (kind, id) = match tick.target {
                    crate::tick_request::TickTarget::Fluid(id) => ("fluid", id),
                    crate::tick_request::TickTarget::Block(id) => {
                        let blocks = &crate::registry_26_1::historical_assets().blocks;
                        let name = &blocks
                            .state(crate::registry_26_1::historical_state(id))
                            .unwrap()
                            .name;
                        let first = blocks.state_range(name).unwrap()[0];
                        (
                            "block",
                            (0..first)
                                .filter(|&id| {
                                    blocks.state(id).unwrap().name
                                        != blocks.state(id + 1).unwrap().name
                                })
                                .count() as u32,
                        )
                    }
                };
                ticks.entry((pos.x, pos.z)).or_default().push(json!([
                    tick.block_pos,
                    kind,
                    id,
                    tick.delay
                ]));
            }
            for &(x, y, z) in chunk.postprocessing_positions() {
                *marks
                    .entry((pos.x * 16 + x as i32, y, pos.z * 16 + z as i32))
                    .or_insert(0) += 1;
            }
        }
        let expected_entities: BTreeMap<_, _> = row["block_entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    point(&r["pos"]),
                    (
                        serde_json::from_value::<Nbt>(r["full"]["nbt"].clone()).unwrap(),
                        serde_json::from_value::<Nbt>(r["update"]["nbt"].clone()).unwrap(),
                    ),
                )
            })
            .collect();
        assert_eq!(
            actual_entities, expected_entities,
            "{label} block entity full/update NBT"
        );
        let mut expected_ticks: BTreeMap<_, Vec<Value>> = BTreeMap::new();
        for tick in row["ticks"].as_array().unwrap() {
            let (x, _, z) = point(&tick[0]);
            expected_ticks
                .entry((x >> 4, z >> 4))
                .or_default()
                .push(tick.clone());
        }
        assert_eq!(ticks, expected_ticks, "{label} ticks");
        let expected_marks: BTreeMap<_, _> = row["marks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (point(r), r[3].as_i64().unwrap()))
            .collect();
        assert_eq!(marks, expected_marks, "{label} marks");
        state.region.end_source();
    }
    assert!(
        pending_entities > 0,
        "village entities must be retained, never dropped"
    );
}

#[test]
fn native_structure_runtime_gaussian_cache_survives_structure_and_feature_reseeds() {
    let mut random = crate::simplex::WorldgenRandom::new(0);
    let mut gaussian = crate::dripstone::CaveRandomState::default();
    let mut decoration = 0;
    for row in fixture()["random_stream"].as_array().unwrap() {
        match row["op"].as_str().unwrap() {
            "decoration" => {
                decoration = random.set_decoration_seed(
                    row["seed"].as_i64().unwrap(),
                    row["x"].as_i64().unwrap() as i32,
                    row["z"].as_i64().unwrap() as i32,
                );
                assert_eq!(json!(decoration), row["value"]);
            }
            "feature" => random.set_feature_seed(
                decoration,
                row["index"].as_i64().unwrap() as i32,
                row["step"].as_i64().unwrap() as i32,
            ),
            "gaussian" => assert_eq!(
                gaussian.next_gaussian(&mut random).to_bits().to_string(),
                row["bits"].as_str().unwrap()
            ),
            "long" => assert_eq!(random.next_long(), row["value"].as_i64().unwrap()),
            _ => panic!("unknown native stream operation"),
        }
    }
}
