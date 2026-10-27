//! Login → configuration → play join flow (offline mode) for protocol 776.
//!
//! Embedded configuration/play captures provide bootstrap state. Identity,
//! login, clocks and streamed terrain come from this connection's server owner.
//!
//! Chunks are **not** replayed: the captured `map_chunk` packets are dropped and
//! the world is streamed from [`crate::chunk`] / [`crate::world`] instead, so the
//! player can walk out of the original 3x3 capture. The capture's trailing
//! `kick_disconnect` (vanilla booting the capture client) is dropped as well.

use std::io::{Cursor, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod stream_flow;

use sha2::Digest;

use bcore_core::varint::encode_varint;

use crate::chat::{
    encode_player_chat, encode_profileless_chat, encode_system_chat, encode_system_message,
    parse_chat_input, parse_gamemode, ChatInput, CHAT_TYPE_SAY_COMMAND, SB_CHANGE_GAMEMODE,
};
use crate::command::{self, CommandContext, Destination, Effect};
use crate::commands::{bcore_command_tree, CB_DECLARE_COMMANDS};
use crate::gameplay::{
    encode_abilities_for, encode_full_health, encode_gamemode_switch, encode_player_info_add,
    encode_player_info_remove, GameMode, CB_ABILITIES, CB_UPDATE_HEALTH, CB_UPDATE_TIME,
};
use crate::nbt::Component;
use crate::packet::{
    read_frame, read_string, read_varint, write_packet, write_string, PacketError,
};
use crate::shared::{PlayerHandle, SharedServer};
use crate::world::{
    PlayerView, CB_CHUNK_BATCH_FINISHED, CB_CHUNK_BATCH_START, CB_MAP_CHUNK, CB_PING_RESPONSE,
    SB_CHUNK_BATCH_RECEIVED, SB_PING_REQUEST,
};

pub const LOGIN_SUCCESS_ID: i32 = 0x02;
pub const LOGIN_ACKNOWLEDGED_ID: i32 = 0x03;
const PLAY_LOGIN_ID: i32 = 0x31;
pub(crate) const PLAY_LOGIN_ENTITY_ID: i32 = 392;
const PLAY_SEA_LEVEL: i32 = 63;

/// Minecraft's `hashedSeed`: SHA-256 of the seed's little-endian bytes,
/// truncated to eight bytes and interpreted as a signed little-endian i64.
/// This matches the value the vanilla 26.1 (protocol 775) server sends for the
/// numeric seed (verified against a live capture: `846692123413862008` ->
/// `-1328540181389140506`).
pub fn hashed_seed(seed: i64) -> i64 {
    let digest = sha2::Sha256::digest(seed.to_le_bytes());
    i64::from_le_bytes(digest[..8].try_into().expect("SHA-256 digest is 32 bytes"))
}
const CONFIG_SELECT_KNOWN_PACKS_ID: i32 = 0x0e;
const CONFIG_KNOWN_PACKS_RESPONSE_ID: i32 = 0x07;
const CONFIG_FINISH_ID: i32 = 0x03;
const PLAY_POSITION_ID: i32 = 0x48;
const PLAY_PLAYER_INFO_ID: i32 = 0x46;
const PLAY_KEEP_ALIVE_ID: i32 = 0x2c;
const PLAY_TELEPORT_CONFIRM_ID: i32 = 0x00;
/// Clientbound `kick_disconnect`: present in the capture, never replayed.
const PLAY_KICK_DISCONNECT_ID: i32 = 0x20;
/// Captured recipe declarations are omitted after template-item decode failures.
/// Recipe-book packets have a separate replay policy.
const PLAY_DECLARE_RECIPES_ID: i32 = 0x85;

/// Default seed for compatibility callers. Live joins and `/seed` use the
/// connection's server-local world seed.
pub const WORLD_SEED: i64 = crate::world_state::DEFAULT_SEED;
/// Player slots announced in the status response and reported by `/list`.
pub const MAX_PLAYERS: usize = 20;

// Embedded vanilla 26.2 captures. Binary format:
// [u32 count][ (i32 pid, u32 len, bytes)... ]
const CONFIG_PACKETS: &[u8] = include_bytes!("../data/config_packets.bin");
const PLAY_PACKETS: &[u8] = include_bytes!("../data/play_packets.bin");

/// Read a login-start payload: player name + 16-byte UUID.
pub fn read_login_start<R: Read>(r: &mut R) -> Result<(String, [u8; 16]), PacketError> {
    let name = read_string(r, 16)?;
    if name.len() > 16 * 3 || name.encode_utf16().count() > 16 {
        return Err(PacketError::TooLarge(name.len()));
    }
    let mut uuid = [0u8; 16];
    r.read_exact(&mut uuid)?;
    Ok((name, uuid))
}

/// Encode profile identity/properties followed by the server-session UUID.
pub fn encode_login_success(uuid: &[u8; 16], name: &str, session_id: &[u8; 16]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(uuid);
    write_string(name, &mut data);
    encode_varint(0, &mut data); // properties count
    data.extend_from_slice(session_id);
    let mut out = Vec::new();
    write_packet(&mut out, LOGIN_SUCCESS_ID, &data);
    out
}

fn encode_play_login(view: &PlayerView, seed: i64, view_distance: i32) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&PLAY_LOGIN_ENTITY_ID.to_be_bytes());
    data.push(0); // hardcore
    encode_varint(3, &mut data);
    for name in [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ] {
        write_string(name, &mut data);
    }
    encode_varint(MAX_PLAYERS as i32, &mut data);
    encode_varint(view_distance, &mut data);
    encode_varint(view_distance, &mut data);
    data.extend_from_slice(&[0, 1, 0]); // reducedDebugInfo, respawnScreen, limitedCrafting
    encode_varint(0, &mut data); // overworld dimension
    write_string("minecraft:overworld", &mut data);
    data.extend_from_slice(&hashed_seed(seed).to_be_bytes());
    data.push(view.game_mode.id() as u8);
    data.push(0xff); // previous gamemode
    data.extend_from_slice(&[0, 0]); // debug, flat
    data.push(0); // last death location: absent (option<bool GlobalPos>)
    encode_varint(0, &mut data); // portal cooldown
    encode_varint(PLAY_SEA_LEVEL, &mut data);
    data.push(0); // onlineMode
    data.push(0); // enforcesSecureChat
    let mut out = Vec::new();
    write_packet(&mut out, PLAY_LOGIN_ID, &data);
    out
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
    if cursor.position() != cursor.get_ref().len() as u64 {
        return Err(PacketError::Malformed("trailing login-start payload"));
    }
    println!("[BCore] join: {name} (offline)");

    stream.write_all(&encode_login_success(&uuid, &name, server.session_id()))?;
    eprintln!("[BCore] login success sent for {name}");

    let (pid, data) = read_frame(stream)?;
    eprintln!("[BCore] login ack: pid=0x{pid:02x}");
    if pid != LOGIN_ACKNOWLEDGED_ID {
        return Err(PacketError::UnexpectedPacket(pid));
    }
    if !data.is_empty() {
        return Err(PacketError::Malformed(
            "login acknowledgement must be empty",
        ));
    }

    config_replay(stream)?;
    eprintln!("[BCore] config replay done");
    // Non-blocking reads for the play phase: teleport confirm may lag the chunks.
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    let mut view = play_replay(stream, &uuid, &name, server)?;

    // Register only once the player is really in the world, so a failed join
    // never leaves a ghost in `/list`.
    let handle = server.join(&name, uuid);
    let result = send_join_state(stream, &view, server, &handle).and_then(|_| {
        announce_join(server, &handle);
        let requests = server.world().payload_stream();
        let sent = stream_initial_chunks(stream, &mut view, server, &handle, &requests)?;
        play_loop(stream, &mut view, server, &handle, sent, &requests)
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
/// tree, full health, the time of day, the abilities for its gamemode, and the
/// tab list (a `player_info` add-player entry for every online player).
fn send_join_state(
    stream: &mut TcpStream,
    view: &PlayerView,
    server: &SharedServer,
    handle: &PlayerHandle,
) -> Result<(), PacketError> {
    let mut out = Vec::new();
    out.extend_from_slice(&encode_abilities_for(view.game_mode));
    out.extend_from_slice(&crate::gameplay::encode_permission_level(
        PLAY_LOGIN_ENTITY_ID,
        if server.is_op(&handle.name) { 4 } else { 0 },
    ));
    out.extend_from_slice(&bcore_command_tree().encode());
    out.extend_from_slice(&encode_full_health());
    // Tab list: every online player. The joiner gets its own gamemode; the
    // others are tracked as survival until gamemode changes are persisted.
    for player in server.players() {
        let mode = if player.id == handle.id {
            view.game_mode
        } else {
            GameMode::Survival
        };
        out.extend_from_slice(&encode_player_info_add(&player.uuid, &player.name, mode, 0));
    }
    stream.write_all(&out)?;
    server.send_clock(handle.id);
    stream.write_all(&handle.outbox.drain())?;
    Ok(())
}

/// Tell everyone a player joined, and greet the player itself.
fn announce_join(server: &SharedServer, handle: &PlayerHandle) {
    let notice = encode_system_chat(
        &Component::colored(format!("{} joined the game", handle.name), "yellow"),
        false,
    );
    server.broadcast_except(handle.id, &notice);
    // Add the joiner to everyone's tab list (it already added itself).
    let add = encode_player_info_add(&handle.uuid, &handle.name, GameMode::Survival, 0);
    server.broadcast_except(handle.id, &add);
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
    // Remove the leaver from everyone's tab list.
    server.broadcast_except(handle.id, &encode_player_info_remove(&handle.uuid));
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
                let (cpid, data) = read_frame(stream)?;
                if cpid == CONFIG_KNOWN_PACKS_RESPONSE_ID {
                    validate_known_packs_response(&data)?;
                    break;
                }
            }
        }
    }
    // Wait for the client's finish-configuration acknowledgement.
    loop {
        let (cpid, data) = read_frame(stream)?;
        if cpid == CONFIG_FINISH_ID {
            if !data.is_empty() {
                return Err(PacketError::Malformed(
                    "configuration acknowledgement must be empty",
                ));
            }
            break;
        }
    }
    Ok(())
}

fn skip_play_capture(id: i32) -> bool {
    matches!(
        id,
        CB_MAP_CHUNK
            | CB_CHUNK_BATCH_START
            | CB_CHUNK_BATCH_FINISHED
            | PLAY_DECLARE_RECIPES_ID
            | CB_DECLARE_COMMANDS
            | CB_UPDATE_HEALTH
            | CB_UPDATE_TIME
            | CB_ABILITIES
            | PLAY_LOGIN_ID
            | PLAY_KICK_DISCONNECT_ID
    )
}

fn validate_known_packs_response(data: &[u8]) -> Result<(), PacketError> {
    let mut cursor = Cursor::new(data);
    let count = read_varint(&mut cursor)?;
    if !(0..=64).contains(&count) {
        return Err(PacketError::Malformed("invalid known-packs count"));
    }
    for _ in 0..count {
        for _ in 0..3 {
            read_string(&mut cursor, 32767)?;
        }
    }
    if cursor.position() != data.len() as u64 {
        return Err(PacketError::Malformed("trailing known-packs payload"));
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
    server: &SharedServer,
) -> Result<PlayerView, PacketError> {
    let packets = parse_captured(PLAY_PACKETS);
    let mut view = PlayerView::new(0.0, 0.0, 0.0).with_view_distance(server.view_distance());
    stream.write_all(&encode_play_login(
        &view,
        server.world().seed(),
        server.view_distance(),
    ))?;
    eprintln!("[BCore] play/login sent");
    for (pid, data) in &packets.items {
        if skip_play_capture(*pid) {
            continue;
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
                view = PlayerView::new(spawn.0, spawn.1, spawn.2)
                    .with_view_distance(server.view_distance());
            }
            let mut cursor = Cursor::new(data.as_slice());
            view.expect_teleport(read_varint(&mut cursor)?);
            // Don't block on the teleport confirm here: the chunks must reach the
            // client immediately, and the confirm is handled later in play_loop.
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
fn stream_initial_chunks(
    stream: &mut TcpStream,
    view: &mut PlayerView,
    server: &SharedServer,
    handle: &PlayerHandle,
    requests: &crate::world_state::PayloadStream,
) -> Result<usize, PacketError> {
    let world = server.world();
    // Spawn on the nearest solid land near the live spawn (the captured
    // `position` packet pins a fixed spawn that may sit in the water).
    let (x, y, z) = world
        .try_land_spawn(view.x.floor() as i32, view.z.floor() as i32)
        .map_err(|error| PacketError::Io(std::io::Error::other(error)))?;
    if (y - view.y).abs() > 0.5 || (x - view.x).abs() > 0.5 || (z - view.z).abs() > 0.5 {
        let frame = view.teleport(x, y, z);
        handle.update_view(view, Some(view.delivery_chunks()));
        stream.write_all(&frame)?;
        view.spawn = (x, y, z);
    }

    // Stream the remaining view through subsequent bounded batches.
    requests.update(view.generation_chunks());
    let sent = stream_world_chunks(stream, view, server, handle, 4)?;
    let (cx, cz) = view.chunk();
    println!(
        "[BCore] streamed {sent} chunks around ({cx}, {cz}); spawn y={:.1} (seed {})",
        view.y,
        world.seed()
    );
    Ok(sent)
}

fn stream_world_chunks(
    stream: &mut TcpStream,
    view: &mut PlayerView,
    server: &SharedServer,
    handle: &PlayerHandle,
    budget: usize,
) -> Result<usize, PacketError> {
    let world = server.world();
    for (x, z) in view
        .generation_chunks()
        .into_iter()
        .take(crate::world_state::VIEW_LOOKAHEAD)
    {
        if let Some(error) = world.payload_error(x, z) {
            return Err(std::io::Error::other(error).into());
        }
    }
    view.stream_ready_chunks_from_with(stream, world, budget, |view, delivered| {
        handle.update_view(view, Some(delivered))
    })
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
    initial_chunks: usize,
    requests: &crate::world_state::PayloadStream,
) -> Result<(), PacketError> {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(PacketError::Io)?;
    let mut last_keepalive = Instant::now();
    let mut keepalive_id: i64 = 0;
    let mut reader = crate::packet::FrameReader::default();
    let mut actions = crate::blocks::PendingActions::default();
    let mut flow = stream_flow::ChunkFlow::new(Instant::now(), initial_chunks);
    handle.update_view(view, None);

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

        actions.poll()?;
        stream.set_read_timeout(Some(flow.time_until_poll(Instant::now())))?;
        match reader.read(stream) {
            Ok((pid, data)) => {
                if pid == PLAY_TELEPORT_CONFIRM_ID {
                    let mut cursor = Cursor::new(data.as_slice());
                    let id = read_varint(&mut cursor)?;
                    if cursor.position() != data.len() as u64 {
                        return Err(PacketError::Malformed("trailing teleport confirmation"));
                    }
                    view.confirm_teleport(id);
                } else if pid == crate::world::SB_PLAYER_LOADED {
                    if !data.is_empty() {
                        return Err(PacketError::Malformed(
                            "player-loaded payload must be empty",
                        ));
                    }
                    view.mark_player_loaded();
                } else if view.apply_movement(pid, &data) {
                    // The clock-paced sender below handles changes of view position.
                } else if let Some(input) = parse_chat_input(pid, &data) {
                    handle_chat_input(stream, view, server, handle, input)?;
                } else if pid == SB_CHANGE_GAMEMODE {
                    // F3+F4 debug screen: the client asks to switch gamemode.
                    if !server.is_op(&handle.name) {
                        continue;
                    }
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
                } else if let Some(packet) = crate::blocks::packets::parse(pid, &data)? {
                    actions.submit(server, handle, packet)?;
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
                    if desired.is_finite() && desired >= 0.0 {
                        flow.acknowledge(desired);
                    }
                }
                handle.update_view(view, None);
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

        // Deliver anything other players queued for us.
        let pending = handle.outbox.drain();
        if !pending.is_empty() && stream.write_all(&pending).is_err() {
            break;
        }

        let allowance = flow.poll(Instant::now());
        if view.teleport_pending() {
            requests.update(std::iter::empty());
        } else if let Some(allowance) = allowance {
            requests.update(view.generation_chunks());
            let sent = stream_world_chunks(stream, view, server, handle, allowance)?;
            flow.sent(sent);
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
        seed: server.world().seed(),
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
            handle.update_view(view, None);
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
            handle.update_view(view, Some(view.delivery_chunks()));
            stream.write_all(&packet)?;
            // The main play loop owns confirmation and defers the new terrain
            // batch without dropping actions or other frames while it waits.
            Ok(())
        }
        Effect::SetDayTime(ticks) => {
            server.set_day_time(*ticks);
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
    fn bootstrap_bytes_match_the_native_validated_26_2_publication() {
        let publication: serde_json::Value =
            serde_json::from_str(include_str!("../data/protocol_26_2.json")).unwrap();
        assert_eq!(publication["schema"], "bcore.protocol-bootstrap.v1");
        assert_eq!(publication["minecraft"], bcore_registry::MC_VERSION);
        assert_eq!(publication["protocol"], bcore_registry::PROTOCOL_VERSION);
        assert_eq!(publication["data_version"], bcore_registry::DATA_VERSION);
        assert_eq!(publication["jar_sha256"], bcore_registry::SERVER_JAR_SHA256);
        for (name, bytes) in [
            ("config_packets.bin", CONFIG_PACKETS),
            ("play_packets.bin", PLAY_PACKETS),
        ] {
            let receipt = &publication["files"][name];
            assert_eq!(receipt["bytes"].as_u64(), Some(bytes.len() as u64));
            assert_eq!(
                receipt["sha256"],
                format!("{:x}", sha2::Sha256::digest(bytes))
            );
            assert_eq!(
                receipt["packet_count"].as_u64(),
                Some(parse_captured(bytes).items.len() as u64)
            );
        }
        assert_eq!(publication["configuration_comparison"]["equal"], true);
        assert_eq!(publication["registry_bindings"]["biome_ids_match"], true);
        assert_eq!(
            publication["registry_bindings"]["biome_count"].as_u64(),
            Some(bcore_registry::BIOME_COUNT as u64)
        );
        let proof = &publication["bcore_validation"];
        assert_eq!(proof["coverage"].as_array().unwrap().len(), 3);
        for session in proof["coverage"].as_array().unwrap() {
            assert_eq!(session["decoded"], session["expected"]);
            assert!(session["decoded"].as_u64().unwrap() > 0);
            assert_eq!(session["coverage"]["network_registry_load"], true);
        }
        let persistence = &proof["persistence"];
        assert_eq!(persistence["status"], "pass");
        assert_ne!(
            persistence["mutation"]["state"],
            persistence["mutation"]["before_state"]
        );
        assert_eq!(
            persistence["mutation"]["sha256"],
            persistence["file_after_restart"]["sha256"]
        );
    }

    #[test]
    fn hashed_seed_matches_protocol_value() {
        assert_eq!(hashed_seed(846692123413862008), -1328540181389140506);
    }

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
        for id in [
            CB_MAP_CHUNK,
            CB_CHUNK_BATCH_START,
            CB_CHUNK_BATCH_FINISHED,
            CB_DECLARE_COMMANDS,
            CB_UPDATE_HEALTH,
            CB_UPDATE_TIME,
            CB_ABILITIES,
            PLAY_LOGIN_ID,
            PLAY_DECLARE_RECIPES_ID,
            PLAY_KICK_DISCONNECT_ID,
        ] {
            assert!(
                skip_play_capture(id),
                "captured packet 0x{id:02x} must be replaced or discarded"
            );
        }
        assert!(
            !skip_play_capture(PLAY_PLAYER_INFO_ID),
            "player info is rebuilt in the replay path"
        );
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
                                // displayName=none, listPriority=0, showHat=true
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]);
    out
}
