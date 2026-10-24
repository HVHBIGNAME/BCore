//! Private parser-cost diagnostic; no generation benchmark contract changes.
use bcore_worldgen::density;
use std::time::Instant;

fn main() {
    let mut rows = Vec::new();
    for field in ["final_density", "preliminary_surface_level", "depth"] {
        std::hint::black_box(density::parse_router("overworld", field).unwrap());
        let mut seconds = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            for _ in 0..20 {
                std::hint::black_box(density::parse_router("overworld", field).unwrap());
            }
            seconds.push(start.elapsed().as_secs_f64() / 20.0);
        }
        rows.push(serde_json::json!({"field": field, "seconds_per_parse": seconds}));
    }
    println!(
        "DENSITY_PARSE_PROBE={}",
        serde_json::to_string(&rows).unwrap()
    );
}
