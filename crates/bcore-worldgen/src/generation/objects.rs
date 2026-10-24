//! Returned object identity is independent of the holder's replaceable futures.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bcore_core::ChunkPos;

use super::full::{block_entity_error, live_block_entity_tags, ChunkLifecycle};
use super::{ChunkAccessKind, ChunkStatus, GenerationCoverage, GenerationError, GenerationWorld};
use crate::block_entity::PendingBlockEntity;
use crate::feature_world::FeatureError;
use crate::structure::template::Nbt;
use crate::tick_queue::{PreparedTickQueues, SavedTick};
use crate::{GeneratedChunk, RuntimeHeightmaps};

type LocalPos = (usize, i32, usize);

#[derive(Default)]
pub(super) struct ChunkObjects {
    ids: [Option<Arc<()>>; 3],
}

impl ChunkObjects {
    fn identity(&mut self, kind: ChunkAccessKind) -> Arc<()> {
        let index = match kind {
            ChunkAccessKind::Proto => 0,
            ChunkAccessKind::ImposterProto => 1,
            ChunkAccessKind::Level => 2,
        };
        self.ids[index].get_or_insert_with(|| Arc::new(())).clone()
    }
}

/// Metadata owned by the old proto, not by the shared section storage. Existing
/// live BE members still read the canonical live objects; level-save promotions
/// are new members of the level map and must not appear in this proto's map.
#[derive(Debug)]
pub(super) struct RetainedProtoMetadata {
    pub status: ChunkStatus,
    pending: BTreeMap<LocalPos, PendingBlockEntity>,
    runtime_heightmaps: Option<RuntimeHeightmaps>,
    core_members: BTreeSet<LocalPos>,
    feature_members: BTreeSet<LocalPos>,
}

impl RetainedProtoMetadata {
    /// The same live member can acquire a typed inventory representation. A
    /// pending tag promoted only on the level must not join this proto's map.
    pub(super) fn promote_core_member(&mut self, local: LocalPos) {
        if self.core_members.remove(&local) {
            self.feature_members.insert(local);
        }
    }

    pub fn capture(chunk: &GeneratedChunk, status: ChunkStatus) -> Self {
        Self {
            status,
            pending: chunk.pending_block_entities.clone(),
            runtime_heightmaps: chunk.runtime_heightmaps.clone(),
            core_members: chunk.block_entities.keys().copied().collect(),
            feature_members: chunk.feature_block_entities.keys().copied().collect(),
        }
    }

    fn project(&self, chunk: &mut GeneratedChunk) -> Result<(), GenerationError> {
        // Removal/recreation needs per-entity gameplay ownership. Do not replace
        // a missing retained live object with invented saved data.
        if !self
            .core_members
            .iter()
            .all(|p| chunk.block_entities.contains_key(p))
            || !self
                .feature_members
                .iter()
                .all(|p| chunk.feature_block_entities.contains_key(p))
        {
            return Err(block_entity_error(
                chunk.pos,
                "retained proto view",
                FeatureError::Unsupported("retained live block-entity member was removed".into()),
            ));
        }
        chunk.pending_block_entities.clone_from(&self.pending);
        chunk
            .runtime_heightmaps
            .clone_from(&self.runtime_heightmaps);
        chunk
            .block_entities
            .retain(|p, _| self.core_members.contains(p));
        chunk
            .feature_block_entities
            .retain(|p, _| self.feature_members.contains(p));
        Ok(())
    }
}

/// One particular native-style object, which never changes into an imposter.
/// Reads follow live storage, including background SPAWN, rather than keeping a
/// GeneratedChunk snapshot alive and causing copy-on-write in the region.
#[derive(Debug, Clone)]
pub struct ReturnedChunk {
    world: Arc<GenerationWorld>,
    pos: ChunkPos,
    kind: ChunkAccessKind,
    identity: Arc<()>,
}

/// An inert observation of an object. Persisted/class status and implementation
/// coverage are separate: a Level object can still have Partial FULL coverage.
#[derive(Debug, Clone)]
pub struct ReturnedChunkView {
    pub chunk: GeneratedChunk,
    pub kind: ChunkAccessKind,
    pub persisted_status: ChunkStatus,
    pub coverage: GenerationCoverage,
}

#[derive(Debug, Clone)]
pub struct ReturnedChunkSave {
    pub view: ReturnedChunkView,
    pub block_entity_tags: BTreeMap<LocalPos, Nbt>,
    pub block_ticks: Vec<SavedTick<u32>>,
    pub fluid_ticks: Vec<SavedTick<u32>>,
}

impl ReturnedChunk {
    pub fn pos(&self) -> ChunkPos {
        self.pos
    }

    pub fn kind(&self) -> ChunkAccessKind {
        self.kind
    }

    pub fn same_object(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }

    pub fn belongs_to(&self, world: &Arc<GenerationWorld>) -> bool {
        Arc::ptr_eq(&self.world, world)
    }

    pub fn view(&self) -> Result<ReturnedChunkView, GenerationError> {
        let state = self
            .world
            .state
            .lock()
            .map_err(|_| GenerationError::Poisoned)?;
        let holder = &state.holders[&(self.pos.x, self.pos.z)];
        let mut chunk = state
            .region
            .owned_chunk_ref(self.pos)
            .map(|chunk| chunk.clone())
            .unwrap_or_else(|| GeneratedChunk::new(self.pos));
        chunk.structures = holder.structures.clone();
        let persisted_status = match (&holder.lifecycle, self.kind) {
            (ChunkLifecycle::Level(level), ChunkAccessKind::Proto) => {
                level.retained_proto.project(&mut chunk)?;
                chunk.worldgen_heightmaps = level.retired_proto_heightmaps().cloned();
                level.retained_proto.status
            }
            (ChunkLifecycle::Level(_), _) => ChunkStatus::Full,
            (ChunkLifecycle::Proto, _) => holder
                .progress
                .available_status()
                .expect("returned object has available data"),
        };
        Ok(ReturnedChunkView {
            chunk,
            kind: self.kind,
            persisted_status,
            coverage: state.coverage(self.pos, persisted_status),
        })
    }

    /// Invoke user code after releasing the generation mutex and all region
    /// borrows. Reentrant observations and requests are safe at this boundary.
    pub fn read<R>(&self, read: impl FnOnce(ReturnedChunkView) -> R) -> Result<R, GenerationError> {
        let view = self.view()?;
        Ok(read(view))
    }

    /// Serialize the selected object, not whichever object the holder now owns.
    /// Proto saving retains pending tags; level/imposter saving delegates to the
    /// existing atomic materialization boundary and leaves tick unpacking explicit.
    pub fn prepare_save(&self, game_time: i64) -> Result<ReturnedChunkSave, GenerationError> {
        if self.kind != ChunkAccessKind::Proto {
            let saved = self.world.prepare_chunk_save(self.pos, game_time)?;
            let mut view = self.view()?;
            view.chunk = saved.chunk;
            return Ok(ReturnedChunkSave {
                view,
                block_entity_tags: saved.block_entity_tags,
                block_ticks: saved.block_ticks,
                fluid_ticks: saved.fluid_ticks,
            });
        }
        let view = self.view()?;
        let mut tags = live_block_entity_tags(&view.chunk)
            .map_err(|e| block_entity_error(self.pos, "proto save", e))?;
        for (&local, pending) in view.chunk.pending_block_entities() {
            // ProtoChunk saves an existing live entity first, otherwise the
            // pending tag verbatim. Only LevelChunk adds keepPacked envelopes.
            if let std::collections::btree_map::Entry::Vacant(entry) = tags.entry(local) {
                entry.insert(
                    pending
                        .full_nbt()
                        .map_err(|e| block_entity_error(self.pos, "proto save", e))?,
                );
            }
        }
        let ticks = PreparedTickQueues::from_requests(view.chunk.tick_requests().iter().copied());
        Ok(ReturnedChunkSave {
            view,
            block_entity_tags: tags,
            block_ticks: ticks.blocks.pack(game_time),
            fluid_ticks: ticks.fluids.pack(game_time),
        })
    }
}

impl GenerationWorld {
    pub(super) fn returned_chunk(
        self: &Arc<Self>,
        pos: ChunkPos,
        requested: ChunkStatus,
    ) -> Result<Option<ReturnedChunk>, GenerationError> {
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        let Some(holder) = state.holders.get_mut(&(pos.x, pos.z)) else {
            return Ok(None);
        };
        if !holder
            .progress
            .available_status()
            .is_some_and(|s| s >= requested)
        {
            return Ok(None);
        }
        let kind = match (&holder.lifecycle, requested) {
            (ChunkLifecycle::Proto, _) => ChunkAccessKind::Proto,
            (ChunkLifecycle::Level(_), ChunkStatus::Full) => ChunkAccessKind::Level,
            _ => ChunkAccessKind::ImposterProto,
        };
        let identity = holder.objects.identity(kind);
        Ok(Some(ReturnedChunk {
            world: self.clone(),
            pos,
            kind,
            identity,
        }))
    }
}
