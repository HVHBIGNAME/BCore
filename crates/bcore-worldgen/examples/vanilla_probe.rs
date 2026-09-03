//! Histogram of terrain heights in one chunk.
use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;

fn main() {
    let gen = WorldGenerator::new(1234);
    let c = gen.generate_chunk_vanilla(ChunkPos::new(0, 0));
    let mut buckets = [0i32; 8];
    let labels = [
        "<-64", "-64..0", "0..32", "32..64", "64..96", "96..128", "128..192", ">192",
    ];
    for x in 0..16 {
        for z in 0..16 {
            let h = c.height_at(x, z);
            let i = if h < -64 {
                0
            } else if h < 0 {
                1
            } else if h < 32 {
                2
            } else if h < 64 {
                3
            } else if h < 96 {
                4
            } else if h < 128 {
                5
            } else if h < 192 {
                6
            } else {
                7
            };
            buckets[i] += 1;
        }
    }
    for i in 0..8 {
        println!("{:>8}: {}", labels[i], "#".repeat(buckets[i] as usize));
    }
    println!("sample heights (x=8,z):");
    for z in 0..16 {
        print!("{:4} ", c.height_at(8, z));
    }
    println!();
}
