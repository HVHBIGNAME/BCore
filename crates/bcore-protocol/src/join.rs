//! Login → configuration → play join flow (offline mode) for protocol 776.
//!
//! The configuration and play packets were captured verbatim from the official
//! vanilla 26.2 server (flat world, offline mode) and are replayed here. Only
//! the player identity (uuid + name) is substituted at runtime. This is a
//! bootstrap path to reach a joinable server; the long-term plan is to generate
//! the registry and chunk data natively (see `bcore-registry`).
//!
//! Chunks are **not** replayed: the captured `map_chunk` packets are dropped and
//! the world is streamed from [`crate::chunk`] / [`crate::world`] instead, so the
//! player can walk out of the original 3x3 capture. The capture's trailing
//! `kick_disconnect` (vanilla booting the capture client) is dropped as well.

use std::io::{Cursor, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bcore_core::varint::encode_varint;

use crate::chat::{
    encode_player_chat, encode_profileless_chat, encode_system_chat, encode_system_message,
    parse_chat_input, parse_gamemode, ChatInput, CHAT_TYPE_SAY_COMMAND, SB_CHANGE_GAMEMODE,
};
use crate::command::{self, CommandContext, Destination, Effect};
use crate::commands::{bcore_command_tree, CB_DECLARE_COMMANDS};
use crate::gameplay::{
    encode_abilities_for, encode_full_health, encode_gamemode_switch, encode_set_day_time,
    encode_time_of_day, CB_ABILITIES, CB_UPDATE_HEALTH, CB_UPDATE_TIME,
};
use crate::nbt::Component;
use crate::packet::{read_frame, read_string, write_packet, write_string, PacketError};
use crate::shared::{PlayerHandle, SharedServer};
use crate::world::{
    PlayerView, CB_CHUNK_BATCH_FINISHED, CB_CHUNK_BATCH_START, CB_MAP_CHUNK, CB_PING_RESPONSE,
    SB_CHUNK_BATCH_RECEIVED, SB_PING_REQUEST,
};

pub const LOGIN_SUCCESS_ID: i32 = 0x02;
pub const LOGIN_ACKNOWLEDGED_ID: i32 = 0x03;
const CONFIG_SELECT_KNOWN_PACKS_ID: i32 = 0x0e;
const CONFIG_KNOWN_PACKS_RESPONSE_ID: i32 = 0x07;
const CONFIG_FINISH_ID: i32 = 0x03;
const PLAY_POSITION_ID: i32 = 0x48;
const PLAY_PLAYER_INFO_ID: i32 = 0x46;
const PLAY_KEEP_ALIVE_ID: i32 = 0x2c;
const PLAY_TELEPORT_CONFIRM_ID: i32 = 0x00;
/// Clientbound `kick_disconnect`: present in the capture, never replayed.
const PLAY_KICK_DISCONNECT_ID: i32 = 0x20;

/// The world seed `/seed` reports.
///
/// This is the seed the terrain generator actually runs with, so `/seed` now
/// reports something meaningful: two servers with this seed generate identical
/// worlds.
pub const WORLD_SEED: i64 = crate::world_state::DEFAULT_SEED;
/// Player slots announced in the status response and reported by `/list`.
pub const MAX_PLAYERS: usize = 20;
/// Ticks per second the world clock advances at.
const TICKS_PER_SECOND: i64 = 20;

// Embedded vanilla 26.2 captures. Binary format:
// [u32 count][ (i32 pid, u32 len, bytes)... ]
const CONFIG_PACKETS: &[u8] = include_bytes!("../data/config_packets.bin");
const PLAY_PACKETS: &[u8] = include_bytes!("../data/play_packets.bin");

/// Read a login-start payload: player name + 16-byte UUID.
pub fn read_login_start<R: Read>(r: &mut R) -> Result<(String, [u8; 16]), PacketError> {
    let name = read_string(r, 16)?;
    let mut uuid = [0u8; 16];
    r.read_exact(&mut uuid)?;
    Ok((name, uuid))
}

/// Encode a login-success packet (uuid + name + properties + profile id).
pub fn encode_login_success(uuid: &[u8; 16], name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(uuid);
    write_string(name, &mut data);
    encode_varint(0, &mut data); // properties count
    data.extend_from_slice(&random_uuid_v4()); // profile id (added in 1.21.2+)
    let mut out = Vec::new();
    write_packet(&mut out, LOGIN_SUCCESS_ID, &data);
    out
}

static PROFILE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// Generate a random version-4 UUID (the login-success "profile id").
fn random_uuid_v4() -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut state = nanos
        ^ PROFILE_ID_COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9e3779b97f4a7c15);
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state = splitmix64(state);
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    bytes
}

struct CapturedPackets {
    items: Vec<(i32, Vec<u8>)>,
}

fn parse_captured(bytes: &[u8]) -> CapturedPackets {
    let mut cur = Cursor::new(bytes);
    let mut count_buf = [0u8; 4];
    cur.read_exact(&mut count_buf).expect("captured data count");
    let count = u32::from_be_bytes(count_buf) as usize;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let mut pid_buf = [0u8; 4];
        cur.read_exact(&mut pid_buf).expect("captured pid");
        let pid = i32::from_be_bytes(pid_buf);
        let mut len_buf = [0u8; 4];
        cur.read_exact(&mut len_buf).expect("captured len");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut data = vec![0u8; len];
        cur.read_exact(&mut data).expect("captured payload");
        items.push((pid, data));
    }
    CapturedPackets { items }
}

/// Drive a full offline-mode join for an already-connected stream.
///
/// `server` is the shared registry every connection thread uses to see the other
/// players; chat and `/list` need it.
pub fn run_login_and_join(
    stream: &mut TcpStream,
    server: &SharedServer,
) -> Result<(), PacketError> {
    let (packet_id, data) = read_frame(stream)?;
    if packet_id != 0x00 {
        return Err(PacketError::UnexpectedPacket(packet_id));
    }
    let mut cursor = Cursor::new(data);
    let (name, uuid) = read_login_start(&mut cursor)?;
    println!("[BCore] join: {name} (offline)");

    stream.write_all(&encode_login_success(&uuid, &name))?;

    let (pid, _) = read_frame(stream)?;
    if pid != LOGIN_ACKNOWLEDGED_ID {
        return Err(PacketError::UnexpectedPacket(pid));
    }

    config_replay(stream)?;
    let mut view = play_replay(stream, &uuid, &name)?;
    stream_initial_chunks(stream, &mut view)?;

    // Register only once the player is really in the world, so a failed join
    // never leaves a ghost in `/list`.
    let handle = server.join(&name, uuid);
    let result = send_join_state(stream, &view).and_then(|_| {
        announce_join(server, &handle);
        play_loop(stream, &mut view, server, &handle)
    });
    server.leave(handle.id);
    announce_leave(server, &handle);
    println!(
        "[BCore] {name} disconnected ({} online)",
        server.player_count()
    );
    result
}

/// Send the chat/command/gameplay state a joining player needs: the command
/// tree, full health, the time of day and the abilities for its gamemode.
fn send_join_state(stream: &mut TcpStream, view: &PlayerView) -> Result<(), PacketError> {
    let mut out = Vec::new();
    out.extend_from_slice(&bcore_command_tree().encode());
    out.extend_from_slice(&encode_full_health());
    let age = world_age_ticks();
    out.extend_from_slice(&encode_time_of_day(age, view.day_time));
    out.extend_from_slice(&encode_abilities_for(view.game_mode));
    stream.write_all(&out)?;
    Ok(())
}

/// Tell everyone a player joined, and greet the player itself.
fn announce_join(server: &SharedServer, handle: &PlayerHandle) {
    let notice = encode_system_chat(
        &Component::colored(format!("{} joined the game", handle.name), "yellow"),
        false,
    );
    server.broadcast_except(handle.id, &notice);
    let online = server.player_count();
    server.send_to(
        handle.id,
        &encode_system_chat(
            &Component::colored(
                format!(
                    "Welcome to BCore, {}! {online} online. Type /help.",
                    handle.name
                ),
                "green",
            ),
            false,
        ),
    );
}

/// Tell everyone still connected that a player left.
fn announce_leave(server: &SharedServer, handle: &PlayerHandle) {
    let notice = encode_system_chat(
        &Component::colored(format!("{} left the game", handle.name), "yellow"),
        false,
    );
    server.broadcast_except(handle.id, &notice);
}

/// The world age in ticks, derived from the wall clock so every connection
/// agrees without a tick loop.
fn world_age_ticks() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 * TICKS_PER_SECOND)
        .unwrap_or(0)
}

fn config_replay(stream: &mut TcpStream) -> Result<(), PacketError> {
    let packets = parse_captured(CONFIG_PACKETS);
    for (pid, data) in &packets.items {
        let mut out = Vec::new();
        write_packet(&mut out, *pid, data);
        stream.write_all(&out)?;
        if *pid == CONFIG_SELECT_KNOWN_PACKS_ID {
            // Wait for the client's known-packs response before registries.
            loop {
                let (cpid, _) = read_frame(stream)?;
                if cpid == CONFIG_KNOWN_PACKS_RESPONSE_ID {
                    break;
                }
            }
        }
    }
    // Wait for the client's finish-configuration acknowledgement.
    loop {
        let (cpid, _) = read_frame(stream)?;
        if cpid == CONFIG_FINISH_ID {
            break;
        }
    }
    Ok(())
}

/// Replay the captured play packets, skipping the recorded chunk batch, the
/// packets BCore now generates itself, and the capture's trailing kick; returns
/// a view centred on the spawn position.
fn play_replay(
    stream: &mut TcpStream,
    uuid: &[u8; 16],
    name: &str,
) -> Result<PlayerView, PacketError> {
    let packets = parse_captured(PLAY_PACKETS);
    let mut view = PlayerView::new(0.0, 0.0, 0.0);
    for (pid, data) in &packets.items {
        match *pid {
            // The world is generated natively; drop the captured 3x3 batch so the
            // streamer owns every chunk the client holds.
            CB_MAP_CHUNK | CB_CHUNK_BATCH_START | CB_CHUNK_BATCH_FINISHED => continue,
            // These are now built natively in `send_join_state`, from BCore's own
            // state rather than the capture's. Replaying them too would send the
            // client two command trees and two clocks.
            CB_DECLARE_COMMANDS | CB_UPDATE_HEALTH | CB_UPDATE_TIME | CB_ABILITIES => continue,
            // The capture ends with vanilla kicking the capture client. Replaying
            // it would disconnect every player the moment they joined.
            PLAY_KICK_DISCONNECT_ID => continue,
            _ => {}
        }

        if *pid == PLAY_PLAYER_INFO_ID && data.len() > 2 {
            let rebuilt = rebuild_player_info(uuid, name);
            let mut out = Vec::new();
            write_packet(&mut out, *pid, &rebuilt);
            stream.write_all(&out)?;
        } else {
            let mut out = Vec::new();
            write_packet(&mut out, *pid, data);
            stream.write_all(&out)?;
        }

        if *pid == PLAY_POSITION_ID {
            if let Some(spawn) = parse_spawn_position(data) {
                view = PlayerView::new(spawn.0, spawn.1, spawn.2);
            }
            // Wait for the client's teleport confirmation before chunks.
            loop {
                let (cpid, cdata) = read_frame(stream)?;
                if cpid == PLAY_TELEPORT_CONFIRM_ID {
                    break;
                }
                view.apply_movement(cpid, &cdata);
            }
        }
    }
    Ok(view)
}

/// Read the spawn `(x, y, z)` out of a clientbound `position` payload
/// (`teleportId` varint followed by three big-endian f64s).
fn parse_spawn_position(data: &[u8]) -> Option<(f64, f64, f64)> {
    let (_, consumed) = bcore_core::varint::decode_varint(data).ok()?;
    let body = data.get(consumed..consumed + 24)?;
    let f = |at: usize| f64::from_be_bytes(body[at..at + 8].try_into().expect("checked length"));
    Some((f(0), f(8), f(16)))
}

/// Send the first chunk batch so the player lands on solid ground.
///
/// The captured `position` packet spawns the player at the superflat surface
/// (y = -60). Real terrain sits far above that, so the player is teleported to
/// the generated surface *before* the chunks go out — otherwise they would spawn
/// buried in stone and suffocate.
fn stream_initial_chunks(stream: &mut TcpStream, view: &mut PlayerView) -> Result<(), PacketError> {
    let world = crate::world_state::shared();
    let (x, y, z) = world.spawn_position(view.x, view.z);
    if (y - view.y).abs() > 0.5 {
        let frame = view.teleport(x, y, z);
        stream.write_all(&frame)?;
        view.spawn = (x, y, z);
    }

    // Bound the very first batch too, so a fresh join does not burst the whole
    // 41x41 view at once (the play loop streams the rest 64 chunks per tick).
    view.set_chunk_batch_size(64);
    let sent = view.stream_chunks(stream)?;
    let (cx, cz) = view.chunk();
    println!(
        "[BCore] streamed {sent} chunks around ({cx}, {cz}); spawn y={:.1} (seed {})",
        view.y,
        world.seed()
    );
    Ok(())
}

/// Play loop: track movement, stream chunks, handle chat/commands, keep alive.
///
/// Also drains the player's shared outbox on every pass, which is how messages
/// other players' threads produced reach this socket. Only this thread ever
/// writes to `stream`.
fn play_loop(
    stream: &mut TcpStream,
    view: &mut PlayerView,
    server: &SharedServer,
    handle: &PlayerHandle,
) -> Result<(), PacketError> {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(PacketError::Io)?;
    let mut last_keepalive = Instant::now();
    let mut keepalive_id: i64 = 0;
    let mut last_chunk = view.chunk();
    view.set_chunk_batch_size(64);

    loop {
        if handle.is_kicked() || server.is_shutting_down() {
            // Flush whatever was queued (usually the kick notice) and close.
            let pending = handle.outbox.drain();
            if !pending.is_empty() {
                let _ = stream.write_all(&pending);
            }
            let reason = if server.is_shutting_down() {
                "Server closed"
            } else {
                "Kicked by an operator"
            };
            let mut kick = Vec::new();
            write_packet(
                &mut kick,
                PLAY_KICK_DISCONNECT_ID,
                &crate::nbt::encode_text(reason),
            );
            let _ = stream.write_all(&kick);
            break;
        }

        match read_frame(stream) {
            Ok((pid, data)) => {
                if view.apply_movement(pid, &data) {
                    let current = view.chunk();
                    if current != last_chunk {
                        last_chunk = current;
                        match view.stream_chunks(stream) {
                            Ok(sent) if sent > 0 => {
                                println!(
                                    "[BCore] player entered chunk ({}, {}): streamed {sent} chunks",
                                    current.0, current.1
                                );
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                } else if let Some(input) = parse_chat_input(pid, &data) {
                    if handle_chat_input(stream, view, server, handle, input).is_err() {
                        break;
                    }
                    last_chunk = view.chunk();
                } else if pid == SB_CHANGE_GAMEMODE {
                    // F3+F4 debug screen: the client asks to switch gamemode.
                    if let Some(mode) = parse_gamemode(&data) {
                        println!("[BCore] change_gamemode (F3+F4) -> {mode:?}");
                        view.game_mode = mode;
                        // Vanilla order: abilities, player_info, game_state_change, abilities.
                        let packets = encode_gamemode_switch(&handle.uuid, mode);
                        let player_info = &packets[1];
                        for packet in &packets {
                            stream.write_all(packet)?;
                        }
                        server.broadcast_except(handle.id, player_info);
                    }
                } else if pid == SB_PING_REQUEST {
                    // ping_request expects a ping_response echo.
                    let mut out = Vec::new();
                    write_packet(&mut out, CB_PING_RESPONSE, &data);
                    if stream.write_all(&out).is_err() {
                        break;
                    }
                } else if pid == SB_CHUNK_BATCH_RECEIVED && data.len() >= 4 {
                    // Vanilla sends the desired chunks/tick as an f32.
                    let desired = f32::from_be_bytes(data[..4].try_into().expect("checked length"));
                    if desired.is_finite() && desired >= 1.0 {
                        view.set_chunk_batch_size(desired.floor() as usize);
                    }
                }
                // Client keep-alive (0x1c), pong (0x2d), player_loaded (0x2c),
                // message_acknowledgement (0x06) and chat_session_update (0x0a)
                // need no reply.
            }
            Err(PacketError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break, // peer closed or sent something unreadable
        }

        // Continue a large teleport/movement stream on each tick. The first
        // batch is deliberately small; this prevents a 1681-chunk burst.
        if view.has_pending_chunks() {
            match view.stream_chunks(stream) {
                Ok(_) => {}
                Err(_) => break,
            }
        }

        // Deliver anything other players queued for us.
        let pending = handle.outbox.drain();
        if !pending.is_empty() && stream.write_all(&pending).is_err() {
            break;
        }

        if last_keepalive.elapsed() >= Duration::from_secs(10) {
            keepalive_id += 1;
            let mut out = Vec::new();
            write_packet(&mut out, PLAY_KEEP_ALIVE_ID, &keepalive_id.to_be_bytes());
            if stream.write_all(&out).is_err() {
                break;
            }
            last_keepalive = Instant::now();
        }
    }
    Ok(())
}

/// Handle one chat message or command from this player.
fn handle_chat_input(
    stream: &mut TcpStream,
    view: &mut PlayerView,
    server: &SharedServer,
    handle: &PlayerHandle,
    input: ChatInput,
) -> Result<(), PacketError> {
    match input {
        ChatInput::Message(message) => {
            if message.is_empty() {
                return Ok(());
            }
            println!("[BCore] <{}> {message}", handle.name);
            // Offline mode cannot sign messages, so the unsigned player_chat
            // form vanilla itself uses without a chat session is sent.
            let packet = encode_player_chat(
                server.next_chat_index(),
                &handle.uuid,
                &handle.name,
                &message,
                millis_now(),
            );
            // The sender sees its own message on this thread; everyone else gets
            // it through their own outbox.
            stream.write_all(&packet)?;
            server.broadcast_except(handle.id, &packet);
            Ok(())
        }
        ChatInput::Command(command) => {
            println!("[BCore] {} ran /{command}", handle.name);
            run_command(stream, view, server, handle, &command)
        }
    }
}

/// Execute a command and apply its packets and effects.
fn run_command(
    stream: &mut TcpStream,
    view: &mut PlayerView,
    server: &SharedServer,
    handle: &PlayerHandle,
    command: &str,
) -> Result<(), PacketError> {
    let online = server.player_names();
    let ctx = CommandContext {
        sender_name: &handle.name,
        online: &online,
        max_players: MAX_PLAYERS,
        seed: WORLD_SEED,
        spawn: view.spawn,
        is_op: server.is_op(&handle.name),
    };
    let outcome = command::execute(command, &ctx);

    for packet in &outcome.packets {
        match packet.destination {
            Destination::Sender => stream.write_all(&packet.bytes)?,
            Destination::Everyone => {
                stream.write_all(&packet.bytes)?;
                server.broadcast_except(handle.id, &packet.bytes);
            }
            Destination::Others => server.broadcast_except(handle.id, &packet.bytes),
        }
    }

    for effect in &outcome.effects {
        apply_effect(stream, view, server, handle, effect)?;
    }
    Ok(())
}

/// Apply one command effect: state change plus the packets it implies.
fn apply_effect(
    stream: &mut TcpStream,
    view: &mut PlayerView,
    server: &SharedServer,
    handle: &PlayerHandle,
    effect: &Effect,
) -> Result<(), PacketError> {
    match effect {
        Effect::SetGameMode(mode) => {
            view.game_mode = *mode;
            let packets = encode_gamemode_switch(&handle.uuid, *mode);
            let player_info = &packets[1];
            for packet in &packets {
                stream.write_all(packet)?;
            }
            server.broadcast_except(handle.id, player_info);
            Ok(())
        }
        Effect::Teleport { x, y, z } => {
            let packet = view.teleport(*x, *y, *z);
            stream.write_all(&packet)?;
            // Wait for the client's teleport confirmation before streaming the
            // new view, so the chunks land after the client has actually moved.
            // Give up after ~5s (50ms read timeout * 100) rather than hanging.
            for _ in 0..100 {
                match read_frame(stream) {
                    Ok((cpid, cdata)) => {
                        if cpid == PLAY_TELEPORT_CONFIRM_ID {
                            break;
                        }
                        view.apply_movement(cpid, &cdata);
                    }
                    Err(PacketError::Io(e))
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => return Ok(()), // peer closed; nothing more to do
                }
            }
            view.stream_chunks(stream)?;
            Ok(())
        }
        Effect::SetDayTime(ticks) => {
            view.day_time = *ticks;
            let packet = encode_set_day_time(world_age_ticks(), *ticks);
            stream.write_all(&packet)?;
            // The world clock is shared, so everyone sees the change.
            server.broadcast_except(handle.id, &packet);
            Ok(())
        }
        Effect::Kick(name) => {
            if let Some(target) = server.find_by_name(name) {
                let notice = encode_system_chat(
                    &Component::colored(format!("{name} was kicked from the game"), "yellow"),
                    false,
                );
                server.broadcast_except(target.id, &notice);
                target
                    .outbox
                    .push(&encode_system_message("You were kicked."));
                target.kick();
            }
            Ok(())
        }
        Effect::SetOp { name, op } => {
            if *op {
                server.add_op(name);
                println!("[BCore] {name} is now a server operator");
            } else {
                server.remove_op(name);
                println!("[BCore] {name} is no longer a server operator");
            }
            server.save_ops();
            Ok(())
        }
        Effect::Stop => {
            let notice = encode_profileless_chat(
                "Server shutting down",
                CHAT_TYPE_SAY_COMMAND,
                &handle.name,
            );
            server.broadcast(&notice);
            server.request_shutdown();
            println!("[BCore] /stop issued by {}", handle.name);
            Ok(())
        }
    }
}

/// Current Unix time in milliseconds (the `player_chat` timestamp).
fn millis_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_position_is_read_from_the_captured_packet() {
        let packets = parse_captured(PLAY_PACKETS);
        let position = packets
            .items
            .iter()
            .find(|(pid, _)| *pid == PLAY_POSITION_ID)
            .map(|(_, data)| data.clone())
            .expect("capture contains a position packet");
        let spawn = parse_spawn_position(&position).expect("parses");
        assert_eq!(spawn, (10.5, -60.0, -3.5));
        // The spawn chunk must match the centre vanilla itself announced.
        let view = PlayerView::new(spawn.0, spawn.1, spawn.2);
        assert_eq!(view.chunk(), (0, -1));
    }

    #[test]
    fn short_position_payloads_do_not_panic() {
        assert!(parse_spawn_position(&[]).is_none());
        assert!(parse_spawn_position(&[0x01, 0x00]).is_none());
    }

    #[test]
    fn replay_drops_chunks_natively_generated_packets_and_the_capture_kick() {
        let packets = parse_captured(PLAY_PACKETS);
        // The capture really does contain all of these, otherwise the filters
        // below are dead code.
        assert!(packets.items.iter().any(|(pid, _)| *pid == CB_MAP_CHUNK));
        assert!(packets
            .items
            .iter()
            .any(|(pid, _)| *pid == PLAY_KICK_DISCONNECT_ID));
        for pid in [
            CB_DECLARE_COMMANDS,
            CB_UPDATE_HEALTH,
            CB_UPDATE_TIME,
            CB_ABILITIES,
        ] {
            assert!(
                packets.items.iter().any(|(captured, _)| *captured == pid),
                "capture should contain 0x{pid:02x}"
            );
        }
        let replayed = packets
            .items
            .iter()
            .filter(|(pid, _)| {
                !matches!(
                    *pid,
                    CB_MAP_CHUNK
                        | CB_CHUNK_BATCH_START
                        | CB_CHUNK_BATCH_FINISHED
                        | CB_DECLARE_COMMANDS
                        | CB_UPDATE_HEALTH
                        | CB_UPDATE_TIME
                        | CB_ABILITIES
                        | PLAY_KICK_DISCONNECT_ID
                )
            })
            .count();
        // 38 captured packets - 9 chunks - batch start - batch finished - kick
        // - declare_commands - update_health - update_time - abilities.
        assert_eq!(replayed, packets.items.len() - 16);
    }

    #[test]
    fn the_natively_generated_join_packets_are_not_replayed_from_the_capture() {
        // Each of these is now produced by `send_join_state` instead, so the
        // client must never receive the captured version.
        let packets = parse_captured(PLAY_PACKETS);
        let captured_tree = packets
            .items
            .iter()
            .find(|(pid, _)| *pid == CB_DECLARE_COMMANDS)
            .map(|(_, data)| data.clone())
            .expect("capture has a command tree");
        let ours = bcore_command_tree().encode();
        assert!(
            !ours.ends_with(&captured_tree),
            "BCore must send its own tree, not vanilla's"
        );
        // Vanilla's non-op tree has 26 nodes; BCore's has its own count.
        assert_eq!(captured_tree[0], 26);
    }
}

/// Rebuild the `player_info` add-player packet with the given identity.
///
/// Template structure (action 0xff, all flags): action(u8) + count(varint) +
/// uuid(16) + name(string) + properties(varint 0) + trailing fixed fields
/// (chatSession, gamemode, listed, latency, displayName, listPriority, showHat).
fn rebuild_player_info(uuid: &[u8; 16], name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0xff); // action = all flags
    encode_varint(1, &mut out); // count
    out.extend_from_slice(uuid);
    write_string(name, &mut out);
    encode_varint(0, &mut out); // properties
                                // chatSession=false, gamemode=0(survival), listed=true, latency=0,
                                // displayName=none, listPriority=0, showHat=false
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    out
}
