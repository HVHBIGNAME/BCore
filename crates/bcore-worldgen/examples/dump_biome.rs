use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;
use std::env;
fn main() {
    let args: Vec<String> = env::args().collect();
    let seed: i64 = args[1].parse().unwrap();
    let cx: i32 = args[2].parse().unwrap();
    let cz: i32 = args[3].parse().unwrap();
    let chunk = WorldGenerator::new(seed).generate_chunk_vanilla(ChunkPos::new(cx, cz));
    println!("chunk ({cx},{cz}) biome map (16x16, z rows):");
    for z in 0..16 {
        let mut row = String::new();
        for x in 0..16 {
            row.push_str(&format!("{:?} ", chunk.biome_at(x, z)));
        }
        println!("z={z:2}: {row}");
    }
}
