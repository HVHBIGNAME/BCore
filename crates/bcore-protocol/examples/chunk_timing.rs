//! Timing for terrain generation, encoding and persistence.
//!
//! Chunk streaming has a hard latency budget: a join or a long teleport asks for
//! `(2*view+1)^2 = 441` chunks at once, so per-chunk cost is multiplied by 441.
//! Run with:
//!
//! ```text
//! cargo run -q --release -p bcore-protocol --example chunk_timing
//! ```

use std::time::Instant;

use bcore_core::ChunkPos;
use bcore_protocol::chunk::ChunkColumn;
use bcore_protocol::chunk_store::{encode_chunk, ChunkStore};
use bcore_protocol::world_state::World;
use bcore_worldgen::WorldGenerator;

const SEED: i64 = 0x0BC0_0E00_1234_5678u64 as i64;
const VIEW_CHUNKS: usize = 441; // (2 * 10 + 1)^2

fn main() {
    let generator = WorldGenerator::new(SEED);

    // --- generation ---
    let start = Instant::now();
    let mut generated = Vec::with_capacity(64);
    for i in 0..64 {
        generated.push(generator.generate_chunk(ChunkPos::new(i % 8, i / 8)));
    }
    let gen_total = start.elapsed();
    let gen_each = gen_total / 64;
    println!("generate      : {gen_each:>10.2?} per chunk  ({gen_total:?} for 64)");

    // --- to column + encode payload ---
    let columns: Vec<ChunkColumn> = generated.iter().map(ChunkColumn::from_generated).collect();
    let start = Instant::now();
    let mut bytes = 0usize;
    for (i, column) in columns.iter().enumerate() {
        bytes += column.encode_payload(i as i32, 0).len();
    }
    let enc_total = start.elapsed();
    println!(
        "encode payload: {:>10.2?} per chunk  (avg {} bytes)",
        enc_total / 64,
        bytes / 64
    );

    // --- disk format ---
    let start = Instant::now();
    let mut disk_bytes = 0usize;
    for (i, column) in columns.iter().enumerate() {
        disk_bytes += encode_chunk(i as i32, 0, column).len();
    }
    println!(
        "encode on-disk: {:>10.2?} per chunk  (avg {} bytes)",
        start.elapsed() / 64,
        disk_bytes / 64
    );

    // --- save + load round trip ---
    let mut dir = std::env::temp_dir();
    dir.push(format!("bcore-timing-{}", std::process::id()));
    let store = ChunkStore::at(&dir);
    let start = Instant::now();
    for (i, column) in columns.iter().enumerate() {
        store.save(i as i32, 0, column).expect("save");
    }
    println!("save to disk  : {:>10.2?} per chunk", start.elapsed() / 64);

    let start = Instant::now();
    for i in 0..64 {
        store.load(i as i32, 0).expect("load").expect("present");
    }
    println!("load from disk: {:>10.2?} per chunk", start.elapsed() / 64);
    std::fs::remove_dir_all(&dir).ok();

    // --- what a full view costs ---
    println!();
    let world = World::in_memory(SEED);
    let start = Instant::now();
    for i in 0..VIEW_CHUNKS {
        let x = (i % 21) as i32 - 10;
        let z = (i / 21) as i32 - 10;
        world.chunk_payload(x, z);
    }
    let view_cold = start.elapsed();
    println!("FULL VIEW ({VIEW_CHUNKS} chunks), cold: {view_cold:.2?}");

    let start = Instant::now();
    for i in 0..VIEW_CHUNKS {
        let x = (i % 21) as i32 - 10;
        let z = (i / 21) as i32 - 10;
        world.chunk_payload(x, z);
    }
    println!(
        "FULL VIEW ({VIEW_CHUNKS} chunks), cached: {:.2?}",
        start.elapsed()
    );
}
