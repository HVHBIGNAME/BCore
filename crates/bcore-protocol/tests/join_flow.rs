//! End-to-end join-flow integration test.
//!
//! Unlike the earlier Python probe, this PARSES packet payloads (not just IDs):
//! it decodes the login-success packet including the 16-byte profile-id field
//! (added in 1.21.2+), and verifies the player reaches play state with chunks.

use std::io::{Cursor, Read, Write};
use std::net::TcpListener;

use bcore_core::varint::encode_varint;
use bcore_protocol::packet::{read_frame, read_string, read_varint, write_packet, write_string};
use bcore_protocol::server;

#[test]
fn join_flow_reaches_play_state() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || server::run(listener));

    let mut stream = std::net::TcpStream::connect(addr).expect("connect");
    stream.set_nodelay(true).ok();

    // Handshake -> login (protocol 776).
    let mut hs = Vec::new();
    encode_varint(776, &mut hs);
    write_string("127.0.0.1", &mut hs);
    hs.extend_from_slice(&addr.port().to_be_bytes());
    encode_varint(2, &mut hs);
    let mut frame = Vec::new();
    write_packet(&mut frame, 0x00, &hs);
    stream.write_all(&frame).expect("handshake");

    // Login start: name + 16-byte UUID.
    let mut ls = Vec::new();
    write_string("TestPlayer", &mut ls);
    ls.extend_from_slice(&[0xAB; 16]);
    let mut frame = Vec::new();
    write_packet(&mut frame, 0x00, &ls);
    stream.write_all(&frame).expect("login start");

    // Login success: parse uuid + name + properties + profile-id (16 bytes).
    let (pid, data) = read_frame(&mut stream).expect("login success");
    assert_eq!(pid, 0x02, "expected login success");
    let mut cur = Cursor::new(data);
    let mut uuid = [0u8; 16];
    cur.read_exact(&mut uuid).expect("uuid");
    let name = read_string(&mut cur, 16).expect("name");
    assert_eq!(name, "TestPlayer");
    let props = read_varint(&mut cur).expect("props");
    assert_eq!(props, 0, "offline login has no properties");
    let mut profile_id = [0u8; 16];
    cur.read_exact(&mut profile_id).expect("profile id");
    // profile id must be a valid v4 UUID (version nibble == 4).
    assert_eq!(profile_id[6] >> 4, 4, "profile id must be uuid v4");
    assert_eq!(
        cur.position() as usize,
        uuid.len() + name.len() + 1 + 1 + 16,
        "login success must be fully consumed"
    );

    // Login acknowledged.
    let mut ack = Vec::new();
    write_packet(&mut ack, 0x03, &[]);
    stream.write_all(&ack).expect("login ack");

    // Configuration: collect registry-data packets, respond to known-packs.
    let mut registry_count = 0;
    loop {
        let (pid, _) = read_frame(&mut stream).expect("config packet");
        match pid {
            0x07 => registry_count += 1,
            0x0e => {
                let mut resp = Vec::new();
                write_packet(&mut resp, 0x07, &[0x00]); // empty known packs
                stream.write_all(&resp).expect("known packs response");
            }
            0x03 => break, // finish configuration
            _ => {}
        }
    }
    assert_eq!(registry_count, 29, "expected 29 registry-data packets");

    // Finish configuration ack -> play state.
    let mut fin = Vec::new();
    write_packet(&mut fin, 0x03, &[]);
    stream.write_all(&fin).expect("finish config");

    // Play: verify login (JoinGame), position, player info, and chunk batch.
    let mut saw_login = false;
    let mut saw_position = false;
    let mut saw_player_info = false;
    let mut saw_chunk = false;
    let mut saw_chunk_batch_end = false;
    while !saw_chunk_batch_end {
        let (pid, data) = read_frame(&mut stream).expect("play packet");
        match pid {
            0x31 => saw_login = true,
            0x48 => {
                saw_position = true;
                // teleport confirm: echo the leading varint teleport id.
                let (_, n) = bcore_core::varint::decode_varint(&data).unwrap();
                let mut resp = Vec::new();
                write_packet(&mut resp, 0x00, &data[..n]);
                stream.write_all(&resp).expect("teleport confirm");
            }
            0x46 => saw_player_info = true,
            0x2d => saw_chunk = true,
            0x0b => saw_chunk_batch_end = true,
            _ => {}
        }
    }
    assert!(saw_login, "missing JoinGame");
    assert!(saw_position, "missing position");
    assert!(saw_player_info, "missing player info");
    assert!(saw_chunk, "missing chunk data");
}
