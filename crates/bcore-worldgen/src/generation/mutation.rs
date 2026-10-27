//! Compare-and-write on canonical Level data without generation or BE replacement.
use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::sync::{MutexGuard, TryLockError};

use bcore_core::{BlockPos, ChunkPos};

use super::full::ChunkLifecycle;
use super::{GenerationError, GenerationState, GenerationWorld};

#[derive(Debug, Clone)]
pub struct LevelBlockEdit {
    pub pos: BlockPos,
    pub before: u32,
    pub after: u32,
    pub changed_chunks: Vec<ChunkPos>,
    pub light_changed_chunks: Vec<ChunkPos>,
}

/// One loaded-world batch, acquired between generation stages. Methods never
/// request generation or re-enter the generation mutex.
pub struct LevelAccess<'a> {
    state: MutexGuard<'a, GenerationState>,
}

impl GenerationWorld {
    pub fn level_access(&self) -> Result<LevelAccess<'_>, GenerationError> {
        Ok(LevelAccess {
            state: self.state.lock().map_err(|_| GenerationError::Poisoned)?,
        })
    }

    pub fn try_level_access(&self) -> Result<Option<LevelAccess<'_>>, GenerationError> {
        match self.state.try_lock() {
            Ok(state) => {
                self.runtime_waiting.store(false, Ordering::Release);
                self.runtime_wake.notify_one();
                Ok(Some(LevelAccess { state }))
            }
            Err(TryLockError::WouldBlock) => {
                self.runtime_waiting.store(true, Ordering::Release);
                Ok(None)
            }
            Err(TryLockError::Poisoned(_)) => Err(GenerationError::Poisoned),
        }
    }

    /// Mutate an existing Level object. The protocol owner validates the player,
    /// thread, mode, reach and supported block set before entering this boundary.
    pub fn edit_level_block(
        &self,
        pos: BlockPos,
        expected: u32,
        after: u32,
    ) -> Result<Option<LevelBlockEdit>, GenerationError> {
        self.level_access()?.edit_block(pos, expected, after)
    }
}

impl LevelAccess<'_> {
    pub fn block(&self, pos: BlockPos) -> Option<u32> {
        if self.state.level(pos.chunk()).is_err() {
            return None;
        }
        let (x, y, z) = pos.local();
        self.state
            .region
            .owned_chunk_ref(pos.chunk())?
            .get(x as usize, y, z as usize)
    }

    pub fn snapshot(&self, pos: ChunkPos) -> Result<crate::GeneratedChunk, GenerationError> {
        self.state.level(pos)?;
        Ok(self
            .state
            .region
            .owned_chunk_ref(pos)
            .expect("Level storage")
            .clone())
    }

    pub fn read_light<R>(
        &self,
        pos: ChunkPos,
        read: impl FnOnce(&crate::lighting::ChunkLight) -> R,
    ) -> Option<R> {
        self.state.level(pos).ok()?;
        let chunk = self.state.region.owned_chunk_ref(pos)?;
        chunk.light().map(read)
    }

    pub fn edit_block(
        &mut self,
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
        let state = &mut *self.state;
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
        match state.light.registered_block((pos.x, pos.y, pos.z)) {
            Some(light_state) if light_state == before => {}
            Some(_) => return Err(error("canonical block and light input disagree".into())),
            None => {
                let chunk = state
                    .region
                    .owned_chunk(pos.chunk())
                    .expect("Level storage");
                state
                    .light
                    .register_chunk(pos.chunk(), chunk.states())
                    .map_err(|e| error(e.to_string()))?;
            }
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
        let mut light_changed = Vec::new();
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
            let differs = state
                .region
                .owned_chunk_ref(column)
                .expect("retained light column")
                .light()
                != light.as_ref();
            if !differs {
                continue;
            }
            state.region.owned_chunk_mut(column).light = light;
            if state
                .holders
                .get(&(column.x, column.z))
                .is_some_and(|holder| matches!(holder.lifecycle, ChunkLifecycle::Level(_)))
            {
                changed.insert(column);
                light_changed.push(column);
            }
        }
        Ok(Some(LevelBlockEdit {
            pos,
            before,
            after,
            changed_chunks: changed.into_iter().collect(),
            light_changed_chunks: light_changed,
        }))
    }
}
