//! Framing and primitive readers/writers for the Minecraft protocol.

use std::io::Read;

use bcore_core::varint::{self, VarIntError};

/// Errors that can occur while reading or writing protocol packets.
#[derive(Debug)]
pub enum PacketError {
    Io(std::io::Error),
    VarInt(VarIntError),
    Utf8(std::string::FromUtf8Error),
    TooLarge(usize),
    UnexpectedPacket(i32),
    Malformed(&'static str),
}

impl std::fmt::Display for PacketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PacketError::Io(e) => write!(f, "io: {e}"),
            PacketError::VarInt(e) => write!(f, "varint: {e}"),
            PacketError::Utf8(e) => write!(f, "utf8: {e}"),
            PacketError::TooLarge(n) => write!(f, "field too large: {n} bytes"),
            PacketError::UnexpectedPacket(id) => write!(f, "unexpected packet id 0x{id:02x}"),
            PacketError::Malformed(m) => write!(f, "malformed packet: {m}"),
        }
    }
}

impl std::error::Error for PacketError {}

impl From<std::io::Error> for PacketError {
    fn from(e: std::io::Error) -> Self {
        PacketError::Io(e)
    }
}

impl From<VarIntError> for PacketError {
    fn from(e: VarIntError) -> Self {
        PacketError::VarInt(e)
    }
}

impl From<std::string::FromUtf8Error> for PacketError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        PacketError::Utf8(e)
    }
}

/// Read a protocol VarInt from a byte stream.
pub fn read_varint<R: Read>(r: &mut R) -> Result<i32, PacketError> {
    let mut result: u32 = 0;
    for i in 0..varint::VARINT_MAX_LEN {
        let mut buf = [0u8; 1];
        r.read_exact(&mut buf)?;
        let b = buf[0];
        result |= ((b & 0x7f) as u32) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(result as i32);
        }
    }
    Err(VarIntError::TooBig.into())
}

/// Read a length-prefixed UTF-8 string, enforcing a maximum character count.
pub fn read_string<R: Read>(r: &mut R, max_chars: usize) -> Result<String, PacketError> {
    let len = read_varint(r)?;
    if len < 0 || len as usize > max_chars.saturating_mul(4) {
        return Err(PacketError::TooLarge(len as usize));
    }
    let mut bytes = vec![0u8; len as usize];
    r.read_exact(&mut bytes)?;
    let s = String::from_utf8(bytes)?;
    if s.chars().count() > max_chars {
        return Err(PacketError::TooLarge(s.len()));
    }
    Ok(s)
}

/// Write a length-prefixed UTF-8 string.
pub fn write_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    varint::encode_varint(bytes.len() as i32, out);
    out.extend_from_slice(bytes);
}

/// Write a full framed packet (length prefix + id + data) into `out`.
pub fn write_packet(out: &mut Vec<u8>, packet_id: i32, data: &[u8]) {
    let mut body = Vec::with_capacity(5 + data.len());
    varint::encode_varint(packet_id, &mut body);
    body.extend_from_slice(data);

    varint::encode_varint(body.len() as i32, out);
    out.extend_from_slice(&body);
}

/// Read one framed packet, returning `(packet_id, payload)`.
pub fn read_frame<R: Read>(r: &mut R) -> Result<(i32, Vec<u8>), PacketError> {
    let len = read_varint(r)?;
    if len < 0 || len > 8 * 1024 * 1024 {
        return Err(PacketError::TooLarge(len as usize));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    let (id, consumed) = varint::decode_varint(&buf)?;
    let payload = buf[consumed..].to_vec();
    Ok((id, payload))
}
