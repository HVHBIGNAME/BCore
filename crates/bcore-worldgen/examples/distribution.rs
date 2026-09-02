//! Diagnostic: print the biome/height distribution of a seed.
//!
//! Not a test of correctness — a tuning tool. Run with:
//!
//! ```text
//! cargo run -p bcore-worldgen --example distribution
//! ```

use bcore_worldgen::{Biome, WorldGenerator, MAX_SURFACE, MIN_SURFACE, SEA_LEVEL};

fn main() {
    for seed in [1234i64, 0, -99, 777] {
        let gen = WorldGenerator::new(seed);
        let mut counts: Vec<(Biome, usize)> = Vec::new();
        let mut heights: Vec<i32> = Vec::new();
        let mut continents: Vec<f64> = Vec::new();

        // Sample every 4 blocks over a 2048x2048 region.
        let step = 4i32;
        let half = 1024i32;
        let mut total = 0usize;
        let mut x = -half;
        while x < half {
            let mut z = -half;
            while z < half {
                let h = gen.height_at(x, z);
                let b = gen.biome_at(x, z, h);
                heights.push(h);
                continents.push(gen.continent(x, z));
                match counts.iter_mut().find(|(kind, _)| *kind == b) {
                    Some(entry) => entry.1 += 1,
                    None => counts.push((b, 1)),
                }
                total += 1;
                z += step;
            }
            x += step;
        }

        counts.sort_by_key(|&(kind, count)| (usize::MAX - count, kind));
        heights.sort_unstable();
        continents.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));

        let pct = |v: usize| v as f64 * 100.0 / total as f64;
        let quantile = |data: &[i32], q: f64| data[((data.len() - 1) as f64 * q) as usize];
        let cq = |data: &[f64], q: f64| data[((data.len() - 1) as f64 * q) as usize];

        println!("=== seed {seed} ({total} samples over 2048x2048) ===");
        for (kind, count) in &counts {
            println!(
                "  {:<12} {:>6.2}%  ({count})",
                format!("{kind:?}"),
                pct(*count)
            );
        }
        println!(
            "  height   min={} p05={} p50={} p95={} max={}  (band {MIN_SURFACE}..{MAX_SURFACE}, sea {SEA_LEVEL})",
            heights[0],
            quantile(&heights, 0.05),
            quantile(&heights, 0.50),
            quantile(&heights, 0.95),
            heights[heights.len() - 1],
        );
        println!(
            "  continent min={:.3} p05={:.3} p50={:.3} p95={:.3} max={:.3}",
            continents[0],
            cq(&continents, 0.05),
            cq(&continents, 0.50),
            cq(&continents, 0.95),
            continents[continents.len() - 1],
        );
        let below_sea = heights.iter().filter(|&&h| h < SEA_LEVEL).count();
        println!("  below sea level: {:.2}%", pct(below_sea));
        println!();
    }
}
