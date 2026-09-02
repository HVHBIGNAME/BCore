//! Block and chunk position types.

use std::ops::{Add, Sub};

/// A position of a single block, in absolute block coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The chunk column containing this block.
    pub fn chunk(self) -> ChunkPos {
        ChunkPos::new(self.x >> 4, self.z >> 4)
    }

    /// Coordinates local to the containing chunk: `(x in 0..16, y, z in 0..16)`.
    pub fn local(self) -> (u8, i32, u8) {
        ((self.x & 15) as u8, self.y, (self.z & 15) as u8)
    }
}

impl Add for BlockPos {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for BlockPos {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

/// A chunk column position, in chunk coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// The block position of the chunk's minimum corner (y = 0).
    pub fn block_start(self) -> BlockPos {
        BlockPos::new(self.x << 4, 0, self.z << 4)
    }
}

/// A floating-point position (entities / players).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
