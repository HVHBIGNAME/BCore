//! Probe vanilla final_density and generated column heights.
use bcore_core::ChunkPos;
use bcore_worldgen::{
    density::{self, EvalContext},
    WorldGenerator,
};

fn main() {
    let ctx = EvalContext {
        seed: 1234,
        ..Default::default()
    };
    let settings = std::fs::read_to_string(
        "target/datapack/data/minecraft/worldgen/noise_settings/overworld.json",
    )
    .unwrap();
    let fd = density::parse_json(
        settings
            .split("\"final_density\":")
            .nth(1)
            .unwrap()
            .split("\"vein_toggle\"")
            .next()
            .unwrap(),
    )
    .unwrap();
    let (x, z) = (-533.0, 65.0);
    println!("y | final_density");
    for y in [
        -64.0, -32.0, 0.0, 32.0, 63.0, 80.0, 100.0, 128.0, 160.0, 200.0, 256.0, 300.0,
    ] {
        println!("{y:5} | {:.3}", density::evaluate(&fd, x, y, z, &ctx));
    }
    let gen = WorldGenerator::new(1234);
    for (cx, cz) in [(-35, 3), (-35, 4), (-34, 3), (-33, 4), (-33, 5)] {
        let chunk = gen.generate_chunk_vanilla(ChunkPos::new(cx, cz));
        let mut heights = Vec::new();
        for z in 0..16 {
            for x in 0..16 {
                heights.push(chunk.height_at(x, z));
            }
        }
        println!(
            "chunk ({cx},{cz}) BCore top min={} max={} samples={:?}",
            heights.iter().min().unwrap(),
            heights.iter().max().unwrap(),
            &heights[..4]
        );
    }
}
