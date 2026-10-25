//! Single-tick-owner creative block actions on canonical, already loaded Level data.
mod actions;
mod bindings;
pub mod packets;
mod queue;
mod runtime;

use std::collections::BTreeSet;
use std::sync::Arc;

use bcore_core::BlockPos;

use crate::gameplay::GameMode;

pub(crate) use queue::BlockQueue;
pub(crate) use runtime::BlockOwner;

#[derive(Debug, Clone)]
pub(crate) struct PlayerContext {
    pub position: [f64; 3],
    pub game_mode: GameMode,
    pub loaded: Arc<BTreeSet<(i32, i32)>>,
    pub ready: bool,
}

impl Default for PlayerContext {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            game_mode: GameMode::Survival,
            loaded: Arc::new(BTreeSet::new()),
            ready: false,
        }
    }
}

impl PlayerContext {
    fn has_chunk(&self, pos: BlockPos) -> bool {
        self.loaded.contains(&(pos.chunk().x, pos.chunk().z))
    }

    fn can_edit(&self, pos: BlockPos) -> bool {
        let bounds = bindings::player_bounds();
        let eyes = [
            self.position[0],
            self.position[1] + bounds.eye_height,
            self.position[2],
        ];
        self.ready
            && self.game_mode == GameMode::Creative
            && self.has_chunk(pos)
            && in_world(pos)
            && packets::within_reach(
                eyes,
                pos,
                bounds.block_range_default + bounds.creative_block_range_addition + 1.0,
            )
    }

    fn overlaps(&self, pos: BlockPos) -> bool {
        if !self.ready || self.game_mode == GameMode::Spectator {
            return false;
        }
        let bounds = bindings::player_bounds();
        let [x, y, z] = self.position;
        let half = bounds.width / 2.0;
        x - half < f64::from(pos.x) + 1.0
            && x + half > f64::from(pos.x)
            && y < f64::from(pos.y) + 1.0
            && y + bounds.height > f64::from(pos.y)
            && z - half < f64::from(pos.z) + 1.0
            && z + half > f64::from(pos.z)
    }
}

fn in_world(pos: BlockPos) -> bool {
    (crate::chunk::MIN_Y..=crate::chunk::MAX_Y).contains(&pos.y)
        && (-30_000_000..30_000_000).contains(&pos.x)
        && (-30_000_000..30_000_000).contains(&pos.z)
}

#[cfg(test)]
mod tests;
