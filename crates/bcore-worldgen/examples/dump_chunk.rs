//! Dump generated chunks as newline-delimited JSON for parity scripts.
use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 || args.len() % 2 != 0 {
        eprintln!("usage: dump_chunk <seed> <chunk_x> <chunk_z> [<chunk_x> <chunk_z> ...]");
        std::process::exit(2);
    }
    let seed: i64 = args[1].parse().expect("seed must be an i64");
    for pair in args[2..].chunks_exact(2) {
        let x: i32 = pair[0].parse().expect("chunk x must be an i32");
        let z: i32 = pair[1].parse().expect("chunk z must be an i32");
        dump(seed, x, z);
    }
}

fn dump(seed: i64, x: i32, z: i32) {
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
    print!("],\"heights\":[");
    for z in 0..16 {
        for x in 0..16 {
            if x != 0 || z != 0 {
                print!(",");
            }
            print!("{}", chunk.height_at(x, z));
        }
    }
    print!("],\"biomes\":[");
    for y in (-64..320).step_by(4) {
        for z in (0..16).step_by(4) {
            for x in (0..16).step_by(4) {
                if x != 0 || z != 0 || y != -64 {
                    print!(",");
                }
                print!(
                    "\"minecraft:{}\"",
                    bcore_worldgen::biome::name(chunk.noise_biome_at(x, y, z))
                );
            }
        }
    }
    let entities: Vec<_> = chunk
        .block_entities()
        .iter()
        .map(|(&(lx, y, lz), data)| {
            let pos = (x * 16 + lx as i32, y, z * 16 + lz as i32);
            serde_json::json!({"pos":pos,"type":data.type_id(),"nbt":data.full_data(pos)})
        })
        .collect();
    let generated: Vec<_> = chunk
        .entities()
        .iter()
        .map(|e| serde_json::json!({"pos": e.position(), "type": e.type_id(), "nbt": e.data()}))
        .collect();
    println!(
        "],\"block_entities\":{},\"entities\":{},\"structures\":{}}}",
        serde_json::to_string(&entities).expect("generated block entities"),
        serde_json::to_string(&generated).expect("generated entities"),
        chunk.structures().native_data(chunk.pos)
    );
}
