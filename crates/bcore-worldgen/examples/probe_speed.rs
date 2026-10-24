use bcore_core::ChunkPos;
use bcore_worldgen::GenerationWorld;
use std::time::Instant;

fn main() {
    let world = GenerationWorld::new(0x0BC0_0E00_1234_5678u64 as i64);
    let t0 = Instant::now();
    let c1 = world
        .generate_chunk(ChunkPos::new(0, -1))
        .expect("chunk generation failed");
    let d1 = t0.elapsed();
    let t1 = Instant::now();
    let c2 = world
        .generate_chunk(ChunkPos::new(1, -1))
        .expect("chunk generation failed");
    let d2 = t1.elapsed();
    let t2 = Instant::now();
    let c3 = world
        .generate_chunk(ChunkPos::new(2, -1))
        .expect("chunk generation failed");
    let d3 = t2.elapsed();
    eprintln!(
        "coverage complete: {}, {}, {}",
        c1.coverage.is_complete(),
        c2.coverage.is_complete(),
        c3.coverage.is_complete()
    );
    println!("chunk1 (incl. datapack load): {d1:?}");
    println!("chunk2 (cached graph):       {d2:?}");
    println!("chunk3 (cached graph):       {d3:?}");
}
