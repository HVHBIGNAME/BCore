//! Minimal TCP server: accept connections and handle handshake/status/login.
//!
//! One thread per connection, plus one piece of shared state
//! ([`crate::shared::SharedServer`]) so players can see each other in chat and
//! in `/list`. The accept loop stops once `/stop` has flagged a shutdown.

use std::io::{Cursor, Write};
use std::net::{TcpListener, TcpStream};

use bcore_core::version::{IMPLEMENTATION_NAME, MC_VERSION, PROTOCOL_VERSION};

use crate::packet::{read_frame, write_packet, PacketError};
use crate::shared::{new_shared_server, SharedServer};
use crate::status::{
    encode_status_response, read_handshake, NextState, StatusDescription, StatusPlayers,
    StatusResponse, StatusVersion,
};

/// Run the accept loop until `/stop` is issued (or the process exits).
pub fn run(listener: TcpListener) {
    run_with_state(listener, new_shared_server());
}

/// Run the accept loop against an existing shared state.
///
/// Exposed so tests (and future embedders) can inspect the player registry the
/// connections share.
pub fn run_with_state(listener: TcpListener, server: SharedServer) {
    for incoming in listener.incoming() {
        if server.is_shutting_down() {
            println!("[{IMPLEMENTATION_NAME}] shutting down: no longer accepting connections");
            break;
        }
        match incoming {
            Ok(stream) => {
                let server = SharedServer::clone(&server);
                std::thread::spawn(move || {
                    let _ = stream.set_nodelay(true);
                    if let Err(e) = handle_connection(stream, &server) {
                        eprintln!("connection error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

fn handle_connection(mut stream: TcpStream, server: &SharedServer) -> Result<(), PacketError> {
    let (packet_id, data) = read_frame(&mut stream)?;
    if packet_id != 0x00 {
        return Err(PacketError::UnexpectedPacket(packet_id));
    }
    let mut cursor = Cursor::new(data);
    let handshake = read_handshake(&mut cursor)?;

    match handshake.next_state {
        NextState::Status => handle_status(&mut stream, server),
        NextState::Login => handle_login(&mut stream, server),
    }
}

fn handle_status(stream: &mut TcpStream, server: &SharedServer) -> Result<(), PacketError> {
    loop {
        let (packet_id, data) = read_frame(stream)?;
        match packet_id {
            0x00 => {
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
