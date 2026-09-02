//! Parity against chunks the live vanilla 26.2 server generated **during movement**.
//!
//! `scripts/capture_walk.py` joins a running vanilla flat-world server, walks the
//! player far outside the spawn area and records every distinct `map_chunk` it
//! receives. This test replays that fixture through BCore's native encoder, so
//! parity is proven for chunks vanilla streamed while the player moved — not just
//! for the join batch checked by `chunk_format.rs`.
//!
//! Both vanilla encodings appear in the fixture and both are accounted for:
//! chunks whose below-world section vanilla had already lit carry a third
//! sky-light layer (9322 bytes); the steady-state form (7280 bytes) is what BCore
//! emits and is compared byte for byte.

use bcore_protocol::chunk::flat_chunk_payload;

const WALK_CHUNKS: &[u8] = include_bytes!("../data/vanilla_walk_chunks.bin");
const MAP_CHUNK_ID: i32 = 0x2d;
/// Offset of the first light byte in a flat chunk payload (see `chunk_format.rs`).
const LIGHT_OFFSET: usize = 3149;

fn fixture_chunks() -> Vec<Vec<u8>> {
    let count = u32::from_be_bytes(WALK_CHUNKS[0..4].try_into().expect("count")) as usize;
    let mut out = Vec::with_capacity(count);
    let mut at = 4usize;
    for _ in 0..count {
        let pid = i32::from_be_bytes(WALK_CHUNKS[at..at + 4].try_into().expect("pid"));
        assert_eq!(pid, MAP_CHUNK_ID, "fixture holds only map_chunk packets");
        let len = u32::from_be_bytes(WALK_CHUNKS[at + 4..at + 8].try_into().expect("len")) as usize;
        at += 8;
        out.push(WALK_CHUNKS[at..at + len].to_vec());
        at += len;
    }
    out
}

fn coords(payload: &[u8]) -> (i32, i32) {
    (
        i32::from_be_bytes(payload[0..4].try_into().expect("x")),
        i32::from_be_bytes(payload[4..8].try_into().expect("z")),
    )
}

#[test]
fn fixture_covers_chunks_far_outside_the_spawn_area() {
    let chunks = fixture_chunks();
    assert!(
        chunks.len() >= 40,
        "expected a long walk, got {} chunks",
        chunks.len()
    );
    let far = chunks
        .iter()
        .filter(|c| {
            let (x, z) = coords(c);
            x.abs() > 2 || z.abs() > 2
        })
        .count();
    assert!(
        far >= 30,
        "fixture should reach well past the join batch, only {far} distant chunks"
    );
    // No chunk was recorded twice.
    let mut seen: Vec<(i32, i32)> = chunks.iter().map(|c| coords(c)).collect();
    seen.sort_unstable();
    let total = seen.len();
    seen.dedup();
    assert_eq!(
        seen.len(),
        total,
        "fixture contains duplicate chunk positions"
    );
}

#[test]
fn every_vanilla_walk_chunk_matches_the_native_encoder() {
    let chunks = fixture_chunks();
    let mut compared = 0usize;
    let mut light_variant = 0usize;

    for reference in &chunks {
        let (x, z) = coords(reference);
        let generated = flat_chunk_payload(x, z);

        if reference.len() == generated.len() {
            if *reference != generated {
                let at = reference
                    .iter()
                    .zip(generated.iter())
                    .position(|(a, b)| a != b)
                    .expect("same length, so a differing byte exists");
                panic!(
                    "chunk ({x}, {z}) differs at offset {at}: vanilla 0x{:02x}, BCore 0x{:02x}",
                    reference[at], generated[at]
                );
            }
            compared += 1;
        } else {
            // The other vanilla encoding: identical terrain, one extra sky-light
            // layer for the section below the world. Everything before the light
            // section must still match exactly.
            assert_eq!(
                reference[..LIGHT_OFFSET],
                generated[..LIGHT_OFFSET],
                "chunk ({x}, {z}): terrain differs from the native encoder"
            );
            assert_eq!(
                reference.len() - generated.len(),
                2042,
                "chunk ({x}, {z}): unexpected light-section size delta"
            );
            light_variant += 1;
        }
    }

    assert!(
        compared > 0,
        "no chunk matched byte-for-byte ({light_variant} light variants)"
    );
    println!(
        "{compared} chunks byte-identical, {light_variant} matched terrain with vanilla's \
         extra below-world light layer"
    );
}

#[test]
fn coordinates_are_the_only_difference_between_vanilla_flat_chunks() {
    // Every same-sized vanilla chunk shares one payload beyond the x/z header,
    // which is why caching a single template is correct.
    let chunks = fixture_chunks();
    for size in [7280usize, 9322] {
        let mut group = chunks.iter().filter(|c| c.len() == size);
        let Some(first) = group.next() else { continue };
        for other in group {
            assert_eq!(
                first[8..],
                other[8..],
                "two {size}-byte vanilla chunks differ beyond their coordinates"
            );
        }
    }
}
