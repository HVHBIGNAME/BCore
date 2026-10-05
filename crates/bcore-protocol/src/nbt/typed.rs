//! Binary encoding of the physical typed NBT tree returned by native captures.
use bcore_worldgen::structure::template::Nbt;

const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NbtEncodeError {
    StringTooLong(usize),
    CollectionTooLong(usize),
    InvalidListType(u8),
    ListTypeMismatch { declared: u8, actual: u8 },
    DepthLimit,
}

impl std::fmt::Display for NbtEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StringTooLong(size) => write!(f, "NBT modified-UTF string has {size} bytes"),
            Self::CollectionTooLong(size) => write!(f, "NBT collection has {size} entries"),
            Self::InvalidListType(kind) => write!(f, "invalid NBT list type {kind}"),
            Self::ListTypeMismatch { declared, actual } => {
                write!(f, "NBT list declares type {declared}, found {actual}")
            }
            Self::DepthLimit => f.write_str("NBT nesting exceeds 512 levels"),
        }
    }
}

impl std::error::Error for NbtEncodeError {}

/// Encode an anonymous root: its type byte and payload, without a root name.
///
/// This input is the physical typed tree, including explicit list element types.
/// Native heterogeneous ListTag values have already become wrapper compounds in
/// this representation. Compound entries follow the input BTreeMap's key order.
pub fn encode_typed_nbt(value: &Nbt) -> Result<Vec<u8>, NbtEncodeError> {
    let mut out = vec![tag_id(value)];
    write_payload(value, &mut out, 0)?;
    Ok(out)
}

fn tag_id(value: &Nbt) -> u8 {
    match value {
        Nbt::Byte(_) => 1,
        Nbt::Short(_) => 2,
        Nbt::Int(_) => 3,
        Nbt::Long(_) => 4,
        Nbt::Float(_) => 5,
        Nbt::Double(_) => 6,
        Nbt::ByteArray(_) => 7,
        Nbt::String(_) => 8,
        Nbt::List { .. } => 9,
        Nbt::Compound(_) => 10,
        Nbt::IntArray(_) => 11,
        Nbt::LongArray(_) => 12,
    }
}

pub(super) fn write_string(value: &str, out: &mut Vec<u8>) -> Result<(), NbtEncodeError> {
    let length: usize = value
        .encode_utf16()
        .map(|unit| match unit {
            1..=0x7f => 1,
            0..=0x7ff => 2,
            _ => 3,
        })
        .sum();
    let short = u16::try_from(length).map_err(|_| NbtEncodeError::StringTooLong(length))?;
    out.extend_from_slice(&short.to_be_bytes());
    for unit in value.encode_utf16() {
        match unit {
            1..=0x7f => out.push(unit as u8),
            0..=0x7ff => {
                out.push(0xc0 | (unit >> 6) as u8);
                out.push(0x80 | (unit & 0x3f) as u8);
            }
            _ => {
                out.push(0xe0 | (unit >> 12) as u8);
                out.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                out.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
    Ok(())
}

fn write_length(length: usize, out: &mut Vec<u8>) -> Result<(), NbtEncodeError> {
    let length = i32::try_from(length).map_err(|_| NbtEncodeError::CollectionTooLong(length))?;
    out.extend_from_slice(&length.to_be_bytes());
    Ok(())
}

fn write_payload(value: &Nbt, out: &mut Vec<u8>, depth: usize) -> Result<(), NbtEncodeError> {
    if depth > MAX_DEPTH {
        return Err(NbtEncodeError::DepthLimit);
    }
    match value {
        Nbt::Byte(value) => out.push(*value as u8),
        Nbt::Short(value) => out.extend_from_slice(&value.to_be_bytes()),
        Nbt::Int(value) => out.extend_from_slice(&value.to_be_bytes()),
        Nbt::Long(value) => out.extend_from_slice(&value.to_be_bytes()),
        Nbt::Float(value) => {
            // Java DataOutput.writeFloat uses Float.floatToIntBits, canonicalizing NaNs.
            let bits = if value.is_nan() {
                0x7fc0_0000
            } else {
                value.to_bits()
            };
            out.extend_from_slice(&bits.to_be_bytes());
        }
        Nbt::Double(value) => {
            let bits = if value.is_nan() {
                0x7ff8_0000_0000_0000
            } else {
                value.to_bits()
            };
            out.extend_from_slice(&bits.to_be_bytes());
        }
        Nbt::ByteArray(values) => {
            write_length(values.len(), out)?;
            out.extend(values.iter().map(|value| *value as u8));
        }
        Nbt::String(value) => write_string(value, out)?,
        Nbt::List {
            element_type,
            values,
        } => {
            if *element_type > 12 || (*element_type == 0 && !values.is_empty()) {
                return Err(NbtEncodeError::InvalidListType(*element_type));
            }
            for value in values {
                let actual = tag_id(value);
                if actual != *element_type {
                    return Err(NbtEncodeError::ListTypeMismatch {
                        declared: *element_type,
                        actual,
                    });
                }
            }
            out.push(*element_type);
            write_length(values.len(), out)?;
            for value in values {
                write_payload(value, out, depth + 1)?;
            }
        }
        Nbt::Compound(values) => {
            for (name, value) in values {
                out.push(tag_id(value));
                write_string(name, out)?;
                write_payload(value, out, depth + 1)?;
            }
            out.push(0);
        }
        Nbt::IntArray(values) => {
            write_length(values.len(), out)?;
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        Nbt::LongArray(values) => {
            write_length(values.len(), out)?;
            for value in values {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_physical_list_descriptors() {
        assert_eq!(
            encode_typed_nbt(&Nbt::List {
                element_type: 0,
                values: vec![Nbt::Byte(1)]
            }),
            Err(NbtEncodeError::InvalidListType(0))
        );
        assert_eq!(
            encode_typed_nbt(&Nbt::List {
                element_type: 13,
                values: vec![]
            }),
            Err(NbtEncodeError::InvalidListType(13))
        );
        assert_eq!(
            encode_typed_nbt(&Nbt::List {
                element_type: 1,
                values: vec![Nbt::Int(1)]
            }),
            Err(NbtEncodeError::ListTypeMismatch {
                declared: 1,
                actual: 3
            })
        );
    }

    #[test]
    fn modified_utf_limit_is_measured_after_java_encoding() {
        let valid = Nbt::String("\0".repeat(32767));
        assert_eq!(encode_typed_nbt(&valid).unwrap().len(), 65537);
        let mut output = vec![17];
        assert_eq!(
            write_string(&"\0".repeat(32768), &mut output),
            Err(NbtEncodeError::StringTooLong(65536))
        );
        assert_eq!(
            output,
            [17],
            "oversize detection precedes the length prefix"
        );
        assert!(encode_typed_nbt(&Nbt::String("x".repeat(65535))).is_ok());
        assert!(encode_typed_nbt(&Nbt::String("x".repeat(65536))).is_err());
    }

    #[test]
    fn rejects_excessive_nesting() {
        let mut value = Nbt::Byte(0);
        for _ in 0..=MAX_DEPTH {
            value = Nbt::Compound([(String::from("child"), value)].into());
        }
        assert_eq!(encode_typed_nbt(&value), Err(NbtEncodeError::DepthLimit));
    }
}
