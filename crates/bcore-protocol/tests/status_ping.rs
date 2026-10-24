//! End-to-end server-list ping test over a real TCP socket.

use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use bcore_core::varint::encode_varint;
use bcore_protocol::packet::{read_frame, read_string, write_packet, write_string};
use bcore_protocol::server;
use bcore_protocol::shared::{new_shared_server, SharedServer};

struct TestServer {
    addr: SocketAddr,
    state: SharedServer,
    worker: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let state = new_shared_server();
        let owned = Arc::clone(&state);
        let worker = Some(std::thread::spawn(move || {
            server::run_with_state(listener, owned)
        }));
        Self {
            addr,
            state,
            worker,
        }
    }

    fn connect(&self, protocol: i32, next_state: i32) -> TcpStream {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut handshake = Vec::new();
        encode_varint(protocol, &mut handshake);
        write_string("127.0.0.1", &mut handshake);
        handshake.extend_from_slice(&self.addr.port().to_be_bytes());
        encode_varint(next_state, &mut handshake);
        send(&mut stream, 0x00, &handshake);
        stream
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.request_shutdown();
        if let Some(worker) = self.worker.take() {
            worker.join().expect("server stopped");
        }
    }
}

fn send(stream: &mut TcpStream, id: i32, data: &[u8]) {
    let mut frame = Vec::new();
    write_packet(&mut frame, id, data);
    stream.write_all(&frame).unwrap();
}

#[test]
fn server_list_status_and_ping() {
    let server = TestServer::start();
    for protocol in [775, 776, 777] {
        let mut stream = server.connect(protocol, 1);

        send(&mut stream, 0x00, &[]);
        let (id, data) = read_frame(&mut stream).expect("read status response");
        assert_eq!(id, 0x00);
        let mut cursor = Cursor::new(data.as_slice());
        let json_str = read_string(&mut cursor, 32767).expect("status json string");
        let json: serde_json::Value = serde_json::from_str(&json_str).expect("valid json");
        assert_eq!(json["version"]["protocol"], 776);
        assert_eq!(json["version"]["name"], "26.2");
        assert_eq!(cursor.position() as usize, data.len());

        let payload = 0x1234_5678_9abc_def0u64 as i64;
        send(&mut stream, 0x01, &payload.to_be_bytes());
        let (id, data) = read_frame(&mut stream).expect("read pong");
        assert_eq!(id, 0x01);
        assert_eq!(data.len(), 8);
        assert_eq!(i64::from_be_bytes(data[..8].try_into().unwrap()), payload);
    }
    assert_eq!(server.state.player_count(), 0);
}

#[test]
fn only_login_rejects_unsupported_protocol_versions() {
    let server = TestServer::start();
    for protocol in [775, 777] {
        let mut stream = server.connect(protocol, 2);
        let (id, data) = read_frame(&mut stream).expect("login disconnect");
        assert_eq!(id, 0x00);
        let mut cursor = Cursor::new(data.as_slice());
        let reason = read_string(&mut cursor, 32767).unwrap();
        let reason: serde_json::Value = serde_json::from_str(&reason).unwrap();
        let text = reason["text"].as_str().unwrap();
        assert!(text.contains("26.2"));
        assert!(text.contains("776"));
        assert!(text.contains(&protocol.to_string()));
        assert_eq!(cursor.position() as usize, data.len());
        assert_eq!(
            stream.read(&mut [0u8; 1]).unwrap(),
            0,
            "disconnect closes login"
        );
    }
    assert_eq!(server.state.player_count(), 0);
    assert!(server.state.world().store().is_none());
}
