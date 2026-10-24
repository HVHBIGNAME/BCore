//! Compare BCore surface vs vanilla (chunk -34,3).
use bcore_core::ChunkPos;
use bcore_worldgen::GenerationWorld;

fn main() {
    let world = GenerationWorld::new(1234);
    let result = world
        .generate_chunk(ChunkPos::new(-34, 3))
        .expect("chunk generation failed");
    eprintln!("coverage complete: {}", result.coverage.is_complete());
    let c = result.chunk;
    let vanilla = [
        77, 76, 78, 79, 79, 79, 78, 64, 63, 64, 64, 64, 64, 64, 65, 65,
    ];
    println!("x | vanilla | bcore");
    for x in 0..16 {
        println!("{x:2} | {:7} | {}", vanilla[x], c.height_at(x, 0));
    }
}
