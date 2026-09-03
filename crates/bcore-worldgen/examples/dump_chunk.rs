//! Dump one generated chunk as JSON for scripts/parity_suite.py.
use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: dump_chunk <seed> <chunk_x> <chunk_z>");
        std::process::exit(2);
    }
    let seed: i64 = args[1].parse().expect("seed must be an i64");
    let x: i32 = args[2].parse().expect("chunk x must be an i32");
    let z: i32 = args[3].parse().expect("chunk z must be an i32");
    let chunk = WorldGenerator::new(seed).generate_chunk_vanilla(ChunkPos::new(x, z));
    // JSON is deliberately emitted without a dependency: this is a diagnostic
    // binary, and the array is consumed by the Python parity runner.
    print!("{{\"seed\":{seed},\"x\":{x},\"z\":{z},\"states\":[");
    for (i, state) in chunk.states().iter().enumerate() {
        if i != 0 {
            print!(",");
        }
        print!("{state}");
    }
    println!("]}}\n");
}
