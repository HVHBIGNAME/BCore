//! Deterministic value noise used by the world generator.
//!
//! Everything here is a pure function of `(seed, coordinates)`:
//!
//! * [`splitmix64`] — the mixing function that turns a lattice coordinate into
//!   pseudo-random bits.
//! * [`value_noise_2d`] / [`value_noise_3d`] — smoothstep-interpolated value
//!   noise on the integer lattice, in `-1.0..=1.0`.
//! * [`fbm2`] / [`fbm3`] — fractal (multi-octave) sums, normalised so the result
//!   stays in `-1.0..=1.0` regardless of octave count.
//!
//! No floating-point state, no tables, no `rand`, no time: identical inputs give
//! bit-identical outputs on every run and every platform that uses IEEE-754
//! doubles.

/// SplitMix64 finaliser — a strong 64-bit avalanche mix.
#[inline]
pub fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut result = value;
    result = (result ^ (result >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    result = (result ^ (result >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    result ^ (result >> 31)
}

/// A lattice value in `-1.0..=1.0` for the 2D integer point `(x, z)`.
#[inline]
fn lattice_2d(seed: i64, x: i64, z: i64) -> f64 {
    let mixed = (seed as u64)
        ^ (x as u64).wrapping_mul(0x9e37_79b9_85eb_ca87)
        ^ (z as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    to_unit(splitmix64(mixed))
}

/// A lattice value in `-1.0..=1.0` for the 3D integer point `(x, y, z)`.
#[inline]
fn lattice_3d(seed: i64, x: i64, y: i64, z: i64) -> f64 {
    let mixed = (seed as u64)
        ^ (x as u64).wrapping_mul(0x9e37_79b9_85eb_ca87)
        ^ (y as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
        ^ (z as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    to_unit(splitmix64(mixed))
}

/// Map 64 random bits onto `-1.0..=1.0`.
#[inline]
fn to_unit(bits: u64) -> f64 {
    // Use the top 53 bits so the value is exactly representable as an f64.
    let unit = (bits >> 11) as f64 / (1u64 << 53) as f64; // 0.0..1.0
    unit * 2.0 - 1.0
}

/// A hash of an integer 2D position in `0.0..1.0`, for probabilistic placement.
///
/// Unlike [`value_noise_2d`] this is *not* interpolated: neighbouring positions
/// are uncorrelated, which is what feature scattering (trees, plants) wants.
#[inline]
pub fn hash_2d(seed: i64, x: i64, z: i64) -> f64 {
    let mixed = (seed as u64)
        ^ (x as u64).wrapping_mul(0x9e37_79b9_85eb_ca87)
        ^ (z as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    let bits = splitmix64(splitmix64(mixed));
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// Smoothstep, so interpolated noise has continuous first derivatives.
#[inline]
fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Smoothstep-interpolated 2D value noise in `-1.0..=1.0`.
pub fn value_noise_2d(seed: i64, x: f64, z: f64) -> f64 {
    let x0 = x.floor();
    let z0 = z.floor();
    let xi = x0 as i64;
    let zi = z0 as i64;
    let tx = smooth(x - x0);
    let tz = smooth(z - z0);

    let a = lattice_2d(seed, xi, zi);
    let b = lattice_2d(seed, xi + 1, zi);
    let c = lattice_2d(seed, xi, zi + 1);
    let d = lattice_2d(seed, xi + 1, zi + 1);

    let ab = a + (b - a) * tx;
    let cd = c + (d - c) * tx;
    ab + (cd - ab) * tz
}

/// Smoothstep-interpolated 3D value noise in `-1.0..=1.0`.
pub fn value_noise_3d(seed: i64, x: f64, y: f64, z: f64) -> f64 {
    let x0 = x.floor();
    let y0 = y.floor();
    let z0 = z.floor();
    let xi = x0 as i64;
    let yi = y0 as i64;
    let zi = z0 as i64;
    let tx = smooth(x - x0);
    let ty = smooth(y - y0);
    let tz = smooth(z - z0);

    let c000 = lattice_3d(seed, xi, yi, zi);
    let c100 = lattice_3d(seed, xi + 1, yi, zi);
    let c010 = lattice_3d(seed, xi, yi + 1, zi);
    let c110 = lattice_3d(seed, xi + 1, yi + 1, zi);
    let c001 = lattice_3d(seed, xi, yi, zi + 1);
    let c101 = lattice_3d(seed, xi + 1, yi, zi + 1);
    let c011 = lattice_3d(seed, xi, yi + 1, zi + 1);
    let c111 = lattice_3d(seed, xi + 1, yi + 1, zi + 1);

    let x00 = c000 + (c100 - c000) * tx;
    let x10 = c010 + (c110 - c010) * tx;
    let x01 = c001 + (c101 - c001) * tx;
    let x11 = c011 + (c111 - c011) * tx;

    let y0v = x00 + (x10 - x00) * ty;
    let y1v = x01 + (x11 - x01) * ty;

    y0v + (y1v - y0v) * tz
}

/// Fractal 2D noise: `octaves` octaves at halving wavelength, in `-1.0..=1.0`.
///
/// `scale` is the wavelength of the first octave in blocks; `persistence` is the
/// amplitude ratio between successive octaves.
pub fn fbm2(seed: i64, x: f64, z: f64, scale: f64, octaves: u32, persistence: f64) -> f64 {
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut normaliser = 0.0;
    let mut frequency = 1.0 / scale;
    for octave in 0..octaves {
        // Salting the seed per octave keeps the octaves independent.
        let octave_seed =
            splitmix64((seed as u64) ^ (octave as u64 + 1).wrapping_mul(0x2545_f491_4f6c_dd1d))
                as i64;
        total += value_noise_2d(octave_seed, x * frequency, z * frequency) * amplitude;
        normaliser += amplitude;
        amplitude *= persistence;
        frequency *= 2.0;
    }
    if normaliser == 0.0 {
        return 0.0;
    }
    total / normaliser
}

/// Fractal 3D noise: `octaves` octaves at halving wavelength, in `-1.0..=1.0`.
pub fn fbm3(seed: i64, x: f64, y: f64, z: f64, scale: f64, octaves: u32, persistence: f64) -> f64 {
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut normaliser = 0.0;
    let mut frequency = 1.0 / scale;
    for octave in 0..octaves {
        let octave_seed =
            splitmix64((seed as u64) ^ (octave as u64 + 1).wrapping_mul(0x2545_f491_4f6c_dd1d))
                as i64;
        total +=
            value_noise_3d(octave_seed, x * frequency, y * frequency, z * frequency) * amplitude;
        normaliser += amplitude;
        amplitude *= persistence;
        frequency *= 2.0;
    }
    if normaliser == 0.0 {
        return 0.0;
    }
    total / normaliser
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_stable_and_avalanches() {
        // Regression pins: these exact values must never drift, or every saved
        // chunk in the world directory would decode to different terrain.
        assert_eq!(splitmix64(0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(splitmix64(1), 0x910a_2dec_89025cc1);
        // One input bit flips roughly half the output bits.
        let flipped = (splitmix64(12345) ^ splitmix64(12345 ^ 1)).count_ones();
        assert!(
            (16..=48).contains(&flipped),
            "weak avalanche: {flipped} bits"
        );
    }

    #[test]
    fn value_noise_is_bounded_and_deterministic() {
        for i in 0..2000 {
            let x = i as f64 * 0.37;
            let z = i as f64 * -0.11;
            let n = value_noise_2d(42, x, z);
            assert!((-1.0..=1.0).contains(&n), "2d noise out of range: {n}");
            assert_eq!(n, value_noise_2d(42, x, z), "2d noise not deterministic");

            let m = value_noise_3d(42, x, z * 0.5, z);
            assert!((-1.0..=1.0).contains(&m), "3d noise out of range: {m}");
            assert_eq!(m, value_noise_3d(42, x, z * 0.5, z));
        }
    }

    #[test]
    fn value_noise_hits_lattice_values_exactly() {
        // At integer coordinates the interpolation weights are 0, so the noise
        // must equal the raw lattice value.
        for x in -5..5i64 {
            for z in -5..5i64 {
                let expect = lattice_2d(7, x, z);
                assert_eq!(value_noise_2d(7, x as f64, z as f64), expect);
            }
        }
    }

    #[test]
    fn value_noise_is_continuous() {
        // Neighbouring samples must not jump: max slope over a 0.01 step is small.
        let mut max_jump = 0.0f64;
        let mut previous = value_noise_2d(9, 0.0, 3.3);
        for i in 1..1000 {
            let n = value_noise_2d(9, i as f64 * 0.01, 3.3);
            max_jump = max_jump.max((n - previous).abs());
            previous = n;
        }
        assert!(
            max_jump < 0.1,
            "noise is discontinuous, max jump {max_jump}"
        );
    }

    #[test]
    fn different_seeds_decorrelate() {
        let a: Vec<f64> = (0..200)
            .map(|i| value_noise_2d(1, i as f64 * 0.3, 0.7))
            .collect();
        let b: Vec<f64> = (0..200)
            .map(|i| value_noise_2d(2, i as f64 * 0.3, 0.7))
            .collect();
        assert_ne!(a, b);
        let same = a.iter().zip(&b).filter(|(x, y)| x == y).count();
        assert!(same < 5, "seeds are correlated: {same}/200 identical");
    }

    #[test]
    fn fbm_stays_in_range_and_is_deterministic() {
        for i in 0..500 {
            let x = i as f64 * 1.7;
            let z = i as f64 * -2.3;
            let n = fbm2(5, x, z, 64.0, 4, 0.5);
            assert!((-1.0..=1.0).contains(&n), "fbm2 out of range: {n}");
            assert_eq!(n, fbm2(5, x, z, 64.0, 4, 0.5));

            let m = fbm3(5, x, 30.0, z, 64.0, 3, 0.5);
            assert!((-1.0..=1.0).contains(&m), "fbm3 out of range: {m}");
            assert_eq!(m, fbm3(5, x, 30.0, z, 64.0, 3, 0.5));
        }
    }

    #[test]
    fn fbm_adds_detail_over_a_single_octave() {
        // More octaves must change the field (detail is actually being added).
        let one = fbm2(11, 100.0, 200.0, 64.0, 1, 0.5);
        let four = fbm2(11, 100.0, 200.0, 64.0, 4, 0.5);
        assert_ne!(one, four);
        // Zero octaves is defined as flat.
        assert_eq!(fbm2(11, 100.0, 200.0, 64.0, 0, 0.5), 0.0);
    }

    #[test]
    fn hash_is_uniform_ish_and_uncorrelated() {
        let mut buckets = [0usize; 10];
        for x in 0..100i64 {
            for z in 0..100i64 {
                let h = hash_2d(3, x, z);
                assert!((0.0..1.0).contains(&h), "hash out of range: {h}");
                buckets[(h * 10.0) as usize % 10] += 1;
            }
        }
        // 10000 samples over 10 buckets: expect ~1000 each, allow generous slack.
        for (i, &count) in buckets.iter().enumerate() {
            assert!(
                (700..=1300).contains(&count),
                "bucket {i} has {count} samples, distribution is skewed"
            );
        }
    }

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(hash_2d(1, 5, -9), hash_2d(1, 5, -9));
        assert_ne!(hash_2d(1, 5, -9), hash_2d(2, 5, -9));
        assert_ne!(hash_2d(1, 5, -9), hash_2d(1, -9, 5));
    }
}
