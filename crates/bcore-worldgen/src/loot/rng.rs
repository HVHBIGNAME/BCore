//! The explicit-seed source and the caller-owned RNG boundary.

/// Native public operations used by loot. Implementations must preserve their
/// own source's draw semantics (notably bounded integers and booleans).
pub trait LootRng {
    /// Uniform integer in `0..bound`; the engine always supplies a positive bound.
    fn next_int(&mut self, bound: i32) -> i32;
    fn next_float(&mut self) -> f32;
    fn next_bool(&mut self) -> bool;

    /// Optional diagnostics. A caller-owned non-Legacy source should return None.
    fn legacy_state(&self) -> Option<u64> {
        None
    }
}

/// Java's 48-bit LegacyRandomSource, with an observable checkpoint.
///
/// A literal zero is valid here, but NOT as an optional container loot seed.
/// `LootTable::fill` rejects seed zero; `fill_with_rng` uses exactly the supplied
/// source and never selects, creates, or reseeds a server sequence.
#[derive(Clone, Debug)]
pub struct LegacyRandom {
    state: u64,
}

impl LegacyRandom {
    const MASK: u64 = (1 << 48) - 1;
    const MULTIPLIER: u64 = 25_214_903_917;

    pub fn new(seed: i64) -> Self {
        Self {
            state: (seed as u64 ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    pub fn state(&self) -> u64 {
        self.state
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.state = self.state.wrapping_mul(Self::MULTIPLIER).wrapping_add(11) & Self::MASK;
        (self.state >> (48 - bits)) as i32
    }

    pub fn next_long(&mut self) -> i64 {
        let high = i64::from(self.next(32));
        let low = i64::from(self.next(32));
        (high << 32).wrapping_add(low)
    }

    pub fn next_double(&mut self) -> f64 {
        let high = i64::from(self.next(26));
        let low = i64::from(self.next(27));
        ((high << 27) + low) as f64 * (1.0 / (1_u64 << 53) as f64)
    }
}

impl LootRng for LegacyRandom {
    fn next_int(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "LegacyRandom bound must be positive");
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let value = bits % bound;
            if bits.wrapping_sub(value).wrapping_add(bound - 1) >= 0 {
                return value;
            }
        }
    }

    fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * (1.0 / (1_u32 << 24) as f32)
    }

    fn next_bool(&mut self) -> bool {
        self.next(1) != 0
    }

    fn legacy_state(&self) -> Option<u64> {
        Some(self.state)
    }
}

impl LootRng for crate::simplex::JavaRandom {
    fn next_int(&mut self, bound: i32) -> i32 {
        assert!(bound > 0);
        self.next_int(bound as usize) as i32
    }

    fn next_float(&mut self) -> f32 {
        self.next_float()
    }

    fn next_bool(&mut self) -> bool {
        self.next_bool()
    }
}

/// `Mth.nextInt`, including its no-draw degenerate interval.
pub(super) fn between(rng: &mut dyn LootRng, min: i32, max: i32) -> super::Result<i32> {
    if min >= max {
        return Ok(min);
    }
    let bound = max
        .checked_sub(min)
        .and_then(|v| v.checked_add(1))
        .ok_or(super::LootError::LimitExceeded("integer provider interval"))?;
    Ok(rng.next_int(bound) + min)
}

pub(super) fn shuffle<T>(values: &mut [T], rng: &mut dyn LootRng) {
    for size in (2..=values.len()).rev() {
        let index = rng.next_int(size as i32) as usize;
        values.swap(size - 1, index);
    }
}
