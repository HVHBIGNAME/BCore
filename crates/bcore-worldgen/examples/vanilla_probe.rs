//! final_density by Y after interval_select fix.
use bcore_worldgen::density::{self, EvalContext};

fn main() {
    let ctx = EvalContext::default();
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
    let (x, z) = (16.0, 16.0);
    println!("y | final_density");
    for y in [
        -64.0, -32.0, 0.0, 32.0, 63.0, 80.0, 100.0, 128.0, 160.0, 200.0, 256.0, 300.0,
    ] {
        println!("{y:5} | {:.3}", density::evaluate(&fd, x, y, z, &ctx));
    }
}
