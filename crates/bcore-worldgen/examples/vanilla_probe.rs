//! final_density + sloped_cheese by Y after firstOctave fix.
use bcore_worldgen::density::{self, EvalContext};

fn main() {
    let ctx = EvalContext::default();
    let load = |name: &str| {
        let p = format!(
            "target/datapack/data/minecraft/worldgen/density_function/overworld/{name}.json"
        );
        density::parse_json(&std::fs::read_to_string(p).unwrap()).unwrap()
    };
    let sc = load("sloped_cheese");
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
    println!("y | sloped_cheese | final_density");
    for y in [-64.0, -32.0, 0.0, 32.0, 63.0, 80.0, 100.0, 128.0, 160.0] {
        println!(
            "{y:5} | {:.3} | {:.3}",
            density::evaluate(&sc, x, y, z, &ctx),
            density::evaluate(&fd, x, y, z, &ctx),
        );
    }
}
