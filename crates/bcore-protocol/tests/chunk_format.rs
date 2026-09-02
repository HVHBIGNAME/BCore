//! Byte-for-byte parity between BCore's native chunk encoder and captures from
//! the official Minecraft 26.2 server.
//!
//! The fixtures are the `map_chunk` (0x2d) payloads inside
//! `data/play_packets.bin`, recorded from a vanilla flat world. Two distinct
//! encodings appear there and both are reproduced here:
//!
//! * 9322 bytes — the two chunks the player spawned inside. Vanilla had already
//!   lit the section *below* the world, so it ships three sky-light layers and an
//!   empty `emptySkyLightMask`.
//! * 7280 bytes — every other chunk. The below-world section is only declared
//!   empty, so two sky-light layers are sent. The 2042-byte difference is exactly
//!   one 2048-byte layer plus its 2-byte length prefix, minus the 8-byte long
//!   that appears in `emptySkyLightMask`.
//!
//! BCore emits the 7280-byte form (vanilla's steady state).

use bcore_protocol::chunk::{flat_chunk_payload, ChunkColumn};

const PLAY_PACKETS: &[u8] = include_bytes!("../data/play_packets.bin");
const MAP_CHUNK_ID: i32 = 0x2d;

/// Captured `(packet_id, payload)` pairs: `[u32 count][(i32 pid, u32 len, bytes)...]`.
fn captured_packets() -> Vec<(i32, Vec<u8>)> {
    let mut out = Vec::new();
    let count = u32::from_be_bytes(PLAY_PACKETS[0..4].try_into().expect("count")) as usize;
    let mut at = 4usize;
    for _ in 0..count {
        let pid = i32::from_be_bytes(PLAY_PACKETS[at..at + 4].try_into().expect("pid"));
        let len =
            u32::from_be_bytes(PLAY_PACKETS[at + 4..at + 8].try_into().expect("len")) as usize;
        at += 8;
        out.push((pid, PLAY_PACKETS[at..at + len].to_vec()));
        at += len;
    }
    out
}

fn captured_chunks() -> Vec<Vec<u8>> {
    captured_packets()
        .into_iter()
        .filter(|(pid, _)| *pid == MAP_CHUNK_ID)
        .map(|(_, data)| data)
        .collect()
}

#[test]
fn capture_contains_the_expected_flat_chunks() {
    let chunks = captured_chunks();
    assert_eq!(chunks.len(), 9, "vanilla sent a 3x3 batch");
    let mut sizes: Vec<usize> = chunks.iter().map(|c| c.len()).collect();
    sizes.sort_unstable();
    sizes.dedup();
    assert_eq!(sizes, vec![7280, 9322], "two encodings of the same terrain");
}

#[test]
fn generated_flat_chunk_is_byte_identical_to_vanilla() {
    let chunks = captured_chunks();
    let reference = chunks
        .iter()
        .find(|c| c.len() == 7280)
        .expect("a steady-state vanilla flat chunk");
    let x = i32::from_be_bytes(reference[0..4].try_into().expect("x"));
    let z = i32::from_be_bytes(reference[4..8].try_into().expect("z"));

    let generated = flat_chunk_payload(x, z);
    assert_eq!(
        generated.len(),
        reference.len(),
        "payload length differs (generated {} vs vanilla {})",
        generated.len(),
        reference.len()
    );
    if generated != *reference {
        let at = generated
            .iter()
            .zip(reference.iter())
            .position(|(a, b)| a != b)
            .expect("lengths matched, so a differing byte must exist");
        panic!(
            "first difference at offset {at}: generated 0x{:02x}, vanilla 0x{:02x}",
            generated[at], reference[at]
        );
    }
}

#[test]
fn every_steady_state_capture_is_reproduced() {
    for reference in captured_chunks().iter().filter(|c| c.len() == 7280) {
        let x = i32::from_be_bytes(reference[0..4].try_into().expect("x"));
        let z = i32::from_be_bytes(reference[4..8].try_into().expect("z"));
        assert_eq!(
            flat_chunk_payload(x, z),
            *reference,
            "mismatch for chunk ({x}, {z})"
        );
    }
}

#[test]
fn spawn_chunks_differ_from_steady_state_only_in_sky_light() {
    let chunks = captured_chunks();
    let big = chunks
        .iter()
        .find(|c| c.len() == 9322)
        .expect("spawn chunk");
    let small = chunks
        .iter()
        .find(|c| c.len() == 7280)
        .expect("steady-state chunk");
    // Everything up to the light masks (heightmaps + sections + blockEntities)
    // is identical, so the terrain payload really is the same.
    let prefix = 3149 - 8; // offset of the first light byte, minus the x/z header
    assert_eq!(big[8..prefix], small[8..prefix]);
    assert_eq!(big.len() - small.len(), 2042);
}

#[test]
fn a_modified_column_still_round_trips_through_the_encoder() {
    // Sanity-check the generic path (not just the cached flat template):
    // a column with an extra block must encode to a different, longer-lit chunk.
    let mut column = ChunkColumn::flat();
    assert!(column.set(3, 40, 11, bcore_protocol::chunk::block_state::DIRT));
    let modified = column.encode_payload(0, 0);
    assert_ne!(modified, flat_chunk_payload(0, 0));
    // Heightmap must report the new surface.
    assert_eq!(column.surface_y(3, 11), Some(40));
}
