//! Terrain chunk encoding: structural parity with real vanilla terrain captures.
//!
//! `tests/chunk_format.rs` already proves BCore reproduces a *flat* vanilla chunk
//! byte-for-byte. Flat chunks never exercise the paths realistic terrain needs,
//! so this file checks the terrain encoder two ways:
//!
//! 1. **Self round-trip** — parse BCore's own `map_chunk` payload back and prove
//!    every block, heightmap and count survives the encode.
//! 2. **Vanilla structural parity** — decode the real overworld chunks captured in
//!    `data/vanilla_terrain_chunks.bin` (49 chunks from the official 26.2 server,
//!    recorded by `scripts/capture_terrain.py`) and assert BCore's chunks share
//!    their structure: same container discipline, same heightmap kinds in the same
//!    order, `blockCount`/`fluidCount` computed by the same rules.
//!
//! Byte-for-byte equality with vanilla terrain is *not* the goal — BCore's
//! generator is not vanilla's. What must match is the wire format.

use bcore_protocol::chunk::{
    block_state, ChunkColumn, HEIGHTMAP_MOTION_BLOCKING, HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES,
    HEIGHTMAP_WORLD_SURFACE, MAX_INDIRECT_BLOCK_BITS, MIN_Y, SECTION_BIOMES, SECTION_COUNT,
    SECTION_VOLUME, WORLD_HEIGHT,
};
use bcore_protocol::world_state::World;

const TERRAIN_CAPTURE: &[u8] = include_bytes!("../data/vanilla_terrain_chunks.bin");
const MAP_CHUNK_ID: i32 = 0x2d;
const SEED: i64 = 1234;

// ---------------------------------------------------------------- decoding ---

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn varint(&mut self) -> i32 {
        let (value, used) =
            bcore_core::varint::decode_varint(&self.data[self.at..]).expect("varint");
        self.at += used;
        value
    }

    fn i16(&mut self) -> i16 {
        let v = i16::from_be_bytes(self.data[self.at..self.at + 2].try_into().expect("i16"));
        self.at += 2;
        v
    }

    fn i32(&mut self) -> i32 {
        let v = i32::from_be_bytes(self.data[self.at..self.at + 4].try_into().expect("i32"));
        self.at += 4;
        v
    }

    fn u8(&mut self) -> u8 {
        let v = self.data[self.at];
        self.at += 1;
        v
    }

    fn longs(&mut self, n: usize) -> Vec<i64> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(i64::from_be_bytes(
                self.data[self.at..self.at + 8].try_into().expect("long"),
            ));
            self.at += 8;
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Container {
    bits: u8,
    /// Empty when the global (direct) palette is in use.
    palette: Vec<u32>,
    values: Vec<u32>,
}

impl Container {
    fn is_single(&self) -> bool {
        self.bits == 0
    }

    fn is_direct(&self, entries: usize) -> bool {
        entries == SECTION_VOLUME && self.bits > MAX_INDIRECT_BLOCK_BITS
    }
}

#[derive(Debug, Clone)]
struct Section {
    block_count: i16,
    fluid_count: i16,
    blocks: Container,
    biomes: Container,
}

#[derive(Debug, Clone)]
struct Chunk {
    x: i32,
    z: i32,
    /// Heightmap kinds in transmission order, with their unpacked values.
    heightmaps: Vec<(i32, Vec<u16>)>,
    sections: Vec<Section>,
    block_entities: i32,
    size: usize,
}

impl Chunk {
    fn heightmap(&self, kind: i32) -> &[u16] {
        &self
            .heightmaps
            .iter()
            .find(|(k, _)| *k == kind)
            .unwrap_or_else(|| panic!("heightmap kind {kind} missing"))
            .1
    }

    /// The block state at chunk-local coordinates.
    fn get(&self, x: usize, y: i32, z: usize) -> u32 {
        let level = (y - MIN_Y) as usize;
        let section = &self.sections[level / 16];
        section.blocks.values[(level % 16) * 256 + z * 16 + x]
    }
}

fn unpack(bits: u8, longs: &[i64], entries: usize) -> Vec<u32> {
    if bits == 0 {
        return vec![0; entries];
    }
    let per_long = 64 / bits as usize;
    let mask = (1u64 << bits) - 1;
    (0..entries)
        .map(|i| {
            let word = longs[i / per_long] as u64;
            ((word >> ((i % per_long) * bits as usize)) & mask) as u32
        })
        .collect()
}

fn read_container(cur: &mut Cursor<'_>, entries: usize) -> Container {
    let bits = cur.u8();
    if bits == 0 {
        let single = cur.varint() as u32;
        return Container {
            bits,
            palette: vec![single],
            values: vec![single; entries],
        };
    }
    // Blocks switch to the global palette above 8 bits; biomes above 3.
    let direct = entries == SECTION_VOLUME && bits > MAX_INDIRECT_BLOCK_BITS;
    let mut palette = Vec::new();
    if !direct {
        let len = cur.varint();
        for _ in 0..len {
            palette.push(cur.varint() as u32);
        }
    }
    let per_long = 64 / bits as usize;
    let longs = cur.longs(entries.div_ceil(per_long));
    let raw = unpack(bits, &longs, entries);
    let values = if direct {
        raw
    } else {
        raw.iter()
            .map(|&i| {
                *palette
                    .get(i as usize)
                    .unwrap_or_else(|| panic!("palette index {i} out of range"))
            })
            .collect()
    };
    Container {
        bits,
        palette,
        values,
    }
}

/// Parse a `map_chunk` payload (no packet id).
fn decode_chunk(payload: &[u8]) -> Chunk {
    let mut cur = Cursor::new(payload);
    let x = cur.i32();
    let z = cur.i32();

    let count = cur.varint();
    let mut heightmaps = Vec::new();
    for _ in 0..count {
        let kind = cur.varint();
        let n = cur.varint() as usize;
        let longs = cur.longs(n);
        let values = unpack(9, &longs, 256)
            .into_iter()
            .map(|v| v as u16)
            .collect();
        heightmaps.push((kind, values));
    }

    let data_len = cur.varint() as usize;
    let end = cur.at + data_len;
    let mut sections = Vec::new();
    while cur.at < end {
        let block_count = cur.i16();
        let fluid_count = cur.i16();
        let blocks = read_container(&mut cur, SECTION_VOLUME);
        let biomes = read_container(&mut cur, SECTION_BIOMES);
        sections.push(Section {
            block_count,
            fluid_count,
            blocks,
            biomes,
        });
    }
    assert_eq!(cur.at, end, "section data overran its declared length");

    let block_entities = cur.varint();
    Chunk {
        x,
        z,
        heightmaps,
        sections,
        block_entities,
        size: payload.len(),
    }
}

/// The captured vanilla terrain chunks.
fn vanilla_terrain() -> Vec<Chunk> {
    let count = u32::from_be_bytes(TERRAIN_CAPTURE[0..4].try_into().expect("count")) as usize;
    let mut at = 4usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let pid = i32::from_be_bytes(TERRAIN_CAPTURE[at..at + 4].try_into().expect("pid"));
        let len =
            u32::from_be_bytes(TERRAIN_CAPTURE[at + 4..at + 8].try_into().expect("len")) as usize;
        at += 8;
        assert_eq!(pid, MAP_CHUNK_ID);
        out.push(decode_chunk(&TERRAIN_CAPTURE[at..at + len]));
        at += len;
    }
    out
}

// ------------------------------------------------------------------- tests ---

#[test]
fn the_capture_really_contains_varied_terrain() {
    let chunks = vanilla_terrain();
    assert_eq!(chunks.len(), 49, "expected the recorded 49-chunk walk");

    // Terrain, not superflat: surfaces vary and sections carry multi-entry palettes.
    let mut max_palette = 0usize;
    let mut fluid_chunks = 0usize;
    for chunk in &chunks {
        for section in &chunk.sections {
            max_palette = max_palette.max(section.blocks.palette.len());
        }
        if chunk.sections.iter().any(|s| s.fluid_count > 0) {
            fluid_chunks += 1;
        }
    }
    assert!(
        max_palette > 20,
        "flat-looking capture: widest palette is only {max_palette}"
    );
    assert!(
        fluid_chunks > 10,
        "expected water in the capture, found it in {fluid_chunks} chunks"
    );
}

#[test]
fn generated_terrain_round_trips_through_our_own_encoder() {
    let world = World::in_memory(SEED);
    for &(cx, cz) in &[(0, 0), (5, -3), (-12, 8)] {
        let column = world.generate(cx, cz);
        let payload = column.encode_payload(cx, cz);
        let decoded = decode_chunk(&payload);

        assert_eq!((decoded.x, decoded.z), (cx, cz), "coordinates round trip");
        assert_eq!(decoded.sections.len(), SECTION_COUNT);
        assert_eq!(decoded.block_entities, 0);

        // Every single block must survive the palette encode.
        for y in MIN_Y..MIN_Y + WORLD_HEIGHT {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(
                        decoded.get(x, y, z),
                        column.get(x, y, z).expect("in range"),
                        "block mismatch at ({x},{y},{z}) of chunk ({cx},{cz})"
                    );
                }
            }
        }

        // And so must all three heightmaps.
        assert_eq!(
            decoded.heightmap(HEIGHTMAP_WORLD_SURFACE),
            &column.heightmap()[..]
        );
        assert_eq!(
            decoded.heightmap(HEIGHTMAP_MOTION_BLOCKING),
            &column.heightmap_motion_blocking()[..]
        );
        assert_eq!(
            decoded.heightmap(HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES),
            &column.heightmap_motion_blocking_no_leaves()[..]
        );
    }
}

#[test]
fn heightmaps_are_sent_as_the_same_three_kinds_as_vanilla() {
    // The *set* of kinds is fixed; the ORDER is not. Vanilla itself is
    // inconsistent: `scripts/compare_heightmap_order.py` shows the flat captures
    // (play_packets.bin, vanilla_walk_chunks.bin — 78 chunks) send [1, 5, 4]
    // while the terrain capture (vanilla_terrain_chunks.bin — 49 chunks) sends
    // [5, 4, 1]. The client reads (kind, values) pairs, so order is irrelevant.
    //
    // BCore emits [1, 5, 4] because that is what reproduces the flat capture
    // byte-for-byte in `tests/chunk_format.rs`.
    let vanilla_terrain_kinds: Vec<i32> = vanilla_terrain()[0]
        .heightmaps
        .iter()
        .map(|(k, _)| *k)
        .collect();
    let ours = decode_chunk(&World::in_memory(SEED).generate(0, 0).encode_payload(0, 0));
    let our_kinds: Vec<i32> = ours.heightmaps.iter().map(|(k, _)| *k).collect();

    let sorted = |mut v: Vec<i32>| {
        v.sort_unstable();
        v
    };
    assert_eq!(
        sorted(our_kinds.clone()),
        sorted(vanilla_terrain_kinds.clone()),
        "the three heightmap kinds must match vanilla's"
    );
    assert_eq!(
        sorted(our_kinds.clone()),
        vec![
            HEIGHTMAP_WORLD_SURFACE,
            HEIGHTMAP_MOTION_BLOCKING,
            HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES
        ]
    );
    // BCore's own order is stable, matching the flat-capture parity test.
    assert_eq!(
        our_kinds,
        vec![
            HEIGHTMAP_WORLD_SURFACE,
            HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES,
            HEIGHTMAP_MOTION_BLOCKING
        ]
    );
    // Each heightmap is 256 entries at 9 bits = 37 longs.
    for (_, values) in &ours.heightmaps {
        assert_eq!(values.len(), 256);
    }
}

#[test]
fn the_three_heightmaps_differ_on_terrain_with_trees() {
    // Terrain has leaves and plants, so the kinds must not be identical copies —
    // otherwise the predicates are not actually being applied.
    let world = World::in_memory(SEED);
    let mut found_difference = false;
    for i in 0..40i32 {
        let (cx, cz) = (i % 10, i / 10);
        let column = world.generate(cx, cz);
        let surface = column.heightmap();
        let no_leaves = column.heightmap_motion_blocking_no_leaves();
        if surface[..] != no_leaves[..] {
            found_difference = true;
            // Where they differ, WORLD_SURFACE must be the higher one (it counts
            // leaves and plants that NO_LEAVES skips).
            for (a, b) in surface.iter().zip(no_leaves.iter()) {
                assert!(
                    a >= b,
                    "WORLD_SURFACE {a} below MOTION_BLOCKING_NO_LEAVES {b}"
                );
            }
            break;
        }
    }
    assert!(
        found_difference,
        "no chunk had leaves or plants; the heightmap predicates went untested"
    );
}

#[test]
fn block_and_fluid_counts_follow_the_rules_measured_from_vanilla() {
    // The same two rules are verified against the vanilla capture itself, then
    // applied to our own output — so a drift in either is caught.
    for (label, chunks) in [
        ("vanilla", vanilla_terrain()),
        (
            "bcore",
            (0..6)
                .map(|i| {
                    let column = World::in_memory(SEED).generate(i, i * 3);
                    decode_chunk(&column.encode_payload(i, i * 3))
                })
                .collect(),
        ),
    ] {
        for chunk in &chunks {
            for (i, section) in chunk.sections.iter().enumerate() {
                let non_air = section
                    .blocks
                    .values
                    .iter()
                    .filter(|&&s| s != block_state::AIR)
                    .count();
                assert_eq!(
                    section.block_count as usize, non_air,
                    "{label}: blockCount != non-air count in section {i} of ({}, {})",
                    chunk.x, chunk.z
                );
                assert!(
                    section.fluid_count >= 0,
                    "{label}: negative fluidCount in section {i}"
                );
                assert!(
                    section.fluid_count as usize <= non_air,
                    "{label}: fluidCount {} exceeds non-air {non_air} in section {i}",
                    section.fluid_count
                );
            }
        }
    }
}

#[test]
fn our_water_sections_report_a_matching_fluid_count() {
    let world = World::in_memory(SEED);
    // Find a chunk with ocean in it, the same way the generator's own tests do.
    let mut checked = 0;
    for i in 0..64i32 {
        let cx = (i % 8) * 6 - 24;
        let cz = (i / 8) * 6 - 24;
        let column = world.generate(cx, cz);
        let decoded = decode_chunk(&column.encode_payload(cx, cz));
        for (s, section) in decoded.sections.iter().enumerate() {
            let water = section
                .blocks
                .values
                .iter()
                .filter(|&&state| state == block_state::WATER)
                .count();
            assert_eq!(
                section.fluid_count as usize, water,
                "fluidCount must equal the water blocks in section {s} of ({cx},{cz})"
            );
            if water > 0 {
                checked += 1;
            }
        }
    }
    assert!(
        checked > 0,
        "no water found in 64 sampled chunks; the fluid path went untested"
    );
}

#[test]
fn container_widths_stay_inside_the_vanilla_discipline() {
    // Vanilla's own chunks are the reference for what widths are legal.
    let mut vanilla_block_bits: Vec<u8> = Vec::new();
    let mut vanilla_biome_bits: Vec<u8> = Vec::new();
    for chunk in &vanilla_terrain() {
        for section in &chunk.sections {
            vanilla_block_bits.push(section.blocks.bits);
            vanilla_biome_bits.push(section.biomes.bits);
        }
    }
    vanilla_block_bits.sort_unstable();
    vanilla_block_bits.dedup();
    vanilla_biome_bits.sort_unstable();
    vanilla_biome_bits.dedup();

    // Vanilla only ever used single (0) or indirect (4..=8) block widths in this
    // capture — it never needed the global palette for natural terrain, which is
    // why BCore's indirect-only output is legitimate here.
    for &bits in &vanilla_block_bits {
        assert!(
            bits == 0 || (4..=MAX_INDIRECT_BLOCK_BITS).contains(&bits),
            "vanilla used block width {bits}, outside single/indirect"
        );
    }
    assert!(
        vanilla_block_bits.len() > 1,
        "capture should show several palette widths, got {vanilla_block_bits:?}"
    );

    let world = World::in_memory(SEED);
    for i in 0..8i32 {
        let decoded = decode_chunk(&world.generate(i * 7, -i * 5).encode_payload(i * 7, -i * 5));
        for (s, section) in decoded.sections.iter().enumerate() {
            let bits = section.blocks.bits;
            assert!(
                bits == 0 || (4..=MAX_INDIRECT_BLOCK_BITS).contains(&bits),
                "section {s} used block width {bits}, which is neither single nor indirect"
            );
            // Air-only sections must collapse to a single-value palette, like vanilla.
            if section.block_count == 0 {
                assert!(
                    section.blocks.is_single(),
                    "an empty section should be a single-value container"
                );
                assert_eq!(section.blocks.palette, vec![block_state::AIR]);
            }
            assert!(
                section.biomes.bits <= 3,
                "biome width {} exceeds the indirect limit",
                section.biomes.bits
            );
            assert!(!section.blocks.is_direct(SECTION_VOLUME));
        }
    }
}

#[test]
fn our_terrain_payload_is_in_the_same_size_class_as_vanillas() {
    let vanilla_sizes: Vec<usize> = vanilla_terrain().iter().map(|c| c.size).collect();
    let v_min = *vanilla_sizes.iter().min().expect("sizes");
    let v_max = *vanilla_sizes.iter().max().expect("sizes");

    let world = World::in_memory(SEED);
    for i in 0..8i32 {
        let size = world.chunk_payload(i * 3, i).len();
        // Not equality — different generators — but a terrain chunk must not be
        // flat-sized (~7 KB) nor absurdly large.
        assert!(
            size > 10_000,
            "chunk {i} encoded to {size} bytes, suspiciously close to a flat chunk"
        );
        assert!(
            size < v_max * 3,
            "chunk {i} encoded to {size} bytes, far above vanilla's {v_min}..{v_max}"
        );
    }
}

#[test]
fn terrain_surface_varies_between_chunks_unlike_the_flat_world() {
    let world = World::in_memory(SEED);
    let mut surfaces = Vec::new();
    for cx in 0..10i32 {
        let column = world.generate(cx, 0);
        surfaces.push(column.surface_y(8, 8).expect("solid ground"));
    }
    let min = *surfaces.iter().min().expect("surfaces");
    let max = *surfaces.iter().max().expect("surfaces");
    assert!(
        max > min,
        "surface is constant across 10 chunks ({surfaces:?}) — still flat?"
    );
    // And nowhere near the superflat surface height.
    assert!(
        min > 0,
        "terrain surface {min} is below y=0; superflat sits at -61"
    );
}

#[test]
fn every_section_carries_a_biome_container_with_real_ids() {
    let world = World::in_memory(SEED);
    let decoded = decode_chunk(&world.generate(0, 0).encode_payload(0, 0));
    assert_eq!(decoded.sections.len(), SECTION_COUNT);
    for (i, section) in decoded.sections.iter().enumerate() {
        assert_eq!(
            section.biomes.values.len(),
            SECTION_BIOMES,
            "section {i} biome cell count"
        );
        for &id in &section.biomes.values {
            // 66 biomes in the 26.2 registry; anything outside that is a bug.
            assert!(
                id < 66,
                "section {i} used biome id {id}, outside the registry"
            );
        }
    }
}

#[test]
fn a_column_reconstructed_from_the_wire_equals_the_original() {
    // Full structural equality: rebuild a ChunkColumn out of the decoded payload
    // and compare it to the source column.
    let world = World::in_memory(SEED);
    let column = world.generate(-7, 4);
    let decoded = decode_chunk(&column.encode_payload(-7, 4));

    let mut states = Vec::with_capacity(256 * WORLD_HEIGHT as usize);
    for section in &decoded.sections {
        states.extend_from_slice(&section.blocks.values);
    }
    let mut biomes = Vec::with_capacity(SECTION_COUNT * SECTION_BIOMES);
    for section in &decoded.sections {
        biomes.extend_from_slice(&section.biomes.values);
    }

    let rebuilt = ChunkColumn::from_parts(states, biomes);
    assert_eq!(rebuilt, column, "wire round trip lost information");
}
