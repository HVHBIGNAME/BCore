use bcore_worldgen::WorldGenerator;
use std::env;
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
    match WorldGenerator::cave_density_probe(seed, x, y, z) {
        Some((f, n, c, e)) => println!(
            "x={x} y={y} z={z} final={f:.17e} noodle={n:?} cave_cheese={c:?} entrances={e:?}"
        ),
        None => {
            eprintln!("datapack graph unavailable");
            std::process::exit(1);
        }
    }
}
