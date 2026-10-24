//! Read native uncompressed named-root NBT for semantic, numeric-width-aware
//! comparison. Compound iteration order is intentionally not an NBT value.
use bcore_worldgen::structure::template::Nbt;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};

pub fn decode(text: &str) -> Nbt {
    let mut bytes = Vec::new();
    let mut bits = 0;
    let mut accumulator = 0_u32;
    for byte in text.bytes().take_while(|&b| b != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("invalid fixture base64"),
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((accumulator >> bits) as u8);
        }
    }
    let mut reader = Reader(Cursor::new(bytes.as_slice()));
    let kind = reader.byte();
    assert!(reader.string().is_empty());
    let result = reader.value(kind);
    assert_eq!(reader.0.position(), bytes.len() as u64);
    result
}

struct Reader<'a>(Cursor<&'a [u8]>);
impl Reader<'_> {
    fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut result = [0; N];
        self.0.read_exact(&mut result).unwrap();
        result
    }
    fn byte(&mut self) -> u8 {
        self.bytes::<1>()[0]
    }
    fn int(&mut self) -> i32 {
        i32::from_be_bytes(self.bytes())
    }
    fn string(&mut self) -> String {
        let length = u16::from_be_bytes(self.bytes()) as usize;
        let mut bytes = vec![0; length];
        self.0.read_exact(&mut bytes).unwrap();
        // The captured resource identifiers/component keys are all ASCII.
        assert!(bytes.is_ascii());
        String::from_utf8(bytes).unwrap()
    }
    fn value(&mut self, kind: u8) -> Nbt {
        match kind {
            1 => Nbt::Byte(self.byte() as i8),
            2 => Nbt::Short(i16::from_be_bytes(self.bytes())),
            3 => Nbt::Int(self.int()),
            4 => Nbt::Long(i64::from_be_bytes(self.bytes())),
            5 => Nbt::Float(f32::from_be_bytes(self.bytes())),
            6 => Nbt::Double(f64::from_be_bytes(self.bytes())),
            7 => {
                let count = self.int();
                Nbt::ByteArray((0..count).map(|_| self.byte() as i8).collect())
            }
            8 => Nbt::String(self.string()),
            9 => {
                let element_type = self.byte();
                let count = self.int();
                Nbt::List {
                    element_type,
                    values: (0..count).map(|_| self.value(element_type)).collect(),
                }
            }
            10 => {
                let mut fields = BTreeMap::new();
                loop {
                    let kind = self.byte();
                    if kind == 0 {
                        break;
                    }
                    let name = self.string();
                    assert!(fields.insert(name, self.value(kind)).is_none());
                }
                Nbt::Compound(fields)
            }
            11 => {
                let count = self.int();
                Nbt::IntArray((0..count).map(|_| self.int()).collect())
            }
            12 => {
                let count = self.int();
                Nbt::LongArray(
                    (0..count)
                        .map(|_| i64::from_be_bytes(self.bytes()))
                        .collect(),
                )
            }
            _ => panic!("unexpected native NBT type {kind}"),
        }
    }
}
