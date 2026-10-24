//! Null-player inventory access on the retained level's block entity.
use bcore_core::ChunkPos;

use super::full::{block_entity_error, core_block_entity_tag, ChunkLifecycle};
use super::{FeatureBlockEntity, GenerationError, GenerationState, GenerationWorld};
use crate::block_entity::{BlockEntity, ChestInventory, MaterializedBlockEntity};
use crate::feature_world::{FeatureError, Pos};
use crate::loot::{self, FillOutcome, LootContext, LootEngine, LootRng, LootWorld};
use crate::structure::template::{Nbt, TemplateBlockEntity};
use crate::structure::template_pool::StructureAssets;
use crate::GeneratedChunk;

type LocalPos = (usize, i32, usize);
const BOUNDARY: &str = "chest unpack";

impl GenerationWorld {
    /// Access an existing, single chest on a converted level chunk. This is the
    /// null-player lazy-inventory boundary, not player menu/access validation.
    /// Missing/non-chest positions return None. It schedules no generation.
    /// A pending key is consumed once; subsequent reads consume no random values.
    pub fn unpack_chest(
        &self,
        pos: ChunkPos,
        local: LocalPos,
    ) -> Result<Option<ChestInventory>, GenerationError> {
        let engine = LootEngine::vanilla().map_err(|e| loot_error(pos, e))?;
        self.access_chest(pos, local, engine, |inventory, world, origin| {
            inventory.unpack_with_context(
                engine,
                LootContext::CHEST.with_location(Some(center(origin)), world),
            )
        })
    }

    /// The caller owns selection/persistence of this source. It is used exactly
    /// as supplied, without reseeding from the chest or selecting a world sequence.
    /// No-key access neither validates the context nor advances the source.
    /// The location and loaded-world lookup come from the canonical chest owner.
    /// Do not reenter this world from the source while the generation lock is held.
    pub fn unpack_chest_with_rng(
        &self,
        pos: ChunkPos,
        local: LocalPos,
        context: LootContext<'_>,
        rng: &mut dyn LootRng,
    ) -> Result<Option<ChestInventory>, GenerationError> {
        let engine = LootEngine::vanilla().map_err(|e| loot_error(pos, e))?;
        self.access_chest(pos, local, engine, |inventory, world, origin| {
            inventory.unpack_with_rng(
                engine,
                context.with_location(Some(center(origin)), world),
                rng,
            )
        })
    }

    fn access_chest(
        &self,
        pos: ChunkPos,
        local: LocalPos,
        engine: &LootEngine,
        unpack: impl FnOnce(
            &mut ChestInventory,
            &dyn LootWorld,
            Pos,
        ) -> loot::Result<Option<FillOutcome>>,
    ) -> Result<Option<ChestInventory>, GenerationError> {
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        state.level(pos)?;
        let Some(chunk) = state.region.owned_chunk_ref(pos) else {
            return Ok(None);
        };
        let Some((entity, block_state, world_pos, was_core)) =
            load_entity(&chunk, local).map_err(|e| block_entity_error(pos, BOUNDARY, e))?
        else {
            return Ok(None);
        };
        let mut inventory = read_inventory(engine, &entity).map_err(|e| loot_error(pos, e))?;
        let had_key = inventory.loot_table().is_some();
        let world = ChestLootWorld {
            state: &state,
            zoom: crate::biome_zoom::BiomeZoom::new(self.generator.seed()),
        };
        let result = unpack(&mut inventory, &world, world_pos);
        let consumed = had_key && inventory.loot_table().is_none();
        if let Err(error) = &result {
            if !consumed {
                return Err(loot_error(pos, error.clone()));
            }
        }
        if !had_key && !chunk.pending_block_entities.contains_key(&local) {
            return Ok(Some(inventory));
        }
        // Evaluation can fail after consuming the key. Persist that transition
        // before reporting the error, so another access cannot reroll the table.
        let entity = write_inventory(&entity, block_state, world_pos, &inventory)
            .map_err(|e| block_entity_error(pos, BOUNDARY, e))?;
        drop(chunk);
        let chunk = state.region.owned_chunk_mut(pos);
        chunk.pending_block_entities.remove(&local);
        chunk.block_entities.remove(&local);
        chunk.feature_block_entities.insert(local, entity);
        if was_core {
            let holder = state
                .holders
                .get_mut(&(pos.x, pos.z))
                .expect("retained level");
            let ChunkLifecycle::Level(level) = &mut holder.lifecycle else {
                unreachable!("conversion checked under the same lock")
            };
            level.retained_proto.promote_core_member(local);
        }
        result.map_err(|e| loot_error(pos, e))?;
        Ok(Some(inventory))
    }
}

fn center((x, y, z): Pos) -> [f64; 3] {
    [x, y, z].map(|value| f64::from(value) + 0.5)
}

struct ChestLootWorld<'a> {
    state: &'a GenerationState,
    zoom: crate::biome_zoom::BiomeZoom,
}

impl LootWorld for ChestLootWorld<'_> {
    fn biome_if_loaded(&self, [x, y, z]: [i32; 3]) -> loot::Result<Option<u32>> {
        if self.state.level(ChunkPos::new(x >> 4, z >> 4)).is_err() {
            return Ok(None);
        }
        let (qx, qy, qz) = self.zoom.quart_at((x, y, z));
        let owner = ChunkPos::new(qx >> 2, qz >> 2);
        let chunk =
            self.state
                .region
                .owned_chunk_ref(owner)
                .ok_or(loot::LootError::UnsupportedContext(
                    "loaded-world noise-biome owner",
                ))?;
        let y = qy.clamp(crate::MIN_Y >> 2, crate::MAX_Y >> 2) * 4;
        let cells = chunk
            .noise_biomes
            .as_ref()
            .ok_or(loot::LootError::UnsupportedContext(
                "loaded-world quart biomes",
            ))?;
        let index =
            ((y - crate::MIN_Y) as usize / 4) * 16 + (qz & 3) as usize * 4 + (qx & 3) as usize;
        Ok(Some(cells[index]))
    }
}

fn load_entity(
    chunk: &GeneratedChunk,
    (x, y, z): LocalPos,
) -> Result<Option<(FeatureBlockEntity, u32, Pos, bool)>, FeatureError> {
    let Some(state) = chunk.get(x, y, z) else {
        return Ok(None);
    };
    let blocks = &StructureAssets::bundled().blocks;
    let block = blocks.state(state)?;
    if block.name != "minecraft:chest" {
        return Ok(None);
    }
    if block.properties.get("type").map(String::as_str) != Some("single") {
        return Err(FeatureError::Unsupported(
            "double-chest inventory access".into(),
        ));
    }
    let pos = (chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32);
    let local = (x, y, z);
    let was_core = chunk.block_entities.contains_key(&local);
    let entity = if let Some(entity) = chunk.block_entities.get(&local) {
        if !matches!(entity, BlockEntity::DungeonChest { .. }) {
            return Err(FeatureError::InvalidConfig(
                "non-chest entity on chest state".into(),
            ));
        }
        let template =
            TemplateBlockEntity::from_load(blocks, state, pos, core_block_entity_tag(entity, pos))?;
        FeatureBlockEntity::from_template(&template)?
    } else if let Some(entity) = chunk.feature_block_entities.get(&local) {
        if !entity.valid_for(state, pos) {
            return Err(FeatureError::InvalidConfig(
                "invalid live chest payload".into(),
            ));
        }
        entity.clone()
    } else if let Some(pending) = chunk.pending_block_entities.get(&local) {
        match pending.materialize(state, pos)? {
            MaterializedBlockEntity::Feature(entity) => entity,
            _ => {
                return Err(FeatureError::InvalidConfig(
                    "non-chest pending payload".into(),
                ))
            }
        }
    } else {
        return Ok(None);
    };
    Ok(Some((entity, state, pos, was_core)))
}

fn read_inventory(
    engine: &LootEngine,
    entity: &FeatureBlockEntity,
) -> loot::Result<ChestInventory> {
    let nbt = entity.full_nbt().map_err(invalid_payload)?;
    let mut inventory = ChestInventory::from_nbt(engine, &nbt)?;
    if let Some(load) = &entity.template_load_data {
        let load = Nbt::from_typed_json(load).map_err(invalid_payload)?;
        match load.get("LootTableSeed") {
            Some(Nbt::Long(seed)) => inventory.restore_retained_seed(*seed),
            None => {}
            Some(_) => return Err(invalid_payload("LootTableSeed must be Long")),
        }
    }
    Ok(inventory)
}

fn write_inventory(
    entity: &FeatureBlockEntity,
    state: u32,
    pos: Pos,
    inventory: &ChestInventory,
) -> Result<FeatureBlockEntity, FeatureError> {
    let mut load = entity.full_nbt()?;
    inventory
        .write_nbt(&mut load)
        .map_err(|error| FeatureError::Unsupported(error.to_string()))?;
    let fields = load.compound_mut()?;
    fields.remove("keepPacked");
    // TemplateBlockEntity's full save omits this field when there is no table.
    // Retain it only in the runtime load payload, not in the native saved tag.
    if inventory.loot_seed() != 0 {
        fields.insert("LootTableSeed".into(), Nbt::Long(inventory.loot_seed()));
    }
    let template =
        TemplateBlockEntity::from_load(&StructureAssets::bundled().blocks, state, pos, load)?;
    FeatureBlockEntity::from_template(&template)
}

fn invalid_payload(error: impl std::fmt::Display) -> loot::LootError {
    loot::LootError::InvalidData {
        path: "chest payload".into(),
        message: error.to_string(),
    }
}

fn loot_error(pos: ChunkPos, error: loot::LootError) -> GenerationError {
    GenerationError::BlockEntityFailed {
        pos,
        boundary: BOUNDARY,
        detail: error.to_string(),
    }
}

#[cfg(test)]
#[path = "chest_tests.rs"]
mod tests;
