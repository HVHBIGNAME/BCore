//! One server's world, clock, login session, and online-player registry.
//!
//! Connection threads share an [`Arc`] of [`ServerState`]. Separately constructed
//! servers retain independent worlds, clocks, identities, and persistence roots.
//!
//! Broadcast is queue-based rather than socket-based on purpose: a player thread
//! must never write to another player's `TcpStream` (a slow or dead peer would
//! block or corrupt the writer's framing). Instead the sender appends fully
//! framed packets to the recipient's [`Outbox`], and each player's own play loop
//! drains its outbox between reads. The mutex is therefore only ever held for
//! the duration of a `Vec` push.
//!
//! Player ids are handed out by a monotonic counter, and the registry is a
//! [`BTreeMap`] so `/list` and broadcast order are deterministic.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::BuildHasher;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, RwLock};

use crate::chunk_store::{ChunkStore, ChunkStoreError};
use crate::gameplay::{encode_set_day_time, WorldClock};
use crate::world_state::{World, DEFAULT_SEED};

/// Identifier of a connected player, unique for the server's lifetime.
pub type PlayerId = u64;

/// A player's pending outbound bytes (fully framed packets).
///
/// Producers only clone/use the sender: `push` never waits on a lock. The
/// receiver is briefly serialized only while the owning connection drains it.
#[derive(Debug)]
pub struct Outbox {
    tx: Sender<Vec<u8>>,
    rx: Mutex<Receiver<Vec<u8>>>,
    queued_bytes: AtomicUsize,
}

impl Default for Outbox {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Mutex::new(rx),
            queued_bytes: AtomicUsize::new(0),
        }
    }
}

impl Outbox {
    /// Append framed packet bytes for the owning connection to flush.
    pub fn push(&self, bytes: &[u8]) {
        self.queued_bytes.fetch_add(bytes.len(), Ordering::Relaxed);
        if self.tx.send(bytes.to_vec()).is_err() {
            self.queued_bytes.fetch_sub(bytes.len(), Ordering::Relaxed);
        }
    }

    /// Take everything queued so far, leaving the outbox empty.
    pub fn drain(&self) -> Vec<u8> {
        let Ok(rx) = self.rx.lock() else {
            return Vec::new();
        };
        let mut drained = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(bytes) => {
                    self.queued_bytes.fetch_sub(bytes.len(), Ordering::Relaxed);
                    drained.extend(bytes);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        drained
    }

    /// Bytes currently queued.
    pub fn len(&self) -> usize {
        self.queued_bytes.load(Ordering::Relaxed)
    }

    /// True when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A registry entry for one connected player.
#[derive(Debug, Clone)]
pub struct PlayerHandle {
    pub id: PlayerId,
    pub name: String,
    pub uuid: [u8; 16],
    pub outbox: Arc<Outbox>,
    /// Set when the player should be disconnected (`/kick`, `/stop`).
    pub kicked: Arc<AtomicBool>,
}

impl PlayerHandle {
    /// Ask the owning connection to close after flushing.
    pub fn kick(&self) {
        self.kicked.store(true, Ordering::Relaxed);
    }

    /// True once [`PlayerHandle::kick`] has been called.
    pub fn is_kicked(&self) -> bool {
        self.kicked.load(Ordering::Relaxed)
    }
}

/// State shared by every connection thread.
#[derive(Debug)]
pub struct ServerState {
    world: World,
    session_id: [u8; 16],
    clock: Mutex<WorldClock>,
    view_distance: i32,
    state_directory: Option<PathBuf>,
    players: RwLock<BTreeMap<PlayerId, PlayerHandle>>,
    next_id: AtomicU64,
    /// Global chat message counter (`player_chat`'s `globalIndex`).
    chat_index: AtomicU64,
    /// Set by `/stop`: the accept loop and every play loop should wind down.
    shutdown: AtomicBool,
    /// Operator names, persisted only inside an explicitly admitted world root.
    ops: Mutex<BTreeSet<String>>,
    bans: Mutex<BTreeMap<String, String>>,
    pub scoreboard: Mutex<crate::scoreboard::Scoreboard>,
}

/// Cheap-to-clone handle to the shared server state.
pub type SharedServer = Arc<ServerState>;

/// JSON array of operator names inside the admitted world root.
const OPS_FILE: &str = "ops.json";

/// Create isolated in-memory state, without reading or writing working-directory files.
pub fn new_shared_server() -> SharedServer {
    Arc::new(ServerState::default())
}

/// Open a versioned world and its operator/ban files before starting any workers.
pub fn open_shared_server(
    root: impl AsRef<Path>,
    seed: i64,
) -> Result<SharedServer, ChunkStoreError> {
    let root = root.as_ref();
    let world = World::open(seed, ChunkStore::at(root))?;
    let state_directory = world
        .store()
        .expect("admitted persistent world")
        .root()
        .to_path_buf();
    let mut server = ServerState::with_world(world);
    server.state_directory = Some(state_directory);
    server.load_ops()?;
    server.load_bans()?;
    Ok(Arc::new(server))
}

fn new_session_id() -> [u8; 16] {
    static NEXT_SESSION: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    // Independent inputs to per-instance randomized hash keys supply UUID halves.
    let random = RandomState::new();
    let mut id = [0; 16];
    id[..8].copy_from_slice(
        &random
            .hash_one(("bcore-session-high", sequence))
            .to_be_bytes(),
    );
    id[8..].copy_from_slice(
        &random
            .hash_one(("bcore-session-low", sequence))
            .to_be_bytes(),
    );
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

impl Default for ServerState {
    fn default() -> Self {
        Self::with_world(World::in_memory(DEFAULT_SEED))
    }
}

impl ServerState {
    /// Attach a world without enabling operator/ban persistence.
    pub fn with_world(world: World) -> Self {
        Self {
            world,
            session_id: new_session_id(),
            clock: Mutex::new(WorldClock::default()),
            view_distance: crate::world::default_view_distance(),
            state_directory: None,
            players: RwLock::new(BTreeMap::new()),
            next_id: AtomicU64::new(0),
            chat_index: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            ops: Mutex::new(BTreeSet::new()),
            bans: Mutex::new(BTreeMap::new()),
            scoreboard: Mutex::new(crate::scoreboard::Scoreboard::default()),
        }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn session_id(&self) -> &[u8; 16] {
        &self.session_id
    }

    pub fn with_view_distance(mut self, distance: i32) -> Self {
        self.view_distance = distance.clamp(1, crate::world::VIEW_DISTANCE);
        self
    }

    pub fn view_distance(&self) -> i32 {
        self.view_distance
    }

    pub fn clock_snapshot(&self) -> WorldClock {
        *self.clock.lock().expect("world clock lock")
    }

    /// Queue the snapshot while holding the clock lock so a tick/time command
    /// cannot enqueue an older update after it.
    pub fn send_clock(&self, player: PlayerId) {
        let clock = self.clock.lock().expect("world clock lock");
        self.send_to(player, &clock.encode());
    }

    pub fn tick_clock(&self) -> WorldClock {
        let mut clock = self.clock.lock().expect("world clock lock");
        clock.advance();
        self.broadcast(&clock.encode());
        *clock
    }

    pub fn set_day_time(&self, ticks: i64) {
        let mut clock = self.clock.lock().expect("world clock lock");
        clock.overworld_time = ticks;
        self.broadcast(&encode_set_day_time(clock.game_time, ticks));
    }

    /// Register a player and return its handle (with a fresh outbox).
    pub fn join(&self, name: &str, uuid: [u8; 16]) -> PlayerHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let handle = PlayerHandle {
            id,
            name: name.to_string(),
            uuid,
            outbox: Arc::new(Outbox::default()),
            kicked: Arc::new(AtomicBool::new(false)),
        };
        if let Ok(mut players) = self.players.write() {
            players.insert(id, handle.clone());
        }
        handle
    }

    /// Remove a player from the registry.
    pub fn leave(&self, id: PlayerId) {
        if let Ok(mut players) = self.players.write() {
            players.remove(&id);
        }
    }

    /// Number of players currently online.
    pub fn player_count(&self) -> usize {
        self.players.read().map(|p| p.len()).unwrap_or(0)
    }

    /// Online player names, sorted (deterministic: the map is a `BTreeMap`,
    /// and names are sorted explicitly so join order does not leak in).
    pub fn player_names(&self) -> Vec<String> {
        let mut names: Vec<String> = match self.players.read() {
            Ok(players) => players.values().map(|p| p.name.clone()).collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    /// Snapshot of all connected players (uuid + name) for the tab list.
    pub fn players(&self) -> Vec<PlayerHandle> {
        self.players
            .read()
            .map(|p| p.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Look up a player by name, case-insensitively.
    pub fn find_by_name(&self, name: &str) -> Option<PlayerHandle> {
        let players = self.players.read().ok()?;
        players
            .values()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .cloned()
    }

    /// Queue framed bytes for every online player.
    pub fn broadcast(&self, bytes: &[u8]) {
        if let Ok(players) = self.players.read() {
            for player in players.values() {
                player.outbox.push(bytes);
            }
        }
    }

    /// Queue framed bytes for everyone except `except`.
    pub fn broadcast_except(&self, except: PlayerId, bytes: &[u8]) {
        if let Ok(players) = self.players.read() {
            for (id, player) in players.iter() {
                if *id != except {
                    player.outbox.push(bytes);
                }
            }
        }
    }

    /// Queue framed bytes for one player, if still online.
    pub fn send_to(&self, id: PlayerId, bytes: &[u8]) {
        if let Ok(players) = self.players.read() {
            if let Some(player) = players.get(&id) {
                player.outbox.push(bytes);
            }
        }
    }

    /// Take the next `player_chat` global index.
    pub fn next_chat_index(&self) -> i32 {
        // Chat indices are varints; wrap well before i32 overflow.
        (self.chat_index.fetch_add(1, Ordering::Relaxed) % (i32::MAX as u64)) as i32
    }

    /// Request a full server shutdown (`/stop`).
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Ok(players) = self.players.read() {
            for player in players.values() {
                player.kick();
            }
        }
    }

    /// True once a shutdown has been requested.
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// True if `name` is a server operator.
    pub fn is_op(&self, name: &str) -> bool {
        self.ops
            .lock()
            .map(|ops| ops.contains(name))
            .unwrap_or(false)
    }

    /// Promote `name` to operator (in memory; call [`Self::save_ops`] to persist).
    pub fn add_op(&self, name: &str) {
        if let Ok(mut ops) = self.ops.lock() {
            ops.insert(name.to_string());
        }
        self.send_permission_level(name, 4);
    }

    /// Remove operator status from `name` (in memory; call [`Self::save_ops`] to persist).
    pub fn remove_op(&self, name: &str) {
        if let Ok(mut ops) = self.ops.lock() {
            ops.remove(name);
        }
        self.send_permission_level(name, 0);
    }

    fn send_permission_level(&self, name: &str, level: u8) {
        if let Some(player) = self.find_by_name(name) {
            player
                .outbox
                .push(&crate::gameplay::encode_permission_level(
                    crate::join::PLAY_LOGIN_ENTITY_ID,
                    level,
                ));
        }
    }

    /// Load operator names from `ops.json` (a JSON array of strings).
    fn load_ops(&self) -> io::Result<()> {
        if let Some(text) = self.read_state_file(OPS_FILE)? {
            let names: Vec<String> = serde_json::from_str(&text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            self.ops.lock().expect("operator lock").extend(names);
        }
        Ok(())
    }

    /// Write operator names back to `ops.json` as a JSON array of strings.
    pub fn save_ops(&self) {
        if let Ok(ops) = self.ops.lock() {
            let names: Vec<_> = ops.iter().collect();
            let text = serde_json::to_string(&names).expect("serializable operator names");
            self.write_state_file(OPS_FILE, &text);
        }
    }
    fn load_bans(&self) -> io::Result<()> {
        if let Some(text) = self.read_state_file("banned-players.json")? {
            let bans = serde_json::from_str(&text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            *self.bans.lock().expect("ban lock") = bans;
        }
        Ok(())
    }

    fn read_state_file(&self, name: &str) -> io::Result<Option<String>> {
        let Some(root) = &self.state_directory else {
            return Ok(None);
        };
        match std::fs::read_to_string(root.join(name)) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write_state_file(&self, name: &str, text: &str) {
        if let Some(root) = &self.state_directory {
            let path = root.join(name);
            if let Err(error) = std::fs::write(&path, format!("{text}\n")) {
                eprintln!("[bcore] cannot save {}: {error}", path.display());
            }
        }
    }
    pub fn ban(&self, name: &str, reason: &str) {
        if let Ok(mut b) = self.bans.lock() {
            b.insert(name.to_string(), reason.to_string());
        }
        self.save_bans();
    }
    pub fn pardon(&self, name: &str) {
        if let Ok(mut b) = self.bans.lock() {
            b.remove(name);
        }
        self.save_bans();
    }
    pub fn bans(&self) -> BTreeMap<String, String> {
        self.bans.lock().map(|b| b.clone()).unwrap_or_default()
    }
    pub fn is_banned(&self, name: &str) -> Option<String> {
        self.bans
            .lock()
            .ok()?
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, r)| r.clone())
    }
    pub fn save_bans(&self) {
        if let Ok(b) = self.bans.lock() {
            if let Ok(s) = serde_json::to_string_pretty(&*b) {
                self.write_state_file("banned-players.json", &s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_identity_is_shared_by_clones_and_distinct_between_servers() {
        let first = new_shared_server();
        let clone = Arc::clone(&first);
        let second = new_shared_server();
        assert_eq!(first.session_id(), clone.session_id());
        assert_ne!(first.session_id(), second.session_id());
        assert_eq!(first.session_id()[6] >> 4, 4);
        assert_eq!(first.session_id()[8] >> 6, 2);
    }

    #[test]
    fn separately_owned_servers_do_not_share_world_payloads_or_clocks() {
        let first = ServerState::with_world(World::flat_fixture([(0, 0)]));
        let second = ServerState::with_world(World::flat_fixture([(1, 0)]));
        assert!(first.world().cached_payload(0, 0).is_some());
        assert!(second.world().cached_payload(0, 0).is_none());
        assert!(first.world().cached_payload(1, 0).is_none());
        assert!(second.world().cached_payload(1, 0).is_some());
        first.set_day_time(13_000);
        assert_eq!(first.clock_snapshot().overworld_time, 13_000);
        assert_eq!(
            second.clock_snapshot().overworld_time,
            crate::gameplay::TIME_DAY
        );
        assert!(first.world().store().is_none());
        assert!(second.world().store().is_none());
    }

    #[test]
    fn ticks_and_time_commands_share_ordered_state_with_future_players() {
        let server = new_shared_server();
        let first = server.join("First", [1; 16]);
        server.tick_clock();
        server.set_day_time(73_000);
        let clock = server.tick_clock();
        assert_eq!(clock.game_time, 2);
        assert_eq!(clock.end_time, 2);
        assert_eq!(clock.overworld_time, 73_001);
        let future = server.join("Future", [2; 16]);
        server.send_clock(future.id);
        assert_eq!(future.outbox.drain(), clock.encode());
        let mut queued = std::io::Cursor::new(first.outbox.drain());
        let mut count = 0;
        while queued.position() < queued.get_ref().len() as u64 {
            assert_eq!(
                crate::packet::read_frame(&mut queued).unwrap().0,
                crate::gameplay::CB_UPDATE_TIME
            );
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn joining_assigns_distinct_ids_and_tracks_names() {
        let server = new_shared_server();
        let a = server.join("Alpha", [0xA1; 16]);
        let b = server.join("Beta", [0xB2; 16]);
        assert_ne!(a.id, b.id);
        assert_eq!(server.player_count(), 2);
        assert_eq!(server.player_names(), vec!["Alpha", "Beta"]);
        server.leave(a.id);
        assert_eq!(server.player_names(), vec!["Beta"]);
        assert_eq!(server.player_count(), 1);
    }

    #[test]
    fn op_changes_update_only_the_targets_client_permissions() {
        let server = ServerState::default();
        let player = server.join("Operator", [1; 16]);
        let other = server.join("Other", [2; 16]);
        for (op, expected) in [(true, 28), (false, 24)] {
            if op {
                server.add_op("Operator");
            } else {
                server.remove_op("Operator");
            }
            let (id, body) =
                crate::packet::read_frame(&mut std::io::Cursor::new(player.outbox.drain()))
                    .unwrap();
            assert_eq!(id, 0x22);
            assert_eq!(&body[..4], &crate::join::PLAY_LOGIN_ENTITY_ID.to_be_bytes());
            assert_eq!(body[4], expected);
            assert_eq!(server.is_op("Operator"), op);
            assert!(other.outbox.is_empty());
        }
    }

    #[test]
    fn player_names_are_sorted_not_join_ordered() {
        let server = new_shared_server();
        for name in ["zoe", "adam", "mia"] {
            server.join(name, [0; 16]);
        }
        assert_eq!(server.player_names(), vec!["adam", "mia", "zoe"]);
    }

    #[test]
    fn broadcast_reaches_every_outbox_exactly_once() {
        let server = new_shared_server();
        let a = server.join("Alpha", [0; 16]);
        let b = server.join("Beta", [0; 16]);
        server.broadcast(b"hi");
        assert_eq!(a.outbox.drain(), b"hi");
        assert_eq!(b.outbox.drain(), b"hi");
        // Draining empties the outbox.
        assert!(a.outbox.is_empty());
        assert!(a.outbox.drain().is_empty());
    }

    #[test]
    fn broadcast_except_skips_the_sender() {
        let server = new_shared_server();
        let a = server.join("Alpha", [0; 16]);
        let b = server.join("Beta", [0; 16]);
        server.broadcast_except(a.id, b"news");
        assert!(a.outbox.is_empty(), "sender is skipped");
        assert_eq!(b.outbox.drain(), b"news");
    }

    #[test]
    fn queued_bytes_accumulate_in_order() {
        let server = new_shared_server();
        let a = server.join("Alpha", [0; 16]);
        server.broadcast(b"one");
        server.send_to(a.id, b"two");
        assert_eq!(a.outbox.drain(), b"onetwo");
    }

    #[test]
    fn sending_to_a_departed_player_is_a_no_op() {
        let server = new_shared_server();
        let a = server.join("Alpha", [0; 16]);
        server.leave(a.id);
        server.send_to(a.id, b"ignored");
        server.broadcast(b"ignored");
        assert!(a.outbox.is_empty());
    }

    #[test]
    fn lookup_by_name_ignores_case_and_missing_players() {
        let server = new_shared_server();
        server.join("AlphaProbe", [0x11; 16]);
        let found = server.find_by_name("alphaprobe").expect("found");
        assert_eq!(found.name, "AlphaProbe");
        assert_eq!(found.uuid, [0x11; 16]);
        assert!(server.find_by_name("nobody").is_none());
    }

    #[test]
    fn chat_indices_increase_monotonically() {
        let server = new_shared_server();
        assert_eq!(server.next_chat_index(), 0);
        assert_eq!(server.next_chat_index(), 1);
        assert_eq!(server.next_chat_index(), 2);
    }

    #[test]
    fn kicking_flags_only_the_target() {
        let server = new_shared_server();
        let a = server.join("Alpha", [0; 16]);
        let b = server.join("Beta", [0; 16]);
        assert!(!a.is_kicked());
        a.kick();
        assert!(a.is_kicked());
        assert!(!b.is_kicked());
    }

    #[test]
    fn shutdown_flags_the_server_and_every_player() {
        let server = new_shared_server();
        let a = server.join("Alpha", [0; 16]);
        assert!(!server.is_shutting_down());
        server.request_shutdown();
        assert!(server.is_shutting_down());
        assert!(a.is_kicked());
    }

    #[test]
    fn concurrent_broadcast_join_leave_is_thread_safe() {
        let server = new_shared_server();
        let receiver = server.join("Receiver", [0; 16]);
        let mut threads = Vec::new();
        for worker in 0..8 {
            let server = Arc::clone(&server);
            threads.push(std::thread::spawn(move || {
                for iteration in 0..100 {
                    let player = server.join(&format!("worker-{worker}-{iteration}"), [worker; 16]);
                    server.broadcast(b"tick");
                    server.leave(player.id);
                }
            }));
        }
        for thread in threads {
            thread.join().expect("thread");
        }
        assert_eq!(server.player_count(), 1);
        assert_eq!(receiver.outbox.drain().len(), 8 * 100 * b"tick".len());
    }

    #[test]
    fn broadcast_is_safe_from_many_threads() {
        let server = new_shared_server();
        let receiver = server.join("Receiver", [0; 16]);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let server = Arc::clone(&server);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    server.broadcast(b"x");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("thread");
        }
        assert_eq!(receiver.outbox.drain().len(), 8 * 50);
    }
}
