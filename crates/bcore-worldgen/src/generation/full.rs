//! Retained FULL object conversion, independently of saving and ticking readiness.
use std::collections::BTreeMap;

use bcore_core::ChunkPos;

use super::{ChunkStatus, GenerationError, GenerationState, GenerationWorld};
use crate::block_entity::BlockEntity;
use crate::feature_world::{FeatureError, FeatureHeightmap};
use crate::structure::template::Nbt;
use crate::tick_queue::{LevelTickQueue, PreparedTickQueues, SavedTick};
use crate::{GeneratedChunk, RuntimeHeightmaps, WorldgenHeightmaps};

pub(super) enum ChunkLifecycle {
    Proto,
    Level(Box<LevelChunkState>),
}

/// Runtime ownership attached to the holder's canonical region data. The box
/// survives holder-map growth; snapshots do not own or replace this state.
#[derive(Debug)]
pub struct LevelChunkState {
    block_ticks: LevelTickQueue<u32>,
    fluid_ticks: LevelTickQueue<u32>,
    retired_proto_heightmaps: Option<WorldgenHeightmaps>,
    pub(super) retained_proto: super::objects::RetainedProtoMetadata,
}

impl LevelChunkState {
    fn from_proto(chunk: &mut GeneratedChunk, ticks: PreparedTickQueues) -> Self {
        let retained_proto =
            super::objects::RetainedProtoMetadata::capture(chunk, ChunkStatus::Spawn);
        if chunk.runtime_heightmaps.is_none() {
            chunk.runtime_heightmaps = Some(RuntimeHeightmaps::capture(chunk));
        }
        Self {
            block_ticks: LevelTickQueue::from_saved(ticks.blocks.pack(0)),
            fluid_ticks: LevelTickQueue::from_saved(ticks.fluids.pack(0)),
            retired_proto_heightmaps: chunk.worldgen_heightmaps.take(),
            retained_proto,
        }
    }

    pub fn block_ticks(&self) -> &LevelTickQueue<u32> {
        &self.block_ticks
    }

    pub fn fluid_ticks(&self) -> &LevelTickQueue<u32> {
        &self.fluid_ticks
    }

    /// Retired proto metadata, not the maps used by lower-status holder reads.
    pub fn retired_proto_heightmaps(&self) -> Option<&WorldgenHeightmaps> {
        self.retired_proto_heightmaps.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkAccessKind {
    Proto,
    ImposterProto,
    Level,
}

/// A live holder reference. Each read resolves the current canonical data under
/// the world mutex, including conversion or materialization since an earlier read.
/// Cloning this handle does not clone a GeneratedChunk or trigger copy-on-write.
#[derive(Debug, Clone, Copy)]
pub struct ChunkAccess<'world> {
    world: &'world GenerationWorld,
    pos: ChunkPos,
    requested: ChunkStatus,
}

/// Borrowed together under the world mutex; `chunk` is also used by region reads.
pub struct ChunkRead<'a> {
    pub chunk: &'a GeneratedChunk,
    pub kind: ChunkAccessKind,
    /// The wrapped runtime state for inspection, not lower-view tick admission.
    pub level: Option<&'a LevelChunkState>,
}

impl ChunkRead<'_> {
    /// Native highest matching block Y, unlike placement helpers' first-free Y.
    pub fn height(&self, kind: FeatureHeightmap, x: usize, z: usize) -> Option<i32> {
        if x >= 16 || z >= 16 {
            return None;
        }
        if let Some(wg) = self.chunk.worldgen_heightmaps() {
            let map = match kind {
                FeatureHeightmap::WorldSurfaceWg => Some(&wg.world_surface),
                FeatureHeightmap::OceanFloorWg => Some(&wg.ocean_floor),
                _ => None,
            };
            if let Some(map) = map {
                return Some(map[z * 16 + x] - 1);
            }
        }
        self.chunk
            .runtime_heightmaps()?
            .first_free(kind, x, z)
            .map(|y| y - 1)
    }
}

impl ChunkAccess<'_> {
    /// Holder identity, independent of the requested status or a detached snapshot.
    pub fn same_holder(&self, other: &Self) -> bool {
        std::ptr::eq(self.world, other.world) && self.pos == other.pos
    }

    /// The callback must not re-enter this world while its mutex is held.
    pub fn read<R>(&self, read: impl FnOnce(ChunkRead<'_>) -> R) -> Result<R, GenerationError> {
        let state = self
            .world
            .state
            .lock()
            .map_err(|_| GenerationError::Poisoned)?;
        let holder = &state.holders[&(self.pos.x, self.pos.z)];
        let chunk = state
            .region
            .owned_chunk_ref(self.pos)
            .expect("retained chunk access");
        let (kind, level) = match &holder.lifecycle {
            ChunkLifecycle::Proto => (ChunkAccessKind::Proto, None),
            ChunkLifecycle::Level(level) => (
                if self.requested == ChunkStatus::Full {
                    ChunkAccessKind::Level
                } else {
                    ChunkAccessKind::ImposterProto
                },
                Some(level.as_ref()),
            ),
        };
        Ok(read(ChunkRead {
            chunk: &chunk,
            kind,
            level,
        }))
    }

    pub fn snapshot(&self) -> Result<GeneratedChunk, GenerationError> {
        self.read(|view| view.chunk.clone())
    }

    /// Lookup delegates to the live data even through a read-only lower view.
    /// This is a separate, observable boundary from `read` and `snapshot`.
    pub fn lookup_block_entity(&self, x: usize, y: i32, z: usize) -> Result<bool, GenerationError> {
        let mut state = self
            .world
            .state
            .lock()
            .map_err(|_| GenerationError::Poisoned)?;
        state
            .region
            .owned_chunk_mut(self.pos)
            .materialize_block_entity(x, y, z)
            .map_err(|error| block_entity_error(self.pos, "lookup", error))
    }
}

/// Detached output of an explicit level-save boundary. BCC retains `chunk`'s raw
/// request history; native saved queues and BE save tags are separate projections.
#[derive(Debug, Clone)]
pub struct LevelChunkSave {
    pub chunk: GeneratedChunk,
    pub block_entity_tags: BTreeMap<(usize, i32, usize), Nbt>,
    pub block_ticks: Vec<SavedTick<u32>>,
    pub fluid_ticks: Vec<SavedTick<u32>>,
}

impl GenerationWorld {
    /// Acquire an already available holder without scheduling generation. Earlier
    /// handles resolve to an ImposterProto view after this holder converts to FULL.
    pub fn chunk_access(
        &self,
        pos: ChunkPos,
        requested: ChunkStatus,
    ) -> Result<Option<ChunkAccess<'_>>, GenerationError> {
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        if !state
            .holders
            .get(&(pos.x, pos.z))
            .and_then(|holder| holder.progress.available_status())
            .is_some_and(|available| available >= requested)
        {
            return Ok(None);
        }
        if state.region.owned_chunk_ref(pos).is_none() {
            state.region.owned_chunk_mut(pos);
        }
        Ok(Some(ChunkAccess {
            world: self,
            pos,
            requested,
        }))
    }

    /// Materialize supported pending tags on the retained level chunk, then copy
    /// its save projections. Failure preserves the live data and FULL claim.
    /// This neither unpacks ticks nor consumes generation postprocessing marks.
    pub fn prepare_chunk_save(
        &self,
        pos: ChunkPos,
        game_time: i64,
    ) -> Result<LevelChunkSave, GenerationError> {
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        state.level(pos)?;
        let chunk = state.region.owned_chunk_mut(pos);
        // Stage the detached save output before committing any promotions. Only
        // newly loaded entities are installed; existing live objects stay in place.
        let mut saved = chunk.clone();
        saved
            .materialize_block_entities()
            .map_err(|error| block_entity_error(pos, "save", error))?;
        let tags = level_block_entity_tags(&saved)
            .map_err(|error| block_entity_error(pos, "save", error))?;
        for &local in chunk.pending_block_entities.keys() {
            if let Some(entity) = saved.block_entities.get(&local) {
                chunk.block_entities.insert(local, entity.clone());
            } else {
                let entity = saved
                    .feature_block_entities
                    .get(&local)
                    .expect("materialized pending entity");
                chunk.feature_block_entities.insert(local, entity.clone());
            }
        }
        chunk.pending_block_entities.clear();
        let level = state.level(pos)?;
        Ok(LevelChunkSave {
            chunk: saved,
            block_entity_tags: tags,
            block_ticks: level.block_ticks.pack(game_time),
            fluid_ticks: level.fluid_ticks.pack(game_time),
        })
    }

    /// Activate saved queues at an explicit caller-supplied time. No tick is
    /// polled or executed and this does not claim world registration/readiness.
    pub fn unpack_chunk_ticks(&self, pos: ChunkPos, game_time: i64) -> Result<(), GenerationError> {
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        let Some(holder) = state.holders.get_mut(&(pos.x, pos.z)) else {
            return Err(GenerationError::NotConverted(pos));
        };
        let ChunkLifecycle::Level(level) = &mut holder.lifecycle else {
            return Err(GenerationError::NotConverted(pos));
        };
        level.block_ticks.unpack(game_time);
        level.fluid_ticks.unpack(game_time);
        Ok(())
    }
}

impl GenerationState {
    pub(super) fn level(&self, pos: ChunkPos) -> Result<&LevelChunkState, GenerationError> {
        match self
            .holders
            .get(&(pos.x, pos.z))
            .map(|holder| &holder.lifecycle)
        {
            Some(ChunkLifecycle::Level(level)) => Ok(level),
            _ => Err(GenerationError::NotConverted(pos)),
        }
    }

    pub(super) fn convert_full(&mut self, pos: ChunkPos) -> Result<Vec<String>, String> {
        let holder = self
            .holders
            .get_mut(&(pos.x, pos.z))
            .expect("claimed FULL holder");
        if let ChunkLifecycle::Proto = holder.lifecycle {
            let chunk = self.region.owned_chunk_mut(pos);
            if chunk
                .tick_requests()
                .iter()
                .any(|request| !request.valid_for(pos))
            {
                return Err("invalid proto tick request at FULL conversion".into());
            }
            let prepared = PreparedTickQueues::from_requests(chunk.tick_requests().iter().copied());
            holder.lifecycle =
                ChunkLifecycle::Level(Box::new(LevelChunkState::from_proto(chunk, prepared)));
        }
        Ok(vec![
            "Ticket expiry/unload, off-thread request dispatch and independent light-dispatch scheduling remain unimplemented".into(),
            "Post-load entity integration, live block-entity/tick-container world registration, full-status supplier and unsaved listener remain unimplemented".into(),
            "Template clear and dungeon/spawner callbacks on converted lower-status holders require verified live block-entity/inventory integration".into(),
        ])
    }
}

pub(super) fn block_entity_error(
    pos: ChunkPos,
    boundary: &'static str,
    error: FeatureError,
) -> GenerationError {
    GenerationError::BlockEntityFailed {
        pos,
        boundary,
        detail: error.to_string(),
    }
}

pub(super) fn live_block_entity_tags(
    chunk: &GeneratedChunk,
) -> Result<BTreeMap<(usize, i32, usize), Nbt>, FeatureError> {
    let mut tags = BTreeMap::new();
    for (&local, entity) in chunk.block_entities() {
        let (x, y, z) = local;
        let pos = (chunk.pos.x * 16 + x as i32, y, chunk.pos.z * 16 + z as i32);
        tags.insert(local, core_block_entity_tag(entity, pos));
    }
    for (&local, entity) in chunk.feature_block_entities() {
        tags.insert(local, entity.full_nbt()?);
    }
    Ok(tags)
}

fn level_block_entity_tags(
    chunk: &GeneratedChunk,
) -> Result<BTreeMap<(usize, i32, usize), Nbt>, FeatureError> {
    let mut tags = live_block_entity_tags(chunk)?;
    for tag in tags.values_mut() {
        tag.compound_mut()?
            .insert("keepPacked".into(), Nbt::Byte(0));
    }
    Ok(tags)
}

pub(super) fn core_block_entity_tag(entity: &BlockEntity, (x, y, z): (i32, i32, i32)) -> Nbt {
    let mut fields = BTreeMap::from([
        ("x".into(), Nbt::Int(x)),
        ("y".into(), Nbt::Int(y)),
        ("z".into(), Nbt::Int(z)),
        ("components".into(), Nbt::empty_compound()),
    ]);
    let id = match entity {
        BlockEntity::DungeonChest { loot_seed } => {
            fields.insert(
                "LootTable".into(),
                Nbt::String("minecraft:chests/simple_dungeon".into()),
            );
            if *loot_seed != 0 {
                fields.insert("LootTableSeed".into(), Nbt::Long(*loot_seed));
            }
            "minecraft:chest"
        }
        BlockEntity::Spawner { mob } => {
            for (name, value) in [
                ("Delay", 20),
                ("MaxNearbyEntities", 6),
                ("MaxSpawnDelay", 800),
                ("MinSpawnDelay", 200),
                ("RequiredPlayerRange", 16),
                ("SpawnCount", 4),
                ("SpawnRange", 4),
            ] {
                fields.insert(name.into(), Nbt::Short(value));
            }
            fields.insert(
                "SpawnData".into(),
                Nbt::Compound(BTreeMap::from([(
                    "entity".into(),
                    Nbt::Compound(BTreeMap::from([(
                        "id".into(),
                        Nbt::String(mob.name().into()),
                    )])),
                )])),
            );
            fields.insert(
                "SpawnPotentials".into(),
                Nbt::List {
                    element_type: 0,
                    values: Vec::new(),
                },
            );
            "minecraft:mob_spawner"
        }
        BlockEntity::Beehive { ticks_in_hive } => {
            let bees = ticks_in_hive
                .iter()
                .map(|&ticks| {
                    Nbt::Compound(BTreeMap::from([
                        (
                            "entity_data".into(),
                            Nbt::Compound(BTreeMap::from([(
                                "id".into(),
                                Nbt::String("minecraft:bee".into()),
                            )])),
                        ),
                        ("ticks_in_hive".into(), Nbt::Int(ticks)),
                        ("min_ticks_in_hive".into(), Nbt::Int(600)),
                    ]))
                })
                .collect::<Vec<_>>();
            fields.insert(
                "bees".into(),
                Nbt::List {
                    element_type: if bees.is_empty() { 0 } else { 10 },
                    values: bees,
                },
            );
            "minecraft:beehive"
        }
    };
    fields.insert("id".into(), Nbt::String(id.into()));
    Nbt::Compound(fields)
}

#[cfg(test)]
#[path = "full_tests.rs"]
mod tests;
