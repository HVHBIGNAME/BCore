//! Shared server state: the online-player registry used for chat broadcast.
//!
//! `server::run` spawns one thread per connection, so nothing was shared
//! between players before this module. [`SharedServer`] is the one piece of
//! global state: an `Arc<Mutex<..>>` registry mapping a per-connection id to the
//! player's name and an outbound byte queue.
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Identifier of a connected player, unique for the server's lifetime.
pub type PlayerId = u64;

/// A player's pending outbound bytes (fully framed packets).
#[derive(Debug, Default)]
pub struct Outbox {
    queued: Mutex<Vec<u8>>,
}

impl Outbox {
    /// Append framed packet bytes for the owning connection to flush.
    pub fn push(&self, bytes: &[u8]) {
        if let Ok(mut queued) = self.queued.lock() {
            queued.extend_from_slice(bytes);
        }
    }

    /// Take everything queued so far, leaving the outbox empty.
    pub fn drain(&self) -> Vec<u8> {
        match self.queued.lock() {
            Ok(mut queued) => std::mem::take(&mut *queued),
            Err(_) => Vec::new(),
        }
    }

    /// Bytes currently queued.
    pub fn len(&self) -> usize {
        self.queued.lock().map(|q| q.len()).unwrap_or(0)
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
#[derive(Debug, Default)]
pub struct ServerState {
    players: Mutex<BTreeMap<PlayerId, PlayerHandle>>,
    next_id: AtomicU64,
    /// Global chat message counter (`player_chat`'s `globalIndex`).
    chat_index: AtomicU64,
    /// Set by `/stop`: the accept loop and every play loop should wind down.
    shutdown: AtomicBool,
    /// Server operator names (from `ops.json`), persisted on change.
    ops: Mutex<BTreeSet<String>>,
}

/// Cheap-to-clone handle to the shared server state.
pub type SharedServer = Arc<ServerState>;

/// File holding operator names, one per line (BCore's simple `ops.json`).
const OPS_FILE: &str = "ops.json";

/// Create fresh shared state and load operators from `ops.json`.
pub fn new_shared_server() -> SharedServer {
    let server = ServerState::default();
    server.load_ops();
    Arc::new(server)
}

impl ServerState {
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
        if let Ok(mut players) = self.players.lock() {
            players.insert(id, handle.clone());
        }
        handle
    }

    /// Remove a player from the registry.
    pub fn leave(&self, id: PlayerId) {
        if let Ok(mut players) = self.players.lock() {
            players.remove(&id);
        }
    }

    /// Number of players currently online.
    pub fn player_count(&self) -> usize {
        self.players.lock().map(|p| p.len()).unwrap_or(0)
    }

    /// Online player names, sorted (deterministic: the map is a `BTreeMap`,
    /// and names are sorted explicitly so join order does not leak in).
    pub fn player_names(&self) -> Vec<String> {
        let mut names: Vec<String> = match self.players.lock() {
            Ok(players) => players.values().map(|p| p.name.clone()).collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    /// Look up a player by name, case-insensitively.
    pub fn find_by_name(&self, name: &str) -> Option<PlayerHandle> {
        let players = self.players.lock().ok()?;
        players
            .values()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .cloned()
    }

    /// Queue framed bytes for every online player.
    pub fn broadcast(&self, bytes: &[u8]) {
        if let Ok(players) = self.players.lock() {
            for player in players.values() {
                player.outbox.push(bytes);
            }
        }
    }

    /// Queue framed bytes for everyone except `except`.
    pub fn broadcast_except(&self, except: PlayerId, bytes: &[u8]) {
        if let Ok(players) = self.players.lock() {
            for (id, player) in players.iter() {
                if *id != except {
                    player.outbox.push(bytes);
                }
            }
        }
    }

    /// Queue framed bytes for one player, if still online.
    pub fn send_to(&self, id: PlayerId, bytes: &[u8]) {
        if let Ok(players) = self.players.lock() {
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
        if let Ok(players) = self.players.lock() {
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
    }

    /// Remove operator status from `name` (in memory; call [`Self::save_ops`] to persist).
    pub fn remove_op(&self, name: &str) {
        if let Ok(mut ops) = self.ops.lock() {
            ops.remove(name);
        }
    }

    /// Load operator names from `ops.json` (a JSON array of strings).
    fn load_ops(&self) {
        if let Ok(text) = std::fs::read_to_string(OPS_FILE) {
            if let Ok(mut ops) = self.ops.lock() {
                for name in text.split('"').skip(1).step_by(2) {
                    if !name.is_empty() {
                        ops.insert(name.to_string());
                    }
                }
            }
        }
    }

    /// Write operator names back to `ops.json` as a JSON array of strings.
    pub fn save_ops(&self) {
        if let Ok(ops) = self.ops.lock() {
            let mut names: Vec<&String> = ops.iter().collect();
            names.sort();
            let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
            let text = format!("[{}]\n", quoted.join(", "));
            let _ = std::fs::write(OPS_FILE, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
