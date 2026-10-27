//! Versioned inert snapshots; only a completed write can clear its dirty revision.
use bcore_core::ChunkPos;
use bcore_worldgen::generation::LevelAccess;

use super::{World, WorldError, WorldMutationOwner, WorldUpdates};
use crate::chunk::ChunkColumn;
use crate::chunk_store::ChunkStore;

struct DirtyColumn {
    pos: ChunkPos,
    revision: u64,
    column: ChunkColumn,
}

#[derive(Default)]
pub(crate) struct DirtyBatch {
    columns: Vec<DirtyColumn>,
}

impl DirtyBatch {
    pub(crate) fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub(crate) fn write(&self, store: Option<&ChunkStore>) -> Result<(), WorldError> {
        if let Some(store) = store {
            for saved in &self.columns {
                store.save(saved.pos.x, saved.pos.z, &saved.column)?;
            }
        }
        Ok(())
    }
}

impl WorldUpdates<'_> {
    pub(crate) fn dirty_snapshot(&self) -> Result<DirtyBatch, WorldError> {
        self.world.dirty_snapshot(&self.access)
    }
}

impl World {
    fn dirty_snapshot(&self, access: &LevelAccess<'_>) -> Result<DirtyBatch, WorldError> {
        let positions: Vec<_> = self
            .inner
            .dirty_chunks
            .lock()
            .expect("dirty chunk lock")
            .iter()
            .map(|(&pos, &revision)| (pos, revision))
            .collect();
        let mut columns = Vec::with_capacity(positions.len());
        for (pos, revision) in positions {
            let chunk = access.snapshot(pos)?;
            columns.push(DirtyColumn {
                pos,
                revision,
                column: ChunkColumn::from_generated(&chunk),
            });
        }
        Ok(DirtyBatch { columns })
    }

    pub(crate) fn complete_dirty_save(
        &self,
        owner: &WorldMutationOwner,
        batch: &DirtyBatch,
    ) -> Result<(), WorldError> {
        self.check_mutation_owner(owner)?;
        let mut dirty = self.inner.dirty_chunks.lock().expect("dirty chunk lock");
        for saved in &batch.columns {
            if dirty.get(&saved.pos) == Some(&saved.revision) {
                dirty.remove(&saved.pos);
            }
        }
        Ok(())
    }

    /// Shutdown/retry boundary. Live ticks send the same inert snapshots to their
    /// single save worker, keeping encoding, file validation and fsync off-tick.
    pub(crate) fn flush_dirty_chunks(
        &self,
        owner: &WorldMutationOwner,
    ) -> Result<usize, WorldError> {
        self.check_mutation_owner(owner)?;
        if self
            .inner
            .dirty_chunks
            .lock()
            .expect("dirty chunk lock")
            .is_empty()
        {
            return Ok(0);
        }
        let batch = {
            let _publication = self
                .inner
                .publication
                .write()
                .expect("world publication lock");
            self.dirty_snapshot(&self.inner.generation.level_access()?)?
        };
        batch.write(self.store())?;
        self.complete_dirty_save(owner, &batch)?;
        Ok(batch.columns.len())
    }
}
