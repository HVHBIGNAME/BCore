//! Deferred feature requests. These retain request order and duplicates; they
//! are not the native tick scheduler's deduplicated, game-time-ordered queue.
use bcore_core::ChunkPos;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TickTarget {
    /// Native default block-state ID, identifying the block to tick.
    Block(u32),
    /// Native BuiltInRegistries.FLUID ID.
    Fluid(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TickRequest {
    pub block_pos: [i32; 3],
    pub target: TickTarget,
    /// Relative delay from the native feature call, at NORMAL priority (0).
    pub delay: i32,
}

impl TickRequest {
    pub fn valid_for(&self, owner: ChunkPos) -> bool {
        let [x, y, z] = self.block_pos;
        x >> 4 == owner.x
            && z >> 4 == owner.z
            && (crate::MIN_Y..=crate::MAX_Y).contains(&y)
            && match self.target {
                TickTarget::Block(state) => state < 29_873,
                TickTarget::Fluid(id) => id < 5,
            }
    }
}
