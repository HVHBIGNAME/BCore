//! Native chunk-column encoding for the 26.2 `map_chunk` packet (id `0x2d`).
//!
//! # Wire format (protocol 776)
//!
//! ```text
//! x: i32, z: i32
//! heightmaps: varint count, then per entry { varint kind, varint len, len * i64 }
//! chunkData:  varint byte length, then `SECTION_COUNT` chunk sections back to back
//! blockEntities: varint count, then per entry { u8 packedXZ, i16 y, varint type, NBT }
//! skyLightMask / blockLightMask / emptySkyLightMask / emptyBlockLightMask:
//!            varint count, then count * i64 (bitsets over LIGHT_SECTION_COUNT)
//! skyLight / blockLight: varint count, then per entry { varint len, len * u8 }
//! ```
//!
//! A chunk section is:
//!
//! ```text
//! blockCount: i16   (non-air blocks)
//! fluidCount: i16   (NEW in 26.x — waterlogged blocks + water/lava)
//! blocks: PalettedContainer over 4096 entries
//! biomes: PalettedContainer over 64 entries
//! ```
//!
//! A paletted container is `bitsPerEntry: u8`, then the palette, then the packed
//! data array. Since 1.21.5 the data array is **not** length-prefixed: its length
//! is `ceil(entries / floor(64 / bitsPerEntry))` longs. `bitsPerEntry == 0` means
//! a single-valued palette (one varint, no data array).
//!
//! These facts were verified byte-for-byte against captures from the official
//! 26.2 server: `flat_chunk_payload` reproduces a captured flat chunk exactly
//! (see `tests/chunk_format.rs`).

use std::sync::OnceLock;

use bcore_core::varint::encode_varint;

/// Lowest block Y coordinate of the overworld.
pub const MIN_Y: i32 = -64;
/// Total overworld height in blocks.
pub const WORLD_HEIGHT: i32 = 384;
/// Number of 16-block-tall sections in a column (`MIN_Y..MIN_Y + WORLD_HEIGHT`).
pub const SECTION_COUNT: usize = (WORLD_HEIGHT / 16) as usize;
/// Light is also sent for one section below and one above the world.
pub const LIGHT_SECTION_COUNT: usize = SECTION_COUNT + 2;
/// Blocks per chunk section.
pub const SECTION_VOLUME: usize = 16 * 16 * 16;
/// Biome cells per chunk section (4x4x4 regions).
pub const SECTION_BIOMES: usize = 4 * 4 * 4;
/// Bytes in one light data layer: 4096 nibbles.
pub const LIGHT_LAYER_BYTES: usize = SECTION_VOLUME / 2;

/// Block-state ids as assigned by the vanilla 26.2 block-state registry.
///
/// Unlike [`bcore_core::registry`] these are the real network ids, read back out
/// of captured `map_chunk` payloads from the official server.
pub mod block_state {
    pub const AIR: u32 = 0;
    pub const GRASS_BLOCK: u32 = 9;
    pub const DIRT: u32 = 10;
    pub const BEDROCK: u32 = 85;
}

/// `minecraft:plains` in the 26.2 worldgen/biome registry.
pub const BIOME_PLAINS: u32 = 40;

/// Heightmap kinds, in the order the vanilla server transmits them.
pub const HEIGHTMAP_WORLD_SURFACE: i32 = 1;
pub const HEIGHTMAP_MOTION_BLOCKING: i32 = 4;
pub const HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES: i32 = 5;

/// Bits used to encode 256 heightmap entries (9 bits covers 0..=384).
const HEIGHTMAP_BITS: u8 = 9;

/// Superflat surface: bedrock at -64, dirt at -63/-62, grass at -61.
pub const FLAT_SURFACE_Y: i32 = -61;

/// Palette layout of a [`PalettedContainer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    /// One value for every entry; no data array is sent.
    Single,
    /// A local palette; entries are indices into it.
    Indirect,
}

/// Number of bits per entry for a block-state palette of `len` values.
///
/// Vanilla's `Strategy` clamps linear block palettes to a 4-bit floor and
/// switches to the global palette above 8 bits.
pub fn block_palette_bits(len: usize) -> Option<u8> {
    match ceil_log2(len) {
        0 => Some(0),
        1..=4 => Some(4),
        b @ 5..=8 => Some(b),
        _ => None, // global palette: not produced by BCore yet
    }
}

/// Number of bits per entry for a biome palette of `len` values.
pub fn biome_palette_bits(len: usize) -> Option<u8> {
    match ceil_log2(len) {
        0 => Some(0),
        b @ 1..=3 => Some(b),
        _ => None, // global palette: not produced by BCore yet
    }
}

fn ceil_log2(len: usize) -> u8 {
    if len <= 1 {
        return 0;
    }
    let mut bits = 0u8;
    while (1usize << bits) < len {
        bits += 1;
    }
    bits
}

/// Number of longs used by a packed data array.
pub fn packed_len(bits: u8, entries: usize) -> usize {
    if bits == 0 {
        return 0;
    }
    let per_long = 64 / bits as usize;
    entries.div_ceil(per_long)
}

/// Pack `entries` as `bits`-wide little-endian-ordered values into longs.
///
/// Entries never straddle a long boundary; the spare high bits are left zero.
pub fn pack_entries(bits: u8, entries: &[u16]) -> Vec<i64> {
    if bits == 0 {
        return Vec::new();
    }
    let per_long = 64 / bits as usize;
    let mut out = vec![0i64; packed_len(bits, entries.len())];
    for (i, &value) in entries.iter().enumerate() {
        let shift = (i % per_long) * bits as usize;
        out[i / per_long] |= ((value as u64) << shift) as i64;
    }
    out
}

/// Write `bits`, the palette and the (unprefixed) data array of a container.
pub fn write_paletted_container(out: &mut Vec<u8>, bits: u8, palette: &[u32], indices: &[u16]) {
    out.push(bits);
    if bits == 0 {
        encode_varint(palette[0] as i32, out);
        return;
    }
    encode_varint(palette.len() as i32, out);
    for &value in palette {
        encode_varint(value as i32, out);
    }
    for long in pack_entries(bits, indices) {
        out.extend_from_slice(&long.to_be_bytes());
    }
}

/// Write one heightmap entry: kind, long count, longs.
pub fn write_heightmap(out: &mut Vec<u8>, kind: i32, values: &[u16; 256]) {
    encode_varint(kind, out);
    let longs = pack_entries(HEIGHTMAP_BITS, values);
    encode_varint(longs.len() as i32, out);
    for long in longs {
        out.extend_from_slice(&long.to_be_bytes());
    }
}

/// Build a light-section bitset (one long per 64 sections) from set bits.
pub fn light_bitset(bits: &[usize]) -> Vec<i64> {
    if bits.is_empty() {
        return Vec::new();
    }
    let longs = LIGHT_SECTION_COUNT.div_ceil(64);
    let mut out = vec![0i64; longs];
    for &bit in bits {
        out[bit / 64] |= (1u64 << (bit % 64)) as i64;
    }
    out
}

fn write_long_array(out: &mut Vec<u8>, longs: &[i64]) {
    encode_varint(longs.len() as i32, out);
    for long in longs {
        out.extend_from_slice(&long.to_be_bytes());
    }
}

fn write_light_arrays(out: &mut Vec<u8>, arrays: &[Vec<u8>]) {
    encode_varint(arrays.len() as i32, out);
    for array in arrays {
        encode_varint(array.len() as i32, out);
        out.extend_from_slice(array);
    }
}

/// A full 16 x [`WORLD_HEIGHT`] x 16 column of block states plus biomes.
///
/// Block states are indexed `[(y - MIN_Y) * 256 + z * 16 + x]`, matching the
/// wire order (increasing x, then z, then y). Biomes are one id per 4x4x4 cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkColumn {
    states: Vec<u32>,
    biomes: Vec<u32>,
}

impl ChunkColumn {
    /// An all-air column with every biome cell set to `biome`.
    pub fn new(biome: u32) -> Self {
        Self {
            states: vec![block_state::AIR; 256 * WORLD_HEIGHT as usize],
            biomes: vec![biome; SECTION_COUNT * SECTION_BIOMES],
        }
    }

    /// The vanilla superflat profile: bedrock, two dirt layers, grass, then air.
    pub fn flat() -> Self {
        let mut column = Self::new(BIOME_PLAINS);
        for z in 0..16 {
            for x in 0..16 {
                column.set(x, MIN_Y, z, block_state::BEDROCK);
                column.set(x, MIN_Y + 1, z, block_state::DIRT);
                column.set(x, MIN_Y + 2, z, block_state::DIRT);
                column.set(x, FLAT_SURFACE_Y, z, block_state::GRASS_BLOCK);
            }
        }
        column
    }

    fn index(x: usize, y: i32, z: usize) -> Option<usize> {
        if x >= 16 || z >= 16 || y < MIN_Y || y >= MIN_Y + WORLD_HEIGHT {
            return None;
        }
        Some((y - MIN_Y) as usize * 256 + z * 16 + x)
    }

    /// Read a block state by chunk-local coordinates.
    pub fn get(&self, x: usize, y: i32, z: usize) -> Option<u32> {
        Self::index(x, y, z).map(|i| self.states[i])
    }

    /// Write a block state by chunk-local coordinates; returns `false` if out of range.
    pub fn set(&mut self, x: usize, y: i32, z: usize, state: u32) -> bool {
        match Self::index(x, y, z) {
            Some(i) => {
                self.states[i] = state;
                true
            }
            None => false,
        }
    }

    /// Set every biome cell of the column.
    pub fn fill_biome(&mut self, biome: u32) {
        self.biomes.iter_mut().for_each(|b| *b = biome);
    }

    /// Highest non-air Y at `(x, z)`, or `None` for an empty column.
    pub fn surface_y(&self, x: usize, z: usize) -> Option<i32> {
        (MIN_Y..MIN_Y + WORLD_HEIGHT)
            .rev()
            .find(|&y| self.get(x, y, z).is_some_and(|s| s != block_state::AIR))
    }

    /// Heightmap values: `highest non-air Y + 1 - MIN_Y`, or 0 for empty columns.
    pub fn heightmap(&self) -> [u16; 256] {
        let mut out = [0u16; 256];
        for z in 0..16 {
            for x in 0..16 {
                out[z * 16 + x] = self
                    .surface_y(x, z)
                    .map(|y| (y + 1 - MIN_Y) as u16)
                    .unwrap_or(0);
            }
        }
        out
    }

    /// Encode one section (index 0 = lowest) as `blockCount`, `fluidCount`, blocks, biomes.
    fn write_section(&self, section: usize, out: &mut Vec<u8>) {
        let base = section * SECTION_VOLUME;
        let states = &self.states[base..base + SECTION_VOLUME];

        // Deterministic local palette: first-seen order, exactly like vanilla.
        let mut palette: Vec<u32> = Vec::new();
        let mut indices = vec![0u16; SECTION_VOLUME];
        let mut block_count = 0i16;
        for (i, &state) in states.iter().enumerate() {
            if state != block_state::AIR {
                block_count += 1;
            }
            let idx = match palette.iter().position(|&p| p == state) {
                Some(p) => p,
                None => {
                    palette.push(state);
                    palette.len() - 1
                }
            };
            indices[i] = idx as u16;
        }

        out.extend_from_slice(&block_count.to_be_bytes());
        out.extend_from_slice(&0i16.to_be_bytes()); // fluidCount: no fluids yet

        let bits =
            block_palette_bits(palette.len()).expect("block palette fits an indirect palette");
        write_paletted_container(out, bits, &palette, &indices);

        let cells = &self.biomes[section * SECTION_BIOMES..(section + 1) * SECTION_BIOMES];
        let mut biome_palette: Vec<u32> = Vec::new();
        let mut biome_indices = vec![0u16; SECTION_BIOMES];
        for (i, &biome) in cells.iter().enumerate() {
            let idx = match biome_palette.iter().position(|&p| p == biome) {
                Some(p) => p,
                None => {
                    biome_palette.push(biome);
                    biome_palette.len() - 1
                }
            };
            biome_indices[i] = idx as u16;
        }
        let biome_bits = biome_palette_bits(biome_palette.len())
            .expect("biome palette fits an indirect palette");
        write_paletted_container(out, biome_bits, &biome_palette, &biome_indices);
    }

    /// Sky light for the section containing world Y `MIN_Y + section * 16`.
    ///
    /// A block is fully lit when nothing in its own column is above it. This is
    /// exact for heightmap terrain (no overhangs, no light-blocking transparency),
    /// which is what BCore generates today.
    fn sky_layer(&self, section: usize) -> Vec<u8> {
        let mut layer = vec![0u8; LIGHT_LAYER_BYTES];
        let section_min_y = MIN_Y + section as i32 * 16;
        for y_off in 0..16i32 {
            let y = section_min_y + y_off;
            for z in 0..16usize {
                for x in 0..16usize {
                    let lit = self.surface_y(x, z).is_none_or(|surface| y > surface);
                    if !lit {
                        continue;
                    }
                    let nibble = y_off as usize * 256 + z * 16 + x;
                    let byte = &mut layer[nibble / 2];
                    if nibble % 2 == 0 {
                        *byte |= 0x0f;
                    } else {
                        *byte |= 0xf0;
                    }
                }
            }
        }
        layer
    }

    /// Light sections that carry sky-light data, as light-bitset indices.
    ///
    /// Light index `i` is section `i - 1` of the column, so index 0 is the
    /// section *below* the world. Vanilla only transmits layers around the
    /// terrain and lets the client default everything above to full daylight;
    /// we reproduce that (surface section + one above, below-world declared empty).
    fn lit_sections(&self) -> (Vec<usize>, Vec<usize>) {
        let surfaces: Vec<i32> = (0..16usize)
            .flat_map(|z| (0..16usize).map(move |x| (x, z)))
            .filter_map(|(x, z)| self.surface_y(x, z))
            .collect();
        let Some(&max) = surfaces.iter().max() else {
            // Empty column: nothing to light, everything below-world is empty.
            return (Vec::new(), vec![0]);
        };
        let min = *surfaces.iter().min().expect("non-empty");
        let lowest = ((min - MIN_Y) / 16) as usize;
        let highest = ((max - MIN_Y) / 16) as usize;
        let data: Vec<usize> = (lowest + 1..=(highest + 2).min(LIGHT_SECTION_COUNT - 1)).collect();
        let empty: Vec<usize> = if lowest == 0 { vec![0] } else { Vec::new() };
        (data, empty)
    }

    /// Encode the full `map_chunk` payload (without the packet id) for `(x, z)`.
    pub fn encode_payload(&self, x: i32, z: i32) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 * 1024);
        out.extend_from_slice(&x.to_be_bytes());
        out.extend_from_slice(&z.to_be_bytes());

        let heights = self.heightmap();
        encode_varint(3, &mut out);
        write_heightmap(&mut out, HEIGHTMAP_WORLD_SURFACE, &heights);
        write_heightmap(&mut out, HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES, &heights);
        write_heightmap(&mut out, HEIGHTMAP_MOTION_BLOCKING, &heights);

        let mut sections = Vec::with_capacity(2 * 1024 + 256);
        for section in 0..SECTION_COUNT {
            self.write_section(section, &mut sections);
        }
        encode_varint(sections.len() as i32, &mut out);
        out.extend_from_slice(&sections);

        encode_varint(0, &mut out); // blockEntities

        let (sky_bits, empty_sky_bits) = self.lit_sections();
        write_long_array(&mut out, &light_bitset(&sky_bits));
        write_long_array(&mut out, &[]); // blockLightMask: no block light sources
        write_long_array(&mut out, &light_bitset(&empty_sky_bits));
        let mut empty_block: Vec<usize> = sky_bits.clone();
        empty_block.extend_from_slice(&empty_sky_bits);
        empty_block.sort_unstable();
        write_long_array(&mut out, &light_bitset(&empty_block));

        let sky: Vec<Vec<u8>> = sky_bits.iter().map(|&i| self.sky_layer(i - 1)).collect();
        write_light_arrays(&mut out, &sky);
        write_light_arrays(&mut out, &[]); // blockLight
        out
    }
}

/// Cached encoding of a flat chunk. Only `x`/`z` differ between columns, so the
/// column is built and encoded once and the coordinates are spliced per chunk.
fn flat_template() -> &'static [u8] {
    static TEMPLATE: OnceLock<Vec<u8>> = OnceLock::new();
    TEMPLATE.get_or_init(|| ChunkColumn::flat().encode_payload(0, 0))
}

/// The `map_chunk` payload for a superflat chunk at `(x, z)`.
pub fn flat_chunk_payload(x: i32, z: i32) -> Vec<u8> {
    let mut out = flat_template().to_vec();
    out[0..4].copy_from_slice(&x.to_be_bytes());
    out[4..8].copy_from_slice(&z.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_bit_widths_match_vanilla_strategy() {
        assert_eq!(block_palette_bits(1), Some(0));
        assert_eq!(block_palette_bits(2), Some(4));
        assert_eq!(block_palette_bits(4), Some(4));
        assert_eq!(block_palette_bits(16), Some(4));
        assert_eq!(block_palette_bits(17), Some(5));
        assert_eq!(block_palette_bits(256), Some(8));
        assert_eq!(block_palette_bits(257), None);
        assert_eq!(biome_palette_bits(1), Some(0));
        assert_eq!(biome_palette_bits(2), Some(1));
        assert_eq!(biome_palette_bits(8), Some(3));
        assert_eq!(biome_palette_bits(9), None);
    }

    #[test]
    fn packed_entries_never_straddle_longs() {
        // 4 bits: 16 entries per long, no padding.
        assert_eq!(packed_len(4, 4096), 256);
        // 9 bits: 7 entries per long, 1 spare bit.
        assert_eq!(packed_len(9, 256), 37);
        let longs = pack_entries(9, &[4u16; 256]);
        assert_eq!(longs.len(), 37);
        assert_eq!(longs[0] as u64, 0x0100_8040_2010_0804);
        // Last long only holds 256 - 36 * 7 = 4 entries.
        assert_eq!(longs[36] as u64, 0x2010_0804);
    }

    #[test]
    fn flat_column_layers_are_correct() {
        let column = ChunkColumn::flat();
        assert_eq!(column.get(0, -64, 0), Some(block_state::BEDROCK));
        assert_eq!(column.get(0, -63, 0), Some(block_state::DIRT));
        assert_eq!(column.get(0, -62, 0), Some(block_state::DIRT));
        assert_eq!(column.get(0, -61, 0), Some(block_state::GRASS_BLOCK));
        assert_eq!(column.get(0, -60, 0), Some(block_state::AIR));
        assert_eq!(column.get(15, 319, 15), Some(block_state::AIR));
        assert_eq!(column.get(16, 0, 0), None);
        assert_eq!(column.surface_y(7, 9), Some(FLAT_SURFACE_Y));
        assert_eq!(column.heightmap()[0], 4);
    }

    #[test]
    fn flat_payload_is_deterministic_and_carries_coordinates() {
        assert_eq!(flat_chunk_payload(3, -7), flat_chunk_payload(3, -7));
        let payload = flat_chunk_payload(3, -7);
        assert_eq!(&payload[0..4], &3i32.to_be_bytes());
        assert_eq!(&payload[4..8], &(-7i32).to_be_bytes());
        // Only the coordinates differ between flat chunks.
        assert_eq!(&payload[8..], &flat_chunk_payload(100, 200)[8..]);
    }

    #[test]
    fn flat_light_matches_vanilla_section_selection() {
        let column = ChunkColumn::flat();
        let (data, empty) = column.lit_sections();
        // Surface section is index 0 of the column -> light indices 1 and 2.
        assert_eq!(data, vec![1, 2]);
        assert_eq!(empty, vec![0]);
        let surface = column.sky_layer(0);
        assert_eq!(surface.len(), LIGHT_LAYER_BYTES);
        // y -64..-61 are solid or the surface itself: dark.
        assert!(surface[..512].iter().all(|&b| b == 0x00));
        // y -60..-49 see the sky: full daylight.
        assert!(surface[512..].iter().all(|&b| b == 0xff));
        assert!(column.sky_layer(1).iter().all(|&b| b == 0xff));
    }

    #[test]
    fn empty_column_encodes_without_light_layers() {
        let column = ChunkColumn::new(BIOME_PLAINS);
        assert_eq!(column.heightmap()[0], 0);
        let (data, empty) = column.lit_sections();
        assert!(data.is_empty());
        assert_eq!(empty, vec![0]);
        // Still a structurally valid payload.
        assert!(column.encode_payload(0, 0).len() > 8);
    }

    #[test]
    fn modified_column_reports_new_surface() {
        let mut column = ChunkColumn::flat();
        assert!(column.set(4, 20, 5, block_state::DIRT));
        assert_eq!(column.surface_y(4, 5), Some(20));
        assert_eq!(column.heightmap()[5 * 16 + 4], (20 + 1 - MIN_Y) as u16);
        // Adding a block at y=20 pushes lit sections up to that section + 1.
        let (data, _) = column.lit_sections();
        assert_eq!(
            *data.last().expect("lit sections"),
            ((20 - MIN_Y) / 16) as usize + 2
        );
    }
}
