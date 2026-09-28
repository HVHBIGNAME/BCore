//! Minecraft 26.1 `Mth` sine/cosine lookup, returning native `float` values.
//!
//! The JAR declares only double-argument overloads. Java float arguments widen
//! before the scale multiplication, as do [`sin_f32`] and [`cos_f32`]. Indexing
//! truncates to a saturating signed 64-bit integer (Java `d2l`, including NaN to
//! zero), then masks the low 16 bits. Cosine adds its phase before that cast.

use std::sync::OnceLock;

pub const SIN_TABLE_LEN: usize = 65_536;
const SIN_SCALE: f64 = 10430.378350470453;
static SIN: OnceLock<Box<[f32; SIN_TABLE_LEN]>> = OnceLock::new();

/// Shared native-quantized table, initialized once on the heap (256 KiB).
///
/// Each entry uses the JAR's `Math.sin(index / SIN_SCALE)` followed by a float
/// cast. `tests/mth_reference.rs` checks every entry against the captured JAR.
#[inline]
pub fn sin_table() -> &'static [f32; SIN_TABLE_LEN] {
    SIN.get_or_init(|| {
        let values: Vec<f32> = (0..SIN_TABLE_LEN)
            .map(|index| (index as f64 / SIN_SCALE).sin() as f32)
            .collect();
        values
            .into_boxed_slice()
            .try_into()
            .expect("fixed sine table length")
    })
}

/// Native `Mth.sin(double) -> float` for an angle in radians.
#[inline]
pub fn sin(angle: f64) -> f32 {
    let index = ((angle * SIN_SCALE) as i64 & 0xffff) as usize;
    sin_table()[index]
}

/// Native `Mth.cos(double) -> float` for an angle in radians.
#[inline]
pub fn cos(angle: f64) -> f32 {
    // Adding after the cast changes negative fractions and saturation behavior.
    let index = ((angle * SIN_SCALE + 16384.0) as i64 & 0xffff) as usize;
    sin_table()[index]
}

/// Java float-argument call: widen to double before native sine indexing.
#[inline]
pub fn sin_f32(angle: f32) -> f32 {
    sin(f64::from(angle))
}

/// Java float-argument call: widen to double before native cosine indexing.
#[inline]
pub fn cos_f32(angle: f32) -> f32 {
    cos(f64::from(angle))
}
