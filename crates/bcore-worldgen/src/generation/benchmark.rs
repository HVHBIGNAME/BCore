//! Reproducible NOISE material-fill measurements, without a simulated server load.
//!
//! Both engines receive fresh chunks, empty structure references and no blending.
//! Allocation, registry loading, hashing and JSON output are outside the timer.
//! This measures the production material-fill kernel, not FEATURES/FULL or TPS.
use std::time::Instant;

use rayon::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{ChunkPos, GeneratedChunk, VanillaGraph, WorldGenerator, MIN_Y, WORLD_HEIGHT};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoiseFingerprint {
    pub chunk: [i32; 2],
    pub blocks_sha256: String,
    pub marks_sha256: String,
    pub marks: usize,
}

#[derive(Debug, Serialize)]
pub struct NoiseMeasurement {
    pub engine: &'static str,
    pub scope: &'static str,
    pub seed: String,
    pub workers: usize,
    pub warmup_batches: usize,
    pub chunks_per_batch: usize,
    pub seconds: Vec<f64>,
    pub fingerprints: Vec<NoiseFingerprint>,
}

fn fingerprint(chunk: &GeneratedChunk) -> NoiseFingerprint {
    let mut blocks = Sha256::new();
    for state in chunk.states() {
        blocks.update(state.to_le_bytes());
    }
    let mut marks = Sha256::new();
    // Native ProtoChunk stores separate lists for ascending sections, preserving
    // insertion order and duplicates inside each list.
    for section in 0..WORLD_HEIGHT / 16 {
        for &(x, y, z) in chunk.postprocessing_positions() {
            if (y - MIN_Y) / 16 == section {
                for value in [x as i32, y, z as i32] {
                    marks.update(value.to_le_bytes());
                }
            }
        }
    }
    NoiseFingerprint {
        chunk: [chunk.pos.x, chunk.pos.z],
        blocks_sha256: format!("{:x}", blocks.finalize()),
        marks_sha256: format!("{:x}", marks.finalize()),
        marks: chunk.postprocessing_positions().len(),
    }
}

pub fn noise_fill(
    seed: i64,
    positions: &[ChunkPos],
    warmup_batches: usize,
    measured_batches: usize,
) -> Result<NoiseMeasurement, String> {
    if positions.is_empty() || measured_batches == 0 {
        return Err("noise benchmark needs chunks and measured batches".into());
    }
    if positions.iter().any(|p| {
        [p.x, p.z].into_iter().any(|v| {
            i64::from(v) * 16 < i64::from(i32::MIN) + 32
                || i64::from(v) * 16 > i64::from(i32::MAX) - 32
        })
    }) {
        return Err("benchmark chunk outside supported coordinate range".into());
    }
    let graph = VanillaGraph::load()
        .ok_or("missing vanilla noise graph")?
        .fork();
    let generator = WorldGenerator::new(seed);
    let mut reference = None;
    let mut seconds = Vec::with_capacity(measured_batches);
    for batch in 0..warmup_batches + measured_batches {
        let mut chunks: Vec<_> = positions.iter().copied().map(GeneratedChunk::new).collect();
        let start = Instant::now();
        chunks
            .par_iter_mut()
            .for_each(|chunk| generator.generate_noise(chunk, &graph));
        let elapsed = start.elapsed().as_secs_f64();
        let fingerprints: Vec<_> = chunks.iter().map(fingerprint).collect();
        if let Some(expected) = &reference {
            if expected != &fingerprints {
                return Err(format!("NOISE result changed in batch {batch}"));
            }
        } else {
            reference = Some(fingerprints);
        }
        if batch >= warmup_batches {
            seconds.push(elapsed);
        }
    }
    Ok(NoiseMeasurement {
        engine: "BCore",
        scope: "noise-fill-kernel",
        seed: seed.to_string(),
        workers: rayon::current_num_threads(),
        warmup_batches,
        chunks_per_batch: positions.len(),
        seconds,
        fingerprints: reference.expect("at least one measured batch"),
    })
}
