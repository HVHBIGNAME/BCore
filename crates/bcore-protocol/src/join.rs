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

use crate::packet::{read_frame, read_string, write_packet, write_string, PacketError};
use crate::world::{
    PlayerView, CB_CHUNK_BATCH_FINISHED, CB_CHUNK_BATCH_START, CB_MAP_CHUNK, CB_PING_RESPONSE,
    SB_PING_REQUEST,
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
pub fn run_login_and_join(stream: &mut TcpStream) -> Result<(), PacketError> {
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
    play_loop(stream, &mut view)
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

/// Replay the captured play packets, skipping the recorded chunk batch and the
/// capture's trailing kick, and return a view centred on the spawn position.
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
fn stream_initial_chunks(stream: &mut TcpStream, view: &mut PlayerView) -> Result<(), PacketError> {
    let sent = view.stream_chunks(stream)?;
    let (cx, cz) = view.chunk();
    println!("[BCore] streamed {sent} chunks around ({cx}, {cz})");
    Ok(())
}

/// Play loop: track movement, stream chunks on chunk change, keep the connection alive.
fn play_loop(stream: &mut TcpStream, view: &mut PlayerView) -> Result<(), PacketError> {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(PacketError::Io)?;
    let mut last_keepalive = Instant::now();
    let mut keepalive_id: i64 = 0;
    let mut last_chunk = view.chunk();

    loop {
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
                } else if pid == SB_PING_REQUEST {
                    // ping_request expects a ping_response echo.
                    let mut out = Vec::new();
                    write_packet(&mut out, CB_PING_RESPONSE, &data);
                    if stream.write_all(&out).is_err() {
                        break;
                    }
                }
                // Client keep-alive (0x1c), pong (0x2d), chunk_batch_received
                // (0x0b) and player_loaded (0x2c) need no reply.
            }
            Err(PacketError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break, // peer closed or sent something unreadable
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
    fn replay_drops_chunks_and_the_capture_kick() {
        let packets = parse_captured(PLAY_PACKETS);
        // The capture really does contain both, otherwise these filters are dead code.
        assert!(packets.items.iter().any(|(pid, _)| *pid == CB_MAP_CHUNK));
        assert!(packets
            .items
            .iter()
            .any(|(pid, _)| *pid == PLAY_KICK_DISCONNECT_ID));
        let replayed = packets
            .items
            .iter()
            .filter(|(pid, _)| {
                !matches!(
                    *pid,
                    CB_MAP_CHUNK
                        | CB_CHUNK_BATCH_START
                        | CB_CHUNK_BATCH_FINISHED
                        | PLAY_KICK_DISCONNECT_ID
                )
            })
            .count();
        // 38 captured packets - 9 chunks - batch start - batch finished - kick.
        assert_eq!(replayed, packets.items.len() - 12);
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
