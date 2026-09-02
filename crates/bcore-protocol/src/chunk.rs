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
//! fluidCount: i16   (NEW in 26.x — blocks with a non-empty fluid state)
//! blocks: PalettedContainer over 4096 entries
//! biomes: PalettedContainer over 64 entries
//! ```
//!
//! A paletted container is `bitsPerEntry: u8`, then the palette, then the packed
//! data array. Since 1.21.5 the data array is **not** length-prefixed: its length
//! is `ceil(entries / floor(64 / bitsPerEntry))` longs. `bitsPerEntry == 0` means
//! a single-valued palette (one varint, no data array), and a width above the
//! indirect limit (8 for blocks) means the **global** palette: the entries are
//! raw block-state ids and no palette array is sent.
//!
//! # Where the semantics come from
//!
//! Every rule below was measured against captures from the official 26.2 server
//! rather than assumed. `scripts/capture_terrain.py` records real overworld
//! chunks and `scripts/analyze_terrain_capture.py` checks candidate rules against
//! them, over 49 chunks / 1176 sections / 12544 columns:
//!
//! * `blockCount == count(state != air)` — matched **every** section.
//! * `WORLD_SURFACE == highest non-air Y + 1 - MIN_Y` — matched **every** column.
//! * `fluidCount` counts blocks with a non-empty *fluid state*: water, lava and
//!   waterlogged blocks (vanilla's seagrass/kelp are waterlogged, which is what
//!   made the naive "water only" rule miss 52 of 1176 sections). BCore only ever
//!   places `water`, so [`ChunkColumn::is_fluid`] is exactly right for the blocks
//!   BCore generates.
//! * `MOTION_BLOCKING` counts blocks that block motion *or* hold a fluid, so
//!   water and leaves are included but tall grass is not;
//!   `MOTION_BLOCKING_NO_LEAVES` additionally skips leaves. Sampling vanilla's
//!   own heightmap tops confirmed this: `MOTION_BLOCKING` tops were grass_block,
//!   water and *_leaves, while `MOTION_BLOCKING_NO_LEAVES` tops were
//!   grass_block, water and *_log — never leaves.
//! * The highest water block in the capture is `y = 62`, i.e. water fills
//!   `y < 63`, which is why [`SEA_LEVEL`] is 63 and [`WATER_SURFACE_Y`] is 62.
//!
//! The heightmap **order** is deliberately not treated as fixed: vanilla is
//! inconsistent about it. The flat captures send `[WORLD_SURFACE,
//! MOTION_BLOCKING_NO_LEAVES, MOTION_BLOCKING]` while the terrain capture sends
//! `[MOTION_BLOCKING_NO_LEAVES, MOTION_BLOCKING, WORLD_SURFACE]`
//! (`scripts/compare_heightmap_order.py`). Since the wire carries `(kind, values)`
//! pairs the client does not care; BCore emits the flat-capture order so
//! `tests/chunk_format.rs` keeps its byte-for-byte parity.
//!
//! `flat_chunk_payload` reproduces a captured flat chunk byte-for-byte
//! (see `tests/chunk_format.rs`), and the terrain path is checked structurally
//! against the terrain capture in `tests/chunk_terrain.rs`.

use std::sync::OnceLock;

use bcore_core::varint::encode_varint;
use bcore_worldgen::{Biome, GeneratedChunk};

/// Lowest block Y coordinate of the overworld.
pub const MIN_Y: i32 = -64;
/// Total overworld height in blocks.
pub const WORLD_HEIGHT: i32 = 384;
/// Highest block Y coordinate of the overworld.
pub const MAX_Y: i32 = MIN_Y + WORLD_HEIGHT - 1;
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
/// Columns in a chunk (16 x 16).
pub const COLUMNS: usize = 256;

/// Sea level: the water *surface* plane. Water occupies `y < SEA_LEVEL`.
pub const SEA_LEVEL: i32 = 63;
/// The highest Y a water block occupies — verified against vanilla captures.
pub const WATER_SURFACE_Y: i32 = SEA_LEVEL - 1;

/// Block-state ids as assigned by the vanilla 26.2 block-state registry.
///
/// These are the real network ids used by `map_chunk` paletted containers, taken
/// from the official server's own datagen block report
/// (`java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports`,
/// extracted by `scripts/extract_block_states.py`).
///
/// The four ids BCore already used were read back out of captured `map_chunk`
/// payloads, and the datagen report agrees with all four
/// (`air=0`, `grass_block=9`, `dirt=10`, `bedrock=85`), which cross-validates the
/// whole table.
pub mod block_state {
    /// `minecraft:air`
    pub const AIR: u32 = 0;
    /// `minecraft:stone`
    pub const STONE: u32 = 1;
    /// `minecraft:grass_block[snowy=false]`
    pub const GRASS_BLOCK: u32 = 9;
    /// `minecraft:dirt`
    pub const DIRT: u32 = 10;
    /// `minecraft:coarse_dirt`
    pub const COARSE_DIRT: u32 = 11;
    /// `minecraft:podzol[snowy=false]`
    pub const PODZOL: u32 = 13;
    /// `minecraft:bedrock`
    pub const BEDROCK: u32 = 85;
    /// `minecraft:water[level=0]`
    pub const WATER: u32 = 86;
    /// `minecraft:lava[level=0]`
    pub const LAVA: u32 = 102;
    /// `minecraft:sand`
    pub const SAND: u32 = 118;
    /// `minecraft:gravel`
    pub const GRAVEL: u32 = 124;
    /// `minecraft:gold_ore`
    pub const GOLD_ORE: u32 = 129;
    /// `minecraft:iron_ore`
    pub const IRON_ORE: u32 = 131;
    /// `minecraft:coal_ore`
    pub const COAL_ORE: u32 = 133;
    /// `minecraft:oak_log[axis=y]`
    pub const OAK_LOG: u32 = 137;
    /// `minecraft:birch_log[axis=y]`
    pub const BIRCH_LOG: u32 = 143;
    /// `minecraft:oak_leaves[distance=1,persistent=false,waterlogged=false]`
    pub const OAK_LEAVES: u32 = 255;
    /// `minecraft:birch_leaves[distance=1,persistent=false,waterlogged=false]`
    pub const BIRCH_LEAVES: u32 = 311;
    /// `minecraft:lapis_ore`
    pub const LAPIS_ORE: u32 = 563;
    /// `minecraft:sandstone`
    pub const SANDSTONE: u32 = 578;
    /// `minecraft:short_grass`
    pub const SHORT_GRASS: u32 = 2248;
    /// `minecraft:dead_bush`
    pub const DEAD_BUSH: u32 = 2250;
    /// `minecraft:diamond_ore`
    pub const DIAMOND_ORE: u32 = 5307;
    /// `minecraft:redstone_ore[lit=false]`
    pub const REDSTONE_ORE: u32 = 6882;
    /// `minecraft:snow_block`
    pub const SNOW_BLOCK: u32 = 6928;
    /// `minecraft:cactus[age=0]`
    pub const CACTUS: u32 = 6929;
    /// `minecraft:packed_ice`
    pub const PACKED_ICE: u32 = 12914;
    /// `minecraft:copper_ore`
    pub const COPPER_ORE: u32 = 27790;
    /// `minecraft:deepslate[axis=y]`
    pub const DEEPSLATE: u32 = 30417;

    /// Total block states in the 26.2 registry (`scripts/count_block_states.py`).
    pub const BLOCK_STATE_COUNT: usize = 32366;
}

/// Biome network ids from the `minecraft:worldgen/biome` registry sync.
///
/// Biomes are a *datapack* registry, so unlike blocks their ids are not in the
/// datagen reports: they come from the order of entries in the clientbound
/// `registry_data` packet BCore already replays from
/// `data/config_packets.bin`. Extracted by `scripts/extract_biomes.py`.
pub mod biome {
    /// `minecraft:beach`
    pub const BEACH: u32 = 3;
    /// `minecraft:desert`
    pub const DESERT: u32 = 14;
    /// `minecraft:forest`
    pub const FOREST: u32 = 21;
    /// `minecraft:ocean`
    pub const OCEAN: u32 = 35;
    /// `minecraft:plains`
    pub const PLAINS: u32 = 40;
    /// `minecraft:river`
    pub const RIVER: u32 = 41;
    /// `minecraft:snowy_slopes`
    pub const SNOWY_SLOPES: u32 = 47;
    /// `minecraft:windswept_hills`
    pub const WINDSWEPT_HILLS: u32 = 63;
}

/// `minecraft:plains` — kept for compatibility with the flat-world path.
pub const BIOME_PLAINS: u32 = biome::PLAINS;

/// Heightmap kinds, in the order the vanilla server transmits them.
pub const HEIGHTMAP_WORLD_SURFACE: i32 = 1;
pub const HEIGHTMAP_MOTION_BLOCKING: i32 = 4;
pub const HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES: i32 = 5;

/// Bits used to encode 256 heightmap entries (9 bits covers 0..=384).
const HEIGHTMAP_BITS: u8 = 9;

/// Superflat surface: bedrock at -64, dirt at -63/-62, grass at -61.
pub const FLAT_SURFACE_Y: i32 = -61;

/// Widest indirect block palette vanilla accepts before the global palette.
pub const MAX_INDIRECT_BLOCK_BITS: u8 = 8;
/// Bit width of the global (direct) block palette: `ceil(log2(32366))`.
pub const DIRECT_PALETTE_BITS: u8 = 15;

/// Palette layout of a paletted container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKind {
    /// One value for every entry; no data array is sent.
    Single,
    /// A local palette; entries are indices into it.
    Indirect,
    /// The global block-state palette; entries are raw ids and no palette is sent.
    Direct,
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
        _ => None, // caller must fall back to the global palette
    }
}

/// The container layout and bit width used for a block palette of `len` values.
pub fn block_palette_layout(len: usize) -> (PaletteKind, u8) {
    match block_palette_bits(len) {
        Some(0) => (PaletteKind::Single, 0),
        Some(bits) => (PaletteKind::Indirect, bits),
        None => (PaletteKind::Direct, DIRECT_PALETTE_BITS),
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

/// Pack raw `u32` values (used by the global palette, whose ids exceed `u16`).
pub fn pack_entries_u32(bits: u8, entries: &[u32]) -> Vec<i64> {
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

/// Write a global-palette container: a bit width above the indirect limit, no
/// palette array, and the raw block-state ids as the packed entries.
pub fn write_direct_container(out: &mut Vec<u8>, states: &[u32]) {
    out.push(DIRECT_PALETTE_BITS);
    for long in pack_entries_u32(DIRECT_PALETTE_BITS, states) {
        out.extend_from_slice(&long.to_be_bytes());
    }
}

/// Write one heightmap entry: kind, long count, longs.
pub fn write_heightmap(out: &mut Vec<u8>, kind: i32, values: &[u16; COLUMNS]) {
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
            states: vec![block_state::AIR; COLUMNS * WORLD_HEIGHT as usize],
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

    /// Rebuild a column from raw parts (used by the on-disk chunk decoder).
    ///
    /// # Panics
    ///
    /// If the slices are not exactly one column's worth of data.
    pub fn from_parts(states: Vec<u32>, biomes: Vec<u32>) -> Self {
        assert_eq!(
            states.len(),
            COLUMNS * WORLD_HEIGHT as usize,
            "block state count must be one full column"
        );
        assert_eq!(
            biomes.len(),
            SECTION_COUNT * SECTION_BIOMES,
            "biome count must be one full column"
        );
        Self { states, biomes }
    }

    /// Convert a generated chunk into an encodable column.
    ///
    /// The generator stores one surface biome per `(x, z)`; the wire format wants
    /// one id per 4x4x4 cell, so each cell takes the biome of the column at its
    /// centre — the same 4-block biome resolution vanilla itself uses.
    pub fn from_generated(chunk: &GeneratedChunk) -> Self {
        let states = chunk.states().to_vec();
        let mut biomes = Vec::with_capacity(SECTION_COUNT * SECTION_BIOMES);
        for _section in 0..SECTION_COUNT {
            for cell_y in 0..4 {
                let _ = cell_y;
                for cell_z in 0..4 {
                    for cell_x in 0..4 {
                        // Centre of the 4x4 area this cell covers.
                        let x = cell_x * 4 + 2;
                        let z = cell_z * 4 + 2;
                        biomes.push(chunk.biome_at(x, z).network_id());
                    }
                }
            }
        }
        Self::from_parts(states, biomes)
    }

    #[inline]
    fn index(x: usize, y: i32, z: usize) -> Option<usize> {
        if x >= 16 || z >= 16 || y < MIN_Y || y > MAX_Y {
            return None;
        }
        Some((y - MIN_Y) as usize * COLUMNS + z * 16 + x)
    }

    /// Read a block state by chunk-local coordinates.
    #[inline]
    pub fn get(&self, x: usize, y: i32, z: usize) -> Option<u32> {
        Self::index(x, y, z).map(|i| self.states[i])
    }

    /// Write a block state by chunk-local coordinates; returns `false` if out of range.
    #[inline]
    pub fn set(&mut self, x: usize, y: i32, z: usize, state: u32) -> bool {
        match Self::index(x, y, z) {
            Some(i) => {
                self.states[i] = state;
                true
            }
            None => false,
        }
    }

    /// Every block state in wire order (`x` fastest, then `z`, then `y`).
    pub fn states(&self) -> &[u32] {
        &self.states
    }

    /// Every biome cell, one per 4x4x4 region, in wire order.
    pub fn biomes(&self) -> &[u32] {
        &self.biomes
    }

    /// Set every biome cell of the column.
    pub fn fill_biome(&mut self, biome: u32) {
        self.biomes.iter_mut().for_each(|b| *b = biome);
    }

    /// Whether a state holds a fluid, i.e. counts toward a section's `fluidCount`.
    ///
    /// Vanilla counts every block with a non-empty fluid state (water, lava and
    /// waterlogged blocks). BCore only places `water`, so this is exact for the
    /// blocks BCore generates; `lava` is listed because the constant exists.
    #[inline]
    pub fn is_fluid(state: u32) -> bool {
        matches!(state, block_state::WATER | block_state::LAVA)
    }

    /// Whether a state is a leaf block (skipped by `MOTION_BLOCKING_NO_LEAVES`).
    #[inline]
    pub fn is_leaves(state: u32) -> bool {
        matches!(state, block_state::OAK_LEAVES | block_state::BIRCH_LEAVES)
    }

    /// Whether a state is a non-colliding plant (skipped by both
    /// `MOTION_BLOCKING` heightmaps, but present in `WORLD_SURFACE`).
    #[inline]
    pub fn is_passable_plant(state: u32) -> bool {
        matches!(state, block_state::SHORT_GRASS | block_state::DEAD_BUSH)
    }

    /// Highest non-air Y at `(x, z)`, or `None` for an empty column.
    pub fn surface_y(&self, x: usize, z: usize) -> Option<i32> {
        self.column_top(x, z, |state| state != block_state::AIR)
    }

    /// Highest Y at `(x, z)` whose state satisfies `keep`.
    fn column_top<F: Fn(u32) -> bool>(&self, x: usize, z: usize, keep: F) -> Option<i32> {
        if x >= 16 || z >= 16 {
            return None;
        }
        let base = z * 16 + x;
        for level in (0..WORLD_HEIGHT as usize).rev() {
            let state = self.states[level * COLUMNS + base];
            if keep(state) {
                return Some(MIN_Y + level as i32);
            }
        }
        None
    }

    /// Per-column topmost Y for a predicate, as heightmap values.
    ///
    /// A heightmap entry is `highest matching Y + 1 - MIN_Y`, or 0 when no block
    /// in the column matches — verified against vanilla for `WORLD_SURFACE`.
    ///
    /// Like [`Self::surfaces`] this is a single top-down pass over the state
    /// array with an early exit once every column has been resolved.
    fn heightmap_for<F: Fn(u32) -> bool>(&self, keep: F) -> [u16; COLUMNS] {
        let mut out = [0u16; COLUMNS];
        let mut done = [false; COLUMNS];
        let mut remaining = COLUMNS;
        for level in (0..WORLD_HEIGHT as usize).rev() {
            if remaining == 0 {
                break;
            }
            let slice = &self.states[level * COLUMNS..(level + 1) * COLUMNS];
            for (i, &state) in slice.iter().enumerate() {
                if !done[i] && keep(state) {
                    out[i] = (MIN_Y + level as i32 + 1 - MIN_Y) as u16;
                    done[i] = true;
                    remaining -= 1;
                }
            }
        }
        out
    }

    /// `WORLD_SURFACE`: the highest non-air block, plants and leaves included.
    pub fn heightmap(&self) -> [u16; COLUMNS] {
        self.heightmap_for(|state| state != block_state::AIR)
    }

    /// `MOTION_BLOCKING`: highest block that blocks motion or holds a fluid.
    pub fn heightmap_motion_blocking(&self) -> [u16; COLUMNS] {
        self.heightmap_for(|state| state != block_state::AIR && !Self::is_passable_plant(state))
    }

    /// `MOTION_BLOCKING_NO_LEAVES`: as above, but leaves do not count.
    pub fn heightmap_motion_blocking_no_leaves(&self) -> [u16; COLUMNS] {
        self.heightmap_for(|state| {
            state != block_state::AIR && !Self::is_passable_plant(state) && !Self::is_leaves(state)
        })
    }

    /// Encode one section (index 0 = lowest) as `blockCount`, `fluidCount`, blocks, biomes.
    fn write_section(&self, section: usize, out: &mut Vec<u8>) {
        let base = section * SECTION_VOLUME;
        let states = &self.states[base..base + SECTION_VOLUME];

        // Deterministic local palette: first-seen order, exactly like vanilla.
        //
        // `palette.iter().position()` would make this O(volume * palette), which
        // for a 40-entry terrain section is 160k comparisons per section and
        // dominated encoding. A last-hit cache exploits the fact that terrain is
        // strongly runs-of-the-same-block, so the scan almost always hits index 0
        // of the check.
        let mut palette: Vec<u32> = Vec::new();
        let mut indices = vec![0u16; SECTION_VOLUME];
        let mut block_count = 0i16;
        let mut fluid_count = 0i16;
        let mut last: Option<(u32, u16)> = None;
        for (i, &state) in states.iter().enumerate() {
            if state != block_state::AIR {
                block_count += 1;
            }
            if Self::is_fluid(state) {
                fluid_count += 1;
            }
            let idx = match last {
                Some((cached, idx)) if cached == state => idx,
                _ => {
                    let idx = match palette.iter().position(|&p| p == state) {
                        Some(p) => p as u16,
                        None => {
                            palette.push(state);
                            (palette.len() - 1) as u16
                        }
                    };
                    last = Some((state, idx));
                    idx
                }
            };
            indices[i] = idx;
        }

        out.extend_from_slice(&block_count.to_be_bytes());
        out.extend_from_slice(&fluid_count.to_be_bytes());

        match block_palette_layout(palette.len()) {
            (PaletteKind::Direct, _) => write_direct_container(out, states),
            (_, bits) => write_paletted_container(out, bits, &palette, &indices),
        }

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
        let biome_bits =
            biome_palette_bits(biome_palette.len()).expect("at most 8 biomes per section");
        write_paletted_container(out, biome_bits, &biome_palette, &biome_indices);
    }

    /// Sky light for the section containing world Y `MIN_Y + section * 16`.
    ///
    /// A block is fully lit when nothing in its own column is above it. This is
    /// exact for heightmap terrain and conservative (dark) under overhangs and
    /// inside caves, which is what BCore generates today.
    ///
    /// `surfaces` is the precomputed per-column topmost non-air Y, so this does
    /// not rescan the column for every one of the 4096 nibbles.
    fn sky_layer_with(&self, section: usize, surfaces: &[Option<i32>; COLUMNS]) -> Vec<u8> {
        let mut layer = vec![0u8; LIGHT_LAYER_BYTES];
        let section_min_y = MIN_Y + section as i32 * 16;
        for y_off in 0..16i32 {
            let y = section_min_y + y_off;
            for z in 0..16usize {
                for x in 0..16usize {
                    let lit = surfaces[z * 16 + x].is_none_or(|surface| y > surface);
                    if !lit {
                        continue;
                    }
                    let nibble = y_off as usize * COLUMNS + z * 16 + x;
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

    /// Sky light for one section (convenience wrapper used by tests).
    pub fn sky_layer(&self, section: usize) -> Vec<u8> {
        self.sky_layer_with(section, &self.surfaces())
    }

    /// Topmost non-air Y per column, in wire order.
    ///
    /// Scans all 256 columns in one top-down pass over the state array instead of
    /// calling `surface_y` 256 times: terrain tops out well below y=320, so the
    /// upper (all-air) levels are skipped once for the whole chunk rather than
    /// once per column.
    fn surfaces(&self) -> [Option<i32>; COLUMNS] {
        let mut out = [None; COLUMNS];
        let mut remaining = COLUMNS;
        for level in (0..WORLD_HEIGHT as usize).rev() {
            if remaining == 0 {
                break;
            }
            let slice = &self.states[level * COLUMNS..(level + 1) * COLUMNS];
            // Whole-level fast path: an all-air level cannot be any column's top.
            if slice.iter().all(|&s| s == block_state::AIR) {
                continue;
            }
            for (i, &state) in slice.iter().enumerate() {
                if out[i].is_none() && state != block_state::AIR {
                    out[i] = Some(MIN_Y + level as i32);
                    remaining -= 1;
                }
            }
        }
        out
    }

    /// Light sections that carry sky-light data, as light-bitset indices.
    ///
    /// Light index `i` is section `i - 1` of the column, so index 0 is the
    /// section *below* the world. Vanilla only transmits layers around the
    /// terrain and lets the client default everything above to full daylight;
    /// we reproduce that (surface sections + one above, below-world declared
    /// empty when the terrain reaches the bottom section).
    fn lit_sections_from(&self, surfaces: &[Option<i32>; COLUMNS]) -> (Vec<usize>, Vec<usize>) {
        let present: Vec<i32> = surfaces.iter().filter_map(|&s| s).collect();
        let Some(&max) = present.iter().max() else {
            // Empty column: nothing to light, everything below-world is empty.
            return (Vec::new(), vec![0]);
        };
        let min = *present.iter().min().expect("non-empty");
        let lowest = ((min - MIN_Y) / 16) as usize;
        let highest = ((max - MIN_Y) / 16) as usize;
        let data: Vec<usize> = (lowest + 1..=(highest + 2).min(LIGHT_SECTION_COUNT - 1)).collect();
        let empty: Vec<usize> = if lowest == 0 { vec![0] } else { Vec::new() };
        (data, empty)
    }

    /// Light sections that carry sky-light data (convenience wrapper).
    pub fn lit_sections(&self) -> (Vec<usize>, Vec<usize>) {
        self.lit_sections_from(&self.surfaces())
    }

    /// Encode the full `map_chunk` payload (without the packet id) for `(x, z)`.
    pub fn encode_payload(&self, x: i32, z: i32) -> Vec<u8> {
        let mut out = Vec::with_capacity(48 * 1024);
        out.extend_from_slice(&x.to_be_bytes());
        out.extend_from_slice(&z.to_be_bytes());

        // The three kinds differ once plants and leaves exist, so each is built
        // from its own predicate rather than reusing one array.
        encode_varint(3, &mut out);
        write_heightmap(&mut out, HEIGHTMAP_WORLD_SURFACE, &self.heightmap());
        write_heightmap(
            &mut out,
            HEIGHTMAP_MOTION_BLOCKING_NO_LEAVES,
            &self.heightmap_motion_blocking_no_leaves(),
        );
        write_heightmap(
            &mut out,
            HEIGHTMAP_MOTION_BLOCKING,
            &self.heightmap_motion_blocking(),
        );

        let mut sections = Vec::with_capacity(24 * 1024);
        for section in 0..SECTION_COUNT {
            self.write_section(section, &mut sections);
        }
        encode_varint(sections.len() as i32, &mut out);
        out.extend_from_slice(&sections);

        encode_varint(0, &mut out); // blockEntities

        let surfaces = self.surfaces();
        let (sky_bits, empty_sky_bits) = self.lit_sections_from(&surfaces);
        write_long_array(&mut out, &light_bitset(&sky_bits));
        write_long_array(&mut out, &[]); // blockLightMask: no block light sources
        write_long_array(&mut out, &light_bitset(&empty_sky_bits));
        let mut empty_block: Vec<usize> = sky_bits.clone();
        empty_block.extend_from_slice(&empty_sky_bits);
        empty_block.sort_unstable();
        write_long_array(&mut out, &light_bitset(&empty_block));

        let sky: Vec<Vec<u8>> = sky_bits
            .iter()
            .map(|&i| self.sky_layer_with(i - 1, &surfaces))
            .collect();
        write_light_arrays(&mut out, &sky);
        write_light_arrays(&mut out, &[]); // blockLight
        out
    }
}

/// The biome id a generator [`Biome`] maps to on the wire.
pub fn biome_network_id(biome: Biome) -> u32 {
    biome.network_id()
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
    fn palette_layout_escalates_single_indirect_direct() {
        assert_eq!(block_palette_layout(1), (PaletteKind::Single, 0));
        assert_eq!(block_palette_layout(2), (PaletteKind::Indirect, 4));
        assert_eq!(block_palette_layout(256), (PaletteKind::Indirect, 8));
        assert_eq!(
            block_palette_layout(257),
            (PaletteKind::Direct, DIRECT_PALETTE_BITS)
        );
        // The direct width must cover every real block state.
        assert!(
            (1usize << DIRECT_PALETTE_BITS) >= block_state::BLOCK_STATE_COUNT,
            "{DIRECT_PALETTE_BITS} bits cannot address {} states",
            block_state::BLOCK_STATE_COUNT
        );
        assert!((1usize << (DIRECT_PALETTE_BITS - 1)) < block_state::BLOCK_STATE_COUNT);
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
    fn u32_packing_matches_u16_packing_for_small_values() {
        let small: Vec<u16> = (0..300).map(|i| (i % 17) as u16).collect();
        let wide: Vec<u32> = small.iter().map(|&v| v as u32).collect();
        assert_eq!(pack_entries(15, &small), pack_entries_u32(15, &wide));
        // And a value that does not fit in the indirect palette still round-trips.
        let longs = pack_entries_u32(15, &[30417, 0, 32365]);
        let per_long = 64 / 15;
        let first = longs[0] as u64;
        assert_eq!(first & 0x7fff, 30417);
        assert_eq!((first >> 30) & 0x7fff, 32365);
        assert_eq!(per_long, 4);
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

    #[test]
    fn from_parts_rejects_wrong_sizes() {
        let states = vec![0u32; COLUMNS * WORLD_HEIGHT as usize];
        let biomes = vec![BIOME_PLAINS; SECTION_COUNT * SECTION_BIOMES];
        // Correct sizes are accepted.
        let column = ChunkColumn::from_parts(states.clone(), biomes.clone());
        assert_eq!(column.states().len(), COLUMNS * WORLD_HEIGHT as usize);
        assert_eq!(column.biomes().len(), SECTION_COUNT * SECTION_BIOMES);
    }

    #[test]
    #[should_panic(expected = "one full column")]
    fn from_parts_panics_on_a_short_state_array() {
        ChunkColumn::from_parts(
            vec![0u32; 10],
            vec![BIOME_PLAINS; SECTION_COUNT * SECTION_BIOMES],
        );
    }

    // ---- heightmap predicates -------------------------------------------

    #[test]
    fn the_three_heightmaps_agree_on_plain_terrain() {
        let column = ChunkColumn::flat();
        assert_eq!(column.heightmap(), column.heightmap_motion_blocking());
        assert_eq!(
            column.heightmap(),
            column.heightmap_motion_blocking_no_leaves()
        );
    }

    #[test]
    fn plants_only_raise_world_surface() {
        let mut column = ChunkColumn::flat();
        // Tall grass on top of the flat surface.
        column.set(0, FLAT_SURFACE_Y + 1, 0, block_state::SHORT_GRASS);
        let surface = column.heightmap()[0];
        let motion = column.heightmap_motion_blocking()[0];
        let no_leaves = column.heightmap_motion_blocking_no_leaves()[0];
        // WORLD_SURFACE sees the grass; neither MOTION_BLOCKING map does.
        assert_eq!(surface, (FLAT_SURFACE_Y + 1 + 1 - MIN_Y) as u16);
        assert_eq!(motion, (FLAT_SURFACE_Y + 1 - MIN_Y) as u16);
        assert_eq!(no_leaves, motion);
    }

    #[test]
    fn leaves_raise_world_surface_and_motion_blocking_but_not_no_leaves() {
        let mut column = ChunkColumn::flat();
        column.set(0, FLAT_SURFACE_Y + 5, 0, block_state::OAK_LEAVES);
        let surface = column.heightmap()[0];
        let motion = column.heightmap_motion_blocking()[0];
        let no_leaves = column.heightmap_motion_blocking_no_leaves()[0];
        let leaf_height = (FLAT_SURFACE_Y + 5 + 1 - MIN_Y) as u16;
        assert_eq!(surface, leaf_height, "leaves are part of WORLD_SURFACE");
        assert_eq!(motion, leaf_height, "leaves block motion");
        assert_eq!(
            no_leaves,
            (FLAT_SURFACE_Y + 1 - MIN_Y) as u16,
            "NO_LEAVES must skip the canopy"
        );
    }

    #[test]
    fn water_counts_in_every_heightmap() {
        let mut column = ChunkColumn::flat();
        column.set(0, 40, 0, block_state::WATER);
        let want = (40 + 1 - MIN_Y) as u16;
        assert_eq!(column.heightmap()[0], want);
        assert_eq!(column.heightmap_motion_blocking()[0], want);
        assert_eq!(column.heightmap_motion_blocking_no_leaves()[0], want);
    }

    #[test]
    fn block_classification_matches_the_measured_vanilla_rules() {
        assert!(ChunkColumn::is_fluid(block_state::WATER));
        assert!(ChunkColumn::is_fluid(block_state::LAVA));
        assert!(!ChunkColumn::is_fluid(block_state::STONE));
        assert!(!ChunkColumn::is_fluid(block_state::AIR));

        assert!(ChunkColumn::is_leaves(block_state::OAK_LEAVES));
        assert!(ChunkColumn::is_leaves(block_state::BIRCH_LEAVES));
        assert!(!ChunkColumn::is_leaves(block_state::OAK_LOG));

        assert!(ChunkColumn::is_passable_plant(block_state::SHORT_GRASS));
        assert!(ChunkColumn::is_passable_plant(block_state::DEAD_BUSH));
        // Cactus and leaves do block motion, so they are not "passable".
        assert!(!ChunkColumn::is_passable_plant(block_state::CACTUS));
        assert!(!ChunkColumn::is_passable_plant(block_state::OAK_LEAVES));
    }

    // ---- fluidCount ------------------------------------------------------

    #[test]
    fn fluid_count_counts_water_in_the_encoded_section() {
        let mut column = ChunkColumn::new(BIOME_PLAINS);
        // Section 8 spans y = 64..79. Put 3 water blocks in it.
        for (i, x) in [0usize, 1, 2].into_iter().enumerate() {
            column.set(x, 64 + i as i32, 0, block_state::WATER);
        }
        // And a stone block, which must not be counted as fluid.
        column.set(5, 70, 5, block_state::STONE);

        let payload = column.encode_payload(0, 0);
        let section = decode_section(&payload, 8);
        assert_eq!(section.fluid_count, 3, "three water blocks");
        assert_eq!(section.block_count, 4, "water and stone are both non-air");
    }

    #[test]
    fn a_section_without_fluid_reports_zero() {
        let column = ChunkColumn::flat();
        let payload = column.encode_payload(0, 0);
        let section = decode_section(&payload, 0);
        assert_eq!(section.fluid_count, 0);
        // bedrock + 2 dirt + grass = 4 layers of 256 blocks.
        assert_eq!(section.block_count, 4 * 256);
    }

    // ---- direct palette --------------------------------------------------

    #[test]
    fn a_section_with_more_than_256_states_uses_the_global_palette() {
        let mut column = ChunkColumn::new(BIOME_PLAINS);
        // Section 8 spans y = 64..79: fill it with 300 distinct states.
        let mut placed = 0u32;
        'fill: for y in 64..80 {
            for z in 0..16 {
                for x in 0..16 {
                    column.set(x, y, z, placed + 1);
                    placed += 1;
                    if placed == 300 {
                        break 'fill;
                    }
                }
            }
        }
        let payload = column.encode_payload(0, 0);
        let section = decode_section(&payload, 8);
        assert_eq!(
            section.bits, DIRECT_PALETTE_BITS,
            "should have switched to the global palette"
        );
        assert!(
            section.palette.is_empty(),
            "no palette array in direct mode"
        );
        // The decoded entries are the raw state ids.
        assert_eq!(section.states[0], 1);
        assert_eq!(section.states[299], 300);
        assert_eq!(section.states[300], block_state::AIR);
    }

    // ---- test-only decoder ----------------------------------------------

    struct DecodedSection {
        block_count: i16,
        fluid_count: i16,
        bits: u8,
        palette: Vec<u32>,
        states: Vec<u32>,
    }

    /// Minimal `map_chunk` reader used to assert on our own output.
    fn decode_section(payload: &[u8], want: usize) -> DecodedSection {
        let mut at = 8usize; // skip x, z
        let (count, n) = bcore_core::varint::decode_varint(&payload[at..]).expect("hm count");
        at += n;
        for _ in 0..count {
            let (_kind, n) = bcore_core::varint::decode_varint(&payload[at..]).expect("kind");
            at += n;
            let (longs, n) = bcore_core::varint::decode_varint(&payload[at..]).expect("len");
            at += n + longs as usize * 8;
        }
        let (_data_len, n) = bcore_core::varint::decode_varint(&payload[at..]).expect("data len");
        at += n;

        for section in 0..=want {
            let block_count = i16::from_be_bytes(payload[at..at + 2].try_into().expect("bc"));
            let fluid_count = i16::from_be_bytes(payload[at + 2..at + 4].try_into().expect("fc"));
            at += 4;

            let (bits, palette, states, used) = read_container(&payload[at..], SECTION_VOLUME);
            at += used;
            // Biomes.
            let (_bb, _bp, _bs, used) = read_container(&payload[at..], SECTION_BIOMES);
            at += used;

            if section == want {
                return DecodedSection {
                    block_count,
                    fluid_count,
                    bits,
                    palette,
                    states,
                };
            }
        }
        unreachable!("section {want} not reached")
    }

    /// Read a paletted container, returning `(bits, palette, values, bytes_used)`.
    fn read_container(data: &[u8], entries: usize) -> (u8, Vec<u32>, Vec<u32>, usize) {
        let bits = data[0];
        let mut at = 1usize;
        if bits == 0 {
            let (single, n) = bcore_core::varint::decode_varint(&data[at..]).expect("single");
            at += n;
            return (bits, vec![single as u32], vec![single as u32; entries], at);
        }

        // Blocks use the global palette above 8 bits; biomes above 3.
        let direct = entries == SECTION_VOLUME && bits > MAX_INDIRECT_BLOCK_BITS;
        let mut palette = Vec::new();
        if !direct {
            let (len, n) = bcore_core::varint::decode_varint(&data[at..]).expect("palette len");
            at += n;
            for _ in 0..len {
                let (value, n) = bcore_core::varint::decode_varint(&data[at..]).expect("entry");
                at += n;
                palette.push(value as u32);
            }
        }

        let per_long = 64 / bits as usize;
        let longs = entries.div_ceil(per_long);
        let mask = (1u64 << bits) - 1;
        let mut values = Vec::with_capacity(entries);
        for i in 0..entries {
            let long = u64::from_be_bytes(
                data[at + (i / per_long) * 8..at + (i / per_long) * 8 + 8]
                    .try_into()
                    .expect("long"),
            );
            let raw = ((long >> ((i % per_long) * bits as usize)) & mask) as u32;
            values.push(if direct { raw } else { palette[raw as usize] });
        }
        at += longs * 8;
        (bits, palette, values, at)
    }
}
