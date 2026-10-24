use bcore_core::ChunkPos;
use bcore_worldgen::GenerationWorld;

pub fn generate_chunk_blocks(seed: i64, cx: i32, cz: i32) -> Vec<u32> {
    GenerationWorld::new(seed)
        .generate_chunk(ChunkPos::new(cx, cz))
        .expect("chunk generation failed")
        .chunk
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
