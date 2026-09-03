//! Vanilla-compatible Minecraft simplex noise and small legacy helpers.
use std::fs;
use std::path::Path;

const GRAD3: [[f64; 3]; 12] = [
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
];
const F3: f64 = 1.0 / 3.0;
const G3: f64 = 1.0 / 6.0;
const TRIG_MULTIPLIER: f64 = std::f64::consts::PI;

/// Java's 48-bit Random, required because Minecraft seeds noise with Java Random.
#[derive(Clone)]
pub struct JavaRandom(u64);
impl JavaRandom {
    pub fn new(seed: i64) -> Self {
        Self((((seed as u64) ^ 0x5deece66d) & ((1u64 << 48) - 1)))
    }
    fn next(&mut self, bits: u32) -> u32 {
        self.0 = (self.0.wrapping_mul(0x5deece66d).wrapping_add(0xb)) & ((1u64 << 48) - 1);
        (self.0 >> (48 - bits)) as u32
    }
    pub fn next_double(&mut self) -> f64 {
        (((self.next(26) as u64) << 27 | self.next(27) as u64) as f64) / 9007199254740992.0
    }
    pub fn next_int(&mut self, bound: usize) -> usize {
        let b = bound as u32;
        if b.is_power_of_two() {
            return ((b as u64 * self.next(31) as u64) >> 31) as usize;
        }
        loop {
            let r = self.next(31);
            let m = r % b;
            if r.wrapping_sub(m).wrapping_add(b - 1) < (1u32 << 31) {
                return m as usize;
            }
        }
    }
    /// Java `Random.nextLong` — combines two 32-bit draws, as vanilla's
    /// `WorldgenRandom` does for `fork()`.
    pub fn next_long(&mut self) -> i64 {
        ((self.next(32) as u64) << 32 | self.next(32) as u64) as i64
    }
    /// Vanilla `RandomSource.fork()`: a fresh random seeded from this one.
    pub fn fork(&mut self) -> Self {
        Self::new(self.next_long())
    }
}

/// Xoroshiro128++ random source used by modern world generation.
#[derive(Clone)]
pub struct Xoroshiro128 {
    lo: u64,
    hi: u64,
}

impl Xoroshiro128 {
    /// `upgradeSeedTo128bit(seed).mixed()` — exactly as `RandomSupport`.
    pub fn new(seed: i64) -> Self {
        let lo = (seed as u64) ^ 0x6a09e667f3bcc909u64; // SILVER_RATIO_64
        let hi = lo.wrapping_add(0x9e3779b97f4a7c15); // GOLDEN_RATIO_64
        Self {
            lo: mix_stafford13(lo),
            hi: mix_stafford13(hi),
        }
    }

    pub fn from_state(lo: u64, hi: u64) -> Self {
        Self { lo, hi }
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

    pub fn next_bits(&mut self, bits: u32) -> u64 {
        self.next_long() >> (64 - bits)
    }
}

#[inline]
fn mix_stafford13(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// `WorldgenRandom` — a `BitRandomSource` wrapper, exactly as vanilla.
pub struct WorldgenRandom {
    pub source: Xoroshiro128,
    count: u32,
}
impl WorldgenRandom {
    pub fn new(seed: i64) -> Self {
        Self {
            source: Xoroshiro128::new(seed),
            count: 0,
        }
    }
    pub fn set_seed(&mut self, seed: i64) {
        self.source = Xoroshiro128::new(seed);
    }
    fn next_bits(&mut self, bits: u32) -> u64 {
        self.count += 1;
        self.source.next_bits(bits)
    }
    /// `BitRandomSource.nextLong` = `((next(32) << 32) | next(32))`.
    pub fn next_long(&mut self) -> i64 {
        ((self.next_bits(32) << 32) | self.next_bits(32)) as i64
    }
    /// `BitRandomSource.nextInt(bound)` — `next(31)` rejection.
    pub fn next_int(&mut self, bound: usize) -> usize {
        let b = bound as u32;
        if b.is_power_of_two() {
            return ((b as u64 * self.next_bits(31)) >> 31) as usize;
        }
        loop {
            let r = self.next_bits(31) as u32;
            let m = r % b;
            if r.wrapping_sub(m).wrapping_add(b - 1) < (1u32 << 31) {
                return m as usize;
            }
        }
    }
    pub fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 * 5.9604645e-8_f32
    }
    /// `BitRandomSource.nextDouble` = `((next(26) << 27) | next(27)) * DOUBLE_UNIT`.
    pub fn next_double(&mut self) -> f64 {
        ((self.next_bits(26) << 27) | self.next_bits(27)) as f64 * 1.1102230246251565e-16
    }
    pub fn set_decoration_seed(&mut self, seed: i64, chunk_x: i32, chunk_z: i32) -> i64 {
        self.set_seed(seed);
        let x_scale = self.next_long() | 1;
        let z_scale = self.next_long() | 1;
        let result = (chunk_x as i64)
            .wrapping_mul(x_scale)
            .wrapping_add((chunk_z as i64).wrapping_mul(z_scale))
            ^ seed;
        self.set_seed(result);
        result
    }
    pub fn set_feature_seed(&mut self, seed: i64, index: i32, step: i32) {
        self.set_seed(
            seed.wrapping_add(index as i64)
                .wrapping_add(10000i64.wrapping_mul(step as i64)),
        );
    }
}

/// The exact 3D `net.minecraft.world.level.levelgen.synth.SimplexNoise`.
#[derive(Clone)]
pub struct SimplexNoise {
    pub xo: f64,
    pub yo: f64,
    pub zo: f64,
    pub p: [u8; 256],
}
impl SimplexNoise {
    pub fn new(seed: i64) -> Self {
        let mut r = JavaRandom::new(seed);
        let xo = r.next_double() * 256.;
        let yo = r.next_double() * 256.;
        let zo = r.next_double() * 256.;
        let mut p = [0u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256 {
            let j = i + r.next_int(256 - i);
            p.swap(i, j);
        }
        Self { xo, yo, zo, p }
    }
    #[inline]
    fn grad(&self, x: i32, y: i32, z: i32) -> [f64; 3] {
        let h = self.p[((self.p[((self.p[(x & 255) as usize] as i32 + (y & 255)) & 255) as usize]
            as i32
            + (z & 255))
            & 255) as usize] as usize
            % 12;
        GRAD3[h]
    }
    #[inline]
    fn corner(&self, x: f64, y: f64, z: f64, ix: i32, iy: i32, iz: i32) -> f64 {
        let q = 0.6 - x * x - y * y - z * z;
        if q < 0. {
            0.
        } else {
            let q2 = q * q;
            let g = self.grad(ix, iy, iz);
            q2 * q2 * (g[0] * x + g[1] * y + g[2] * z)
        }
    }
    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f64 {
        let x = x + self.xo;
        let y = y + self.yo;
        let z = z + self.zo;
        let s = (x + y + z) * F3;
        let i = (x + s).floor() as i32;
        let j = (y + s).floor() as i32;
        let k = (z + s).floor() as i32;
        let t = i.wrapping_add(j).wrapping_add(k) as f64 * G3;
        let x0 = x - (i as f64 - t);
        let y0 = y - (j as f64 - t);
        let z0 = z - (k as f64 - t);
        let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
            if y0 >= z0 {
                (1, 0, 0, 1, 1, 0)
            } else if x0 >= z0 {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };
        let n0 = self.corner(x0, y0, z0, i, j, k);
        let n1 = self.corner(
            x0 - i1 as f64 + G3,
            y0 - j1 as f64 + G3,
            z0 - k1 as f64 + G3,
            i.wrapping_add(i1),
            j.wrapping_add(j1),
            k.wrapping_add(k1),
        );
        let n2 = self.corner(
            x0 - i2 as f64 + 2. * G3,
            y0 - j2 as f64 + 2. * G3,
            z0 - k2 as f64 + 2. * G3,
            i.wrapping_add(i2),
            j.wrapping_add(j2),
            k.wrapping_add(k2),
        );
        let n3 = self.corner(
            x0 - 1. + 3. * G3,
            y0 - 1. + 3. * G3,
            z0 - 1. + 3. * G3,
            i.wrapping_add(1),
            j.wrapping_add(1),
            k.wrapping_add(1),
        );
        32. * (n0 + n1 + n2 + n3)
    }
}

/// Per-thread cache of `SimplexNoise` instances keyed by seed.
///
/// `SimplexNoise::new` shuffles a 256-entry permutation table, which is far too
/// expensive to repeat for every sampled block. Worldgen reuses a handful of
/// seeds per octave, so caching collapses that cost to a hash lookup + a 256-byte
/// clone.
thread_local! {
    static NORMAL_CACHE: std::cell::RefCell<std::collections::HashMap<i64, crate::noise_perlin::NormalNoise>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}
fn cached_normal(seed: i64, first: i32, amplitudes: &[f64]) -> crate::noise_perlin::NormalNoise {
    NORMAL_CACHE.with(|c| {
        let mut m = c.borrow_mut();
        m.entry(seed)
            .or_insert_with(|| crate::noise_perlin::NormalNoise::new(seed, first, amplitudes))
            .clone()
    })
}

fn cached_simplex(seed: i64) -> SimplexNoise {
    SimplexNoise::new(seed)
}

#[derive(Clone, Debug)]
pub struct NoiseDefinition {
    pub first_octave: i32,
    pub amplitudes: Vec<f64>,
}
#[derive(Clone, Default)]
pub struct NoiseRegistry {
    pub defs: std::collections::HashMap<String, NoiseDefinition>,
}
impl NoiseRegistry {
    pub fn load_dir<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let mut out = Self::default();
        for e in fs::read_dir(path)? {
            let e = e?;
            if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let s = fs::read_to_string(e.path())?;
            let first = extract_num(&s, "firstOctave").unwrap_or(0.) as i32;
            let amplitudes = extract_array(&s, "amplitudes");
            if let Some(n) = e.path().file_stem().and_then(|x| x.to_str()) {
                out.defs.insert(
                    n.to_string(),
                    NoiseDefinition {
                        first_octave: first,
                        amplitudes,
                    },
                );
            }
        }
        Ok(out)
    }
    pub fn sample(&self, name: &str, seed: i64, x: f64, y: f64, z: f64) -> f64 {
        let key = name.rsplit(':').next().unwrap_or(name);
        let Some(d) = self.defs.get(key) else {
            return 0.;
        };
        // Vanilla RandomState: `random = WorldgenRandom(LegacyRandomSource(seed)).forkPositional()`,
        // then each noise is `NormalNoise.create(random.fromHashOf("minecraft:<key>"), ...)`.
        // fromHashOf(name) = LegacyRandomSource(name.hashCode() ^ forkSeed).
        let fork_seed = fork_seed_for(seed);
        let noise_seed =
            crate::noise_perlin::java_string_hash(&format!("minecraft:{key}")) as i64 ^ fork_seed;
        let n = cached_normal(noise_seed, d.first_octave, &d.amplitudes);
        n.get_value(x, y, z)
    }
}

thread_local! {
    static FORK_SEED_CACHE: std::cell::RefCell<std::collections::HashMap<i64, i64>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}
fn fork_seed_for(seed: i64) -> i64 {
    FORK_SEED_CACHE.with(|c| {
        let mut m = c.borrow_mut();
        *m.entry(seed)
            .or_insert_with(|| JavaRandom::new(seed).next_long())
    })
}
fn extract_num(s: &str, k: &str) -> Option<f64> {
    let p = s.find(&format!("\"{k}\""))?;
    s[p..]
        .split(':')
        .nth(1)?
        .trim_start()
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .next()?
        .parse()
        .ok()
}
fn extract_array(s: &str, k: &str) -> Vec<f64> {
    let Some(p) = s.find(&format!("\"{k}\"")) else {
        return vec![];
    };
    let Some(a) = s[p..].find('[') else {
        return vec![];
    };
    let Some(b) = s[p + a..].find(']') else {
        return vec![];
    };
    s[p + a + 1..p + a + b]
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect()
}

pub fn splitmix64(mut v: u64) -> u64 {
    v = v.wrapping_add(0x9e3779b97f4a7c15);
    let mut r = v;
    r = (r ^ (r >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    r = (r ^ (r >> 27)).wrapping_mul(0x94d049bb133111eb);
    r ^ (r >> 31)
}
pub fn hash_2d(seed: i64, x: i64, z: i64) -> f64 {
    ((splitmix64(splitmix64(
        seed as u64
            ^ (x as u64).wrapping_mul(0x9e3779b985ebca87)
            ^ (z as u64).wrapping_mul(0xc2b2ae3d27d4eb4f),
    )) >> 11) as f64)
        / 9007199254740992.
}
pub fn value_noise_2d(seed: i64, x: f64, z: f64) -> f64 {
    cached_simplex(seed).get_value(x, 0., z)
}
pub fn value_noise_3d(seed: i64, x: f64, y: f64, z: f64) -> f64 {
    cached_simplex(seed).get_value(x, y, z)
}
pub fn fbm2(seed: i64, x: f64, z: f64, scale: f64, octaves: u32, persistence: f64) -> f64 {
    fbm3(seed, x, 0., z, scale, octaves, persistence)
}
pub fn fbm3(seed: i64, x: f64, y: f64, z: f64, scale: f64, octaves: u32, persistence: f64) -> f64 {
    let mut t = 0.;
    let mut a = 1.;
    let mut n = 0.;
    for i in 0..octaves {
        t += value_noise_3d(seed + i as i64, x / scale, y / scale, z / scale) * a;
        n += a;
        a *= persistence;
    }
    if n == 0. {
        0.
    } else {
        t / n
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simplex_deterministic() {
        let a = SimplexNoise::new(1);
        assert_eq!(a.get_value(1.2, 3.4, 5.6), a.get_value(1.2, 3.4, 5.6));
    }
}
