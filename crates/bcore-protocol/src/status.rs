//! Handshake and server-list status packets.

use std::io::Read;

use serde::Serialize;

use crate::packet::{read_string, read_varint, write_packet, write_string, PacketError};

pub const HANDSHAKE_ID: i32 = 0x00;
pub const STATUS_REQUEST_ID: i32 = 0x00;
pub const STATUS_RESPONSE_ID: i32 = 0x00;
pub const PING_ID: i32 = 0x01;
pub const PONG_ID: i32 = 0x01;

/// The next protocol state requested by a handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextState {
    Status = 1,
    Login = 2,
}

#[derive(Debug)]
pub struct Handshake {
    pub protocol_version: i32,
    pub server_address: String,
    pub server_port: u16,
    pub next_state: NextState,
}

/// Parse a handshake packet payload (the data after the packet id).
pub fn read_handshake<R: Read>(r: &mut R) -> Result<Handshake, PacketError> {
    let protocol_version = read_varint(r)?;
    let server_address = read_string(r, 255)?;
    let mut port_buf = [0u8; 2];
    r.read_exact(&mut port_buf)?;
    let server_port = u16::from_be_bytes(port_buf);
    let next_state = match read_varint(r)? {
        1 => NextState::Status,
        2 => NextState::Login,
        other => return Err(PacketError::UnexpectedPacket(other)),
    };
    Ok(Handshake {
        protocol_version,
        server_address,
        server_port,
        next_state,
    })
}

#[derive(Serialize)]
pub struct StatusVersion {
    pub name: &'static str,
    pub protocol: i32,
}

#[derive(Serialize)]
pub struct StatusPlayer {
    pub name: String,
    pub id: String,
}

#[derive(Serialize)]
pub struct StatusPlayers {
    pub max: i32,
    pub online: i32,
    pub sample: Vec<StatusPlayer>,
}

#[derive(Serialize)]
pub struct StatusDescription {
    pub text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub version: StatusVersion,
    pub players: StatusPlayers,
    pub description: StatusDescription,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    pub enforces_secure_chat: bool,
    pub previews_chat: bool,
}

/// Encode a status-response packet.
pub fn encode_status_response(response: &StatusResponse) -> Vec<u8> {
    let json = serde_json::to_string(response).expect("status response is always serializable");
    let mut data = Vec::new();
    write_string(&json, &mut data);
    let mut out = Vec::new();
    write_packet(&mut out, STATUS_RESPONSE_ID, &data);
    out
}
