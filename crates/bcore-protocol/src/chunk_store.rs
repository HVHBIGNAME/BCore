//! On-disk chunk persistence.
//!
//! # Why a custom format
//!
//! Vanilla stores 32x32 chunk groups in `.mca` region files with per-chunk
//! zlib-compressed NBT. BCore does not need that yet: what it needs is a format
//! that round-trips a [`ChunkColumn`] exactly, is trivially verifiable, and is
//! deterministic byte-for-byte so `save -> load -> save` is stable. Region files
//! are a later concern.
//!
//! # Format (`world/chunks/c.<x>.<z>.bcc`)
//!
//! All integers are **little-endian**; the file is a palette plus indices, which
//! keeps a typical terrain chunk in the low tens of KB.
//!
//! ```text
//! magic:          4 bytes  "BCC1"
//! version:        u16      = 1
//! flags:          u16      = 0 (reserved)
//! chunk_x:        i32
//! chunk_z:        i32
//! min_y:          i32      = MIN_Y  (guards against a world-height change)
//! world_height:   i32      = WORLD_HEIGHT
//! palette_len:    u32      n distinct block states
//! palette:        n * u32  the block-state ids, in first-seen order
//! index_bits:     u8       8, 16 or 32 — width of one index
//! indices:        256 * WORLD_HEIGHT entries of index_bits/8 bytes,
//!                 in wire order (x fastest, then z, then y)
//! biome_len:      u32      m distinct biome ids
//! biome_palette:  m * u32
//! biomes:         SECTION_COUNT * SECTION_BIOMES entries of u16
//! checksum:       u32      FNV-1a over everything above
//! ```
//!
//! The checksum makes a truncated or corrupted file a clean `Err` rather than a
//! silently wrong world.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::chunk::{ChunkColumn, MIN_Y, SECTION_BIOMES, SECTION_COUNT, WORLD_HEIGHT};

/// File magic: BCore Chunk v1.
pub const MAGIC: &[u8; 4] = b"BCC1";
/// Format version written into every file.
pub const FORMAT_VERSION: u16 = 1;
/// Default directory chunks are stored under, relative to the server's cwd.
pub const DEFAULT_WORLD_DIR: &str = "world";

/// Total block entries in one column.
const COLUMN_ENTRIES: usize = 256 * WORLD_HEIGHT as usize;
/// Total biome cells in one column.
const BIOME_ENTRIES: usize = SECTION_COUNT * SECTION_BIOMES;

/// Something went wrong reading or writing a chunk file.
#[derive(Debug)]
pub enum ChunkStoreError {
    /// The underlying filesystem operation failed.
    Io(io::Error),
    /// The file is not a BCore chunk file.
    BadMagic([u8; 4]),
    /// The file was written by an incompatible version.
    UnsupportedVersion(u16),
    /// The file's world geometry does not match this build.
    GeometryMismatch { min_y: i32, world_height: i32 },
    /// The file ended earlier than its header promised.
    Truncated { expected: usize, got: usize },
    /// The stored checksum does not match the data.
    ChecksumMismatch { stored: u32, computed: u32 },
    /// A palette index pointed outside the palette.
    BadPaletteIndex { index: u32, palette_len: usize },
    /// The index width byte was not 8, 16 or 32.
    BadIndexBits(u8),
}

impl std::fmt::Display for ChunkStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "chunk io error: {e}"),
            Self::BadMagic(m) => write!(f, "not a BCore chunk file (magic {m:?})"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported chunk format version {v}"),
            Self::GeometryMismatch {
                min_y,
                world_height,
            } => write!(
                f,
                "chunk was saved with min_y={min_y}, height={world_height}; \
                 this build uses min_y={MIN_Y}, height={WORLD_HEIGHT}"
            ),
            Self::Truncated { expected, got } => {
                write!(
                    f,
                    "chunk file truncated: expected {expected} bytes, got {got}"
                )
            }
            Self::ChecksumMismatch { stored, computed } => write!(
                f,
                "chunk checksum mismatch: stored {stored:#010x}, computed {computed:#010x}"
            ),
            Self::BadPaletteIndex { index, palette_len } => {
                write!(f, "palette index {index} out of range (len {palette_len})")
            }
            Self::BadIndexBits(bits) => write!(f, "invalid index width {bits} (want 8/16/32)"),
        }
    }
}

impl std::error::Error for ChunkStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ChunkStoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// FNV-1a, 32-bit. Small, dependency-free and good enough to catch truncation.
fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// A directory of saved chunks.
///
/// Chunks live in `<root>/chunks/`; the directory is created on demand by
/// [`ChunkStore::save`].
#[derive(Debug, Clone)]
pub struct ChunkStore {
    root: PathBuf,
}

impl ChunkStore {
    /// A store rooted at `world/` in the current directory.
    pub fn new() -> Self {
        Self::at(DEFAULT_WORLD_DIR)
    }

    /// A store rooted at an arbitrary directory (used by tests).
    pub fn at<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// The world root this store writes under.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory holding chunk files.
    pub fn chunks_dir(&self) -> PathBuf {
        self.root.join("chunks")
    }

    /// The path of one chunk's file.
    pub fn chunk_path(&self, x: i32, z: i32) -> PathBuf {
        self.chunks_dir().join(format!("c.{x}.{z}.bcc"))
    }

    /// Whether a chunk is already on disk.
    pub fn contains(&self, x: i32, z: i32) -> bool {
        self.chunk_path(x, z).is_file()
    }

    /// Write a chunk, creating the directory if needed.
    ///
    /// The write is atomic: the bytes go to a `.tmp` sibling which is then
    /// renamed, so a crash mid-write cannot leave a half-written chunk that
    /// would later fail its checksum.
    pub fn save(&self, x: i32, z: i32, column: &ChunkColumn) -> Result<(), ChunkStoreError> {
        let dir = self.chunks_dir();
        fs::create_dir_all(&dir)?;
        let encoded = encode_chunk(x, z, column);
        let final_path = self.chunk_path(x, z);
        let tmp_path = final_path.with_extension("bcc.tmp");
        fs::write(&tmp_path, &encoded)?;
        // Windows rename fails if the destination exists, so clear it first.
        if final_path.exists() {
            fs::remove_file(&final_path)?;
        }
        fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Read a chunk back, or `None` if it was never saved.
    ///
    /// A corrupt or unreadable file is an `Err`, distinct from a missing one:
    /// silently regenerating over corruption would hide real bugs.
    pub fn load(&self, x: i32, z: i32) -> Result<Option<ChunkColumn>, ChunkStoreError> {
        let path = self.chunk_path(x, z);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        decode_chunk(&bytes).map(Some)
    }

    /// Saved chunk coordinates, sorted. Deterministic: no directory-order leak.
    pub fn saved_chunks(&self) -> Result<Vec<(i32, i32)>, ChunkStoreError> {
        let dir = self.chunks_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some(pos) = parse_chunk_name(name) {
                out.push(pos);
            }
        }
        out.sort_unstable();
        Ok(out)
    }
}

impl Default for ChunkStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse `c.<x>.<z>.bcc` back into coordinates.
fn parse_chunk_name(name: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix("c.")?.strip_suffix(".bcc")?;
    let (x, z) = rest.split_once('.')?;
    Some((x.parse().ok()?, z.parse().ok()?))
}

/// Serialise a column into the on-disk format.
pub fn encode_chunk(x: i32, z: i32, column: &ChunkColumn) -> Vec<u8> {
    let states = column.states();
    let biomes = column.biomes();

    // First-seen palette order keeps the output deterministic.
    // A last-hit cache avoids an O(volume * palette) rescan; terrain is highly
    // run-length friendly so this hits on the vast majority of blocks.
    let mut palette: Vec<u32> = Vec::new();
    let mut indices: Vec<u32> = Vec::with_capacity(states.len());
    let mut last: Option<(u32, u32)> = None;
    for &state in states {
        let index = match last {
            Some((cached, index)) if cached == state => index,
            _ => {
                let index = match palette.iter().position(|&p| p == state) {
                    Some(i) => i as u32,
                    None => {
                        palette.push(state);
                        (palette.len() - 1) as u32
                    }
                };
                last = Some((state, index));
                index
            }
        };
        indices.push(index);
    }

    let index_bits: u8 = if palette.len() <= u8::MAX as usize + 1 {
        8
    } else if palette.len() <= u16::MAX as usize + 1 {
        16
    } else {
        32
    };

    let mut biome_palette: Vec<u32> = Vec::new();
    let mut biome_indices: Vec<u16> = Vec::with_capacity(biomes.len());
    for &biome in biomes {
        let index = match biome_palette.iter().position(|&p| p == biome) {
            Some(i) => i,
            None => {
                biome_palette.push(biome);
                biome_palette.len() - 1
            }
        };
        biome_indices.push(index as u16);
    }

    let mut out =
        Vec::with_capacity(32 + palette.len() * 4 + indices.len() * index_bits as usize / 8);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&z.to_le_bytes());
    out.extend_from_slice(&MIN_Y.to_le_bytes());
    out.extend_from_slice(&WORLD_HEIGHT.to_le_bytes());

    out.extend_from_slice(&(palette.len() as u32).to_le_bytes());
    for &state in &palette {
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.push(index_bits);
    for &index in &indices {
        match index_bits {
            8 => out.push(index as u8),
            16 => out.extend_from_slice(&(index as u16).to_le_bytes()),
            _ => out.extend_from_slice(&index.to_le_bytes()),
        }
    }

    out.extend_from_slice(&(biome_palette.len() as u32).to_le_bytes());
    for &biome in &biome_palette {
        out.extend_from_slice(&biome.to_le_bytes());
    }
    for &index in &biome_indices {
        out.extend_from_slice(&index.to_le_bytes());
    }

    let checksum = fnv1a(&out);
    out.extend_from_slice(&checksum.to_le_bytes());
    out
}

/// Cursor over a chunk file, with bounds checks that produce real errors.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ChunkStoreError> {
        let end = self.at.checked_add(n).ok_or(ChunkStoreError::Truncated {
            expected: usize::MAX,
            got: self.bytes.len(),
        })?;
        if end > self.bytes.len() {
            return Err(ChunkStoreError::Truncated {
                expected: end,
                got: self.bytes.len(),
            });
        }
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ChunkStoreError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ChunkStoreError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }

    fn u32(&mut self) -> Result<u32, ChunkStoreError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn i32(&mut self) -> Result<i32, ChunkStoreError> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
}

/// Parse the on-disk format back into a column.
///
/// Returns the stored coordinates alongside the column via [`decode_chunk_at`];
/// this variant drops them because callers already know which chunk they asked
/// for.
pub fn decode_chunk(bytes: &[u8]) -> Result<ChunkColumn, ChunkStoreError> {
    decode_chunk_at(bytes).map(|(_, _, column)| column)
}

/// Parse the on-disk format, returning `(chunk_x, chunk_z, column)`.
pub fn decode_chunk_at(bytes: &[u8]) -> Result<(i32, i32, ChunkColumn), ChunkStoreError> {
    if bytes.len() < 4 {
        return Err(ChunkStoreError::Truncated {
            expected: 4,
            got: bytes.len(),
        });
    }
    let magic: [u8; 4] = bytes[0..4].try_into().expect("4 bytes");
    if &magic != MAGIC {
        return Err(ChunkStoreError::BadMagic(magic));
    }
    // Verify the checksum before trusting any header field.
    if bytes.len() < 8 {
        return Err(ChunkStoreError::Truncated {
            expected: 8,
            got: bytes.len(),
        });
    }
    let split = bytes.len() - 4;
    let stored = u32::from_le_bytes(bytes[split..].try_into().expect("4 bytes"));
    let computed = fnv1a(&bytes[..split]);
    if stored != computed {
        return Err(ChunkStoreError::ChecksumMismatch { stored, computed });
    }

    let mut cur = Cursor {
        bytes: &bytes[..split],
        at: 4,
    };
    let version = cur.u16()?;
    if version != FORMAT_VERSION {
        return Err(ChunkStoreError::UnsupportedVersion(version));
    }
    let _flags = cur.u16()?;
    let x = cur.i32()?;
    let z = cur.i32()?;
    let min_y = cur.i32()?;
    let world_height = cur.i32()?;
    if min_y != MIN_Y || world_height != WORLD_HEIGHT {
        return Err(ChunkStoreError::GeometryMismatch {
            min_y,
            world_height,
        });
    }

    let palette_len = cur.u32()? as usize;
    let mut palette = Vec::with_capacity(palette_len.min(1 << 16));
    for _ in 0..palette_len {
        palette.push(cur.u32()?);
    }

    let index_bits = cur.u8()?;
    if !matches!(index_bits, 8 | 16 | 32) {
        return Err(ChunkStoreError::BadIndexBits(index_bits));
    }

    let mut states = Vec::with_capacity(COLUMN_ENTRIES);
    for _ in 0..COLUMN_ENTRIES {
        let index = match index_bits {
            8 => cur.u8()? as u32,
            16 => cur.u16()? as u32,
            _ => cur.u32()?,
        };
        let state = *palette
            .get(index as usize)
            .ok_or(ChunkStoreError::BadPaletteIndex { index, palette_len })?;
        states.push(state);
    }

    let biome_len = cur.u32()? as usize;
    let mut biome_palette = Vec::with_capacity(biome_len.min(1 << 16));
    for _ in 0..biome_len {
        biome_palette.push(cur.u32()?);
    }
    let mut biomes = Vec::with_capacity(BIOME_ENTRIES);
    for _ in 0..BIOME_ENTRIES {
        let index = cur.u16()? as u32;
        let biome = *biome_palette
            .get(index as usize)
            .ok_or(ChunkStoreError::BadPaletteIndex {
                index,
                palette_len: biome_len,
            })?;
        biomes.push(biome);
    }

    let column = ChunkColumn::from_parts(states, biomes);
    Ok((x, z, column))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::block_state;

    fn temp_dir(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "bcore-store-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trips_a_flat_column() {
        let column = ChunkColumn::flat();
        let encoded = encode_chunk(3, -7, &column);
        let (x, z, decoded) = decode_chunk_at(&encoded).expect("decodes");
        assert_eq!((x, z), (3, -7));
        assert_eq!(decoded, column, "flat column must survive a round trip");
    }

    #[test]
    fn round_trips_a_column_with_many_states() {
        // Force a 16-bit index width by using more than 256 distinct states.
        let mut column = ChunkColumn::flat();
        for i in 0..400u32 {
            let y = 10 + (i / 256) as i32;
            let x = (i % 16) as usize;
            let z = ((i / 16) % 16) as usize;
            column.set(x, y, z, i + 1);
        }
        let encoded = encode_chunk(0, 0, &column);
        // Verify the width really escalated.
        let decoded = decode_chunk(&encoded).expect("decodes");
        assert_eq!(decoded, column);
    }

    #[test]
    fn encoding_is_deterministic() {
        let column = ChunkColumn::flat();
        assert_eq!(encode_chunk(1, 2, &column), encode_chunk(1, 2, &column));
        // Coordinates are part of the payload.
        assert_ne!(encode_chunk(1, 2, &column), encode_chunk(2, 1, &column));
    }

    #[test]
    fn save_then_load_returns_the_same_blocks() {
        let dir = temp_dir("save-load");
        let store = ChunkStore::at(&dir);
        let column = ChunkColumn::flat();

        assert!(!store.contains(4, -2));
        assert_eq!(store.load(4, -2).expect("load"), None, "missing is None");

        store.save(4, -2, &column).expect("save");
        assert!(store.contains(4, -2));
        let loaded = store.load(4, -2).expect("load").expect("present");
        assert_eq!(loaded, column);
        // The chunk file really is on disk where we said.
        assert!(store.chunk_path(4, -2).is_file());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_twice_overwrites_cleanly() {
        let dir = temp_dir("overwrite");
        let store = ChunkStore::at(&dir);
        let flat = ChunkColumn::flat();
        store.save(0, 0, &flat).expect("first save");

        let mut changed = flat.clone();
        changed.set(1, 50, 1, block_state::STONE);
        store.save(0, 0, &changed).expect("second save");

        let loaded = store.load(0, 0).expect("load").expect("present");
        assert_eq!(loaded, changed);
        assert_eq!(loaded.get(1, 50, 1), Some(block_state::STONE));
        // No leftover temp file.
        assert!(!store.chunk_path(0, 0).with_extension("bcc.tmp").exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saved_chunks_lists_every_coordinate_sorted() {
        let dir = temp_dir("list");
        let store = ChunkStore::at(&dir);
        assert_eq!(store.saved_chunks().expect("empty"), Vec::new());

        let column = ChunkColumn::flat();
        for &(x, z) in &[(5, 5), (-1, 3), (0, 0), (-1, -1)] {
            store.save(x, z, &column).expect("save");
        }
        assert_eq!(
            store.saved_chunks().expect("list"),
            vec![(-1, -1), (-1, 3), (0, 0), (5, 5)]
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn negative_coordinates_survive_the_filename() {
        assert_eq!(parse_chunk_name("c.-5.-9.bcc"), Some((-5, -9)));
        assert_eq!(parse_chunk_name("c.0.0.bcc"), Some((0, 0)));
        assert_eq!(parse_chunk_name("c.12.-3.bcc"), Some((12, -3)));
        assert_eq!(parse_chunk_name("garbage.bcc"), None);
        assert_eq!(parse_chunk_name("c.1.2.txt"), None);

        let dir = temp_dir("negative");
        let store = ChunkStore::at(&dir);
        store.save(-13, -27, &ChunkColumn::flat()).expect("save");
        assert!(store.load(-13, -27).expect("load").is_some());
        assert_eq!(store.saved_chunks().expect("list"), vec![(-13, -27)]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_magic_is_rejected() {
        let mut encoded = encode_chunk(0, 0, &ChunkColumn::flat());
        encoded[0] = b'X';
        assert!(matches!(
            decode_chunk(&encoded),
            Err(ChunkStoreError::BadMagic(_))
        ));
    }

    #[test]
    fn a_flipped_bit_is_caught_by_the_checksum() {
        let mut encoded = encode_chunk(0, 0, &ChunkColumn::flat());
        // Corrupt a byte in the middle of the index array.
        let middle = encoded.len() / 2;
        encoded[middle] ^= 0xff;
        assert!(matches!(
            decode_chunk(&encoded),
            Err(ChunkStoreError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn truncation_is_caught() {
        let encoded = encode_chunk(0, 0, &ChunkColumn::flat());
        for cut in [0usize, 3, 8, 40, encoded.len() / 2, encoded.len() - 1] {
            let err = decode_chunk(&encoded[..cut]);
            assert!(err.is_err(), "truncating to {cut} bytes must fail");
        }
    }

    #[test]
    fn a_bad_version_is_rejected() {
        let mut encoded = encode_chunk(0, 0, &ChunkColumn::flat());
        encoded[4..6].copy_from_slice(&99u16.to_le_bytes());
        // Fix the checksum so the version check is what actually trips.
        let split = encoded.len() - 4;
        let checksum = fnv1a(&encoded[..split]);
        encoded[split..].copy_from_slice(&checksum.to_le_bytes());
        assert!(matches!(
            decode_chunk(&encoded),
            Err(ChunkStoreError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn a_geometry_change_is_rejected() {
        let mut encoded = encode_chunk(0, 0, &ChunkColumn::flat());
        // min_y sits at offset 4+2+2+4+4 = 16.
        encoded[16..20].copy_from_slice(&(-128i32).to_le_bytes());
        let split = encoded.len() - 4;
        let checksum = fnv1a(&encoded[..split]);
        encoded[split..].copy_from_slice(&checksum.to_le_bytes());
        assert!(matches!(
            decode_chunk(&encoded),
            Err(ChunkStoreError::GeometryMismatch { min_y: -128, .. })
        ));
    }

    #[test]
    fn a_flat_chunk_file_has_exactly_the_documented_layout() {
        // Flat terrain has 4 distinct states (air/bedrock/dirt/grass) and 1
        // biome, so indices are 8-bit: one byte per block, not four.
        let encoded = encode_chunk(0, 0, &ChunkColumn::flat());
        let header = 4 + 2 + 2 + 4 + 4 + 4 + 4; // magic, version, flags, x, z, min_y, height
        let blocks = 4 + 4 * 4 + 1 + COLUMN_ENTRIES; // len, palette, index_bits, indices
        let biomes = 4 + 4 + BIOME_ENTRIES * 2; // len, 1-entry palette, indices
        let checksum = 4;
        assert_eq!(encoded.len(), header + blocks + biomes + checksum);
        // Sanity: a whole flat column stays well under 200 KB.
        assert!(encoded.len() < 200 * 1024, "{} bytes", encoded.len());
    }
}
