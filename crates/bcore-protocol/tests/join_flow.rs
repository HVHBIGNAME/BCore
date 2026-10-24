//! Protocol 776 login fields, strict admission, and server-local join state.

use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bcore_core::varint::encode_varint;
use bcore_protocol::chunk::{block_state, ChunkColumn};
use bcore_protocol::chunk_store::ChunkStore;
use bcore_protocol::packet::{read_frame, read_string, read_varint, write_packet, write_string};
use bcore_protocol::server;
use bcore_protocol::shared::{ServerState, SharedServer};
use bcore_protocol::world::chunks_in_view;
use bcore_protocol::world_state::World;

const SEED: i64 = -42;

struct TestServer {
    addr: SocketAddr,
    state: SharedServer,
    worker: Option<JoinHandle<()>>,
    root: PathBuf,
}

impl TestServer {
    fn start() -> Self {
        let state = ServerState::default();
        let suffix: String = state
            .session_id()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let parent = std::env::temp_dir().join("opencode");
        std::fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!("bcore-join-{}-{suffix}", std::process::id()));
        std::fs::create_dir(&root).expect("new isolated world");
        let store = ChunkStore::at(&root);
        store.prepare_world(SEED).unwrap();
        let mut column = ChunkColumn::flat();
        for z in 0..16 {
            for x in 0..16 {
                assert!(column.set(x, 64, z, block_state::STONE));
            }
        }
        for (x, z) in chunks_in_view(0, -1, 1) {
            store.save(x, z, &column).unwrap();
        }
        let state = Arc::new(
            ServerState::with_world(World::open(SEED, store).unwrap()).with_view_distance(1),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let owned = Arc::clone(&state);
        let worker = Some(std::thread::spawn(move || {
            server::run_with_state(listener, owned)
        }));
        Self {
            addr,
            state,
            worker,
            root,
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(self.addr).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
    }

    fn handshake(&self, stream: &mut TcpStream, trailing: &[u8]) {
        let mut payload = Vec::new();
        encode_varint(776, &mut payload);
        write_string("127.0.0.1", &mut payload);
        payload.extend_from_slice(&self.addr.port().to_be_bytes());
        encode_varint(2, &mut payload);
        payload.extend_from_slice(trailing);
        send(stream, 0x00, &payload);
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.request_shutdown();
        if let Some(worker) = self.worker.take() {
            worker.join().expect("server stopped");
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.state.world().pending_payloads() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.state.world().pending_payloads() == 0 {
            std::fs::remove_dir_all(&self.root).expect("remove isolated world");
        }
    }
}

fn send(stream: &mut TcpStream, id: i32, data: &[u8]) {
    let mut frame = Vec::new();
    write_packet(&mut frame, id, data);
    stream.write_all(&frame).unwrap();
}

fn login_start(name: &str, uuid: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    write_string(name, &mut payload);
    payload.extend_from_slice(uuid);
    payload
}

fn login_success(stream: &mut TcpStream, name: &str, expected_uuid: [u8; 16]) -> [u8; 16] {
    let (id, payload) = read_frame(stream).unwrap();
    assert_eq!(id, 0x02);
    let mut cur = Cursor::new(payload.as_slice());
    let mut uuid = [0; 16];
    cur.read_exact(&mut uuid).unwrap();
    assert_eq!(uuid, expected_uuid);
    assert_eq!(read_string(&mut cur, 16).unwrap(), name);
    assert_eq!(
        read_varint(&mut cur).unwrap(),
        0,
        "offline profile properties"
    );
    let mut session = [0; 16];
    cur.read_exact(&mut session)
        .expect("26.2 server-session UUID");
    assert_eq!(cur.position() as usize, payload.len());
    session
}

fn read_u8(cur: &mut Cursor<&[u8]>) -> u8 {
    let mut bytes = [0];
    cur.read_exact(&mut bytes).unwrap();
    bytes[0]
}

fn read_i64(cur: &mut Cursor<&[u8]>) -> i64 {
    let mut bytes = [0; 8];
    cur.read_exact(&mut bytes).unwrap();
    i64::from_be_bytes(bytes)
}

fn assert_play_login(payload: &[u8]) {
    let mut cur = Cursor::new(payload);
    let mut entity = [0; 4];
    cur.read_exact(&mut entity).unwrap();
    assert_eq!(i32::from_be_bytes(entity), 392);
    assert_eq!(read_u8(&mut cur), 0, "hardcore");
    assert_eq!(read_varint(&mut cur).unwrap(), 3);
    for dimension in [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ] {
        assert_eq!(read_string(&mut cur, 32767).unwrap(), dimension);
    }
    assert_eq!(read_varint(&mut cur).unwrap(), 20);
    assert_eq!(
        read_varint(&mut cur).unwrap(),
        1,
        "server-local view distance"
    );
    assert_eq!(read_varint(&mut cur).unwrap(), 1, "simulation distance");
    for flag in [0, 1, 0] {
        assert_eq!(read_u8(&mut cur), flag);
    }
    assert_eq!(
        read_varint(&mut cur).unwrap(),
        0,
        "overworld dimension type"
    );
    assert_eq!(read_string(&mut cur, 32767).unwrap(), "minecraft:overworld");
    assert_eq!(
        read_i64(&mut cur),
        bcore_protocol::join::hashed_seed(SEED),
        "server-local configured seed, not the default world"
    );
    assert_eq!(read_u8(&mut cur), 0, "survival");
    assert_eq!(read_u8(&mut cur), 0xff, "previous game mode");
    for flag in [0, 0, 0] {
        assert_eq!(read_u8(&mut cur), flag, "debug/flat/last-death flags");
    }
    assert_eq!(read_varint(&mut cur).unwrap(), 0, "portal cooldown");
    assert_eq!(read_varint(&mut cur).unwrap(), 63, "sea level");
    assert_eq!(read_u8(&mut cur), 0, "onlineMode precedes secure chat");
    assert_eq!(read_u8(&mut cur), 0, "enforcesSecureChat");
    assert_eq!(
        cur.position() as usize,
        payload.len(),
        "complete native field layout"
    );
}

fn assert_closed_without_packet(stream: &mut TcpStream) {
    let mut byte = [0];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        result => panic!("malformed admission emitted bytes or failed to close: {result:?}"),
    }
}

#[test]
fn join_flow_reaches_play_state() {
    let server = TestServer::start();
    let mut stream = server.connect();
    server.handshake(&mut stream, &[]);
    send(&mut stream, 0x00, &login_start("TestPlayer", &[0xAB; 16]));
    assert_eq!(
        login_success(&mut stream, "TestPlayer", [0xAB; 16]),
        *server.state.session_id()
    );
    send(&mut stream, 0x03, &[]);

    // Configuration: collect registry-data packets, respond to known-packs.
    let mut registry_count = 0;
    loop {
        let (pid, data) = read_frame(&mut stream).expect("config packet");
        match pid {
            0x07 => registry_count += 1,
            0x0e => {
                let mut cur = Cursor::new(data.as_slice());
                assert_eq!(read_varint(&mut cur).unwrap(), 1);
                for value in ["minecraft", "core", "26.2"] {
                    assert_eq!(read_string(&mut cur, 32767).unwrap(), value);
                }
                assert_eq!(cur.position() as usize, data.len());
                send(&mut stream, 0x07, &[0x00]);
            }
            0x03 => {
                assert!(data.is_empty());
                break;
            }
            _ => {}
        }
    }
    assert_eq!(registry_count, 29, "expected 29 registry-data packets");

    // Finish configuration ack -> play state.
    send(&mut stream, 0x03, &[]);

    // Play: verify login (JoinGame), position, player info, and chunk batch.
    let mut saw_login = false;
    let mut saw_position = false;
    let mut saw_player_info = false;
    let mut saw_chunk = false;
    let mut saw_chunk_batch_end = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !saw_chunk_batch_end {
        assert!(
            Instant::now() < deadline,
            "initial chunk batch did not finish"
        );
        let (pid, data) = read_frame(&mut stream).expect("play packet");
        match pid {
            0x31 => {
                assert_play_login(&data);
                saw_login = true;
            }
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

#[test]
fn server_session_uuid_is_reused_across_distinct_login_profiles() {
    let server = TestServer::start();
    let mut first = server.connect();
    server.handshake(&mut first, &[]);
    send(&mut first, 0x00, &login_start("First", &[1; 16]));
    let first_session = login_success(&mut first, "First", [1; 16]);
    let mut second = server.connect();
    server.handshake(&mut second, &[]);
    send(&mut second, 0x00, &login_start("Second", &[2; 16]));
    let second_session = login_success(&mut second, "Second", [2; 16]);
    assert_eq!(first_session, second_session);
    assert_ne!(first_session, [1; 16]);
    assert_ne!(first_session, [2; 16]);
    assert_eq!(
        server.state.player_count(),
        0,
        "login has not admitted play players"
    );
}

#[test]
fn trailing_handshake_payload_is_rejected_before_login() {
    let server = TestServer::start();
    let mut stream = server.connect();
    server.handshake(&mut stream, &[0]);
    assert_closed_without_packet(&mut stream);
    assert_eq!(server.state.player_count(), 0);
}

#[test]
fn trailing_or_truncated_login_start_payload_is_rejected() {
    let server = TestServer::start();
    for uuid in [&[3u8; 15][..], &[3u8; 17][..]] {
        let mut stream = server.connect();
        server.handshake(&mut stream, &[]);
        send(&mut stream, 0x00, &login_start("Malformed", uuid));
        assert_closed_without_packet(&mut stream);
    }
    assert_eq!(server.state.player_count(), 0);
}

#[test]
fn login_acknowledgement_must_have_an_empty_payload() {
    let server = TestServer::start();
    let mut stream = server.connect();
    server.handshake(&mut stream, &[]);
    send(&mut stream, 0x00, &login_start("Ack", &[4; 16]));
    login_success(&mut stream, "Ack", [4; 16]);
    send(&mut stream, 0x03, &[0]);
    assert_closed_without_packet(&mut stream);
    assert_eq!(server.state.player_count(), 0);
}

#[test]
fn login_start_applies_java_utf16_name_length_limits() {
    let server = TestServer::start();
    for name in ["x".repeat(17), "😀".repeat(9)] {
        let mut stream = server.connect();
        server.handshake(&mut stream, &[]);
        send(&mut stream, 0x00, &login_start(&name, &[5; 16]));
        assert_closed_without_packet(&mut stream);
    }
    let mut stream = server.connect();
    server.handshake(&mut stream, &[]);
    let name = "x".repeat(16);
    send(&mut stream, 0x00, &login_start(&name, &[5; 16]));
    login_success(&mut stream, &name, [5; 16]);
}
