//! Minimal TCP server: accept connections and handle handshake/status/login.
//!
//! One thread per connection, plus one piece of shared state
//! ([`crate::shared::SharedServer`]) so players can see each other in chat and
//! in `/list`. The accept loop stops once `/stop` has flagged a shutdown.

use std::io::{Cursor, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::Duration;

use bcore_core::version::{IMPLEMENTATION_NAME, MC_VERSION, PROTOCOL_VERSION};

use crate::packet::{read_frame, write_packet, PacketError};
use crate::shared::{new_shared_server, SharedServer};
use crate::status::{
    encode_status_response, read_handshake, NextState, StatusDescription, StatusPlayers,
    StatusResponse, StatusVersion,
};

/// Run an isolated in-memory server until shutdown.
pub fn run(listener: TcpListener) {
    run_with_state(listener, new_shared_server());
}

/// Validate the world's store, then run connection and tick workers until shutdown.
///
/// Exposed so tests (and future embedders) can inspect the player registry the
/// connections share. Stdin ownership belongs to the executable/embedding caller.
pub fn run_with_state(listener: TcpListener, server: SharedServer) {
    if let Err(error) = server.world().prepare_store() {
        eprintln!("[{IMPLEMENTATION_NAME}] world admission failed: {error}");
        server.request_shutdown();
        return;
    }
    let _tick_state = crate::tick::start_owned(SharedServer::clone(&server));
    listener
        .set_nonblocking(true)
        .expect("configure listener as nonblocking");
    let mut connections: Vec<(TcpStream, JoinHandle<()>)> = Vec::new();
    while !server.is_shutting_down() {
        let mut index = 0;
        while index < connections.len() {
            if connections[index].1.is_finished() {
                let (_, worker) = connections.swap_remove(index);
                if worker.join().is_err() {
                    eprintln!("connection worker panicked");
                }
            } else {
                index += 1;
            }
        }
        let incoming = match listener.accept() {
            Ok((stream, _)) => {
                if let Err(e) = stream.set_nonblocking(false) {
                    eprintln!("accept stream configuration error: {e}");
                    continue;
                }
                Ok(stream)
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }
            Err(e) => Err(e),
        };
        if server.is_shutting_down() {
            println!("[{IMPLEMENTATION_NAME}] shutting down: no longer accepting connections");
            break;
        }
        match incoming {
            Ok(stream) => {
                let shutdown_stream = match stream.try_clone() {
                    Ok(clone) => clone,
                    Err(error) => {
                        eprintln!("cannot retain connection shutdown handle: {error}");
                        continue;
                    }
                };
                let server = SharedServer::clone(&server);
                let worker = std::thread::spawn(move || {
                    let _ = stream.set_nodelay(true);
                    if let Err(e) = handle_connection(stream, &server) {
                        eprintln!("connection error: {e}");
                    }
                });
                connections.push((shutdown_stream, worker));
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    for (stream, _) in &connections {
        if let Err(error) = stream.shutdown(Shutdown::Both) {
            if error.kind() != std::io::ErrorKind::NotConnected {
                eprintln!("connection shutdown error: {error}");
            }
        }
    }
    for (_, worker) in connections {
        if worker.join().is_err() {
            eprintln!("connection worker panicked during shutdown");
        }
    }
}

fn handle_connection(mut stream: TcpStream, server: &SharedServer) -> Result<(), PacketError> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let (packet_id, data) = read_frame(&mut stream)?;
    if packet_id != 0x00 {
        return Err(PacketError::UnexpectedPacket(packet_id));
    }
    let mut cursor = Cursor::new(data);
    let handshake = read_handshake(&mut cursor)?;
    if cursor.position() != cursor.get_ref().len() as u64 {
        return Err(PacketError::Malformed("trailing handshake payload"));
    }
    if handshake.server_address.len() > 255 * 3
        || handshake.server_address.encode_utf16().count() > 255
    {
        return Err(PacketError::TooLarge(handshake.server_address.len()));
    }
    eprintln!(
        "[BCore] handshake: protocol={}, state={:?}",
        handshake.protocol_version, handshake.next_state
    );

    match handshake.next_state {
        NextState::Status => handle_status(&mut stream, server),
        NextState::Login if handshake.protocol_version != PROTOCOL_VERSION => {
            stream.write_all(&crate::login::encode_login_disconnect(&format!(
                "This server requires Minecraft {MC_VERSION} (protocol {PROTOCOL_VERSION}); received protocol {}",
                handshake.protocol_version
            )))?;
            Ok(())
        }
        NextState::Login => handle_login(&mut stream, server),
    }
}

fn handle_status(stream: &mut TcpStream, server: &SharedServer) -> Result<(), PacketError> {
    loop {
        let (packet_id, data) = read_frame(stream)?;
        match packet_id {
            0x00 => {
                if !data.is_empty() {
                    return Err(PacketError::Malformed("status request must be empty"));
                }
                let response = StatusResponse {
                    version: StatusVersion {
                        name: MC_VERSION,
                        protocol: PROTOCOL_VERSION,
                    },
                    players: StatusPlayers {
                        max: crate::join::MAX_PLAYERS as i32,
                        // Report the real player count now that it is tracked.
                        online: server.player_count() as i32,
                        sample: Vec::new(),
                    },
                    description: StatusDescription {
                        text: format!("{IMPLEMENTATION_NAME} {MC_VERSION} — native Rust"),
                    },
                    favicon: None,
                    enforces_secure_chat: false,
                    previews_chat: false,
                };
                stream
                    .write_all(&encode_status_response(&response))
                    .map_err(PacketError::Io)?;
            }
            0x01 => {
                if data.len() != 8 {
                    return Err(PacketError::Malformed("ping payload must be 8 bytes"));
                }
                let payload = i64::from_be_bytes(data[..8].try_into().expect("checked length"));
                let mut out = Vec::new();
                write_packet(&mut out, 0x01, &payload.to_be_bytes());
                stream.write_all(&out).map_err(PacketError::Io)?;
                return Ok(());
            }
            _ => return Ok(()),
        }
    }
}

fn handle_login(stream: &mut TcpStream, server: &SharedServer) -> Result<(), PacketError> {
    crate::join::run_login_and_join(stream, server)
}
