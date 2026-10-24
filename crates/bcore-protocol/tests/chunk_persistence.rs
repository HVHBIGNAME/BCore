//! Chunk persistence: `world/chunks/` save → load round trips.
//!
//! BCC encoding is inert; the retained world's explicit level-save boundary can
//! materialize pending block entities first. Comparisons use the same generation
//! history and save boundary, including typed pending/materialized data.
//!
//! They also cover the failure modes that matter for a world directory that
//! outlives a server process: corruption, truncation, registry/seed identity
//! and a directory that cannot be read or written.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bcore_protocol::chunk::{block_state, ChunkColumn, MIN_Y};
use bcore_protocol::chunk_store::{
    decode_chunk_at, encode_chunk, ChunkStore, ChunkStoreError, FORMAT_VERSION, MAGIC,
};
use bcore_protocol::shared::{open_shared_server, ServerState};
use bcore_protocol::world_state::{ChunkOrigin, World, WorldError};
use bcore_registry::{DATA_VERSION, PROTOCOL_VERSION, REGISTRY_FINGERPRINT};

const SEED: i64 = 0x0BC0_0E00_1234_5678u64 as i64;

/// A fresh owned scratch directory per test.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let identity = bcore_protocol::shared::new_shared_server();
        let suffix: String = identity
            .session_id()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let parent = std::env::temp_dir().join("opencode");
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!(
            "bcore-persist-{tag}-{}-{}-{suffix}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("new isolated scratch directory");
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
fn a_loaded_chunk_matches_the_same_explicit_level_save_boundary() {
    let scratch = Scratch::new("indistinguishable");
    let store = scratch.store();
    let persistent = World::with_store(SEED, store.clone());
    let ephemeral = World::in_memory(SEED);

    for &(x, z) in &[(0, 0), (11, 6), (-3, -8)] {
        // Force the save.
        let (_, origin) = persistent.chunk(x, z);
        assert_eq!(origin, ChunkOrigin::Generated);
        // Compare against the in-memory world following the same request order.
        let (loaded, origin) = persistent.chunk(x, z);
        assert_eq!(origin, ChunkOrigin::Loaded);
        let published = ephemeral.generate(x, z);
        let saved = ephemeral.prepare_generated_chunk_save(x, z, 0).unwrap();
        assert_eq!(
            loaded, saved,
            "disk copy of ({x}, {z}) differs after level save"
        );
        assert_eq!(saved.states(), published.states());
        assert_eq!(saved.tick_requests(), published.tick_requests());
        assert_eq!(
            saved.postprocessing_positions(),
            published.postprocessing_positions()
        );
        assert!(saved.pending_block_entities().is_empty());
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
    assert_eq!(i32::from_le_bytes(bytes[24..28].try_into().unwrap()), 4903);
    assert_eq!(i32::from_le_bytes(bytes[28..32].try_into().unwrap()), 776);
    assert_eq!(bytes[32..64], REGISTRY_FINGERPRINT);
    assert_eq!(DATA_VERSION, bcore_core::DATA_VERSION);
    assert_eq!(PROTOCOL_VERSION, bcore_core::PROTOCOL_VERSION);
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
    let persistent = World::with_store(SEED, store.clone());
    assert!(matches!(
        persistent.try_chunk(0, 0),
        Err(WorldError::Store(error)) if matches!(error.as_ref(), ChunkStoreError::ChecksumMismatch { .. })
    ));
    assert!(persistent.generation_progress(0, 0).unwrap().is_none());
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "failed load must not replace corruption"
    );
    assert!(store.save(0, 0, &ChunkColumn::flat()).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "direct save must also preserve corruption"
    );
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
fn different_seeds_cannot_reopen_an_admitted_world_directory() {
    let scratch = Scratch::new("seeds");
    let store = scratch.store();

    let world = World::open(1, store.clone()).unwrap();
    store.save(0, 0, &ChunkColumn::flat()).unwrap();
    let original = fs::read(store.chunk_path(0, 0)).unwrap();
    assert!(matches!(
        World::open(2, store.clone()),
        Err(ChunkStoreError::SeedMismatch {
            stored: 1,
            requested: 2
        })
    ));
    assert_eq!(fs::read(store.chunk_path(0, 0)).unwrap(), original);
    assert_eq!(world.seed(), 1);
    let reopened = World::open(1, store).unwrap();
    assert_eq!(reopened.try_chunk(0, 0).unwrap().1, ChunkOrigin::Loaded);
}

#[test]
fn an_unreadable_chunk_directory_is_not_treated_as_an_empty_world() {
    let scratch = Scratch::new("unwritable");
    fs::create_dir_all(&scratch.path).expect("scratch dir");
    let blocker = scratch.path.join("chunks");
    fs::write(&blocker, b"not a directory").expect("write blocker");

    let world = World::with_store(SEED, ChunkStore::at(&scratch.path));
    assert!(matches!(world.try_chunk(0, 0), Err(WorldError::Store(_))));
    assert!(world.generation_progress(0, 0).unwrap().is_none());
    assert_eq!(fs::read(&blocker).unwrap(), b"not a directory");
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

fn repair_checksum(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let checksum = bytes[..end].iter().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    bytes[end..].copy_from_slice(&checksum.to_le_bytes());
}

fn assert_refused_without_overwrite(
    scratch: &Scratch,
    bytes: &[u8],
    expected: impl Fn(&ChunkStoreError) -> bool,
) {
    let store = scratch.store();
    store.prepare_world(SEED).unwrap();
    fs::create_dir_all(store.chunks_dir()).unwrap();
    let path = store.chunk_path(0, 0);
    fs::write(&path, bytes).unwrap();
    let metadata = fs::read(scratch.path.join("bcore-world.json")).unwrap();
    let world = World::with_store(SEED, store.clone());
    let error = match world.try_chunk(0, 0) {
        Err(WorldError::Store(error)) => error,
        result => panic!("store rejection was lost: {result:?}"),
    };
    assert!(expected(error.as_ref()), "unexpected error: {error}");
    assert!(world.generation_progress(0, 0).unwrap().is_none());
    assert_eq!(world.cached_payloads(), 0);
    let error = store.save(0, 0, &ChunkColumn::flat()).unwrap_err();
    assert!(expected(&error), "direct save returned {error}");
    let error = World::open(SEED, store).unwrap_err();
    assert!(expected(&error), "startup returned {error}");
    assert_eq!(fs::read(path).unwrap(), bytes, "refused file changed");
    assert_eq!(
        fs::read(scratch.path.join("bcore-world.json")).unwrap(),
        metadata
    );
}

#[test]
fn pre_v6_files_without_registry_identity_require_explicit_migration() {
    for version in 1u16..6 {
        let scratch = Scratch::new("old-registry");
        let mut bytes = encode_chunk(0, 0, &ChunkColumn::flat());
        bytes.drain(24..64);
        bytes[4..6].copy_from_slice(&version.to_le_bytes());
        repair_checksum(&mut bytes);
        assert_refused_without_overwrite(
            &scratch,
            &bytes,
            |error| matches!(error, ChunkStoreError::MissingRegistryIdentity(actual) if *actual == version),
        );
    }
}

#[test]
fn wrong_minecraft_data_or_protocol_identity_is_never_overwritten() {
    for (offset, wrong) in [(24, 4786i32), (28, 775i32)] {
        let scratch = Scratch::new("wrong-version");
        let mut bytes = encode_chunk(0, 0, &ChunkColumn::flat());
        bytes[offset..offset + 4].copy_from_slice(&wrong.to_le_bytes());
        repair_checksum(&mut bytes);
        assert_refused_without_overwrite(&scratch, &bytes, |error| {
            matches!(error, ChunkStoreError::MinecraftVersionMismatch { .. })
        });
    }
}

#[test]
fn wrong_registry_fingerprint_is_never_overwritten() {
    let scratch = Scratch::new("wrong-fingerprint");
    let mut bytes = encode_chunk(0, 0, &ChunkColumn::flat());
    bytes[32] ^= 1;
    repair_checksum(&mut bytes);
    assert_refused_without_overwrite(&scratch, &bytes, |error| {
        matches!(error, ChunkStoreError::RegistryMismatch { .. })
    });
}

#[test]
fn out_of_registry_palette_ids_are_rejected_before_loading_the_column() {
    let scratch = Scratch::new("invalid-state");
    let mut bytes = encode_chunk(0, 0, &ChunkColumn::flat());
    // The first block palette entry follows the 64-byte header and u32 count.
    bytes[68..72].copy_from_slice(&(bcore_registry::BLOCK_STATE_COUNT as u32).to_le_bytes());
    repair_checksum(&mut bytes);
    assert_refused_without_overwrite(&scratch, &bytes, |error| {
        matches!(
            error,
            ChunkStoreError::InvalidRegistryId {
                registry: "block_state",
                ..
            }
        )
    });
}

#[test]
fn generation_only_errors_retain_their_original_type() {
    let world = World::in_memory(SEED);
    assert!(matches!(
        world.try_generate(i32::MAX, 0),
        Err(bcore_worldgen::GenerationError::CoordinateOutOfRange(_))
    ));
    assert!(matches!(
        world.try_chunk(i32::MAX, 0),
        Err(WorldError::Generation(
            bcore_worldgen::GenerationError::CoordinateOutOfRange(_)
        ))
    ));
    assert_eq!(world.cached_payloads(), 0);
}

#[test]
fn corrupt_registry_bound_chunk_is_rejected_by_loading_saving_and_startup() {
    let scratch = Scratch::new("corrupt-v6");
    let mut bytes = encode_chunk(0, 0, &ChunkColumn::flat());
    let middle = bytes.len() / 2;
    bytes[middle] ^= 1;
    assert_refused_without_overwrite(&scratch, &bytes, |error| {
        matches!(error, ChunkStoreError::ChecksumMismatch { .. })
    });
}

#[test]
fn chunk_coordinates_must_agree_with_the_store_path() {
    let scratch = Scratch::new("wrong-coordinates");
    let bytes = encode_chunk(1, 0, &ChunkColumn::flat());
    assert_refused_without_overwrite(&scratch, &bytes, |error| {
        matches!(
            error,
            ChunkStoreError::CoordinateMismatch {
                expected: (0, 0),
                stored: (1, 0),
            }
        )
    });
}

#[test]
fn startup_refuses_unidentified_worlds_without_relabeling_files_or_operators() {
    let scratch = Scratch::new("existing-world");
    let operators = scratch.path.join("ops.json");
    fs::write(&operators, b"[\"ExistingOperator\"]\n").unwrap();
    let store = scratch.store();
    fs::create_dir(store.chunks_dir()).unwrap();
    let path = store.chunk_path(0, 0);
    let original = encode_chunk(0, 0, &ChunkColumn::flat());
    fs::write(&path, &original).unwrap();
    assert!(matches!(
        open_shared_server(&scratch.path, SEED),
        Err(ChunkStoreError::MissingWorldMetadata)
    ));
    assert_eq!(fs::read(path).unwrap(), original);
    assert_eq!(fs::read(operators).unwrap(), b"[\"ExistingOperator\"]\n");
    assert!(!scratch.path.join("bcore-world.json").exists());
}

#[test]
fn admitted_servers_keep_operator_files_inside_their_explicit_world_root() {
    let scratch = Scratch::new("operator-root");
    let old_operators = scratch.path.join("ops.json");
    fs::write(&old_operators, b"[\"ExistingOperator\"]\n").unwrap();
    let root = scratch.path.join("fresh-26.2");
    let server = open_shared_server(&root, SEED).unwrap();
    assert!(server.world().store().unwrap().root().is_absolute());
    server.add_op("MigrationOperator");
    server.save_ops();
    assert_eq!(
        fs::read(&old_operators).unwrap(),
        b"[\"ExistingOperator\"]\n"
    );
    let operators: Vec<String> =
        serde_json::from_slice(&fs::read(root.join("ops.json")).unwrap()).unwrap();
    assert_eq!(operators, ["MigrationOperator"]);
    let reopened = open_shared_server(&root, SEED).unwrap();
    assert!(reopened.is_op("MigrationOperator"));
    assert!(!reopened.is_op("ExistingOperator"));
}

#[test]
fn server_startup_gate_rejects_foreign_chunks_before_accept_tick_or_queue_work() {
    let scratch = Scratch::new("server-gate");
    let store = scratch.store();
    store.prepare_world(SEED).unwrap();
    store.save(0, 0, &ChunkColumn::flat()).unwrap();
    let path = store.chunk_path(0, 0);
    let mut original = fs::read(&path).unwrap();
    original[32] ^= 1;
    repair_checksum(&mut original);
    fs::write(&path, &original).unwrap();

    let state = Arc::new(ServerState::with_world(World::with_store(SEED, store)));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    bcore_protocol::server::run_with_state(listener, Arc::clone(&state));
    assert!(state.is_shutting_down());
    assert_eq!(state.player_count(), 0);
    assert_eq!(state.clock_snapshot().game_time, 0);
    assert_eq!(state.world().pending_payloads(), 0);
    assert!(state.world().generation_progress(0, 0).unwrap().is_none());
    assert_eq!(fs::read(path).unwrap(), original);
}
