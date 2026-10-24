//! Admission of registry-validated storage snapshots without replaying generation.
use std::collections::{BTreeMap, BTreeSet};

use bcore_core::ChunkPos;

use super::{
    validate_generation_pos, ChunkHolder, GenerationError, GenerationWorld, StageProgress,
    StageState,
};
use crate::block_entity::{BlockEntity, PendingBlockEntity};
use crate::generated_entity::GeneratedEntity;
use crate::lighting::ChunkLight;
use crate::structure::mineshaft::region::StructureData;
use crate::tick_request::TickRequest;
use crate::{GeneratedChunk, RuntimeHeightmaps, MIN_Y, WORLD_HEIGHT};

type LocalPos = (usize, i32, usize);

/// Storage must authenticate the seed/version/registry before transferring this
/// snapshot. It carries no generation claims, WG maps or world registration.
pub struct SavedChunkData {
    pub pos: ChunkPos,
    pub states: Vec<u32>,
    pub biomes: Vec<u32>,
    pub block_entities: BTreeMap<LocalPos, BlockEntity>,
    pub feature_block_entities: BTreeMap<LocalPos, super::FeatureBlockEntity>,
    pub pending_block_entities: BTreeMap<LocalPos, PendingBlockEntity>,
    pub entities: Vec<GeneratedEntity>,
    pub structure_entities: Vec<super::StructureEntityRequest>,
    pub tick_requests: Vec<TickRequest>,
    pub postprocessing: Vec<LocalPos>,
    pub light: Option<ChunkLight>,
    pub structures: StructureData,
}

impl SavedChunkData {
    fn into_chunk(self) -> Result<GeneratedChunk, GenerationError> {
        validate_generation_pos(self.pos)?;
        let invalid = |detail: &str| GenerationError::InvalidSavedChunk {
            pos: self.pos,
            detail: detail.to_owned(),
        };
        if self.states.len() != WORLD_HEIGHT as usize * 256
            || self.biomes.len() != WORLD_HEIGHT as usize * 4
            || self
                .states
                .iter()
                .any(|&id| id as usize >= bcore_registry::BLOCK_STATE_COUNT)
            || self
                .biomes
                .iter()
                .any(|&id| id as usize >= bcore_registry::BIOME_COUNT)
        {
            return Err(invalid("column geometry or registry IDs"));
        }
        let mut chunk = GeneratedChunk {
            pos: self.pos,
            states: self.states,
            biomes: Vec::new(),
            noise_biomes: Some(self.biomes),
            heights: Vec::new(),
            worldgen_heightmaps: None,
            runtime_heightmaps: None,
            light: self.light,
            block_entities: self.block_entities,
            feature_block_entities: self.feature_block_entities,
            pending_block_entities: self.pending_block_entities,
            entities: self.entities,
            structure_entities: self.structure_entities,
            structures: self.structures,
            postprocessing: self.postprocessing,
            tick_requests: self.tick_requests,
        };
        let world_pos =
            |(x, y, z): LocalPos| (chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32);
        if chunk.block_entities.iter().any(|(&p, entity)| {
            chunk.feature_block_entities.contains_key(&p)
                || chunk.pending_block_entities.contains_key(&p)
                || !chunk
                    .get(p.0, p.1, p.2)
                    .is_some_and(|state| entity.matches_state(state))
        }) || chunk.feature_block_entities.iter().any(|(&p, entity)| {
            chunk.pending_block_entities.contains_key(&p)
                || !chunk
                    .get(p.0, p.1, p.2)
                    .is_some_and(|state| entity.valid_for(state, world_pos(p)))
        }) || chunk.pending_block_entities.iter().any(|(&p, entity)| {
            !chunk
                .get(p.0, p.1, p.2)
                .is_some_and(|state| entity.valid_for(state, world_pos(p)))
        }) {
            return Err(invalid("block entity owner, state or typed data"));
        }
        if !chunk.structures.valid_for(chunk.pos)
            || chunk
                .entities
                .iter()
                .any(|entity| !entity.valid_for(chunk.pos))
            || chunk
                .structure_entities
                .iter()
                .any(|entity| !entity.valid_for(chunk.pos))
            || chunk
                .tick_requests
                .iter()
                .any(|tick| !tick.valid_for(chunk.pos))
            || chunk
                .postprocessing
                .iter()
                .any(|&(x, y, z)| chunk.get(x, y, z).is_none())
            || chunk
                .light
                .as_ref()
                .is_some_and(|light| !light.validate(MIN_Y, WORLD_HEIGHT, true))
        {
            return Err(invalid("effect owner or light geometry"));
        }
        chunk.runtime_heightmaps = Some(RuntimeHeightmaps::capture(&chunk));
        Ok(chunk)
    }
}

impl GenerationWorld {
    /// Validate the whole batch before changing the region. Existing saved holders
    /// remain live; re-reading disk must never replace their objects or mutations.
    pub fn admit_saved_chunks(&self, data: Vec<SavedChunkData>) -> Result<(), GenerationError> {
        let mut positions = BTreeSet::new();
        let chunks = data
            .into_iter()
            .map(|data| {
                if !positions.insert((data.pos.x, data.pos.z)) {
                    return Err(GenerationError::InvalidSavedChunk {
                        pos: data.pos,
                        detail: "duplicate admission position".into(),
                    });
                }
                data.into_chunk()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        for chunk in &chunks {
            if state
                .holders
                .get(&(chunk.pos.x, chunk.pos.z))
                .is_some_and(|holder| {
                    !holder.saved
                        && holder
                            .progress
                            .stages
                            .iter()
                            .any(|stage| stage.state != StageState::Pending)
                })
            {
                return Err(GenerationError::SavedChunkConflict(chunk.pos));
            }
        }
        let prepared = chunks
            .into_iter()
            .filter(|chunk| {
                !state
                    .holders
                    .get(&(chunk.pos.x, chunk.pos.z))
                    .is_some_and(|holder| holder.saved)
            })
            .map(|chunk| {
                let light = state
                    .light
                    .prepare_saved_chunk(chunk.pos, chunk.states(), chunk.light())
                    .map_err(|error| GenerationError::InvalidSavedChunk {
                        pos: chunk.pos,
                        detail: format!("shared light inputs: {error}"),
                    })?;
                Ok((chunk, light))
            })
            .collect::<Result<Vec<_>, GenerationError>>()?;
        for (chunk, light) in prepared {
            let key = (chunk.pos.x, chunk.pos.z);
            let mut holder = ChunkHolder::new(chunk.pos);
            holder.saved = true;
            holder.structures = chunk.structures.clone();
            holder.progress.stages = std::array::from_fn(|_| {
                StageProgress {
                state: StageState::Partial,
                attempts: 0,
                missing: vec!["Saved BCC data has no native generation history, WG maps or world registration".into()],
            }
            });
            holder.lifecycle = super::full::ChunkLifecycle::Level(Box::new(
                super::LevelChunkState::from_saved(&chunk),
            ));
            state.light.admit_saved_chunk(light);
            state.region.insert_owned_chunk(chunk);
            state.holders.insert(key, holder);
        }
        state.capture_light_outputs();
        Ok(())
    }

    pub fn is_saved_chunk(&self, pos: ChunkPos) -> Result<bool, GenerationError> {
        let state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        Ok(state
            .holders
            .get(&(pos.x, pos.z))
            .is_some_and(|holder| holder.saved))
    }
}
