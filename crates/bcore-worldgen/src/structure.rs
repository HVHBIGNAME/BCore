//! First structure-generation scaffold: deterministic village starts and a house.
pub mod mineshaft;
pub mod placement;

use crate::{block, GeneratedChunk, CHUNK_SIZE, MAX_Y, MIN_Y, SEA_LEVEL};
use bcore_core::ChunkPos;
use placement::RandomSpreadPlacement;

pub const VILLAGE_SPACING: i32 = 34;
pub const VILLAGE_SEPARATION: i32 = 8;
pub const VILLAGE_SALT: i64 = 10_387_312;

/// Random-spread candidate only; biome/terrain admission is performed separately.
pub fn village_start(seed: i64, chunk: ChunkPos) -> bool {
    RandomSpreadPlacement::VILLAGES.is_candidate(seed, chunk)
}

/// Place a clipped, deterministic oak house on the generated surface.
pub fn place_village_house(seed: i64, chunk: &mut GeneratedChunk) -> bool {
    if !village_start(seed, chunk.pos) {
        return false;
    }
    let cx = CHUNK_SIZE / 2;
    let cz = CHUNK_SIZE / 2;
    let ground = chunk.height_at(cx, cz);
    if ground < SEA_LEVEL || ground + 8 > MAX_Y {
        return false;
    }
    if !matches!(
        chunk.biome_at(cx, cz),
        crate::Biome::Plains | crate::Biome::Savanna
    ) {
        return false;
    }
    let floor = ground + 1;
    let mut placed = false;
    let mut put = |x: i32, y: i32, z: i32, state: u32| {
        if (0..CHUNK_SIZE as i32).contains(&x)
            && (0..CHUNK_SIZE as i32).contains(&z)
            && (MIN_Y..=MAX_Y).contains(&y)
            && chunk.get(x as usize, y, z as usize) == Some(block::AIR)
        {
            chunk.set(x as usize, y, z as usize, state);
            placed = true;
        }
    };
    for dz in -3i32..=3 {
        for dx in -3i32..=3 {
            put(cx as i32 + dx, floor, cz as i32 + dz, block::OAK_PLANKS);
        }
    }
    for y in 1..=4 {
        for dz in -3i32..=3 {
            for dx in -3i32..=3 {
                if dx.abs() == 3 || dz.abs() == 3 {
                    put(
                        cx as i32 + dx,
                        floor + y,
                        cz as i32 + dz,
                        if dx.abs() == 3 && dz.abs() == 3 {
                            block::OAK_LOG
                        } else {
                            block::OAK_PLANKS
                        },
                    );
                }
            }
        }
    }
    for dz in -3i32..=3 {
        for dx in -3i32..=3 {
            put(cx as i32 + dx, floor + 5, cz as i32 + dz, block::OAK_PLANKS);
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn village_start_is_deterministic() {
        let a = (-100..100)
            .filter(|x| village_start(1234, ChunkPos::new(*x, 0)))
            .count();
        let b = (-100..100)
            .filter(|x| village_start(1234, ChunkPos::new(*x, 0)))
            .count();
        assert_eq!(a, b);
    }
}
