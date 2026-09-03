use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;

pub fn generate_chunk_blocks(seed: i64, cx: i32, cz: i32) -> Vec<u32> {
    WorldGenerator::new(seed)
        .generate_chunk_vanilla(ChunkPos::new(cx, cz))
        .states()
        .to_vec()
}

#[test]
fn vanilla_chunk_generation_is_deterministic() {
    let a = generate_chunk_blocks(12345, 3, -2);
    let b = generate_chunk_blocks(12345, 3, -2);
    assert_eq!(a.len(), 384 * 256);
    assert_eq!(a, b);
}
