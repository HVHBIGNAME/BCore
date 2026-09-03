//! Vanilla `ImprovedNoise`/`PerlinNoise`/`NormalNoise` — exact port.
//!
//! The entire vanilla noise pipeline is seeded from a `LegacyRandomSource`
//! (Java 48-bit LCG): `PerlinNoise.create` forks positionally via `nextLong()`,
//! then each octave is `ImprovedNoise::new(LegacyRandomSource(name.hashCode() ^ forkSeed))`.
//! Xoroshiro is only used for *feature placement*, never for noise.

use crate::simplex::JavaRandom;

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

/// Java `String.hashCode()` (identical for ASCII, which all octave names are).
pub fn java_string_hash(s: &str) -> i32 {
    let mut h = 0i32;
    for &b in s.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as i32);
    }
    h
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
    fn from_random(r: &mut JavaRandom) -> Self {
        let xo = r.next_double() * 256.0;
        let yo = r.next_double() * 256.0;
        let zo = r.next_double() * 256.0;
        let mut p = [0u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256 {
            let offset = r.next_int(256 - i);
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
        let yr_fudge;
        if y_scale != 0.0 {
            let limit = if y_fudge >= 0.0 && y_fudge < yr {
                y_fudge
            } else {
                yr
            };
            yr_fudge = (limit / y_scale + 1.0e-7).floor() * y_scale;
        } else {
            yr_fudge = 0.0;
        }
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
    /// Mirrors `PerlinNoise.create(random, firstOctave, amplitudes)` with
    /// `useNewInitialization = true`: one `forkPositional()` (a single `nextLong`),
    /// then `ImprovedNoise(LegacyRandomSource("octave_<n>".hashCode() ^ forkSeed))`.
    fn create(r: &mut JavaRandom, first: i32, amps: &[f64]) -> Self {
        let fork_seed = r.next_long();
        let levels = amps
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if *a == 0.0 {
                    None
                } else {
                    let octave = first + i as i32;
                    let h = java_string_hash(&format!("octave_{}", octave));
                    let mut octave_random = JavaRandom::new((h as i64) ^ fork_seed);
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
    /// Mirrors `NormalNoise.create(random, firstOctave, amplitudes)`: `first` and
    /// `second` are both `PerlinNoise.create` from the *same* sequential random.
    pub fn new(seed: i64, first: i32, amps: &[f64]) -> Self {
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
        let mut r = JavaRandom::new(seed);
        let a = PerlinNoise::create(&mut r, first, amps);
        let b = PerlinNoise::create(&mut r, first, amps);
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
