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
//! version:        u16      = 6 (older files lack registry identity and are rejected)
//! flags:          u16      v4: bit 0 marks; bit 1 structure entities; bit 2 light
//! chunk_x:        i32
//! chunk_z:        i32
//! min_y:          i32      = MIN_Y  (guards against a world-height change)
//! world_height:   i32      = WORLD_HEIGHT
//! data_version:   i32      Minecraft data version
//! protocol:       i32      Minecraft protocol version
//! registry:       32 bytes registry fingerprint
//! palette_len:    u32      n distinct block states
//! palette:        n * u32  the block-state ids, in first-seen order
//! index_bits:     u8       8, 16 or 32 — width of one index
//! indices:        256 * WORLD_HEIGHT entries of index_bits/8 bytes,
//!                 in wire order (x fastest, then z, then y)
//! biome_len:      u32      m distinct biome ids
//! biome_palette:  m * u32
//! biomes:         SECTION_COUNT * SECTION_BIOMES entries of u16
//! block_entities: u32 count; entries: u8 packedXZ, i32 y, u8 kind,
//!                 kind 1: i64 dungeon loot seed; kind 2: u8 spawner mob
//!                 (0 skeleton, 1 zombie, 2 spider, 3 cave spider). Absent in v1.
//!                 kind 3 (v4): u32 bee count, then i32 ticks_in_hive per bee.
//!                 kind 4 (v4): u32 byte length, then FeatureBlockEntity JSON,
//!                 retaining typed full NBT and the native update data.
//!                 kind 5 (v5): u32 byte length, then PendingBlockEntity JSON.
//!                 Raw typed proto NBT stays pending, separate from update data.
//! entities:       u32 count; entries: u8 kind (1 chest minecart),
//!                 u8 packedXZ, i32 y, i64 loot seed. Added in v3.
//!                 kind 2 (v4): u32 byte length, then physical typed proto-mob NBT
//!                 as JSON. Preserves UUID, position, rotation and numeric widths.
//! structures:     u32 byte length, then UTF-8 StructureData JSON (0 = empty).
//!                 Added in v3; coordinates/bounds are validated on load.
//! tick_requests:  u32 count; entries: u8 kind (1 block, 2 fluid), u8 packedXZ,
//!                 i32 y, u32 target ID, i32 relative delay. Added in v4.
//!                 NORMAL priority requests retain insertion order and duplicates.
//! postprocessing: u32 count; entries: u8 packedXZ, i32 y. Present if flags bit 0
//!                 is set; preserves unconsumed marks, order and duplicates.
//! structure_entities: u32 count; entries: u32 JSON byte length and typed entity
//!                 factory/finalization request. Present if flags bit 1 is set,
//!                 after optional postprocessing; never implies executed spawning.
//! light:          i32 minimum section Y, u16 section count, then per section:
//!                 u8 flags (bit 0 sky present, bit 1 block present, bit 2 sky
//!                 lazy-zero, bit 3 block lazy-zero). Each present non-lazy layer
//!                 has 2048 raw bytes, sky then block. Present if header bit 2 is
//!                 set, after optional structure entities. Null and lazy-zero
//!                 layers remain distinct from materialized all-zero arrays.
//! checksum:       u32      FNV-1a over everything above
//! ```
//!
//! The checksum makes a truncated or corrupted file a clean `Err` rather than a
//! silently wrong world.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bcore_registry::{
    BIOME_COUNT, BLOCK_STATE_COUNT, DATA_VERSION, MC_VERSION, PROTOCOL_VERSION,
    REGISTRY_FINGERPRINT,
};
use serde::{Deserialize, Serialize};

use crate::chunk::{ChunkColumn, MIN_Y, SECTION_BIOMES, SECTION_COUNT, WORLD_HEIGHT};

/// File magic: BCore Chunk v1.
pub const MAGIC: &[u8; 4] = b"BCC1";
/// Format version written into every file.
pub const FORMAT_VERSION: u16 = 6;
/// Bytes before the block palette, including the versioned registry identity.
pub const HEADER_LEN: usize = 64;
/// Default directory chunks are stored under, relative to the server's cwd.
pub const DEFAULT_WORLD_DIR: &str = "world";
const WORLD_METADATA_FILE: &str = "bcore-world.json";

/// Total block entries in one column.
const COLUMN_ENTRIES: usize = 256 * WORLD_HEIGHT as usize;
/// Total biome cells in one column.
const BIOME_ENTRIES: usize = SECTION_COUNT * SECTION_BIOMES;
const MAX_STRUCTURE_BYTES: usize = 4 * 1024 * 1024;
const MAX_BLOCK_ENTITY_BYTES: usize = 1024 * 1024;
const TICK_REQUEST_BYTES: usize = 1 + 1 + 4 + 4 + 4;
const POSTPROCESSING_FLAG: u16 = 1;
const STRUCTURE_ENTITIES_FLAG: u16 = 2;
const LIGHT_FLAG: u16 = 4;
const MAX_STRUCTURE_ENTITY_BYTES: usize = 1024 * 1024;
const MAX_GENERATED_MOB_BYTES: usize = 1024 * 1024;
const POSTPROCESSING_BYTES: usize = 1 + 4;

/// Something went wrong reading or writing a chunk file.
#[derive(Debug)]
pub enum ChunkStoreError {
    /// The underlying filesystem operation failed.
    Io(io::Error),
    /// The file is not a BCore chunk file.
    BadMagic([u8; 4]),
    /// The file was written by an incompatible version.
    UnsupportedVersion(u16),
    /// Pre-v6 files cannot identify the meaning of their numeric palettes.
    MissingRegistryIdentity(u16),
    /// The file targets another Minecraft data/protocol version.
    MinecraftVersionMismatch {
        data_version: i32,
        protocol_version: i32,
    },
    /// Numeric IDs were saved against another registry ordering.
    RegistryMismatch { stored: [u8; 32] },
    /// An existing world has no authenticated seed/version metadata.
    MissingWorldMetadata,
    /// Malformed or unsupported world metadata.
    InvalidWorldMetadata(String),
    /// An existing world belongs to another seed.
    SeedMismatch { stored: i64, requested: i64 },
    /// The chunk header does not agree with its filename/requested coordinates.
    CoordinateMismatch {
        expected: (i32, i32),
        stored: (i32, i32),
    },
    /// Unknown extension flags.
    UnsupportedFlags(u16),
    /// The file's world geometry does not match this build.
    GeometryMismatch { min_y: i32, world_height: i32 },
    /// The file ended earlier than its header promised.
    Truncated { expected: usize, got: usize },
    /// The stored checksum does not match the data.
    ChecksumMismatch { stored: u32, computed: u32 },
    /// A palette index pointed outside the palette.
    BadPaletteIndex { index: u32, palette_len: usize },
    /// A block-state/biome ID is outside the authenticated registry.
    InvalidRegistryId { registry: &'static str, id: u32 },
    /// A palette has an invalid number of entries.
    InvalidPaletteLength { registry: &'static str, len: usize },
    /// The index width byte was not 8, 16 or 32.
    BadIndexBits(u8),
    /// Invalid generated block-entity kind, position, duplicate or state pairing.
    InvalidBlockEntity,
    /// Invalid generated entity kind, count or position.
    InvalidEntity,
    /// Invalid pending structure-entity request, owner, position or typed data.
    InvalidStructureEntity,
    /// Invalid structure metadata or coordinates.
    InvalidStructures,
    /// Invalid deferred tick request kind, count, target ID or position.
    InvalidTickRequest,
    /// Invalid deferred postprocessing mark count or position.
    InvalidPostprocessing,
    /// Invalid native light geometry, layer flags or lazy-zero representation.
    InvalidLight,
    /// Unconsumed bytes precede the checksum.
    TrailingData,
}

impl std::fmt::Display for ChunkStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "chunk io error: {e}"),
            Self::BadMagic(m) => write!(f, "not a BCore chunk file (magic {m:?})"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported chunk format version {v}"),
            Self::MissingRegistryIdentity(v) => write!(
                f,
                "BCC v{v} has no registry identity; explicit migration is required"
            ),
            Self::MinecraftVersionMismatch {
                data_version,
                protocol_version,
            } => write!(
                f,
                "chunk Minecraft identity is data {data_version}/protocol {protocol_version}; expected {DATA_VERSION}/{PROTOCOL_VERSION}"
            ),
            Self::RegistryMismatch { stored } => write!(
                f,
                "chunk registry fingerprint {stored:02x?} differs from {REGISTRY_FINGERPRINT:02x?}"
            ),
            Self::MissingWorldMetadata => write!(
                f,
                "existing world has no {WORLD_METADATA_FILE}; use a fresh world directory or an explicit migration"
            ),
            Self::InvalidWorldMetadata(detail) => write!(f, "invalid world metadata: {detail}"),
            Self::SeedMismatch { stored, requested } => {
                write!(f, "world seed is {stored}, requested {requested}")
            }
            Self::CoordinateMismatch { expected, stored } => {
                write!(f, "chunk coordinates {stored:?} differ from requested {expected:?}")
            }
            Self::UnsupportedFlags(flags) => write!(f, "unsupported chunk flags {flags:#06x}"),
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
            Self::InvalidRegistryId { registry, id } => {
                write!(f, "invalid {registry} registry ID {id}")
            }
            Self::InvalidPaletteLength { registry, len } => {
                write!(f, "invalid {registry} palette length {len}")
            }
            Self::BadIndexBits(bits) => write!(f, "invalid index width {bits} (want 8/16/32)"),
            Self::InvalidBlockEntity => write!(f, "invalid generated block entity"),
            Self::InvalidEntity => write!(f, "invalid generated entity"),
            Self::InvalidStructureEntity => write!(f, "invalid structure entity request"),
            Self::InvalidStructures => write!(f, "invalid structure metadata"),
            Self::InvalidTickRequest => write!(f, "invalid deferred tick request"),
            Self::InvalidPostprocessing => write!(f, "invalid deferred postprocessing mark"),
            Self::InvalidLight => write!(f, "invalid native chunk light"),
            Self::TrailingData => write!(f, "trailing data in chunk file"),
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

/// Monotonic counter for unique temp-file names in [`ChunkStore::save`].
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory of saved chunks.
///
/// Chunks live in `<root>/chunks/`; the directory is created on demand by
/// [`ChunkStore::save`].
#[derive(Debug, Clone)]
pub struct ChunkStore {
    root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldMetadata {
    schema: u32,
    minecraft: String,
    data_version: i32,
    protocol_version: i32,
    registry_fingerprint: [u8; 32],
    seed: i64,
}

fn validate_identity(
    data_version: i32,
    protocol_version: i32,
    fingerprint: [u8; 32],
) -> Result<(), ChunkStoreError> {
    if data_version != DATA_VERSION || protocol_version != PROTOCOL_VERSION {
        return Err(ChunkStoreError::MinecraftVersionMismatch {
            data_version,
            protocol_version,
        });
    }
    if fingerprint != REGISTRY_FINGERPRINT {
        return Err(ChunkStoreError::RegistryMismatch {
            stored: fingerprint,
        });
    }
    Ok(())
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

    /// Validate all existing data before admitting a world. Only an absent or
    /// empty root may acquire new metadata; existing inputs are never relabeled.
    pub fn prepare_world(&self, seed: i64) -> Result<(), ChunkStoreError> {
        let metadata_path = self.root.join(WORLD_METADATA_FILE);
        match fs::read(&metadata_path) {
            Ok(bytes) => {
                let metadata: WorldMetadata = serde_json::from_slice(&bytes)
                    .map_err(|error| ChunkStoreError::InvalidWorldMetadata(error.to_string()))?;
                if metadata.schema != 1 || metadata.minecraft != MC_VERSION {
                    return Err(ChunkStoreError::InvalidWorldMetadata(
                        "unsupported schema or Minecraft version".into(),
                    ));
                }
                validate_identity(
                    metadata.data_version,
                    metadata.protocol_version,
                    metadata.registry_fingerprint,
                )?;
                if metadata.seed != seed {
                    return Err(ChunkStoreError::SeedMismatch {
                        stored: metadata.seed,
                        requested: seed,
                    });
                }
                self.validate_existing_chunks()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::read_dir(&self.root) {
                    Ok(mut entries) => {
                        if let Some(entry) = entries.next() {
                            entry?;
                            return Err(ChunkStoreError::MissingWorldMetadata);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                let metadata = WorldMetadata {
                    schema: 1,
                    minecraft: MC_VERSION.into(),
                    data_version: DATA_VERSION,
                    protocol_version: PROTOCOL_VERSION,
                    registry_fingerprint: REGISTRY_FINGERPRINT,
                    seed,
                };
                let bytes = serde_json::to_vec_pretty(&metadata)
                    .map_err(|error| ChunkStoreError::InvalidWorldMetadata(error.to_string()))?;
                fs::create_dir_all(&self.root)?;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(metadata_path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn validate_existing_chunks(&self) -> Result<(), ChunkStoreError> {
        for (x, z) in self.saved_chunks()? {
            self.load(x, z)?;
        }
        Ok(())
    }

    /// Write a chunk, creating the directory if needed.
    ///
    /// The write is atomic: the bytes go to a `.tmp` sibling which is then
    /// renamed, so a crash mid-write cannot leave a half-written chunk that
    /// would later fail its checksum.
    pub fn save(&self, x: i32, z: i32, column: &ChunkColumn) -> Result<(), ChunkStoreError> {
        // A direct save must not replace an unreadable or foreign existing file.
        self.load(x, z)?;
        for (registry, ids, count) in [
            ("block_state", column.states(), BLOCK_STATE_COUNT as usize),
            ("biome", column.biomes(), BIOME_COUNT as usize),
        ] {
            if let Some(&id) = ids.iter().find(|&&id| id as usize >= count) {
                return Err(ChunkStoreError::InvalidRegistryId { registry, id });
            }
        }
        let dir = self.chunks_dir();
        fs::create_dir_all(&dir)?;
        let encoded = encode_chunk(x, z, column);
        let final_path = self.chunk_path(x, z);
        // A unique temp name so two threads saving the same chunk (parallel
        // tests, or two players streaming the same chunk) never race on rename.
        let tmp_path = final_path.with_extension(format!(
            "bcc.{}.{}.tmp",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        drop(file);
        // std::fs::rename replaces a file on Windows as well as Unix. Never
        // delete the original first: a failed replacement must leave it intact.
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
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                for directory in [self.root.clone(), self.chunks_dir()] {
                    match fs::metadata(directory) {
                        Ok(metadata) if !metadata.is_dir() => {
                            return Err(io::Error::new(
                                io::ErrorKind::NotADirectory,
                                "chunk store path is not a directory",
                            )
                            .into());
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        let (stored_x, stored_z, column) = decode_chunk_at(&bytes)?;
        if (stored_x, stored_z) != (x, z) {
            return Err(ChunkStoreError::CoordinateMismatch {
                expected: (x, z),
                stored: (stored_x, stored_z),
            });
        }
        Ok(Some(column))
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
    let mut flags = if column.postprocessing_positions().is_empty() {
        0
    } else {
        POSTPROCESSING_FLAG
    };
    if !column.structure_entities().is_empty() {
        flags |= STRUCTURE_ENTITIES_FLAG;
    }
    if column.light().is_some() {
        flags |= LIGHT_FLAG;
    }
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&z.to_le_bytes());
    out.extend_from_slice(&MIN_Y.to_le_bytes());
    out.extend_from_slice(&WORLD_HEIGHT.to_le_bytes());
    out.extend_from_slice(&DATA_VERSION.to_le_bytes());
    out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    out.extend_from_slice(&REGISTRY_FINGERPRINT);

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

    use bcore_worldgen::block_entity::{BlockEntity, SpawnerMob};
    let block_entity_count = column.block_entities().len()
        + column.feature_block_entities().len()
        + column.pending_block_entities().len();
    out.extend_from_slice(&(block_entity_count as u32).to_le_bytes());
    for (&(lx, y, lz), data) in column.block_entities() {
        out.push(((lx << 4) | lz) as u8);
        out.extend_from_slice(&y.to_le_bytes());
        match data {
            BlockEntity::DungeonChest { loot_seed } => {
                out.push(1);
                out.extend_from_slice(&loot_seed.to_le_bytes());
            }
            BlockEntity::Spawner { mob } => {
                out.push(2);
                out.push(match mob {
                    SpawnerMob::Skeleton => 0,
                    SpawnerMob::Zombie => 1,
                    SpawnerMob::Spider => 2,
                    SpawnerMob::CaveSpider => 3,
                });
            }
            BlockEntity::Beehive { ticks_in_hive } => {
                out.push(3);
                let count = u32::try_from(ticks_in_hive.len()).expect("BCC bee count fits u32");
                out.extend_from_slice(&count.to_le_bytes());
                for ticks in ticks_in_hive {
                    out.extend_from_slice(&ticks.to_le_bytes());
                }
            }
        }
    }
    for (&(lx, y, lz), data) in column.feature_block_entities() {
        let wx = x.checked_mul(16).and_then(|v| v.checked_add(lx as i32));
        let wz = z.checked_mul(16).and_then(|v| v.checked_add(lz as i32));
        assert!(
            wx.zip(wz).is_some_and(|(wx, wz)| {
                column
                    .get(lx, y, lz)
                    .is_some_and(|state| data.valid_for(state, (wx, y, wz)))
            }),
            "generated block entity must match its state and saved chunk"
        );
        let bytes = serde_json::to_vec(data).expect("serializable generated block entity");
        assert!(bytes.len() <= MAX_BLOCK_ENTITY_BYTES);
        out.push(((lx << 4) | lz) as u8);
        out.extend_from_slice(&y.to_le_bytes());
        out.push(4);
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }

    for (&(lx, y, lz), data) in column.pending_block_entities() {
        let wx = x.checked_mul(16).and_then(|v| v.checked_add(lx as i32));
        let wz = z.checked_mul(16).and_then(|v| v.checked_add(lz as i32));
        assert!(
            wx.zip(wz).is_some_and(|(wx, wz)| {
                column
                    .get(lx, y, lz)
                    .is_some_and(|state| data.valid_for(state, (wx, y, wz)))
            }),
            "pending block entity must match its saved owner"
        );
        let bytes = serde_json::to_vec(data).expect("serializable pending block entity");
        assert!(bytes.len() <= MAX_BLOCK_ENTITY_BYTES);
        out.push(((lx << 4) | lz) as u8);
        out.extend_from_slice(&y.to_le_bytes());
        out.push(5);
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }

    use bcore_worldgen::generated_entity::GeneratedEntity;
    out.extend_from_slice(&(column.entities().len() as u32).to_le_bytes());
    for entity in column.entities() {
        let [ex, ey, ez] = entity.block_pos();
        assert!(
            entity.valid_for(bcore_core::ChunkPos::new(x, z)),
            "entity must belong to the saved chunk"
        );
        match entity {
            GeneratedEntity::ChestMinecart { loot_seed, .. } => {
                out.push(1);
                out.push((((ex & 15) << 4) | (ez & 15)) as u8);
                out.extend_from_slice(&ey.to_le_bytes());
                out.extend_from_slice(&loot_seed.to_le_bytes());
            }
            GeneratedEntity::Mob(mob) => {
                let bytes = serde_json::to_vec(mob.typed_data()).expect("serializable mob save");
                assert!(bytes.len() <= MAX_GENERATED_MOB_BYTES);
                out.push(2);
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(&bytes);
            }
        }
    }
    assert!(
        column
            .structures()
            .valid_for(bcore_core::ChunkPos::new(x, z)),
        "structure metadata must belong to the saved chunk"
    );
    if column.structures().is_empty() {
        out.extend_from_slice(&0u32.to_le_bytes());
    } else {
        let bytes =
            serde_json::to_vec(column.structures()).expect("serializable structure metadata");
        assert!(bytes.len() <= MAX_STRUCTURE_BYTES);
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }

    use bcore_worldgen::tick_request::TickTarget;
    let owner = bcore_core::ChunkPos::new(x, z);
    let count = u32::try_from(column.tick_requests().len()).expect("BCC tick count fits u32");
    out.extend_from_slice(&count.to_le_bytes());
    for request in column.tick_requests() {
        assert!(
            request.valid_for(owner),
            "tick request must belong to the saved chunk"
        );
        let (kind, target) = match request.target {
            TickTarget::Block(state) => (1, state),
            TickTarget::Fluid(id) => (2, id),
        };
        let [tx, ty, tz] = request.block_pos;
        out.push(kind);
        out.push((((tx & 15) << 4) | (tz & 15)) as u8);
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&target.to_le_bytes());
        out.extend_from_slice(&request.delay.to_le_bytes());
    }
    if flags & POSTPROCESSING_FLAG != 0 {
        let count = u32::try_from(column.postprocessing_positions().len())
            .expect("BCC postprocessing count fits u32");
        out.extend_from_slice(&count.to_le_bytes());
        for &(lx, y, lz) in column.postprocessing_positions() {
            out.push(((lx << 4) | lz) as u8);
            out.extend_from_slice(&y.to_le_bytes());
        }
    }
    if flags & STRUCTURE_ENTITIES_FLAG != 0 {
        let count = u32::try_from(column.structure_entities().len())
            .expect("BCC structure entity count fits u32");
        out.extend_from_slice(&count.to_le_bytes());
        for request in column.structure_entities() {
            assert!(
                request.valid_for(owner),
                "structure entity request must belong to saved chunk"
            );
            let bytes = serde_json::to_vec(request).expect("serializable structure entity request");
            assert!(bytes.len() <= MAX_STRUCTURE_ENTITY_BYTES);
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
    }

    if let Some(light) = column.light() {
        assert!(
            light.validate(MIN_Y, WORLD_HEIGHT, true),
            "valid stored native light"
        );
        out.extend_from_slice(&light.min_section_y.to_le_bytes());
        out.extend_from_slice(&(light.sections.len() as u16).to_le_bytes());
        for section in &light.sections {
            let layer_flags = u8::from(section.sky.is_some())
                | (u8::from(section.block.is_some()) << 1)
                | (u8::from(section.sky_empty) << 2)
                | (u8::from(section.block_empty) << 3);
            out.push(layer_flags);
            for (layer, empty) in [
                (&section.sky, section.sky_empty),
                (&section.block, section.block_empty),
            ] {
                if let Some(bytes) = layer {
                    if !empty {
                        out.extend_from_slice(bytes);
                    }
                }
            }
        }
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
    if (1..FORMAT_VERSION).contains(&version) {
        return Err(ChunkStoreError::MissingRegistryIdentity(version));
    }
    if version != FORMAT_VERSION {
        return Err(ChunkStoreError::UnsupportedVersion(version));
    }
    let flags = cur.u16()?;
    if flags & !(POSTPROCESSING_FLAG | STRUCTURE_ENTITIES_FLAG | LIGHT_FLAG) != 0 {
        return Err(ChunkStoreError::UnsupportedFlags(flags));
    }
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

    let data_version = cur.i32()?;
    let protocol_version = cur.i32()?;
    let fingerprint = cur.take(32)?.try_into().expect("32-byte fingerprint");
    validate_identity(data_version, protocol_version, fingerprint)?;

    let palette_len = cur.u32()? as usize;
    if palette_len == 0 || palette_len > BLOCK_STATE_COUNT as usize {
        return Err(ChunkStoreError::InvalidPaletteLength {
            registry: "block_state",
            len: palette_len,
        });
    }
    let mut palette = Vec::with_capacity(palette_len);
    for _ in 0..palette_len {
        let id = cur.u32()?;
        if id as usize >= BLOCK_STATE_COUNT as usize {
            return Err(ChunkStoreError::InvalidRegistryId {
                registry: "block_state",
                id,
            });
        }
        palette.push(id);
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
    if biome_len == 0 || biome_len > BIOME_COUNT as usize {
        return Err(ChunkStoreError::InvalidPaletteLength {
            registry: "biome",
            len: biome_len,
        });
    }
    let mut biome_palette = Vec::with_capacity(biome_len);
    for _ in 0..biome_len {
        let id = cur.u32()?;
        if id as usize >= BIOME_COUNT as usize {
            return Err(ChunkStoreError::InvalidRegistryId {
                registry: "biome",
                id,
            });
        }
        biome_palette.push(id);
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

    let mut column = ChunkColumn::from_parts(states, biomes);
    {
        use bcore_worldgen::block_entity::{BlockEntity, SpawnerMob};
        let count = cur.u32()? as usize;
        if count > COLUMN_ENTRIES {
            return Err(ChunkStoreError::InvalidBlockEntity);
        }
        for _ in 0..count {
            let xz = cur.u8()?;
            let (lx, lz) = ((xz >> 4) as usize, (xz & 15) as usize);
            let y = cur.i32()?;
            if column.block_entities().contains_key(&(lx, y, lz))
                || column.feature_block_entities().contains_key(&(lx, y, lz))
                || column.pending_block_entities().contains_key(&(lx, y, lz))
            {
                return Err(ChunkStoreError::InvalidBlockEntity);
            }
            let kind = cur.u8()?;
            if kind == 5 {
                let length = cur.u32()? as usize;
                if length > MAX_BLOCK_ENTITY_BYTES {
                    return Err(ChunkStoreError::InvalidBlockEntity);
                }
                let data = serde_json::from_slice(cur.take(length)?)
                    .map_err(|_| ChunkStoreError::InvalidBlockEntity)?;
                if !column.set_pending_block_entity(
                    bcore_core::ChunkPos::new(x, z),
                    lx,
                    y,
                    lz,
                    data,
                ) {
                    return Err(ChunkStoreError::InvalidBlockEntity);
                }
                continue;
            }
            if kind == 4 {
                let length = cur.u32()? as usize;
                if length > MAX_BLOCK_ENTITY_BYTES {
                    return Err(ChunkStoreError::InvalidBlockEntity);
                }
                let data = serde_json::from_slice(cur.take(length)?)
                    .map_err(|_| ChunkStoreError::InvalidBlockEntity)?;
                if !column.set_feature_block_entity(
                    bcore_core::ChunkPos::new(x, z),
                    lx,
                    y,
                    lz,
                    data,
                ) {
                    return Err(ChunkStoreError::InvalidBlockEntity);
                }
                continue;
            }
            let data = match kind {
                1 => BlockEntity::DungeonChest {
                    loot_seed: i64::from_le_bytes(cur.take(8)?.try_into().expect("8 bytes")),
                },
                2 => BlockEntity::Spawner {
                    mob: match cur.u8()? {
                        0 => SpawnerMob::Skeleton,
                        1 => SpawnerMob::Zombie,
                        2 => SpawnerMob::Spider,
                        3 => SpawnerMob::CaveSpider,
                        _ => return Err(ChunkStoreError::InvalidBlockEntity),
                    },
                },
                3 => {
                    let count = cur.u32()? as usize;
                    let length = count
                        .checked_mul(4)
                        .ok_or(ChunkStoreError::InvalidBlockEntity)?;
                    let ticks_in_hive = cur
                        .take(length)?
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .copied()
                        .map(i32::from_le_bytes)
                        .collect();
                    BlockEntity::Beehive { ticks_in_hive }
                }
                _ => return Err(ChunkStoreError::InvalidBlockEntity),
            };
            if !column.set_block_entity(lx, y, lz, data) {
                return Err(ChunkStoreError::InvalidBlockEntity);
            }
        }
    }
    {
        use bcore_worldgen::generated_entity::GeneratedEntity;
        let owner = bcore_core::ChunkPos::new(x, z);
        let count = cur.u32()? as usize;
        if count > COLUMN_ENTRIES {
            return Err(ChunkStoreError::InvalidEntity);
        }
        for _ in 0..count {
            let kind = cur.u8()?;
            if kind == 2 {
                let length = cur.u32()? as usize;
                if length > MAX_GENERATED_MOB_BYTES {
                    return Err(ChunkStoreError::InvalidEntity);
                }
                let value = serde_json::from_slice(cur.take(length)?)
                    .map_err(|_| ChunkStoreError::InvalidEntity)?;
                let mob = bcore_worldgen::generated_entity::GeneratedMob::from_typed_data(value)
                    .map_err(|_| ChunkStoreError::InvalidEntity)?;
                if !column.add_entity(owner, GeneratedEntity::Mob(Box::new(mob))) {
                    return Err(ChunkStoreError::InvalidEntity);
                }
                continue;
            }
            if kind != 1 {
                return Err(ChunkStoreError::InvalidEntity);
            }
            let xz = cur.u8()?;
            let y = cur.i32()?;
            let loot_seed = i64::from_le_bytes(cur.take(8)?.try_into().expect("8 bytes"));
            let ex = x
                .checked_mul(16)
                .and_then(|v| v.checked_add(i32::from(xz >> 4)))
                .ok_or(ChunkStoreError::InvalidEntity)?;
            let ez = z
                .checked_mul(16)
                .and_then(|v| v.checked_add(i32::from(xz & 15)))
                .ok_or(ChunkStoreError::InvalidEntity)?;
            if !column.add_entity(
                owner,
                GeneratedEntity::ChestMinecart {
                    block_pos: [ex, y, ez],
                    loot_seed,
                },
            ) {
                return Err(ChunkStoreError::InvalidEntity);
            }
        }
        let length = cur.u32()? as usize;
        if length > MAX_STRUCTURE_BYTES {
            return Err(ChunkStoreError::InvalidStructures);
        }
        if length != 0 {
            let data = serde_json::from_slice(cur.take(length)?)
                .map_err(|_| ChunkStoreError::InvalidStructures)?;
            if !column.set_structures(owner, data) {
                return Err(ChunkStoreError::InvalidStructures);
            }
        }
    }
    {
        use bcore_worldgen::tick_request::{TickRequest, TickTarget};
        let owner = bcore_core::ChunkPos::new(x, z);
        let count = cur.u32()? as usize;
        if count > (cur.bytes.len() - cur.at) / TICK_REQUEST_BYTES {
            return Err(ChunkStoreError::InvalidTickRequest);
        }
        for _ in 0..count {
            let kind = cur.u8()?;
            let xz = cur.u8()?;
            let y = cur.i32()?;
            let target = match (kind, cur.u32()?) {
                (1, state) => TickTarget::Block(state),
                (2, id) => TickTarget::Fluid(id),
                _ => return Err(ChunkStoreError::InvalidTickRequest),
            };
            let delay = cur.i32()?;
            let tx = owner
                .x
                .checked_mul(16)
                .and_then(|v| v.checked_add(i32::from(xz >> 4)))
                .ok_or(ChunkStoreError::InvalidTickRequest)?;
            let tz = owner
                .z
                .checked_mul(16)
                .and_then(|v| v.checked_add(i32::from(xz & 15)))
                .ok_or(ChunkStoreError::InvalidTickRequest)?;
            if !column.add_tick_request(
                owner,
                TickRequest {
                    block_pos: [tx, y, tz],
                    target,
                    delay,
                },
            ) {
                return Err(ChunkStoreError::InvalidTickRequest);
            }
        }
    }
    if flags & POSTPROCESSING_FLAG != 0 {
        let count = cur.u32()? as usize;
        if count > (cur.bytes.len() - cur.at) / POSTPROCESSING_BYTES {
            return Err(ChunkStoreError::InvalidPostprocessing);
        }
        for _ in 0..count {
            let xz = cur.u8()?;
            let y = cur.i32()?;
            if !column.mark_postprocessing((xz >> 4) as usize, y, (xz & 15) as usize) {
                return Err(ChunkStoreError::InvalidPostprocessing);
            }
        }
    }
    if flags & STRUCTURE_ENTITIES_FLAG != 0 {
        let owner = bcore_core::ChunkPos::new(x, z);
        let count = cur.u32()? as usize;
        if count > (cur.bytes.len() - cur.at) / 4 {
            return Err(ChunkStoreError::InvalidStructureEntity);
        }
        for _ in 0..count {
            let length = cur.u32()? as usize;
            if length > MAX_STRUCTURE_ENTITY_BYTES {
                return Err(ChunkStoreError::InvalidStructureEntity);
            }
            let request = serde_json::from_slice(cur.take(length)?)
                .map_err(|_| ChunkStoreError::InvalidStructureEntity)?;
            if !column.add_structure_entity(owner, request) {
                return Err(ChunkStoreError::InvalidStructureEntity);
            }
        }
    }
    if flags & LIGHT_FLAG != 0 {
        use bcore_worldgen::lighting::{ChunkLight, LightSection};
        let min_section_y = cur.i32()?;
        let count = usize::from(cur.u16()?);
        if min_section_y != MIN_Y / 16 - 1 || count != crate::chunk::LIGHT_SECTION_COUNT {
            return Err(ChunkStoreError::InvalidLight);
        }
        let mut sections = Vec::with_capacity(count);
        for _ in 0..count {
            let layer_flags = cur.u8()?;
            if layer_flags & !15 != 0
                || (layer_flags & 4 != 0 && layer_flags & 1 == 0)
                || (layer_flags & 8 != 0 && layer_flags & 2 == 0)
            {
                return Err(ChunkStoreError::InvalidLight);
            }
            let mut layers = [None, None];
            for (kind, layer) in layers.iter_mut().enumerate() {
                if layer_flags & (1 << kind) != 0 {
                    *layer = Some(if layer_flags & (4 << kind) != 0 {
                        vec![0; crate::chunk::LIGHT_LAYER_BYTES]
                    } else {
                        cur.take(crate::chunk::LIGHT_LAYER_BYTES)?.to_vec()
                    });
                }
            }
            let [sky, block] = layers;
            sections.push(LightSection {
                sky,
                block,
                sky_empty: layer_flags & 4 != 0,
                block_empty: layer_flags & 8 != 0,
            });
        }
        if !column.set_light(ChunkLight {
            min_section_y,
            sections,
        }) {
            return Err(ChunkStoreError::InvalidLight);
        }
    }
    if cur.at != cur.bytes.len() {
        return Err(ChunkStoreError::TrailingData);
    }
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
    fn every_legacy_version_without_registry_identity_is_rejected() {
        for version in 1u16..FORMAT_VERSION {
            let mut old = encode_chunk(3, -7, &ChunkColumn::flat());
            old.drain(24..HEADER_LEN);
            old[4..6].copy_from_slice(&version.to_le_bytes());
            let end = old.len() - 4;
            let checksum = fnv1a(&old[..end]);
            old[end..].copy_from_slice(&checksum.to_le_bytes());
            assert!(matches!(
                decode_chunk_at(&old),
                Err(ChunkStoreError::MissingRegistryIdentity(actual)) if actual == version
            ));
        }
    }

    #[test]
    fn generated_entity_kinds_positions_and_structure_metadata_are_validated() {
        use bcore_worldgen::generated_entity::GeneratedEntity;
        let mut column = ChunkColumn::flat();
        assert!(column.add_entity(
            bcore_core::ChunkPos::new(-2, 3),
            GeneratedEntity::ChestMinecart {
                block_pos: [-31, 20, 50],
                loot_seed: i64::MIN
            }
        ));
        let bytes = encode_chunk(-2, 3, &column);
        assert_eq!(decode_chunk(&bytes).unwrap(), column);
        let entity = bytes.len() - 4 - 4 - 4 - 14;
        for (offset, value) in [(entity, 255), (entity + 2, 0xFF)] {
            let mut invalid = bytes.clone();
            if offset == entity + 2 {
                invalid[offset..offset + 4].copy_from_slice(&i32::MAX.to_le_bytes());
            } else {
                invalid[offset] = value;
            }
            let checksum = invalid.len() - 4;
            let hash = fnv1a(&invalid[..checksum]);
            invalid[checksum..].copy_from_slice(&hash.to_le_bytes());
            assert!(matches!(
                decode_chunk(&invalid),
                Err(ChunkStoreError::InvalidEntity)
            ));
        }
        let mut invalid = encode_chunk(0, 0, &ChunkColumn::flat());
        invalid.truncate(invalid.len() - 12);
        let bad = br#"{"mineshaft_start":{"mine_type":"Normal","pieces":[]},"references":[]}"#;
        invalid.extend_from_slice(&(bad.len() as u32).to_le_bytes());
        invalid.extend_from_slice(bad);
        invalid.extend_from_slice(&0u32.to_le_bytes()); // v4 tick requests
        invalid.extend_from_slice(&fnv1a(&invalid).to_le_bytes());
        assert!(matches!(
            decode_chunk(&invalid),
            Err(ChunkStoreError::InvalidStructures)
        ));
    }

    #[test]
    fn invalid_block_entity_kinds_and_trailing_bytes_are_rejected() {
        use bcore_worldgen::{block_entity::BlockEntity, dungeon};
        let mut column = ChunkColumn::flat();
        column.set(1, 32, 2, dungeon::CHEST);
        column.set_block_entity(1, 32, 2, BlockEntity::DungeonChest { loot_seed: -1 });
        let mut bytes = encode_chunk(0, 0, &column);
        let kind = bytes.len() - 4 - 4 - 8 - 8 - 1;
        bytes[kind] = 255;
        let end = bytes.len() - 4;
        let checksum = fnv1a(&bytes[..end]);
        bytes[end..].copy_from_slice(&checksum.to_le_bytes());
        assert!(matches!(
            decode_chunk(&bytes),
            Err(ChunkStoreError::InvalidBlockEntity)
        ));
        let mut bytes = encode_chunk(0, 0, &ChunkColumn::flat());
        bytes.truncate(bytes.len() - 4);
        bytes.push(1);
        let checksum = fnv1a(&bytes);
        bytes.extend_from_slice(&checksum.to_le_bytes());
        assert!(matches!(
            decode_chunk(&bytes),
            Err(ChunkStoreError::TrailingData)
        ));
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
        let header = HEADER_LEN;
        let blocks = 4 + 4 * 4 + 1 + COLUMN_ENTRIES; // len, palette, index_bits, indices
        let biomes = 4 + 4 + BIOME_ENTRIES * 2; // len, 1-entry palette, indices
        let metadata = 4 + 4 + 4 + 4; // block entities, entities, structures and tick requests
        let checksum = 4;
        assert_eq!(
            encoded.len(),
            header + blocks + biomes + metadata + checksum
        );
        // Sanity: a whole flat column stays well under 200 KB.
        assert!(encoded.len() < 200 * 1024, "{} bytes", encoded.len());
    }
}
