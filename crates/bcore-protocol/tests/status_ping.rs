//! End-to-end server-list ping test over a real TCP socket.

use std::io::{Cursor, Write};
use std::net::TcpListener;

use bcore_core::varint::encode_varint;
use bcore_protocol::packet::{read_frame, read_string, write_packet, write_string};
use bcore_protocol::server;

#[test]
fn server_list_status_and_ping() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");

    std::thread::spawn(move || server::run(listener));

    let mut stream = std::net::TcpStream::connect(addr).expect("connect");
    stream.set_nodelay(true).ok();

    // Handshake -> status.
    let mut hs = Vec::new();
    encode_varint(776, &mut hs);
    write_string("127.0.0.1", &mut hs);
    hs.extend_from_slice(&25565u16.to_be_bytes());
    encode_varint(1, &mut hs);
    let mut frame = Vec::new();
    write_packet(&mut frame, 0x00, &hs);
    stream.write_all(&frame).expect("write handshake");

    // Status request (empty payload).
    let mut req = Vec::new();
    write_packet(&mut req, 0x00, &[]);
    stream.write_all(&req).expect("write status request");

    // Read status response (payload is a length-prefixed String field).
    let (id, data) = read_frame(&mut stream).expect("read status response");
    assert_eq!(id, 0x00);
    let mut cursor = Cursor::new(data);
    let json_str = read_string(&mut cursor, 32767).expect("status json string");
    let json: serde_json::Value = serde_json::from_str(&json_str).expect("valid json");
    assert_eq!(json["version"]["protocol"], 776);
    assert_eq!(json["version"]["name"], "26.2");

    // Ping -> pong.
    let payload = 0x1234_5678_9abc_def0u64 as i64;
    let mut ping = Vec::new();
    write_packet(&mut ping, 0x01, &payload.to_be_bytes());
    stream.write_all(&ping).expect("write ping");

    let (id, data) = read_frame(&mut stream).expect("read pong");
    assert_eq!(id, 0x01);
    assert_eq!(data.len(), 8);
    assert_eq!(i64::from_be_bytes(data[..8].try_into().unwrap()), payload);
}
