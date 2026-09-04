//! First structure-generation scaffold: deterministic village starts and a house.
use bcore_core::ChunkPos;
use crate::{block, GeneratedChunk, CHUNK_SIZE, MAX_Y, MIN_Y, SEA_LEVEL};

pub const VILLAGE_SPACING: i32 = 34;
pub const VILLAGE_SEPARATION: i32 = 8;
pub const VILLAGE_SALT: i64 = 10_387_312;

/// Vanilla RandomSpreadStructurePlacement-shaped candidate test.
pub fn village_start(seed: i64, chunk: ChunkPos) -> bool {
    let rx = div_floor(chunk.x, VILLAGE_SPACING);
    let rz = div_floor(chunk.z, VILLAGE_SPACING);
    let mixed = (seed as u64)
        .wrapping_add((rx as i64 as u64).wrapping_mul(341_873_128_712))
        .wrapping_add((rz as i64 as u64).wrapping_mul(132_897_987_541))
        .wrapping_add(VILLAGE_SALT as u64);
    let first = crate::splitmix64(mixed);
    let range = (VILLAGE_SPACING - VILLAGE_SEPARATION) as u64;
    let ox = (first % range) as i32;
    let oz = (crate::splitmix64(first) % range) as i32;
    chunk.x == rx * VILLAGE_SPACING + ox && chunk.z == rz * VILLAGE_SPACING + oz
}

/// Place a clipped, deterministic oak house on the generated surface.
pub fn place_village_house(seed: i64, chunk: &mut GeneratedChunk) -> bool {
    if !village_start(seed, chunk.pos) { return false; }
    let cx = CHUNK_SIZE / 2;
    let cz = CHUNK_SIZE / 2;
    let ground = chunk.height_at(cx, cz);
    if ground < SEA_LEVEL || ground + 8 > MAX_Y { return false; }
    if !matches!(chunk.biome_at(cx, cz), crate::Biome::Plains | crate::Biome::Savanna) { return false; }
    let floor = ground + 1;
    let mut placed = false;
    let mut put = |x: i32, y: i32, z: i32, state: u32| {
        if (0..CHUNK_SIZE as i32).contains(&x) && (0..CHUNK_SIZE as i32).contains(&z)
            && (MIN_Y..=MAX_Y).contains(&y) && chunk.get(x as usize, y, z as usize) == Some(block::AIR) {
            chunk.set(x as usize, y, z as usize, state); placed = true;
        }
    };
    for dz in -3i32..=3 { for dx in -3i32..=3 { put(cx as i32 + dx, floor, cz as i32 + dz, block::OAK_PLANKS); } }
    for y in 1..=4 { for dz in -3i32..=3 { for dx in -3i32..=3 {
        if dx.abs() == 3 || dz.abs() == 3 { put(cx as i32 + dx, floor + y, cz as i32 + dz, if dx.abs() == 3 && dz.abs() == 3 { block::OAK_LOG } else { block::OAK_PLANKS }); }
    } } }
    for dz in -3i32..=3 { for dx in -3i32..=3 { put(cx as i32 + dx, floor + 5, cz as i32 + dz, block::OAK_PLANKS); } }
    placed
}

fn div_floor(value: i32, divisor: i32) -> i32 {
    let q = value / divisor;
    if value % divisor < 0 { q - 1 } else { q }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn village_start_is_deterministic() {
        let a = (-100..100).filter(|x| village_start(1234, ChunkPos::new(*x, 0))).count();
        let b = (-100..100).filter(|x| village_start(1234, ChunkPos::new(*x, 0))).count();
        assert_eq!(a, b);
    }
}
