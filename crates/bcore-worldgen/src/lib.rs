//! Deterministic prototype world generation for BCore.
//!
//! This is intentionally not vanilla-parity generation. It provides a small,
//! reproducible seed-based terrain skeleton while registries and parity tests
//! are still under development.

use bcore_core::{registry, BlockPos, ChunkPos};
pub use registry::BlockId;

/// Width and depth of a chunk column in blocks.
pub const CHUNK_SIZE: usize = 16;
/// Height of generated chunk columns.
pub const CHUNK_HEIGHT: usize = 256;
/// Sea level used by the prototype generator.
pub const SEA_LEVEL: i32 = 62;

/// A 16x256x16 block column, stored in y/z/x order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    blocks: Vec<BlockId>,
}

impl Chunk {
    /// Creates an all-air chunk.
    pub fn new() -> Self {
        Self {
            blocks: vec![registry::AIR; CHUNK_SIZE * CHUNK_HEIGHT * CHUNK_SIZE],
        }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        (y * CHUNK_SIZE + z) * CHUNK_SIZE + x
    }

    /// Returns a block, or `None` if the local coordinates are outside the chunk.
    pub fn get(&self, x: usize, y: i32, z: usize) -> Option<BlockId> {
        if x >= CHUNK_SIZE || z >= CHUNK_SIZE || !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return None;
        }
        Some(self.blocks[Self::index(x, y as usize, z)])
    }

    /// Sets a block and returns whether the local coordinates were valid.
    pub fn set(&mut self, x: usize, y: i32, z: usize, block: BlockId) -> bool {
        if x >= CHUNK_SIZE || z >= CHUNK_SIZE || !(0..CHUNK_HEIGHT as i32).contains(&y) {
            return false;
        }
        self.blocks[Self::index(x, y as usize, z)] = block;
        true
    }

    /// Returns a block using absolute block coordinates.
    pub fn get_block(&self, pos: BlockPos, chunk: ChunkPos) -> Option<BlockId> {
        let local = (pos.x - (chunk.x << 4), pos.y, pos.z - (chunk.z << 4));
        self.get(local.0 as usize, local.1, local.2 as usize)
    }

    /// Sets a block using absolute block coordinates.
    pub fn set_block(&mut self, pos: BlockPos, chunk: ChunkPos, block: BlockId) -> bool {
        let local = (pos.x - (chunk.x << 4), pos.y, pos.z - (chunk.z << 4));
        if !(0..CHUNK_SIZE as i32).contains(&local.0) || !(0..CHUNK_SIZE as i32).contains(&local.2)
        {
            return false;
        }
        self.set(local.0 as usize, local.1, local.2 as usize, block)
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

/// Deterministic terrain generator. No global state or non-deterministic maps are used.
#[derive(Debug, Clone, Copy)]
pub struct WorldGenerator {
    seed: i64,
}

impl WorldGenerator {
    pub fn new(seed: i64) -> Self {
        Self { seed }
    }

    /// Returns the deterministic terrain surface height at absolute coordinates.
    pub fn height_at(&self, x: i32, z: i32) -> i32 {
        // Multiple smooth value-noise octaves give structured, reproducible terrain.
        let broad = value_noise(self.seed, x as f64 / 96.0, z as f64 / 96.0);
        let detail = value_noise(
            self.seed ^ (0x517cc1b727220a95_u64 as i64),
            x as f64 / 32.0,
            z as f64 / 32.0,
        );
        let ridges = value_noise(
            self.seed.wrapping_add(0x6a09e667f3bcc909),
            x as f64 / 16.0,
            z as f64 / 16.0,
        );
        (58.0 + broad * 28.0 + detail * 11.0 + ridges * 4.0)
            .round()
            .clamp(8.0, (CHUNK_HEIGHT - 1) as f64) as i32
    }

    /// Generates one complete chunk deterministically from its position and seed.
    pub fn generate_chunk(&self, pos: ChunkPos) -> Chunk {
        let mut chunk = Chunk::new();
        let base_x = pos.x << 4;
        let base_z = pos.z << 4;
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let surface = self.height_at(base_x + x as i32, base_z + z as i32);
                chunk.set(x, 0, z, registry::BEDROCK);
                for y in 1..=surface {
                    let block = if y == surface {
                        registry::GRASS_BLOCK
                    } else if y >= surface - 3 {
                        registry::DIRT
                    } else {
                        registry::STONE
                    };
                    chunk.set(x, y, z, block);
                }
                if surface < SEA_LEVEL {
                    for y in (surface + 1)..=SEA_LEVEL {
                        chunk.set(x, y, z, registry::WATER);
                    }
                }
            }
        }
        chunk
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    let mut result = value;
    result = (result ^ (result >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    result = (result ^ (result >> 27)).wrapping_mul(0x94d049bb133111eb);
    result ^ (result >> 31)
}

fn lattice(seed: i64, x: i64, z: i64) -> f64 {
    let mixed = (seed as u64)
        ^ (x as u64).wrapping_mul(0x9e3779b185ebca87)
        ^ (z as u64).wrapping_mul(0xc2b2ae3d27d4eb4f);
    (splitmix64(mixed) as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(seed: i64, x: f64, z: f64) -> f64 {
    let x0 = x.floor() as i64;
    let z0 = z.floor() as i64;
    let tx = smooth(x - x0 as f64);
    let tz = smooth(z - z0 as f64);
    let a = lattice(seed, x0, z0);
    let b = lattice(seed, x0 + 1, z0);
    let c = lattice(seed, x0, z0 + 1);
    let d = lattice(seed, x0 + 1, z0 + 1);
    let ab = a + (b - a) * tx;
    let cd = c + (d - c) * tx;
    ab + (cd - ab) * tz
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_byte_identical_for_same_seed_and_position() {
        let generator = WorldGenerator::new(12345);
        assert_eq!(
            generator.generate_chunk(ChunkPos::new(-3, 7)),
            generator.generate_chunk(ChunkPos::new(-3, 7))
        );
    }

    #[test]
    fn different_seeds_change_chunk_output() {
        let a = WorldGenerator::new(1).generate_chunk(ChunkPos::new(0, 0));
        let b = WorldGenerator::new(2).generate_chunk(ChunkPos::new(0, 0));
        assert_ne!(a, b);
    }

    #[test]
    fn height_is_stable() {
        let generator = WorldGenerator::new(-99);
        assert_eq!(
            generator.height_at(123, -456),
            generator.height_at(123, -456)
        );
    }

    #[test]
    fn generated_layers_obey_basic_terrain_rules() {
        let generator = WorldGenerator::new(0);
        let pos = ChunkPos::new(0, 0);
        let chunk = generator.generate_chunk(pos);
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                assert_eq!(chunk.get(x, 0, z), Some(registry::BEDROCK));
                let h = generator.height_at(x as i32, z as i32);
                assert_eq!(chunk.get(x, h, z), Some(registry::GRASS_BLOCK));
            }
        }
    }
}
