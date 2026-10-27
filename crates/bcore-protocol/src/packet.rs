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

/// Incremental play-state framing. A socket timeout must not discard a partial
/// length, packet ID or body; later reads continue the same frame.
#[derive(Default)]
pub(crate) struct FrameReader {
    pending: Vec<u8>,
    consumed: usize,
}

impl FrameReader {
    pub(crate) fn read<R: Read>(&mut self, stream: &mut R) -> Result<(i32, Vec<u8>), PacketError> {
        loop {
            let bytes = &self.pending[self.consumed..];
            match varint::decode_varint(bytes) {
                Ok((length, prefix)) => {
                    if !(1..=8 * 1024 * 1024).contains(&length) {
                        return Err(PacketError::TooLarge(length as usize));
                    }
                    let end = prefix + length as usize;
                    if bytes.len() >= end {
                        let (id, used) = varint::decode_varint(&bytes[prefix..end])?;
                        let payload = bytes[prefix + used..end].to_vec();
                        self.consumed += end;
                        if self.consumed == self.pending.len() {
                            self.pending.clear();
                            self.consumed = 0;
                        }
                        return Ok((id, payload));
                    }
                }
                Err(VarIntError::Eof) => {}
                Err(error) => return Err(error.into()),
            }
            if self.consumed != 0 {
                self.pending.drain(..self.consumed);
                self.consumed = 0;
            }
            let mut buffer = [0; 16 * 1024];
            match stream.read(&mut buffer) {
                Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()),
                Ok(length) => self.pending.extend_from_slice(&buffer[..length]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use std::io::{self, Cursor};

    struct TimeoutAfter<'a>(&'a [u8]);
    impl Read for TimeoutAfter<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                Err(io::ErrorKind::TimedOut.into())
            } else {
                self.0.read(output)
            }
        }
    }

    #[test]
    fn partial_headers_and_bodies_survive_timeouts_at_every_byte_boundary() {
        let payload = vec![0x5a; 300];
        let mut bytes = Vec::new();
        write_packet(&mut bytes, 257, &payload);
        for cut in 0..bytes.len() {
            let mut reader = FrameReader::default();
            assert!(matches!(reader.read(&mut TimeoutAfter(&bytes[..cut])),
                Err(PacketError::Io(error)) if error.kind() == io::ErrorKind::TimedOut));
            let mut rest = bytes[cut..].to_vec();
            write_packet(&mut rest, 8, b"next");
            let mut stream = Cursor::new(rest);
            assert_eq!(reader.read(&mut stream).unwrap(), (257, payload.clone()));
            assert_eq!(reader.read(&mut stream).unwrap(), (8, b"next".to_vec()));
        }
    }

    #[test]
    fn invalid_lengths_and_unfinished_frames_fail_without_unbounded_allocation() {
        for length in [-1, 0, 8 * 1024 * 1024 + 1] {
            let mut header = Vec::new();
            varint::encode_varint(length, &mut header);
            assert!(matches!(
                FrameReader::default().read(&mut Cursor::new(header)),
                Err(PacketError::TooLarge(_))
            ));
        }
        assert!(FrameReader::default()
            .read(&mut Cursor::new([0x80; 5]))
            .is_err());
        assert!(
            matches!(FrameReader::default().read(&mut Cursor::new([3, 8, 1])),
            Err(PacketError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof)
        );
    }
}
