//! Login → configuration → play join flow (offline mode) for protocol 776.
//!
//! The configuration and play packets were captured verbatim from the official
//! vanilla 26.2 server (flat world, offline mode) and are replayed here. Only
//! the player identity (uuid + name) is substituted at runtime. This is a
//! bootstrap path to reach a joinable server; the long-term plan is to generate
//! the registry and chunk data natively (see `bcore-registry`).

use std::io::{Cursor, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bcore_core::varint::encode_varint;

use crate::packet::{read_frame, read_string, write_packet, write_string, PacketError};

pub const LOGIN_SUCCESS_ID: i32 = 0x02;
pub const LOGIN_ACKNOWLEDGED_ID: i32 = 0x03;
const CONFIG_SELECT_KNOWN_PACKS_ID: i32 = 0x0e;
const CONFIG_KNOWN_PACKS_RESPONSE_ID: i32 = 0x07;
const CONFIG_FINISH_ID: i32 = 0x03;
const PLAY_POSITION_ID: i32 = 0x48;
const PLAY_PLAYER_INFO_ID: i32 = 0x46;
const PLAY_KEEP_ALIVE_ID: i32 = 0x2c;
const PLAY_TELEPORT_CONFIRM_ID: i32 = 0x00;

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
    play_replay(stream, &uuid, &name)?;
    idle_loop(stream)
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

fn play_replay(stream: &mut TcpStream, uuid: &[u8; 16], name: &str) -> Result<(), PacketError> {
    let packets = parse_captured(PLAY_PACKETS);
    for (pid, data) in &packets.items {
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
            // Wait for the client's teleport confirmation before chunks.
            loop {
                let (cpid, _) = read_frame(stream)?;
                if cpid == PLAY_TELEPORT_CONFIRM_ID {
                    break;
                }
            }
        }
    }
    Ok(())
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

/// Minimal keep-alive loop: respond to keep-alives and periodically send one
/// so the client does not time out. Movement packets are ignored for now.
fn idle_loop(stream: &mut TcpStream) -> Result<(), PacketError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(PacketError::Io)?;
    let mut last_keepalive = Instant::now();
    let mut keepalive_id: i64 = 0;
    loop {
        match read_frame(stream) {
            Ok((pid, data)) => {
                // Client keep-alive (0x1c) and pong (0x2d) are responses to our
                // keep-alive/ping and are ignored. A client ping_request (0x26)
                // expects a ping_response (0x3e) echo.
                if pid == 0x26 {
                    let mut out = Vec::new();
                    write_packet(&mut out, 0x3e, &data);
                    if stream.write_all(&out).is_err() {
                        break;
                    }
                }
            }
            Err(_) => {
                // timeout (or closed); fall through to keep-alive timer
            }
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
