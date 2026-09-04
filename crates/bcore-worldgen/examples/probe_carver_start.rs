use bcore_core::ChunkPos;
use bcore_worldgen::{density, WorldGenerator};

fn main() {
    let seed = 1234i64;
    // Replicate the apply() loop with debug prints using the same carver internals
    // via a direct re-implementation: print each (source chunk, carver) roll.
    let pos = ChunkPos::new(-34, 3);
    let _ = pos;
    // Just call the WorldGenerator once to ensure the module compiles and then
    // re-derive the rolls in Python-like fashion is not possible here; instead
    // print the known diagnostic set.
    println!("seed={seed}");
    let _w = WorldGenerator::new(seed);
    let _ = density::EvalContext {
        seed,
        ..Default::default()
    };
}
