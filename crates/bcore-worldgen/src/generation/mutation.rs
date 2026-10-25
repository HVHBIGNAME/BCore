//! Compare-and-write on canonical Level data without generation or BE replacement.
use std::collections::BTreeSet;

use bcore_core::{BlockPos, ChunkPos};

use super::full::ChunkLifecycle;
use super::{GenerationError, GenerationWorld};

#[derive(Debug, Clone)]
pub struct LevelBlockEdit {
    pub pos: BlockPos,
    pub before: u32,
    pub after: u32,
    pub changed_chunks: Vec<ChunkPos>,
}

impl GenerationWorld {
    /// Mutate an existing Level object. The protocol owner validates the player,
    /// thread, mode, reach and supported block set before entering this boundary.
    pub fn edit_level_block(
        &self,
        pos: BlockPos,
        expected: u32,
        after: u32,
    ) -> Result<Option<LevelBlockEdit>, GenerationError> {
        super::validate_generation_pos(pos.chunk())?;
        let error = |detail: String| GenerationError::BlockEditFailed { pos, detail };
        if !(crate::MIN_Y..=crate::MAX_Y).contains(&pos.y)
            || after as usize >= bcore_registry::BLOCK_STATE_COUNT
        {
            return Err(error("block position or registry ID".into()));
        }
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        if !state
            .holders
            .get(&(pos.chunk().x, pos.chunk().z))
            .is_some_and(|holder| matches!(holder.lifecycle, ChunkLifecycle::Level(_)))
        {
            return Err(GenerationError::NotConverted(pos.chunk()));
        }
        let (x, y, z) = pos.local();
        let before = state
            .region
            .owned_chunk_ref(pos.chunk())
            .expect("Level storage")
            .get(x as usize, y, z as usize)
            .expect("validated block position");
        if before != expected || before == after {
            return Ok(None);
        }
        if crate::block_entity::has_block_entity(before)
            || crate::block_entity::has_block_entity(after)
        {
            return Err(error(
                "block entity replacement requires its ownership boundary".into(),
            ));
        }
        if let Some(reason) = &state.light_missing {
            return Err(error(format!("shared lighting unavailable: {reason}")));
        }
        {
            let chunk = state
                .region
                .owned_chunk(pos.chunk())
                .expect("Level storage");
            state
                .light
                .register_chunk(pos.chunk(), chunk.states())
                .map_err(|e| error(e.to_string()))?;
        }
        state
            .light
            .apply_block_updates(&[((pos.x, pos.y, pos.z), after)])
            .map_err(|e| error(e.to_string()))?;
        assert!(state
            .region
            .owned_chunk_mut(pos.chunk())
            .set(x as usize, y, z as usize, after));
        let mut changed = BTreeSet::from([pos.chunk()]);
        for column in state.light.take_changed_columns() {
            if state.region.owned_chunk(column).is_none() {
                continue;
            }
            let light = state.light.chunk_light(column);
            let light = if state.light.is_initialized(column)
                || light
                    .sections
                    .iter()
                    .any(|section| section.sky.is_some() || section.block.is_some())
            {
                Some(light)
            } else {
                None
            };
            state.region.owned_chunk_mut(column).light = light;
            if state
                .holders
                .get(&(column.x, column.z))
                .is_some_and(|holder| matches!(holder.lifecycle, ChunkLifecycle::Level(_)))
            {
                changed.insert(column);
            }
        }
        Ok(Some(LevelBlockEdit {
            pos,
            before,
            after,
            changed_chunks: changed.into_iter().collect(),
        }))
    }
}
