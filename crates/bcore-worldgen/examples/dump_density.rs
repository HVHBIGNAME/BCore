use bcore_worldgen::{density, WorldGenerator};
use std::{env, fs};
fn main() {
    let a: Vec<String> = env::args().collect();
    if a.len() != 5 {
        eprintln!("usage: dump_density <seed> <x> <y> <z>");
        std::process::exit(2);
    }
    let seed: i64 = a[1].parse().unwrap();
    let x: f64 = a[2].parse().unwrap();
    let y: f64 = a[3].parse().unwrap();
    let z: f64 = a[4].parse().unwrap();
    let ctx = density::EvalContext {
        seed,
        ..Default::default()
    };
    let e = |s: &str| -> f64 { density::parse_json(s).unwrap().evaluate(x, y, z, &ctx) };
    let path =
        |s: &str| format!("target/datapack/data/minecraft/worldgen/density_function/{s}.json");
    let p = |s: &str| e(&fs::read_to_string(path(s)).unwrap());
    let noise = |name: &str, xz: f64, ys: f64| {
        e(&format!(
            r#"{{"type":"minecraft:noise","noise":"minecraft:{name}","xz_scale":{xz},"y_scale":{ys}}}"#
        ))
    };
    println!("layer={} cheese_noise={} cheese_clamp={} slope_term={} cheese={} entrances={} spaghetti={} pillars={} noodle={}",
        noise("cave_layer",1.,8.), noise("cave_cheese",1.,2./3.), (0.27+noise("cave_cheese",1.,2./3.)).clamp(-1.,1.), 0.0, p("overworld/sloped_cheese"), p("overworld/caves/entrances"), p("overworld/caves/spaghetti_2d"), p("overworld/caves/pillars"), p("overworld/caves/noodle"));
    let elev = noise("spaghetti_2d_elevation", 1., 0.);
    let thick = noise("spaghetti_2d_thickness", 2., 1.);
    let mod2d = noise("spaghetti_2d_modulator", 2., 1.);
    let yg = (y - -64.0) / (320.0 - -64.0) * (-40.0 - 8.0) + 8.0;
    println!(
        "elevation={} thickness={} mod2d={} sloped={} layer_cube={}",
        elev,
        thick,
        mod2d,
        (elev * 8.0 + yg).abs(),
        ((elev * 8.0 + yg).abs() + (-0.95 - 0.35 * thick)).powi(3)
    );
    println!(
        "pillar={} pillar_rareness={} pillar_thickness={}",
        noise("pillar", 25., 0.3),
        noise("pillar_rareness", 1., 1.),
        noise("pillar_thickness", 1., 1.)
    );
    if let Some((f, n, c, en)) = WorldGenerator::cave_density_probe(seed, x, y, z) {
        println!(
            "x={x} y={y} z={z} final={f:.17e} noodle={n:?} cave_cheese={c:?} entrances={en:?}"
        );
    }
}
