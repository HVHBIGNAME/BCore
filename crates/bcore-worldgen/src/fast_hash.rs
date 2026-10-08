//! Deterministic, allocation-free hashing for internal lookup tables.
//!
//! These tables are never iterated in an order that affects generation, and
//! their keys are exact bit patterns, so the hash function is an implementation
//! detail: it never changes which entries are found. Only collision behavior and
//! speed change.
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Default, Clone, Copy)]
pub struct FastHasher {
    hash: u64,
}

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buffer = [0u8; 8];
            buffer[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buffer));
        }
        self.add(bytes.len() as u64);
    }
    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }
    #[inline]
    fn write_u128(&mut self, value: u128) {
        self.add(value as u64);
        self.add((value >> 64) as u64);
    }
    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
    #[inline]
    fn write_i8(&mut self, value: i8) {
        self.add(value as u64);
    }
    #[inline]
    fn write_i16(&mut self, value: i16) {
        self.add(value as u64);
    }
    #[inline]
    fn write_i32(&mut self, value: i32) {
        self.add(value as u32 as u64);
    }
    #[inline]
    fn write_i64(&mut self, value: i64) {
        self.add(value as u64);
    }
    #[inline]
    fn write_isize(&mut self, value: isize) {
        self.add(value as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FastBuild = BuildHasherDefault<FastHasher>;
pub type FastMap<K, V> = HashMap<K, V, FastBuild>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_keys_are_distinguished_and_lookup_is_exact() {
        let mut map: FastMap<(usize, i64, u64, u64), u32> = FastMap::default();
        map.insert((1, -2, 3, 4), 5);
        map.insert((0, 0, 0, 0), 6);
        map.insert((usize::MAX, i64::MIN, u64::MAX, u64::MAX), 7);
        assert_eq!(map.get(&(1, -2, 3, 4)), Some(&5));
        assert_eq!(map.get(&(0, 0, 0, 0)), Some(&6));
        assert_eq!(
            map.get(&(usize::MAX, i64::MIN, u64::MAX, u64::MAX)),
            Some(&7)
        );
        assert_eq!(map.get(&(1, -2, 3, 5)), None);
        assert_eq!(map.len(), 3);
    }
}
