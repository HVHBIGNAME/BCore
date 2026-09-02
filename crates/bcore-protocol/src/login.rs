//! Login-state packets (minimal — gameplay not implemented yet).

use std::io::Read;

use serde::Serialize;

use crate::packet::{read_string, write_packet, write_string, PacketError};

pub const LOGIN_START_ID: i32 = 0x00;
pub const DISCONNECT_ID: i32 = 0x00;

#[derive(Serialize)]
struct Disconnect {
    text: String,
}

/// Read the player name from a login-start packet payload.
pub fn read_login_start_name<R: Read>(r: &mut R) -> Result<String, PacketError> {
    read_string(r, 16)
}

/// Encode a login disconnect packet carrying `message`.
pub fn encode_login_disconnect(message: &str) -> Vec<u8> {
    let json = serde_json::to_string(&Disconnect {
        text: message.to_string(),
    })
    .expect("disconnect component is always serializable");
    let mut data = Vec::new();
    write_string(&json, &mut data);
    let mut out = Vec::new();
    write_packet(&mut out, DISCONNECT_ID, &data);
    out
}
