//! Chunk persistence: `world/chunks/` save → load round trips.
//!
//! The load path is only safe if it is *indistinguishable* from generation, so
//! these tests assert the strong property directly: for the same seed, a chunk
//! read back from disk is byte-for-byte the same column — and encodes to the same
//! `map_chunk` payload — as one freshly generated.
//!
//! They also cover the failure modes that matter for a world directory that
//! outlives a server process: corruption, truncation, cross-seed contamination
//! and a directory that cannot be written.

use std::fs;
use std::path::PathBuf;

use bcore_protocol::chunk::{block_state, ChunkColumn, MIN_Y};
use bcore_protocol::chunk_store::{
    decode_chunk_at, encode_chunk, ChunkStore, ChunkStoreError, FORMAT_VERSION, MAGIC,
};
use bcore_protocol::world_state::{ChunkOrigin, World};

const SEED: i64 = 0x0BC0_0E00_1234_5678u64 as i64;

/// A unique scratch directory per test, removed on the way in and out.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "bcore-persist-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        Self { path }
    }

    fn store(&self) -> ChunkStore {
        ChunkStore::at(&self.path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn save_then_load_returns_byte_identical_blocks() {
    let scratch = Scratch::new("roundtrip");
    let store = scratch.store();
    let world = World::in_memory(SEED);

    for &(x, z) in &[(0, 0), (7, -13), (-40, 22)] {
        let original = world.generate(x, z);
        store.save(x, z, &original).expect("save");

        let loaded = store.load(x, z).expect("load").expect("chunk present");
        assert_eq!(
            loaded, original,
            "chunk ({x}, {z}) changed on the way to disk"
        );

        // The wire payload must also be identical, which is what players receive.
        assert_eq!(
            loaded.encode_payload(x, z),
            original.encode_payload(x, z),
            "chunk ({x}, {z}) encodes differently after a round trip"
        );
    }
}

#[test]
fn a_missing_chunk_is_none_not_an_error() {
    let scratch = Scratch::new("missing");
    let store = scratch.store();
    assert_eq!(store.load(5, 5).expect("load must not error"), None);
    assert!(!store.contains(5, 5));
    // An absent directory lists as empty rather than failing.
    assert_eq!(store.saved_chunks().expect("list"), Vec::new());
}

#[test]
fn the_world_generates_once_then_serves_from_disk() {
    let scratch = Scratch::new("world-origin");
    let store = scratch.store();
    let world = World::with_store(SEED, store.clone());

    let (first, origin) = world.chunk(3, -4);
    assert_eq!(origin, ChunkOrigin::Generated, "first visit generates");
    assert!(store.contains(3, -4), "generation must persist the chunk");

    let (second, origin) = world.chunk(3, -4);
    assert_eq!(origin, ChunkOrigin::Loaded, "second visit reads the disk");
    assert_eq!(first, second);

    // A brand-new World over the same directory still loads rather than regenerates.
    let reopened = World::with_store(SEED, store.clone());
    let (third, origin) = reopened.chunk(3, -4);
    assert_eq!(
        origin,
        ChunkOrigin::Loaded,
        "a restart must reuse the world"
    );
    assert_eq!(third, first, "world survived a simulated restart");
}

#[test]
fn a_loaded_chunk_is_indistinguishable_from_a_generated_one() {
    let scratch = Scratch::new("indistinguishable");
    let store = scratch.store();
    let persistent = World::with_store(SEED, store.clone());
    let ephemeral = World::in_memory(SEED);

    for &(x, z) in &[(0, 0), (11, 6), (-3, -8)] {
        // Force the save.
        let (_, origin) = persistent.chunk(x, z);
        assert_eq!(origin, ChunkOrigin::Generated);
        // Now read it back and compare against pure generation.
        let (loaded, origin) = persistent.chunk(x, z);
        assert_eq!(origin, ChunkOrigin::Loaded);
        assert_eq!(
            loaded,
            ephemeral.generate(x, z),
            "disk copy of ({x}, {z}) differs from a fresh generation"
        );
    }
}

#[test]
fn saved_chunks_are_listed_sorted_and_deterministically() {
    let scratch = Scratch::new("listing");
    let store = scratch.store();
    let column = ChunkColumn::flat();

    let coords = [(3, 1), (-2, 9), (0, 0), (-2, -5), (3, -1)];
    for &(x, z) in &coords {
        store.save(x, z, &column).expect("save");
    }

    let listed = store.saved_chunks().expect("list");
    assert_eq!(
        listed,
        vec![(-2, -5), (-2, 9), (0, 0), (3, -1), (3, 1)],
        "listing must be sorted, not directory order"
    );
    // Repeated calls agree: no HashMap iteration leaking into the output.
    assert_eq!(store.saved_chunks().expect("list"), listed);
}

#[test]
fn edits_to_a_chunk_survive_a_save_and_reload() {
    let scratch = Scratch::new("edits");
    let store = scratch.store();
    let world = World::in_memory(SEED);

    let mut column = world.generate(1, 1);
    // Carve a recognisable marker into the column.
    for y in 100..110 {
        column.set(4, y, 6, block_state::DIAMOND_ORE);
    }
    column.set(0, 200, 0, block_state::OAK_LOG);
    store.save(1, 1, &column).expect("save");

    let loaded = store.load(1, 1).expect("load").expect("present");
    assert_eq!(loaded, column);
    for y in 100..110 {
        assert_eq!(loaded.get(4, y, 6), Some(block_state::DIAMOND_ORE));
    }
    assert_eq!(loaded.get(0, 200, 0), Some(block_state::OAK_LOG));
    // The edit really did differ from pure generation, so the test has teeth.
    assert_ne!(loaded, world.generate(1, 1));
}

#[test]
fn overwriting_a_chunk_leaves_no_temp_file_behind() {
    let scratch = Scratch::new("overwrite");
    let store = scratch.store();
    let world = World::in_memory(SEED);

    let first = world.generate(2, 2);
    store.save(2, 2, &first).expect("first save");

    let mut second = first.clone();
    second.set(8, 90, 8, block_state::BEDROCK);
    store.save(2, 2, &second).expect("second save");

    assert_eq!(store.load(2, 2).expect("load").expect("present"), second);
    // Exactly one file for this chunk: no `.bcc.tmp` residue.
    let entries: Vec<_> = fs::read_dir(store.chunks_dir())
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(entries, vec!["c.2.2.bcc".to_string()]);
}

#[test]
fn the_encoded_file_is_stable_across_runs() {
    let world = World::in_memory(SEED);
    let column = world.generate(-6, 15);
    // Same column, same bytes: the on-disk form must be canonical, or a
    // save-load-save cycle would churn the world directory.
    assert_eq!(encode_chunk(-6, 15, &column), encode_chunk(-6, 15, &column));
    // Re-encoding a loaded column reproduces the original file exactly.
    let bytes = encode_chunk(-6, 15, &column);
    let (x, z, loaded) = decode_chunk_at(&bytes).expect("decode");
    assert_eq!((x, z), (-6, 15));
    assert_eq!(
        encode_chunk(x, z, &loaded),
        bytes,
        "encoding is not canonical"
    );
}

#[test]
fn a_chunk_file_starts_with_the_documented_header() {
    let world = World::in_memory(SEED);
    let bytes = encode_chunk(9, -9, &world.generate(9, -9));
    assert_eq!(&bytes[0..4], MAGIC, "magic must be BCC1");
    assert_eq!(
        u16::from_le_bytes(bytes[4..6].try_into().expect("version")),
        FORMAT_VERSION
    );
    assert_eq!(
        i32::from_le_bytes(bytes[8..12].try_into().expect("x")),
        9,
        "chunk x is in the header"
    );
    assert_eq!(
        i32::from_le_bytes(bytes[12..16].try_into().expect("z")),
        -9,
        "chunk z is in the header"
    );
    assert_eq!(
        i32::from_le_bytes(bytes[16..20].try_into().expect("min_y")),
        MIN_Y,
        "world geometry is recorded so a height change is detected"
    );
}

#[test]
fn corruption_is_reported_rather_than_silently_regenerated() {
    let scratch = Scratch::new("corruption");
    let store = scratch.store();
    let world = World::in_memory(SEED);
    store.save(0, 0, &world.generate(0, 0)).expect("save");

    // Flip a byte in the middle of the file.
    let path = store.chunk_path(0, 0);
    let mut bytes = fs::read(&path).expect("read");
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    fs::write(&path, &bytes).expect("write");

    match store.load(0, 0) {
        Err(ChunkStoreError::ChecksumMismatch { .. }) => {}
        other => panic!("corruption must be an error, got {other:?}"),
    }
}

#[test]
fn a_truncated_file_is_an_error_not_a_panic() {
    let scratch = Scratch::new("truncated");
    let store = scratch.store();
    let world = World::in_memory(SEED);
    store.save(1, 0, &world.generate(1, 0)).expect("save");

    let path = store.chunk_path(1, 0);
    let bytes = fs::read(&path).expect("read");
    for fraction in [0usize, 4, 16, 100, bytes.len() / 3, bytes.len() - 1] {
        fs::write(&path, &bytes[..fraction]).expect("write");
        assert!(
            store.load(1, 0).is_err(),
            "a {fraction}-byte file must not load"
        );
    }
}

#[test]
fn a_foreign_file_in_the_chunks_directory_is_ignored_by_listing() {
    let scratch = Scratch::new("foreign");
    let store = scratch.store();
    store.save(0, 0, &ChunkColumn::flat()).expect("save");

    fs::write(store.chunks_dir().join("notes.txt"), b"hello").expect("write");
    fs::write(store.chunks_dir().join("c.bad.bcc"), b"junk").expect("write");

    assert_eq!(
        store.saved_chunks().expect("list"),
        vec![(0, 0)],
        "only well-formed chunk names are listed"
    );
}

#[test]
fn different_seeds_do_not_share_a_world_directory() {
    let scratch = Scratch::new("seeds");
    let store = scratch.store();

    // Seed A generates and saves.
    let a = World::with_store(1, store.clone());
    let (chunk_a, origin) = a.chunk(0, 0);
    assert_eq!(origin, ChunkOrigin::Generated);

    // Seed B pointed at the same directory *loads A's chunk*: the store is keyed
    // by position only. This documents the current, intentional behaviour — the
    // world directory belongs to one seed, exactly like a vanilla save.
    let b = World::with_store(2, store.clone());
    let (chunk_b, origin) = b.chunk(0, 0);
    assert_eq!(origin, ChunkOrigin::Loaded);
    assert_eq!(chunk_b, chunk_a, "the saved world wins over the new seed");

    // Whereas without a shared store the two seeds really do differ.
    assert_ne!(
        World::in_memory(1).generate(0, 0),
        World::in_memory(2).generate(0, 0),
        "different seeds must generate different terrain"
    );
}

#[test]
fn an_unwritable_world_directory_degrades_to_generation() {
    // Point the store at a path that cannot become a directory (a file), so
    // `create_dir_all` fails. Players must still get their chunks.
    let scratch = Scratch::new("unwritable");
    fs::create_dir_all(&scratch.path).expect("scratch dir");
    let blocker = scratch.path.join("chunks");
    fs::write(&blocker, b"not a directory").expect("write blocker");

    let world = World::with_store(SEED, ChunkStore::at(&scratch.path));
    let (column, origin) = world.chunk(0, 0);
    assert_eq!(origin, ChunkOrigin::Generated);
    // And the blocks are the real generated terrain, not an empty column.
    assert_eq!(column, World::in_memory(SEED).generate(0, 0));
    // Streaming still works.
    assert!(world.chunk_payload(0, 0).len() > 1000);
}

#[test]
fn a_full_view_worth_of_chunks_persists_and_reloads() {
    let scratch = Scratch::new("bulk");
    let store = scratch.store();
    let world = World::with_store(SEED, store.clone());

    // A 7x7 patch: enough to cover several biomes without being slow.
    let mut expected = Vec::new();
    for cz in -3..=3 {
        for cx in -3..=3 {
            let (column, origin) = world.chunk(cx, cz);
            assert_eq!(origin, ChunkOrigin::Generated);
            expected.push(((cx, cz), column));
        }
    }
    assert_eq!(store.saved_chunks().expect("list").len(), 49);

    // Reopen and verify every chunk loads back unchanged.
    let reopened = World::with_store(SEED, store.clone());
    for ((cx, cz), original) in expected {
        let (loaded, origin) = reopened.chunk(cx, cz);
        assert_eq!(origin, ChunkOrigin::Loaded, "({cx}, {cz}) should load");
        assert_eq!(loaded, original, "({cx}, {cz}) changed on disk");
    }
}
