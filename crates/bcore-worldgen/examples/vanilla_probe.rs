//! Quick probe: print the vanilla-density surface heights across a chunk.
use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;

fn main() {
    let gen = WorldGenerator::new(846692123413862008);
    let pos = ChunkPos::new(0, 0);
    let chunk = gen.generate_chunk_vanilla(pos);

    println!("chunk ({}, {}) heights (x,z -> surface):", pos.x, pos.z);
    for z in (0..16).rev() {
        let mut row = String::new();
        for x in 0..16 {
            let h = chunk.height_at(x, z);
            // crude: print as two-digit
            row.push_str(&format!("{:>4}", h));
        }
        println!("{row}");
    }
}
