use bcore_core::ChunkPos;
use bcore_worldgen::GenerationWorld;
use std::time::Instant;

fn main() {
    let seed = 0x0BC0_0E00_1234_5678u64 as i64;
    let sequential = GenerationWorld::new(seed);
    let parallel = GenerationWorld::new(seed);
    for world in [&sequential, &parallel] {
        let result = world
            .generate_chunk(ChunkPos::new(0, 0))
            .expect("warmup generation failed");
        eprintln!(
            "warmup coverage complete: {}",
            result.coverage.is_complete()
        );
    }

    let coords: Vec<(i32, i32)> = (0..4).flat_map(|x| (0..4).map(move |z| (x, z))).collect();

    let t0 = Instant::now();
    let mut sequential_complete = true;
    for &(x, z) in &coords {
        let result = sequential
            .generate_chunk(ChunkPos::new(x, z))
            .expect("chunk generation failed");
        sequential_complete &= result.coverage.is_complete();
    }
    let seq = t0.elapsed();

    let t1 = Instant::now();
    let parallel_complete = std::thread::scope(|s| {
        let world = &parallel;
        let handles: Vec<_> = coords
            .iter()
            .map(|&(x, z)| {
                s.spawn(move || {
                    world
                        .generate_chunk(ChunkPos::new(x, z))
                        .expect("chunk generation failed")
                        .coverage
                        .is_complete()
                })
            })
            .collect();
        let mut complete = true;
        for h in handles {
            complete &= h.join().expect("generation worker failed");
        }
        complete
    });
    let par = t1.elapsed();
    eprintln!("coverage complete: sequential={sequential_complete}, parallel={parallel_complete}");

    println!(
        "sequential 16 chunks: {seq:?}  ({:.2} ms/chunk)",
        seq.as_millis() as f64 / 16.0
    );
    println!(
        "parallel   16 chunks: {par:?}  ({:.2} ms/chunk)",
        par.as_millis() as f64 / 16.0
    );
    println!("speedup: {:.1}x", seq.as_secs_f64() / par.as_secs_f64());
}
