use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;
use std::time::Instant;

fn main() {
    let g = WorldGenerator::new(0x0BC0_0E00_1234_5678u64 as i64);
    let t0 = Instant::now();
    let _c1 = g.generate_chunk_vanilla(ChunkPos::new(0, -1));
    let d1 = t0.elapsed();
    let t1 = Instant::now();
    let _c2 = g.generate_chunk_vanilla(ChunkPos::new(1, -1));
    let d2 = t1.elapsed();
    let t2 = Instant::now();
    let _c3 = g.generate_chunk_vanilla(ChunkPos::new(2, -1));
    let d3 = t2.elapsed();
    println!("chunk1 (incl. datapack load): {d1:?}");
    println!("chunk2 (cached graph):       {d2:?}");
    println!("chunk3 (cached graph):       {d3:?}");
}
