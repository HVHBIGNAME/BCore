//! Vanilla-compatible randomness for world generation.
//!
//! World generation in vanilla is driven by `XoroshiroRandomSource`
//! (Xoroshiro128++) wrapped in `WorldgenRandom` for feature decoration. Feature
//! positions, tree heights and foliage shapes are all functions of the exact
//! sequence this generator produces, so byte-for-byte terrain parity is only
//! possible with a bit-exact port.
//!
//! The values below are checked against the equivalent Java implementation; see
//! the tests at the bottom of this file for the reference vectors.

const GOLDEN_RATIO_64: u64 = 0x9E37_79B9_7F4A_7C15;
const SILVER_RATIO_64: u64 = 0x6A09_E667_F3BC_C909;

/// Vanilla `XoroshiroRandomSource`.
#[derive(Clone)]
pub struct Xoroshiro {
    seed_lo: u64,
    seed_hi: u64,
    /// Cached second gaussian sample (vanilla stores `NaN` when empty).
    next_gaussian: f64,
}

/// Splitter produced by [`Xoroshiro::next_positional`].
///
/// Vanilla uses this to derive a random source that is a pure function of a
/// position, which is how structures and ores stay deterministic per chunk.
#[derive(Clone)]
pub struct XoroshiroSplitter {
    seed_lo: u64,
    seed_hi: u64,
}

impl Xoroshiro {
    /// Vanilla `XoroshiroRandomSource(long)` — seed is spread and mixed.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        let (lo, hi) = Self::upgrade_seed_to_128_bit(seed);
        Self::new(mix_stafford_13(lo), mix_stafford_13(hi))
    }

    /// Construction without the Stafford mix, matching vanilla's internal
    /// `XoroshiroRandomSource(long, long)` paths.
    #[must_use]
    pub const fn from_seed_unmixed(seed: u64) -> Self {
        let (lo, hi) = Self::upgrade_seed_to_128_bit(seed);
        Self::new(lo, hi)
    }

    const fn new(seed_lo: u64, seed_hi: u64) -> Self {
        // Vanilla substitutes the golden/silver ratios when both halves are zero.
        let (seed_lo, seed_hi) = if (seed_lo | seed_hi) == 0 {
            (GOLDEN_RATIO_64, SILVER_RATIO_64)
        } else {
            (seed_lo, seed_hi)
        };
        Self {
            seed_lo,
            seed_hi,
            next_gaussian: f64::NAN,
        }
    }

    const fn upgrade_seed_to_128_bit(seed: u64) -> (u64, u64) {
        let lo = seed ^ SILVER_RATIO_64;
        (lo, lo.wrapping_add(GOLDEN_RATIO_64))
    }

    /// Core Xoroshiro128++ step.
    const fn next_random(&mut self) -> u64 {
        let l = self.seed_lo;
        let m = self.seed_hi;
        let n = l.wrapping_add(m).rotate_left(17).wrapping_add(l);
        let m = m ^ l;
        self.seed_lo = l.rotate_left(49) ^ m ^ (m << 21);
        self.seed_hi = m.rotate_left(28);
        n
    }

    const fn next_bits(&mut self, bits: u64) -> u64 {
        self.next_random() >> (64 - bits)
    }

    /// Vanilla `XoroshiroRandomSource.setSeed(long)`.
    pub const fn set_seed(&mut self, seed: i64) {
        *self = Self::from_seed(seed as u64);
    }

    /// Vanilla `RandomSource.fork()`.
    pub fn fork(&mut self) -> Self {
        Self::new(self.next_random(), self.next_random())
    }

    /// Vanilla `XoroshiroRandomSource.nextInt()` (raw 32 bits, no `next_i64`).
    pub fn next_i32(&mut self) -> i32 {
        self.next_random() as i32
    }

    /// Vanilla `RandomSupport.generateRandom` — Lemire's bounded method.
    pub fn next_i32_bounded(&mut self, bound: i32) -> i32 {
        debug_assert!(bound > 0, "bound must be positive");
        let mut l = u64::from(self.next_i32() as u32);
        let mut m = l.wrapping_mul(bound as u64);
        let mut n = m & 0xFFFF_FFFF;
        if n < bound as u64 {
            let threshold = u64::from(((!bound as u32).wrapping_add(1)) % bound as u32);
            while n < threshold {
                l = u64::from(self.next_i32() as u32);
                m = l.wrapping_mul(bound as u64);
                n = m & 0xFFFF_FFFF;
            }
        }
        (m >> 32) as i32
    }

    /// Vanilla `nextInt(min, max)` inclusive.
    pub fn next_i32_between(&mut self, min: i32, max: i32) -> i32 {
        self.next_i32_bounded(max - min + 1) + min
    }

    /// Vanilla `XoroshiroRandomSource.nextLong()`.
    pub fn next_i64(&mut self) -> i64 {
        self.next_random() as i64
    }

    /// Vanilla `XoroshiroRandomSource.nextFloat()`.
    pub fn next_f32(&mut self) -> f32 {
        self.next_bits(24) as f32 * 5.960_464_5e-8
    }

    /// Vanilla `XoroshiroRandomSource.nextDouble()`.
    pub fn next_f64(&mut self) -> f64 {
        self.next_bits(53) as f64 * f64::from(1.110_223e-16_f32)
    }

    /// Vanilla `XoroshiroRandomSource.nextBoolean()`.
    pub fn next_bool(&mut self) -> bool {
        (self.next_random() & 1) != 0
    }

    /// Vanilla `nextGaussian()` (Marsaglia polar, with cached second sample).
    pub fn next_gaussian(&mut self) -> f64 {
        if !self.next_gaussian.is_nan() {
            let value = self.next_gaussian;
            self.next_gaussian = f64::NAN;
            return value;
        }
        loop {
            let d = 2.0 * self.next_f64() - 1.0;
            let e = 2.0 * self.next_f64() - 1.0;
            let f = d * d + e * e;
            if f < 1.0 && f != 0.0 {
                let g = (-2.0 * f.ln() / f).sqrt();
                self.next_gaussian = e * g;
                return d * g;
            }
        }
    }

    /// Vanilla `RandomSource.nextPositional()`.
    pub fn next_positional(&mut self) -> XoroshiroSplitter {
        XoroshiroSplitter {
            seed_lo: self.next_random(),
            seed_hi: self.next_random(),
        }
    }
}

impl XoroshiroSplitter {
    /// Vanilla positional seeding: `getSeed(x, y, z) ^ seed_lo`.
    #[must_use]
    pub const fn at(&self, x: i32, y: i32, z: i32) -> Xoroshiro {
        Xoroshiro::new(get_seed(x, y, z) as u64 ^ self.seed_lo, self.seed_hi)
    }

    /// Positional source derived from an explicit seed rather than a position.
    #[must_use]
    pub const fn with_seed(&self, seed: u64) -> Xoroshiro {
        Xoroshiro::new(seed ^ self.seed_lo, seed ^ self.seed_hi)
    }
}

/// Vanilla `WorldgenRandom` — the wrapper used during biome decoration.
///
/// Note it does *not* equal a bare `XoroshiroRandomSource`: every `next*` goes
/// through `nextBits` on top of `nextLong`, which changes the bit stream.
pub struct WorldgenRandom {
    source: Xoroshiro,
    next_gaussian: Option<f64>,
}

impl WorldgenRandom {
    /// Creates a generator from a seed.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self {
            source: Xoroshiro::from_seed(seed),
            next_gaussian: None,
        }
    }

    /// Vanilla `WorldgenRandom.setSeed` only reseeds the wrapped source and
    /// deliberately keeps a pending gaussian cached.
    pub const fn set_seed(&mut self, seed: i64) {
        self.source.set_seed(seed);
    }

    /// Vanilla `WorldgenRandom.setDecorationSeed`.
    pub fn set_decoration_seed(&mut self, seed: i64, block_x: i32, block_z: i32) -> i64 {
        self.set_seed(seed);
        let x_scale = self.next_i64() | 1;
        let z_scale = self.next_i64() | 1;
        let decoration_seed = i64::from(block_x)
            .wrapping_mul(x_scale)
            .wrapping_add(i64::from(block_z).wrapping_mul(z_scale))
            ^ seed;
        self.set_seed(decoration_seed);
        decoration_seed
    }

    /// Vanilla `WorldgenRandom.setFeatureSeed`.
    pub const fn set_feature_seed(&mut self, decoration_seed: i64, feature_index: i32, step: i32) {
        let feature_seed = decoration_seed
            .wrapping_add(feature_index as i64)
            .wrapping_add(10_000_i64.wrapping_mul(step as i64));
        self.set_seed(feature_seed);
    }

    /// Vanilla `WorldgenRandom.next(int)` — draws from `nextLong`, not `nextInt`.
    fn next_bits(&mut self, bits: u64) -> u64 {
        self.source.next_i64() as u64 >> (64 - bits)
    }

    /// Vanilla `nextInt()`.
    pub fn next_i32(&mut self) -> i32 {
        self.next_bits(32) as i32
    }

    /// Vanilla `nextInt(int bound)` — power-of-two fast path plus rejection.
    pub fn next_i32_bounded(&mut self, bound: i32) -> i32 {
        debug_assert!(bound > 0, "bound must be positive");
        if bound & bound.wrapping_sub(1) == 0 {
            (i64::from(bound).wrapping_mul(i64::from(self.next_bits(31) as i32)) >> 31) as i32
        } else {
            loop {
                let sample = self.next_bits(31) as i32;
                let modulo = sample % bound;
                if sample
                    .wrapping_sub(modulo)
                    .wrapping_add(bound.wrapping_sub(1))
                    >= 0
                {
                    return modulo;
                }
            }
        }
    }

    /// Vanilla `nextInt(int min, int max)` inclusive.
    pub fn next_i32_between(&mut self, min: i32, max: i32) -> i32 {
        self.next_i32_bounded(max - min + 1) + min
    }

    /// Vanilla `BitRandomSource.nextLong()` — `((long)i << 32) + (long)j`.
    /// Both words are sign-extended and ADDED (bytecode: `i2l; lshl; ladd`),
    /// NOT masked-OR'd, matching Java's signed-word addition.
    pub fn next_i64(&mut self) -> i64 {
        let upper = self.next_i32();
        let lower = self.next_i32();
        (i64::from(upper) << 32).wrapping_add(i64::from(lower))
    }

    /// Vanilla `nextFloat()`.
    pub fn next_f32(&mut self) -> f32 {
        self.next_bits(24) as f32 * 5.960_464_5e-8
    }

    /// Vanilla `nextDouble()`.
    pub fn next_f64(&mut self) -> f64 {
        let combined = ((self.next_bits(26) as i64) << 27) + self.next_bits(27) as i64;
        combined as f64 * (1.0 / (1_i64 << 53) as f64)
    }

    /// Vanilla `nextBoolean()`.
    pub fn next_bool(&mut self) -> bool {
        self.next_bits(1) != 0
    }

    /// Vanilla `nextGaussian()`.
    pub fn next_gaussian(&mut self) -> f64 {
        if let Some(value) = self.next_gaussian.take() {
            return value;
        }
        loop {
            let d = 2.0 * self.next_f64() - 1.0;
            let e = 2.0 * self.next_f64() - 1.0;
            let f = d * d + e * e;
            if f < 1.0 && f != 0.0 {
                let g = (-2.0 * f.ln() / f).sqrt();
                self.next_gaussian = Some(e * g);
                return d * g;
            }
        }
    }
}

/// Vanilla `PositionalRandomFactory`'s position hash (a.k.a. `getSeed`).
#[must_use]
pub const fn get_seed(x: i32, y: i32, z: i32) -> i64 {
    let l =
        (x.wrapping_mul(3_129_871) as i64) ^ ((z as i64).wrapping_mul(116_129_781)) ^ (y as i64);
    let l = l
        .wrapping_mul(l)
        .wrapping_mul(42_317_861)
        .wrapping_add(l.wrapping_mul(11));
    l >> 16
}

/// `mixStafford13` from vanilla's `RandomSupport`.
#[must_use]
pub const fn mix_stafford_13(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
#[expect(
    clippy::unreadable_literal,
    clippy::cast_sign_loss,
    reason = "reference vectors from vanilla Java; raw literals and casts are intentional"
)]
mod tests {
    use super::*;

    const MIX_STAFFORD_13_TEST_CASES: &[(u64, i64)] = &[
        (0, 0),
        (1, 6238072747940578789),
        (64, -8456553050427055661),
        (4096, -1125827887270283392),
        (262144, -120227641678947436),
        (16777216, 6406066033425044679),
        (1073741824, 3143522559155490559),
        (16, -2773008118984693571),
        (1024, 8101005175654470197),
        (65536, -3551754741763842827),
        (4194304, -2737109459693184599),
        (2, -2606959012126976886),
        (128, -5825874238589581082),
        (8192, 1111983794319025228),
        (524288, -7964047577924347155),
        (33554432, -5634612006859462257),
        (2147483648, -1436547171018572641),
        (137438953472, -4514638798598940860),
        (8796093022208, -610572083552328405),
        (562949953421312, -263574021372026223),
        (36028797018963968, 7868130499179604987),
        (253, -4045451768301188906),
        (127, -6873224393826578139),
        (8447, 6670985465942597767),
        (524543, -6228499289678716485),
        (33554687, 2630391896919662492),
        (2147483903, -6879633228472053040),
        (137438953727, -5817997684975131823),
        (8796093022463, 2384436581894988729),
        (562949953421567, -5076179956679497213),
        (36028797018964223, -5993365784811617721),
    ];

    #[test]
    fn mix_stafford_13_matches_java() {
        for &(input, expected) in MIX_STAFFORD_13_TEST_CASES {
            assert_eq!(
                mix_stafford_13(input),
                expected as u64,
                "mix_stafford_13({input}) failed"
            );
        }
    }

    #[test]
    fn xoroshiro_next_i32_matches_java() {
        const EXPECTED: [i32; 10] = [
            -160476802,
            781697906,
            653572596,
            1337520923,
            -505875771,
            -47281585,
            342195906,
            1417498593,
            -1478887443,
            1560080270,
        ];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &EXPECTED {
            assert_eq!(rng.next_i32(), expected);
        }
    }

    #[test]
    fn xoroshiro_next_i32_bounded_matches_java() {
        const SMALL_EXPECTED: [i32; 10] = [9, 1, 1, 3, 8, 9, 0, 3, 6, 3];
        const LARGE_EXPECTED: [i32; 10] = [
            9784805, 470346, 13560642, 7320226, 14949645, 13460529, 2824352, 10938308, 14146127,
            4549185,
        ];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &SMALL_EXPECTED {
            assert_eq!(rng.next_i32_bounded(10), expected);
        }
        for &expected in &LARGE_EXPECTED {
            assert_eq!(rng.next_i32_bounded(0xFF_FFFF), expected);
        }
    }

    #[test]
    fn xoroshiro_next_i64_matches_java() {
        const EXPECTED: [i64; 10] = [
            3038984756725240190,
            -3694039286755638414,
            4633751808701151732,
            2160572957309072155,
            1839370574944072389,
            -4488466507718817201,
            -4199796579929588030,
            -1069045159880208415,
            8864804693509535725,
            -7194800960680693874,
        ];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &EXPECTED {
            assert_eq!(rng.next_i64(), expected);
        }
    }

    #[test]
    fn xoroshiro_next_f64_matches_java() {
        const EXPECTED: [f64; 10] = [
            0.16474369376959186,
            0.7997457290026366,
            0.2511961888876212,
            0.11712489470639631,
            0.0997124786680137,
            0.7566797430601416,
            0.7723285712021574,
            0.9420469457586381,
            0.48056202536813664,
            0.6099690583914598,
        ];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &EXPECTED {
            assert_eq!(rng.next_f64(), expected);
        }
    }

    #[test]
    fn xoroshiro_next_f32_matches_java() {
        const EXPECTED: [f32; 10] = [
            0.16474366,
            0.7997457,
            0.25119615,
            0.117124856,
            0.09971243,
            0.7566797,
            0.77232856,
            0.94204694,
            0.48056197,
            0.609969,
        ];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &EXPECTED {
            assert_eq!(rng.next_f32(), expected);
        }
    }

    #[test]
    fn xoroshiro_next_bool_matches_java() {
        const EXPECTED: [bool; 10] = [
            false, false, false, true, true, true, false, true, true, false,
        ];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &EXPECTED {
            assert_eq!(rng.next_bool(), expected);
        }
    }

    #[test]
    fn xoroshiro_next_gaussian_matches_java() {
        const EXPECTED: [f64; 10] = [
            -0.48540690699780015,
            0.43399227545320296,
            -0.3283265251019599,
            -0.5052497078202575,
            -0.3772512828630807,
            0.2419080215945433,
            -0.42622066207565135,
            2.411315261138953,
            -1.1419147030553274,
            -0.05849758093810378,
        ];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &EXPECTED {
            assert_eq!(rng.next_gaussian(), expected);
        }
    }

    #[test]
    fn xoroshiro_fork_matches_java() {
        let mut rng = Xoroshiro::from_seed(0);
        let mut forked = rng.fork();
        assert_eq!(forked.next_i32(), 542195535);
        assert_eq!(rng.next_i32(), 653572596);
    }

    #[test]
    fn positional_splitter_matches_java() {
        let mut rng = Xoroshiro::from_seed(0);
        let mut forked = rng.fork();
        assert_eq!(forked.next_i32(), 542195535);

        let splitter = forked.next_positional();
        let mut by_seed = splitter.with_seed(42069);
        assert_eq!(by_seed.next_i32(), -340700677);

        let mut by_pos = splitter.at(1337, 80085, -69420);
        assert_eq!(by_pos.next_i32(), 790449132);

        assert_eq!(rng.next_i32(), 653572596);
        assert_eq!(forked.next_i32(), 435917842);
    }

    #[test]
    fn zero_seed_produces_fallback_values() {
        let mut rng = Xoroshiro::new(0, 0);
        assert_eq!(rng.next_i64(), 6807859099481836695);
    }

    #[test]
    fn worldgen_random_set_decoration_seed_matches_vanilla_trace() {
        let mut random = WorldgenRandom::from_seed(0);
        assert_eq!(
            random.set_decoration_seed(13_579, -6_695_392, 5_868_656),
            7_632_291_757_650_236_667,
        );
    }

    #[test]
    fn worldgen_random_feature_seed_matches_vanilla_origin() {
        let mut random = WorldgenRandom::from_seed(0);
        let decoration_seed = random.set_decoration_seed(13_579, -6_695_392, 5_868_656);
        random.set_feature_seed(decoration_seed, 0, 6);

        let x = -6_695_392 + random.next_i32_bounded(16);
        let z = 5_868_656 + random.next_i32_bounded(16);
        let y = random.next_i32_bounded(161);
        assert_eq!((x, y, z), (-6_695_386, 149, 5_868_662));
    }

    #[test]
    fn worldgen_random_preserves_pending_gaussian_across_reseed() {
        let mut random = WorldgenRandom::from_seed(123);
        let _ = random.next_gaussian();
        random.set_feature_seed(456, 7, 8);

        let mut cached_reference = WorldgenRandom::from_seed(123);
        let _ = cached_reference.next_gaussian();
        assert_eq!(random.next_gaussian(), cached_reference.next_gaussian());

        let mut reseeded_reference = WorldgenRandom::from_seed(0);
        reseeded_reference.set_feature_seed(456, 7, 8);
        assert_eq!(random.next_gaussian(), reseeded_reference.next_gaussian());
    }

    #[test]
    fn xoroshiro_next_i32_between_inclusive_matches_java() {
        const EXPECTED: [i32; 10] = [99, 59, 57, 65, 94, 100, 54, 66, 83, 68];
        let mut rng = Xoroshiro::from_seed(0);
        for &expected in &EXPECTED {
            assert_eq!(rng.next_i32_between(50, 100), expected);
        }
    }
}
