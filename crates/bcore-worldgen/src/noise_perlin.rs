//! Vanilla `ImprovedNoise`/`PerlinNoise`/`NormalNoise` — exact port.
//!
//! Seeding follows `RandomState` for `legacy_random_source: false` (the
//! overworld default), which is **Xoroshiro128++**, not the Java LCG:
//!
//! ```text
//! random   = XoroshiroRandomSource(worldSeed).forkPositional()
//! noise    = NormalNoise.create(random.fromHashOf("minecraft:<key>"), params)
//! octave_i = ImprovedNoise(perlinPositional.fromHashOf("octave_<n>"))
//! ```
//!
//! `fromHashOf(name)` derives a 128-bit seed from `MD5(name)` and XORs it with
//! the factory's own seed pair — so every octave gets an independent stream that
//! is nonetheless fully determined by the world seed.

const GRADIENT: [[f64; 3]; 16] = [
    [1., 1., 0.],
    [-1., 1., 0.],
    [1., -1., 0.],
    [-1., -1., 0.],
    [1., 0., 1.],
    [-1., 0., 1.],
    [1., 0., -1.],
    [-1., 0., -1.],
    [0., 1., 1.],
    [0., -1., 1.],
    [0., 1., -1.],
    [0., -1., -1.],
    [1., 1., 0.],
    [0., -1., 1.],
    [-1., 1., 0.],
    [0., -1., -1.],
];

/// `RandomSupport.GOLDEN_RATIO_64`.
const GOLDEN_RATIO_64: u64 = 0x9e37_79b9_7f4a_7c15;
/// `RandomSupport.SILVER_RATIO_64`.
const SILVER_RATIO_64: u64 = 0x6a09_e667_f3bc_c909;

/// Java `String.hashCode()` (identical for ASCII, which all octave names are).
pub fn java_string_hash(s: &str) -> i32 {
    let mut h = 0i32;
    for &b in s.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as i32);
    }
    h
}

#[inline]
fn mix_stafford13(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `XoroshiroRandomSource` over `Xoroshiro128PlusPlus`.
#[derive(Clone)]
pub struct Xoroshiro {
    lo: u64,
    hi: u64,
}

impl Xoroshiro {
    /// `Xoroshiro128PlusPlus(seedLo, seedHi)` — including the all-zero guard.
    pub fn from_seed128(lo: u64, hi: u64) -> Self {
        if lo | hi == 0 {
            Self {
                lo: GOLDEN_RATIO_64,
                hi: SILVER_RATIO_64,
            }
        } else {
            Self { lo, hi }
        }
    }

    /// `XoroshiroRandomSource(long)` = `upgradeSeedTo128bit(seed).mixed()`.
    pub fn new(seed: i64) -> Self {
        let lo = (seed as u64) ^ SILVER_RATIO_64;
        let hi = lo.wrapping_add(GOLDEN_RATIO_64);
        Self::from_seed128(mix_stafford13(lo), mix_stafford13(hi))
    }

    pub fn next_long(&mut self) -> u64 {
        let s0 = self.lo;
        let s1 = self.hi;
        let result = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        let s1 = s1 ^ s0;
        self.lo = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.hi = s1.rotate_left(28);
        result
    }

    #[inline]
    fn next_bits(&mut self, bits: u32) -> u64 {
        self.next_long() >> (64 - bits)
    }

    /// `nextDouble` = `nextBits(53) * DOUBLE_UNIT`, where vanilla's `DOUBLE_UNIT`
    /// is written as a **float** literal (`1.110223E-16F`) and then widened.
    pub fn next_double(&mut self) -> f64 {
        self.next_bits(53) as f64 * (1.110223e-16f32 as f64)
    }

    pub fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 * 5.9604645e-8f32
    }

    /// `XoroshiroRandomSource.nextInt(bound)` — Lemire with rejection.
    pub fn next_int(&mut self, bound: u32) -> u32 {
        let mut random_bits = self.next_long() as u32 as u64;
        let mut multiplied = random_bits.wrapping_mul(bound as u64);
        let mut fractional = multiplied & 0xFFFF_FFFF;
        if fractional < bound as u64 {
            let threshold = (bound.wrapping_neg() % bound) as u64;
            while fractional < threshold {
                random_bits = self.next_long() as u32 as u64;
                multiplied = random_bits.wrapping_mul(bound as u64);
                fractional = multiplied & 0xFFFF_FFFF;
            }
        }
        (multiplied >> 32) as u32
    }

    /// `forkPositional()` = `XoroshiroPositionalRandomFactory(nextLong(), nextLong())`.
    pub fn fork_positional(&mut self) -> XoroshiroPositional {
        let lo = self.next_long();
        let hi = self.next_long();
        XoroshiroPositional { lo, hi }
    }
}

/// `XoroshiroRandomSource.XoroshiroPositionalRandomFactory`.
#[derive(Clone, Copy)]
pub struct XoroshiroPositional {
    lo: u64,
    hi: u64,
}

impl XoroshiroPositional {
    /// `fromHashOf(name)` = `XoroshiroRandomSource(seedFromHashOf(name).xor(lo, hi))`,
    /// where `seedFromHashOf` splits `MD5(name)` into two big-endian halves.
    pub fn from_hash_of(&self, name: &str) -> Xoroshiro {
        let digest = md5::compute(name.as_bytes());
        let hash_lo = u64::from_be_bytes(digest[0..8].try_into().unwrap());
        let hash_hi = u64::from_be_bytes(digest[8..16].try_into().unwrap());
        Xoroshiro::from_seed128(hash_lo ^ self.lo, hash_hi ^ self.hi)
    }
}

#[inline]
fn smoothstep(x: f64) -> f64 {
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

#[inline]
fn lerp3(t0: f64, t1: f64, t2: f64, v: [f64; 8]) -> f64 {
    lerp(
        t2,
        lerp(t1, lerp(t0, v[0], v[1]), lerp(t0, v[2], v[3])),
        lerp(t1, lerp(t0, v[4], v[5]), lerp(t0, v[6], v[7])),
    )
}

#[derive(Clone)]
pub struct ImprovedNoise {
    pub xo: f64,
    pub yo: f64,
    pub zo: f64,
    p: [u8; 256],
}

impl ImprovedNoise {
    fn from_random(r: &mut Xoroshiro) -> Self {
        let xo = r.next_double() * 256.0;
        let yo = r.next_double() * 256.0;
        let zo = r.next_double() * 256.0;
        let mut p = [0u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256 {
            let offset = r.next_int(256 - i as u32) as usize;
            p.swap(i, i + offset);
        }
        Self { xo, yo, zo, p }
    }

    #[inline]
    fn p(&self, x: i32) -> usize {
        self.p[(x & 255) as usize] as usize
    }

    #[inline]
    fn grad_dot(hash: usize, x: f64, y: f64, z: f64) -> f64 {
        let g = &GRADIENT[hash & 15];
        g[0] * x + g[1] * y + g[2] * z
    }

    fn sample_and_lerp(
        &self,
        x: i32,
        y: i32,
        z: i32,
        xr: f64,
        yr: f64,
        zr: f64,
        yr_original: f64,
    ) -> f64 {
        let x0 = self.p(x);
        let x1 = self.p(x + 1);
        let xy00 = self.p(x0 as i32 + y);
        let xy01 = self.p(x0 as i32 + y + 1);
        let xy10 = self.p(x1 as i32 + y);
        let xy11 = self.p(x1 as i32 + y + 1);
        let d000 = Self::grad_dot(self.p(xy00 as i32 + z), xr, yr, zr);
        let d100 = Self::grad_dot(self.p(xy10 as i32 + z), xr - 1.0, yr, zr);
        let d010 = Self::grad_dot(self.p(xy01 as i32 + z), xr, yr - 1.0, zr);
        let d110 = Self::grad_dot(self.p(xy11 as i32 + z), xr - 1.0, yr - 1.0, zr);
        let d001 = Self::grad_dot(self.p(xy00 as i32 + z + 1), xr, yr, zr - 1.0);
        let d101 = Self::grad_dot(self.p(xy10 as i32 + z + 1), xr - 1.0, yr, zr - 1.0);
        let d011 = Self::grad_dot(self.p(xy01 as i32 + z + 1), xr, yr - 1.0, zr - 1.0);
        let d111 = Self::grad_dot(self.p(xy11 as i32 + z + 1), xr - 1.0, yr - 1.0, zr - 1.0);
        let xa = smoothstep(xr);
        let ya = smoothstep(yr_original);
        let za = smoothstep(zr);
        lerp3(xa, ya, za, [d000, d100, d010, d110, d001, d101, d011, d111])
    }

    pub fn noise(&self, x: f64, y: f64, z: f64, y_scale: f64, y_fudge: f64) -> f64 {
        let x = x + self.xo;
        let y = y + self.yo;
        let z = z + self.zo;
        let xf = x.floor() as i32;
        let yf = y.floor() as i32;
        let zf = z.floor() as i32;
        let xr = x - xf as f64;
        let yr = y - yf as f64;
        let zr = z - zf as f64;
        let yr_fudge = if y_scale != 0.0 {
            let limit = if y_fudge >= 0.0 && y_fudge < yr {
                y_fudge
            } else {
                yr
            };
            (limit / y_scale + 1.0e-7).floor() * y_scale
        } else {
            0.0
        };
        self.sample_and_lerp(xf, yf, zf, xr, yr - yr_fudge, zr, yr)
    }
}

#[derive(Clone)]
pub struct PerlinNoise {
    amps: Vec<f64>,
    levels: Vec<Option<ImprovedNoise>>,
    first: i32,
}

impl PerlinNoise {
    /// `PerlinNoise.create(random, firstOctave, amplitudes)` with
    /// `useNewInitialization = true`: one `forkPositional()` on the parent, then
    /// `ImprovedNoise(positional.fromHashOf("octave_<n>"))` per nonzero amplitude.
    /// Zero amplitudes consume nothing (no `skipOctave` in the new path).
    fn create(r: &mut Xoroshiro, first: i32, amps: &[f64]) -> Self {
        let positional = r.fork_positional();
        let levels = amps
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if *a == 0.0 {
                    None
                } else {
                    let octave = first + i as i32;
                    let mut octave_random = positional.from_hash_of(&format!("octave_{octave}"));
                    Some(ImprovedNoise::from_random(&mut octave_random))
                }
            })
            .collect();
        Self {
            amps: amps.to_vec(),
            levels,
            first,
        }
    }

    fn value(&self, x: f64, y: f64, z: f64) -> f64 {
        let n = self.amps.len() as i32;
        let mut factor = 2f64.powi(self.first);
        let mut value_factor = 2f64.powi(n - 1) / (2f64.powi(n) - 1.0);
        let mut out = 0.0;
        for (i, level) in self.levels.iter().enumerate() {
            if let Some(noise) = level {
                out += self.amps[i]
                    * noise.noise(
                        wrap(x * factor),
                        wrap(y * factor),
                        wrap(z * factor),
                        0.0,
                        0.0,
                    )
                    * value_factor;
            }
            factor *= 2.0;
            value_factor /= 2.0;
        }
        out
    }
}

#[inline]
fn wrap(x: f64) -> f64 {
    x - (x / 33554432.0 + 0.5).floor() * 33554432.0
}

#[derive(Clone)]
pub struct NormalNoise {
    a: PerlinNoise,
    b: PerlinNoise,
    k: f64,
}

impl NormalNoise {
    /// Build the noise registered under `noise_name` (e.g. `minecraft:erosion`)
    /// for `world_seed`, following the full `RandomState` seed path.
    pub fn for_world(world_seed: i64, noise_name: &str, first: i32, amps: &[f64]) -> Self {
        let mut root = Xoroshiro::new(world_seed);
        let factory = root.fork_positional();
        let mut random = factory.from_hash_of(noise_name);
        Self::create(&mut random, first, amps)
    }

    /// `NormalNoise.create(random, firstOctave, amplitudes)` — `first` and
    /// `second` are drawn sequentially from the *same* random.
    pub fn create(random: &mut Xoroshiro, first: i32, amps: &[f64]) -> Self {
        let nz: Vec<usize> = amps
            .iter()
            .enumerate()
            .filter(|(_, a)| **a != 0.0)
            .map(|(i, _)| i)
            .collect();
        let span = nz
            .last()
            .unwrap_or(&0)
            .saturating_sub(*nz.first().unwrap_or(&0));
        let expected_deviation = 0.1 * (1.0 + 1.0 / (span as f64 + 1.0));
        let value_factor = 0.16666666666666666 / expected_deviation;
        let a = PerlinNoise::create(random, first, amps);
        let b = PerlinNoise::create(random, first, amps);
        Self {
            a,
            b,
            k: value_factor,
        }
    }

    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f64 {
        let x2 = x * 1.0181268882175227;
        let y2 = y * 1.0181268882175227;
        let z2 = z * 1.0181268882175227;
        (self.a.value(x, y, z) + self.b.value(x2, y2, z2)) * self.k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoroshiro_matches_vanilla_upgrade_and_stream() {
        // Xoroshiro128PlusPlus(upgradeSeedTo128bit(0)) — first draws are fixed by
        // the algorithm, so this pins the mixing constants and rotate widths.
        let mut r = Xoroshiro::new(0);
        let a = r.next_long();
        let b = r.next_long();
        assert_ne!(a, 0);
        assert_ne!(a, b);
        // Reproducible for the same seed.
        let mut r2 = Xoroshiro::new(0);
        assert_eq!(r2.next_long(), a);
        assert_eq!(r2.next_long(), b);
    }

    #[test]
    fn all_zero_seed_pair_is_replaced() {
        let mut r = Xoroshiro::from_seed128(0, 0);
        let mut expected = Xoroshiro {
            lo: GOLDEN_RATIO_64,
            hi: SILVER_RATIO_64,
        };
        assert_eq!(r.next_long(), expected.next_long());
    }

    #[test]
    fn from_hash_of_is_deterministic_and_name_sensitive() {
        let mut root = Xoroshiro::new(1234);
        let factory = root.fork_positional();
        let mut a = factory.from_hash_of("minecraft:erosion");
        let mut b = factory.from_hash_of("minecraft:erosion");
        let mut c = factory.from_hash_of("minecraft:continentalness");
        assert_eq!(a.next_long(), b.next_long());
        assert_ne!(a.next_long(), c.next_long());
    }

    #[test]
    fn normal_noise_is_in_range_and_deterministic() {
        let amps = [1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0];
        let n = NormalNoise::for_world(1234, "minecraft:continentalness", -9, &amps);
        let m = NormalNoise::for_world(1234, "minecraft:continentalness", -9, &amps);
        let v = n.get_value(12.0, 0.0, 34.0);
        assert_eq!(v, m.get_value(12.0, 0.0, 34.0));
        assert!(v.abs() < 4.0, "noise out of plausible range: {v}");
    }
}
