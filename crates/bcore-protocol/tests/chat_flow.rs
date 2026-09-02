//! End-to-end chat and command test with two real clients over TCP.
//!
//! Starts `server::run` on an ephemeral port (never `bcore.exe`, which may be
//! serving live players), joins two clients through the full
//! login → configuration → play handshake and then exercises:
//!
//!   * the join-time extras (`declare_commands`, `update_health`,
//!     `update_time`, `abilities`),
//!   * `/help` and `/list` replies on `system_chat`,
//!   * a plain chat message reaching the **other** player as `player_chat`,
//!   * `/me` and `/say` reaching both players as `profileless_chat`,
//!   * `/gamemode`, `/tp`, `/spawn`, `/time set` and their state packets,
//!   * `/kick`, which disconnects the target.
//!
//! Payloads are parsed, not just counted: chat text is decoded out of the NBT
//! content so a silently-empty message would fail.

use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use bcore_core::varint::{decode_varint, encode_varint};
use bcore_protocol::chat::{
    CB_PLAYER_CHAT, CB_PROFILELESS_CHAT, CB_SYSTEM_CHAT, SB_CHAT_COMMAND, SB_CHAT_MESSAGE,
};
use bcore_protocol::commands::CB_DECLARE_COMMANDS;
use bcore_protocol::gameplay::{
    CB_ABILITIES, CB_GAME_STATE_CHANGE, CB_UPDATE_HEALTH, CB_UPDATE_TIME,
};
use bcore_protocol::packet::{read_frame, read_string, read_varint, write_packet, write_string};
use bcore_protocol::server;
use bcore_protocol::world::CB_POSITION;

const CB_KICK_DISCONNECT: i32 = 0x20;
const CB_CHUNK_BATCH_FINISHED: i32 = 0x0b;
const SB_TELEPORT_CONFIRM: i32 = 0x00;

/// One captured clientbound packet.
#[derive(Debug, Clone)]
struct Packet {
    id: i32,
    data: Vec<u8>,
}

struct Client {
    stream: TcpStream,
    seen: Vec<Packet>,
}

impl Client {
    /// Connect and drive login + configuration + play until the first chunk
    /// batch has been received, capturing every play packet on the way.
    fn join(addr: SocketAddr, name: &str, uuid_byte: u8) -> Self {
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream.set_nodelay(true).ok();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");

        // Handshake -> login (protocol 776).
        let mut hs = Vec::new();
        encode_varint(776, &mut hs);
        write_string("127.0.0.1", &mut hs);
        hs.extend_from_slice(&addr.port().to_be_bytes());
        encode_varint(2, &mut hs);
        send(&mut stream, 0x00, &hs);

        // Login start: name + 16-byte uuid.
        let mut ls = Vec::new();
        write_string(name, &mut ls);
        ls.extend_from_slice(&[uuid_byte; 16]);
        send(&mut stream, 0x00, &ls);

        let (pid, data) = read_frame(&mut stream).expect("login success");
        assert_eq!(pid, 0x02, "expected login success");
        let mut cur = Cursor::new(data);
        let mut uuid = [0u8; 16];
        cur.read_exact(&mut uuid).expect("uuid");
        assert_eq!(read_string(&mut cur, 16).expect("name"), name);
        assert_eq!(read_varint(&mut cur).expect("props"), 0);
        assert_eq!(uuid, [uuid_byte; 16], "offline uuid is echoed back");

        send(&mut stream, 0x03, &[]); // login acknowledged

        // Configuration: answer known-packs, wait for finish.
        loop {
            let (pid, _) = read_frame(&mut stream).expect("config packet");
            match pid {
                0x0e => send(&mut stream, 0x07, &[0x00]),
                0x03 => break,
                _ => {}
            }
        }
        send(&mut stream, 0x03, &[]); // finish configuration -> play

        let mut client = Client {
            stream,
            seen: Vec::new(),
        };
        // Read until the join chunk batch is done, confirming the teleport.
        client.pump_until(Duration::from_secs(15), |seen| {
            seen.iter().any(|p| p.id == CB_CHUNK_BATCH_FINISHED)
        });
        client
    }

    /// Read packets until `done` is satisfied or the deadline passes.
    fn pump_until(&mut self, budget: Duration, done: impl Fn(&[Packet]) -> bool) -> bool {
        let deadline = Instant::now() + budget;
        if done(&self.seen) {
            return true;
        }
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            self.stream
                .set_read_timeout(Some(remaining.max(Duration::from_millis(50))))
                .expect("timeout");
            match read_frame(&mut self.stream) {
                Ok((id, data)) => {
                    // Keep the connection healthy: confirm teleports so the
                    // server does not consider the client stuck.
                    if id == CB_POSITION {
                        if let Ok((_, n)) = decode_varint(&data) {
                            let tid = data[..n].to_vec();
                            send(&mut self.stream, SB_TELEPORT_CONFIRM, &tid);
                        }
                    }
                    self.seen.push(Packet { id, data });
                    if done(&self.seen) {
                        return true;
                    }
                }
                Err(_) => break,
            }
        }
        done(&self.seen)
    }

    /// Drain whatever is already buffered, without waiting for anything new.
    fn drain(&mut self, budget: Duration) {
        self.pump_until(budget, |_| false);
    }

    fn send_chat(&mut self, message: &str) {
        // chat_message: message, timestamp, salt, no signature, offset,
        // acknowledged bitset (3 bytes), checksum.
        let mut data = Vec::new();
        write_string(message, &mut data);
        data.extend_from_slice(&0i64.to_be_bytes());
        data.extend_from_slice(&0i64.to_be_bytes());
        data.push(0x00);
        encode_varint(0, &mut data);
        data.extend_from_slice(&[0, 0, 0]);
        data.push(0x00);
        send(&mut self.stream, SB_CHAT_MESSAGE, &data);
    }

    fn send_command(&mut self, command: &str) {
        let mut data = Vec::new();
        write_string(command, &mut data);
        send(&mut self.stream, SB_CHAT_COMMAND, &data);
    }

    fn clear(&mut self) {
        self.seen.clear();
    }

    fn ids(&self) -> Vec<i32> {
        self.seen.iter().map(|p| p.id).collect()
    }

    fn first(&self, id: i32) -> Option<&Packet> {
        self.seen.iter().find(|p| p.id == id)
    }

    fn all(&self, id: i32) -> Vec<&Packet> {
        self.seen.iter().filter(|p| p.id == id).collect()
    }

    /// Every chat line this client received, as decoded text.
    fn chat_lines(&self) -> Vec<String> {
        self.seen
            .iter()
            .filter(|p| matches!(p.id, CB_SYSTEM_CHAT | CB_PROFILELESS_CHAT | CB_PLAYER_CHAT))
            .filter_map(|p| match p.id {
                CB_PLAYER_CHAT => player_chat_message(&p.data),
                _ => Some(nbt_strings(&p.data).join(" | ")),
            })
            .collect()
    }

    /// Wait until a chat line containing `needle` arrives.
    fn wait_for_chat(&mut self, needle: &str) -> bool {
        let needle = needle.to_string();
        self.pump_until(Duration::from_secs(6), move |seen| {
            seen.iter().any(|p| match p.id {
                CB_PLAYER_CHAT => player_chat_message(&p.data)
                    .map(|m| m.contains(&needle))
                    .unwrap_or(false),
                CB_SYSTEM_CHAT | CB_PROFILELESS_CHAT => {
                    nbt_strings(&p.data).iter().any(|s| s.contains(&needle))
                }
                _ => false,
            })
        })
    }
}

fn send(stream: &mut TcpStream, id: i32, data: &[u8]) {
    let mut frame = Vec::new();
    write_packet(&mut frame, id, data);
    stream.write_all(&frame).expect("write");
}

/// Collect every NBT string payload in a buffer.
///
/// Both component shapes (a bare `TAG_String` root and a compound with a
/// `"text"` entry) store their text as a `u16`-prefixed string, so scanning for
/// well-formed strings recovers the message without a full NBT parser.
fn nbt_strings(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 2 <= data.len() {
        let len = u16::from_be_bytes([data[at], data[at + 1]]) as usize;
        if len >= 2 && at + 2 + len <= data.len() {
            if let Ok(text) = std::str::from_utf8(&data[at + 2..at + 2 + len]) {
                if text.chars().all(|c| !c.is_control()) {
                    out.push(text.to_string());
                }
            }
        }
        at += 1;
    }
    out
}

/// Read the `plainMessage` field out of a `player_chat` payload.
///
/// Layout: `globalIndex varint | senderUuid 16 | index varint |
/// signature option | plainMessage string | ...`
fn player_chat_message(data: &[u8]) -> Option<String> {
    let (_, n) = decode_varint(data).ok()?;
    let mut at = n + 16;
    let (_, m) = decode_varint(data.get(at..)?).ok()?;
    at += m;
    let has_signature = *data.get(at)? != 0;
    at += 1;
    if has_signature {
        at += 256;
    }
    let mut cur = Cursor::new(data.get(at..)?);
    read_string(&mut cur, 256).ok()
}

fn start_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || server::run(listener));
    addr
}

#[test]
fn join_sends_the_command_tree_health_time_and_abilities() {
    let addr = start_server();
    let mut alpha = Client::join(addr, "AlphaProbe", 0xA1);
    // The extras are sent right after the join chunk batch.
    alpha.pump_until(Duration::from_secs(6), |seen| {
        seen.iter().any(|p| p.id == CB_ABILITIES)
            && seen.iter().any(|p| p.id == CB_DECLARE_COMMANDS)
            && seen.iter().any(|p| p.id == CB_UPDATE_HEALTH)
            && seen.iter().any(|p| p.id == CB_UPDATE_TIME)
    });

    let ids = alpha.ids();
    assert!(
        ids.contains(&CB_DECLARE_COMMANDS),
        "missing declare_commands, got {ids:02x?}"
    );

    // update_health: 20 health, 20 food, 5.0 saturation.
    let health = alpha.first(CB_UPDATE_HEALTH).expect("update_health");
    assert_eq!(
        health.data,
        vec![0x41, 0xa0, 0x00, 0x00, 0x14, 0x40, 0xa0, 0x00, 0x00],
        "health must be full at join"
    );

    // abilities: survival flags, vanilla's default speeds.
    let abilities = alpha.first(CB_ABILITIES).expect("abilities");
    assert_eq!(
        abilities.data,
        vec![0x00, 0x3d, 0x4c, 0xcc, 0xcd, 0x3d, 0xcc, 0xcc, 0xcd],
        "survival abilities"
    );

    // update_time: two clock updates (world age + day time), both at rate 1.0.
    let time = alpha.first(CB_UPDATE_TIME).expect("update_time");
    let age = i64::from_be_bytes(time.data[..8].try_into().expect("8 bytes"));
    assert!(age > 0, "world age should be running");
    let mut at = 8usize;
    let (clocks, n) = decode_varint(&time.data[at..]).expect("clock count");
    at += n;
    assert_eq!(clocks, 2, "world age + day time clocks");
    let mut seen_clocks = Vec::new();
    for _ in 0..clocks {
        let (id, n) = decode_varint(&time.data[at..]).expect("clock id");
        at += n;
        let (ticks, n) = bcore_core::varint::decode_varlong(&time.data[at..]).expect("ticks");
        at += n;
        let partial = f32::from_be_bytes(time.data[at..at + 4].try_into().expect("f32"));
        at += 4;
        let rate = f32::from_be_bytes(time.data[at..at + 4].try_into().expect("f32"));
        at += 4;
        assert_eq!(partial, 0.0, "vanilla sends no partial tick");
        assert_eq!(rate, 1.0, "clocks run at normal speed");
        seen_clocks.push((id, ticks));
    }
    assert_eq!(at, time.data.len(), "update_time fully consumed");
    let ids: Vec<i32> = seen_clocks.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![1, 0], "world age clock first, like vanilla");
    // The day-time clock starts at vanilla's morning.
    assert_eq!(seen_clocks[1].1, 1000, "day time starts at 1000");

    // The command tree declares the root plus BCore's commands.
    let tree = alpha.first(CB_DECLARE_COMMANDS).expect("declare_commands");
    let (nodes, _) = decode_varint(&tree.data).expect("node count");
    assert!(nodes > 11, "expected a populated tree, got {nodes} nodes");
    for command in [
        "help", "list", "gamemode", "tp", "seed", "time", "kick", "stop",
    ] {
        assert!(
            find(&tree.data, command.as_bytes()).is_some(),
            "tree should declare /{command}"
        );
    }
}

#[test]
fn help_and_list_reply_on_system_chat() {
    let addr = start_server();
    let mut alpha = Client::join(addr, "AlphaProbe", 0xA1);
    alpha.drain(Duration::from_millis(600));
    alpha.clear();

    alpha.send_command("help");
    // Wait for the LAST help line, so the whole batch has arrived.
    assert!(alpha.wait_for_chat("/stop"), "no /help output");
    let lines = alpha.chat_lines();
    assert!(
        lines.iter().any(|l| l.contains("BCore commands")),
        "missing help header in {lines:?}"
    );
    for command in [
        "/help", "/list", "/me", "/say", "/spawn", "/tp", "/seed", "/stop",
    ] {
        assert!(
            lines.iter().any(|l| l.contains(command)),
            "help missing {command}: {lines:?}"
        );
    }
    assert!(
        alpha.all(CB_SYSTEM_CHAT).len() >= 11,
        "help should be one system_chat per line"
    );

    alpha.clear();
    alpha.send_command("list");
    assert!(alpha.wait_for_chat("AlphaProbe"), "no /list output");
    let lines = alpha.chat_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("1 of a max of 20 players online") && l.contains("AlphaProbe")),
        "unexpected /list output: {lines:?}"
    );
}

#[test]
fn a_chat_message_reaches_the_other_player_as_player_chat() {
    let addr = start_server();
    let mut alpha = Client::join(addr, "AlphaProbe", 0xA1);
    let mut beta = Client::join(addr, "BetaProbe", 0xB2);
    // Alpha is told Beta joined; drain both clients' backlog first.
    alpha.drain(Duration::from_millis(800));
    beta.drain(Duration::from_millis(800));
    alpha.clear();
    beta.clear();

    alpha.send_chat("hello from alpha");
    assert!(
        beta.wait_for_chat("hello from alpha"),
        "broadcast never reached Beta: {:?}",
        beta.chat_lines()
    );
    assert!(
        alpha.wait_for_chat("hello from alpha"),
        "sender should see its own message"
    );

    // Beta received it as player_chat with Alpha's uuid and the plain text.
    let chat = beta.first(CB_PLAYER_CHAT).expect("player_chat");
    assert_eq!(
        player_chat_message(&chat.data).as_deref(),
        Some("hello from alpha")
    );
    let (_, n) = decode_varint(&chat.data).expect("globalIndex");
    assert_eq!(&chat.data[n..n + 16], &[0xA1; 16], "sender uuid");
    // Unsigned form: signature option is absent.
    let (_, m) = decode_varint(&chat.data[n + 16..]).expect("index");
    assert_eq!(chat.data[n + 16 + m], 0x00, "offline chat is unsigned");

    // /list now shows both players, sorted.
    alpha.clear();
    alpha.send_command("list");
    assert!(alpha.wait_for_chat("BetaProbe"));
    let lines = alpha.chat_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("2 of a max of 20") && l.contains("AlphaProbe, BetaProbe")),
        "unexpected /list output: {lines:?}"
    );
}

#[test]
fn me_and_say_broadcast_as_profileless_chat() {
    let addr = start_server();
    let mut alpha = Client::join(addr, "AlphaProbe", 0xA1);
    let mut beta = Client::join(addr, "BetaProbe", 0xB2);
    alpha.drain(Duration::from_millis(800));
    beta.drain(Duration::from_millis(800));
    alpha.clear();
    beta.clear();

    alpha.send_command("me waves at everyone");
    assert!(
        beta.wait_for_chat("waves at everyone"),
        "/me did not reach Beta: {:?}",
        beta.chat_lines()
    );
    let emote = beta.first(CB_PROFILELESS_CHAT).expect("profileless_chat");
    // NBT string root, then chat_type holder = emote_command(1) + 1.
    assert_eq!(emote.data[0], 0x08, "TAG_String content");
    let len = u16::from_be_bytes([emote.data[1], emote.data[2]]) as usize;
    assert_eq!(&emote.data[3..3 + len], b"waves at everyone");
    assert_eq!(emote.data[3 + len], 0x02, "emote_command holder");

    beta.clear();
    alpha.send_command("say server notice");
    assert!(
        beta.wait_for_chat("server notice"),
        "/say did not broadcast"
    );
    let say = beta.first(CB_PROFILELESS_CHAT).expect("profileless_chat");
    let len = u16::from_be_bytes([say.data[1], say.data[2]]) as usize;
    assert_eq!(&say.data[3..3 + len], b"server notice");
    assert_eq!(say.data[3 + len], 0x05, "say_command holder");
}

#[test]
fn gamemode_teleport_spawn_and_time_change_state() {
    let addr = start_server();
    let mut alpha = Client::join(addr, "AlphaProbe", 0xA1);
    alpha.drain(Duration::from_millis(800));
    alpha.clear();

    // /gamemode creative -> abilities 0x0d then game_state_change reason 3.
    alpha.send_command("gamemode creative");
    alpha.pump_until(Duration::from_secs(6), |seen| {
        seen.iter().any(|p| p.id == CB_GAME_STATE_CHANGE)
    });
    let abilities = alpha
        .all(CB_ABILITIES)
        .into_iter()
        .last()
        .expect("abilities");
    assert_eq!(abilities.data[0], 0x0d, "creative ability flags");
    let state = alpha.first(CB_GAME_STATE_CHANGE).expect("state change");
    assert_eq!(
        state.data,
        vec![0x03, 0x3f, 0x80, 0x00, 0x00],
        "gamemode 1.0"
    );

    // /tp moves the player: a `position` packet with the requested coordinates.
    alpha.clear();
    alpha.send_command("tp 300.5 -60 -200.5");
    assert!(
        alpha.pump_until(Duration::from_secs(8), |seen| seen
            .iter()
            .any(|p| p.id == CB_POSITION)),
        "no position packet after /tp"
    );
    let position = alpha.first(CB_POSITION).expect("position");
    let (_, n) = decode_varint(&position.data).expect("teleport id");
    let coord = |at: usize| {
        f64::from_be_bytes(
            position.data[n + at..n + at + 8]
                .try_into()
                .expect("8 bytes"),
        )
    };
    assert_eq!((coord(0), coord(8), coord(16)), (300.5, -60.0, -200.5));
    assert_eq!(
        position.data.len(),
        61,
        "the 61-byte position payload vanilla also uses"
    );

    // /spawn teleports back to where the player entered the world.
    alpha.clear();
    alpha.send_command("spawn");
    assert!(alpha.pump_until(Duration::from_secs(8), |seen| seen
        .iter()
        .any(|p| p.id == CB_POSITION)));
    let position = alpha.first(CB_POSITION).expect("position");
    let (_, n) = decode_varint(&position.data).expect("teleport id");
    let spawn_x = f64::from_be_bytes(position.data[n..n + 8].try_into().expect("8 bytes"));
    assert_eq!(spawn_x, 10.5, "vanilla flat spawn x");

    // /time set night -> one clock update carrying 13000 ticks.
    alpha.clear();
    alpha.send_command("time set night");
    assert!(alpha.pump_until(Duration::from_secs(6), |seen| seen
        .iter()
        .any(|p| p.id == CB_UPDATE_TIME)));
    let time = alpha.first(CB_UPDATE_TIME).expect("update_time");
    assert_eq!(time.data[8], 0x01, "one clock update");
    assert_eq!(time.data[9], 0x00, "day-time clock id");
    let (ticks, _) = bcore_core::varint::decode_varlong(&time.data[10..]).expect("varlong");
    assert_eq!(ticks, 13000, "night is 13000 ticks");
}

#[test]
fn seed_and_unknown_commands_answer_the_sender() {
    let addr = start_server();
    let mut alpha = Client::join(addr, "AlphaProbe", 0xA1);
    alpha.drain(Duration::from_millis(600));
    alpha.clear();

    alpha.send_command("seed");
    assert!(alpha.wait_for_chat("Seed:"), "no /seed reply");

    alpha.clear();
    alpha.send_command("fly");
    assert!(
        alpha.wait_for_chat("Unknown"),
        "unknown command not reported"
    );
    let error = alpha.first(CB_SYSTEM_CHAT).expect("system_chat");
    assert!(
        find(&error.data, b"red").is_some(),
        "errors should be red like vanilla"
    );
}

#[test]
fn kick_disconnects_the_target_and_tells_everyone() {
    let addr = start_server();
    let mut alpha = Client::join(addr, "AlphaProbe", 0xA1);
    let mut beta = Client::join(addr, "BetaProbe", 0xB2);
    alpha.drain(Duration::from_millis(800));
    beta.drain(Duration::from_millis(800));
    alpha.clear();
    beta.clear();

    alpha.send_command("kick BetaProbe");
    // Beta is told and then disconnected.
    assert!(
        beta.pump_until(Duration::from_secs(8), |seen| seen
            .iter()
            .any(|p| p.id == CB_KICK_DISCONNECT)),
        "Beta was never kicked: {:02x?}",
        beta.ids()
    );
    assert!(
        alpha.wait_for_chat("Kicked BetaProbe"),
        "no kick confirmation"
    );

    // Once Beta is gone, /list shows only Alpha.
    alpha.clear();
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut only_alpha = false;
    while Instant::now() < deadline && !only_alpha {
        alpha.send_command("list");
        alpha.wait_for_chat("players online");
        only_alpha = alpha
            .chat_lines()
            .iter()
            .any(|l| l.contains("1 of a max of 20") && !l.contains("BetaProbe"));
        alpha.clear();
    }
    assert!(only_alpha, "/list should drop the kicked player");
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
