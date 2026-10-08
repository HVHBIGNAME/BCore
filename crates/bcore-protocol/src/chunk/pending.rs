//! Retain proto data through BCC without projecting DUMMY into client packets.
use bcore_core::ChunkPos;
use bcore_worldgen::block_entity::{MaterializedBlockEntity, PendingBlockEntity};
use bcore_worldgen::feature_world::FeatureError;

use super::ChunkColumn;

/// BlockEntityInfo.create projects an empty getUpdateTag compound to null. The
/// standalone update-tag encoder still returns the actual compound (10, 0).
pub(super) fn write_chunk_update_tag(out: &mut Vec<u8>, tag: &[u8]) {
    if tag == [10, 0] {
        out.push(0);
    } else {
        out.extend_from_slice(tag);
    }
}

impl ChunkColumn {
    pub fn pending_block_entities(
        &self,
    ) -> &std::collections::BTreeMap<(usize, i32, usize), PendingBlockEntity> {
        &self.pending_block_entities
    }

    pub fn set_pending_block_entity(
        &mut self,
        owner: ChunkPos,
        x: usize,
        y: i32,
        z: usize,
        data: PendingBlockEntity,
    ) -> bool {
        let Some(state) = self.get(x, y, z) else {
            return false;
        };
        let Some(pos) = pending_world_pos(owner, x, y, z) else {
            return false;
        };
        if self.block_entities.contains_key(&(x, y, z))
            || self.feature_block_entities.contains_key(&(x, y, z))
            || !data.valid_for(state, pos)
        {
            return false;
        }
        self.pending_block_entities.insert((x, y, z), data);
        true
    }

    /// Explicit lazy lookup for a persisted pending tag. Packet encoding never
    /// invokes this and continues to include only materialized entities.
    pub fn materialize_block_entity(
        &mut self,
        owner: ChunkPos,
        x: usize,
        y: i32,
        z: usize,
    ) -> Result<bool, FeatureError> {
        let local = (x, y, z);
        if self.block_entities.contains_key(&local)
            || self.feature_block_entities.contains_key(&local)
        {
            return Ok(true);
        }
        let Some(pending) = self.pending_block_entities.get(&local) else {
            return Ok(false);
        };
        let pos = pending_world_pos(owner, x, y, z).ok_or_else(|| {
            FeatureError::InvalidConfig("pending block entity owner overflow".into())
        })?;
        let data =
            pending.materialize(self.get(x, y, z).expect("validated pending position"), pos)?;
        match data {
            MaterializedBlockEntity::Generated(data) => {
                self.block_entities.insert(local, data);
            }
            MaterializedBlockEntity::Feature(data) => {
                self.feature_block_entities.insert(local, data);
            }
        }
        self.pending_block_entities.remove(&local);
        Ok(true)
    }
}

fn pending_world_pos(owner: ChunkPos, x: usize, y: i32, z: usize) -> Option<(i32, i32, i32)> {
    Some((
        owner
            .x
            .checked_mul(16)?
            .checked_add(i32::try_from(x).ok()?)?,
        y,
        owner
            .z
            .checked_mul(16)?
            .checked_add(i32::try_from(z).ok()?)?,
    ))
}
