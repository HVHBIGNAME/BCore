//! The same request JSON is consumed by the native NOISE benchmark.
#[cfg(feature = "diagnostics")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use bcore_core::ChunkPos;
    use bcore_worldgen::generation::benchmark;
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        seed: String,
        workers: usize,
        warmup_batches: usize,
        measured_batches: usize,
        chunks: Vec<[i32; 2]>,
    }
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: noise_bench request.json")?;
    let request: Request = serde_json::from_slice(&std::fs::read(path)?)?;
    if !(1..=64).contains(&request.workers) {
        return Err("workers must be in 1..=64".into());
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(request.workers)
        .build_global()?;
    let chunks: Vec<_> = request
        .chunks
        .iter()
        .map(|&[x, z]| ChunkPos::new(x, z))
        .collect();
    let result = benchmark::noise_fill(
        request.seed.parse()?,
        &chunks,
        request.warmup_batches,
        request.measured_batches,
    )?;
    println!("NOISE_BENCHMARK={}", serde_json::to_string(&result)?);
    Ok(())
}

#[cfg(not(feature = "diagnostics"))]
fn main() {
    eprintln!("noise_bench requires --features diagnostics");
    std::process::exit(2);
}
