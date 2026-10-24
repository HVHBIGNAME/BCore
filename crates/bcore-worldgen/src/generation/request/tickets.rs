//! Quiescent UNKNOWN loading tickets: retained minima, no clock or expiry.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bcore_core::ChunkPos;

use super::super::{ChunkPyramid, ChunkStatus, Key};

pub const FULL_CHUNK_LEVEL: u8 = 33;

/// ChunkLevel.byStatus uses the FULL step's accumulated radius, not ordinals.
pub fn ticket_level(status: ChunkStatus) -> u8 {
    FULL_CHUNK_LEVEL
        + ChunkPyramid::Generation
            .step(ChunkStatus::Full)
            .layer_radius(status)
            .expect("FULL pyramid includes every status") as u8
}

pub fn maximum_loading_level() -> u8 {
    FULL_CHUNK_LEVEL
        + ChunkPyramid::Generation
            .step(ChunkStatus::Full)
            .accumulated
            .radius() as u8
}

pub fn generation_status(level: u8) -> Option<ChunkStatus> {
    if level <= FULL_CHUNK_LEVEL {
        Some(ChunkStatus::Full)
    } else {
        ChunkPyramid::Generation
            .step(ChunkStatus::Full)
            .accumulated
            .at_radius(usize::from(level - FULL_CHUNK_LEVEL))
    }
}

#[derive(Default)]
pub(super) struct LoadingTickets {
    sources: BTreeMap<Key, u8>,
    levels: BTreeMap<Key, u8>,
    pending: VecDeque<(Key, u8)>,
}

impl LoadingTickets {
    pub fn add(&mut self, pos: ChunkPos, level: u8) {
        let key = (pos.x, pos.z);
        if self.sources.get(&key).is_some_and(|&old| old <= level) {
            return;
        }
        self.sources.insert(key, level);
        self.pending.push_back((key, level));
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Relax all eight adjacent chunks at cost one, stopping at the unloaded
    /// boundary. Returns changed positions only, for the holder's two passes.
    pub fn settle(&mut self) -> Vec<(ChunkPos, u8)> {
        let mut changed = BTreeSet::new();
        let maximum = maximum_loading_level();
        while let Some(((x, z), level)) = self.pending.pop_front() {
            if level > maximum || self.levels.get(&(x, z)).is_some_and(|&old| old <= level) {
                continue;
            }
            self.levels.insert((x, z), level);
            changed.insert((x, z));
            if level == maximum {
                continue;
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if dx != 0 || dz != 0 {
                        self.pending.push_back(((x + dx, z + dz), level + 1));
                    }
                }
            }
        }
        changed
            .into_iter()
            .map(|key| (ChunkPos::new(key.0, key.1), self.levels[&key]))
            .collect()
    }
}
