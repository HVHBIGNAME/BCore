use bcore_core::ChunkPos;
use bcore_worldgen::{GenerationWorld, MIN_Y};

fn main() {
    let seed = 0x0BC0_0E00_1234_5678u64 as i64;
    let world = GenerationWorld::new(seed);
    // (cx, cz) coordinates to sample: spawn area + far coordinates.
    let points = [(0, -1), (625, 312), (-938, -625)];
    for (cx, cz) in points {
        let result = world
            .generate_chunk(ChunkPos::new(cx, cz))
            .expect("chunk generation failed");
        eprintln!(
            "chunk ({cx},{cz}) coverage complete: {}",
            result.coverage.is_complete()
        );
        let chunk = result.chunk;
        let states = chunk.states();
        println!("=== chunk ({cx},{cz}) ===");
        // Print the surface height + fluid marker on a coarse grid (every 4 blocks).
        for z in (0..16).step_by(2) {
            let mut row = String::new();
            for x in (0..16).step_by(2) {
                let mut top: i32 = -1;
                let mut fluid = false;
                for y in (0..384).rev() {
                    let st = states[y * 256 + z * 16 + x];
                    if st != 0 {
                        if top < 0 {
                            top = y as i32 + MIN_Y;
                            fluid = st == bcore_worldgen::block::WATER
                                || st == bcore_worldgen::block::LAVA;
                        }
                        break;
                    }
                }
                row.push_str(&format!("{}{} ", top, if fluid { "w" } else { " " }));
            }
            println!("z={z:2} {}", row);
        }
    }
}
