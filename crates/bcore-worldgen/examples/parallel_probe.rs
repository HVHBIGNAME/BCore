use bcore_core::ChunkPos;
use bcore_worldgen::WorldGenerator;
use std::time::Instant;

fn main() {
    let g = WorldGenerator::new(0x0BC0_0E00_1234_5678u64 as i64);
    // Warm the datapack load once.
    let _ = g.generate_chunk_vanilla(ChunkPos::new(0, 0));

    let coords: Vec<(i32, i32)> = (0..4).flat_map(|x| (0..4).map(move |z| (x, z))).collect();

    let t0 = Instant::now();
    for &(x, z) in &coords {
        let _ = g.generate_chunk_vanilla(ChunkPos::new(x, z));
    }
    let seq = t0.elapsed();

    let t1 = Instant::now();
    std::thread::scope(|s| {
        let handles: Vec<_> = coords
            .iter()
            .map(|&(x, z)| s.spawn(move || g.generate_chunk_vanilla(ChunkPos::new(x, z))))
            .collect();
        for h in handles {
            let _ = h.join().unwrap();
        }
    });
    let par = t1.elapsed();

    println!("sequential 16 chunks: {seq:?}  ({:.2} ms/chunk)", seq.as_millis() as f64 / 16.0);
    println!("parallel   16 chunks: {par:?}  ({:.2} ms/chunk)", par.as_millis() as f64 / 16.0);
    println!("speedup: {:.1}x", seq.as_secs_f64() / par.as_secs_f64());
}
