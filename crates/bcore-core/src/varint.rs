//! Minecraft protocol VarInt / VarLong encoding.
//!
//! A VarInt is a little-endian 7-bit-per-byte encoding: the least significant
//! 7 bits of the value go in the first byte, the next 7 in the second, and so
//! on, with the high bit set on every byte except the last. Values are
//! sign-extended to 32/64 bits before encoding, so negative numbers always use
//! the maximum number of bytes (5 for VarInt, 10 for VarLong).

use std::fmt;

/// Maximum encoded length of a VarInt (i32) in bytes.
pub const VARINT_MAX_LEN: usize = 5;
/// Maximum encoded length of a VarLong (i64) in bytes.
pub const VARLONG_MAX_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarIntError {
    /// The input ended before the value was fully decoded.
    Eof,
    /// The value used more bytes than allowed for its width.
    TooBig,
}

impl fmt::Display for VarIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarIntError::Eof => write!(f, "unexpected end of input while decoding varint"),
            VarIntError::TooBig => write!(f, "varint exceeds maximum length"),
        }
    }
}

impl std::error::Error for VarIntError {}

/// Encode an `i32` as a protocol VarInt, appending to `out`.
pub fn encode_varint(value: i32, out: &mut Vec<u8>) {
    let mut v = value as u32;
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

/// Decode a VarInt from the front of `bytes`, returning `(value, bytes_consumed)`.
pub fn decode_varint(bytes: &[u8]) -> Result<(i32, usize), VarIntError> {
    let mut result: u32 = 0;
    for i in 0..VARINT_MAX_LEN {
        let b = *bytes.get(i).ok_or(VarIntError::Eof)?;
        result |= ((b & 0x7f) as u32) << (7 * i);
        if b & 0x80 == 0 {
            return Ok((result as i32, i + 1));
        }
    }
    Err(VarIntError::TooBig)
}

/// Encode an `i64` as a protocol VarLong, appending to `out`.
pub fn encode_varlong(value: i64, out: &mut Vec<u8>) {
    let mut v = value as u64;
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

/// Decode a VarLong from the front of `bytes`, returning `(value, bytes_consumed)`.
pub fn decode_varlong(bytes: &[u8]) -> Result<(i64, usize), VarIntError> {
    let mut result: u64 = 0;
    for i in 0..VARLONG_MAX_LEN {
        let b = *bytes.get(i).ok_or(VarIntError::Eof)?;
        result |= ((b & 0x7f) as u64) << (7 * i);
        if b & 0x80 == 0 {
            return Ok((result as i64, i + 1));
        }
    }
    Err(VarIntError::TooBig)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: i32) {
        let mut out = Vec::new();
        encode_varint(v, &mut out);
        let (back, consumed) = decode_varint(&out).expect("decode");
        assert_eq!(back, v, "value mismatch for {v}");
        assert_eq!(consumed, out.len(), "did not consume the whole encoding");
    }

    #[test]
    fn varint_roundtrip() {
        for v in [
            0,
            1,
            2,
            127,
            128,
            255,
            25565,
            776,
            i32::MAX,
            i32::MIN,
            -1,
            -2147483648,
        ] {
            roundtrip(v);
        }
    }

    #[test]
    fn varint_lengths() {
        // Known encoded lengths: 0..=127 -> 1 byte, 128..=16383 -> 2 bytes.
        let mut out = Vec::new();
        encode_varint(0, &mut out);
        assert_eq!(out.len(), 1);
        out.clear();
        encode_varint(127, &mut out);
        assert_eq!(out.len(), 1);
        out.clear();
        encode_varint(128, &mut out);
        assert_eq!(out.len(), 2);
        out.clear();
        encode_varint(-1, &mut out);
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn varlong_roundtrip() {
        for v in [
            0i64,
            127,
            128,
            776,
            i64::MAX,
            i64::MIN,
            -1,
            0x1234_5678_9abc_def0,
        ] {
            let mut out = Vec::new();
            encode_varlong(v, &mut out);
            let (back, _) = decode_varlong(&out).expect("decode");
            assert_eq!(back, v);
        }
    }
}
