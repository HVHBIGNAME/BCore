//! Vanilla `NoiseBasedAquifer` substance computation.
use crate::{block, density, noise_perlin::Xoroshiro, simplex::NoiseRegistry};

const NO_FLUID: i32 = i32::MIN / 4;
const X_SPACING: i32 = 16;
const Y_SPACING: i32 = 12;
const Z_SPACING: i32 = 16;
const X_RANGE: u32 = 10;
const Y_RANGE: u32 = 9;
const Z_RANGE: u32 = 10;

#[derive(Clone, Copy, Debug)]
struct FluidStatus {
    level: i32,
    lava: bool,
}
impl FluidStatus {
    #[inline]
    fn at(self, y: i32) -> u32 {
        if y < self.level {
            if self.lava {
                block::LAVA
            } else {
                block::WATER
            }
        } else {
            block::AIR
        }
    }
}

/// Stateful per-column resolver; the only state is the small center cache.
pub struct Aquifer<'a> {
    seed: i64,
    fallback_surface: i32,
    water_column: bool,
    noises: &'a NoiseRegistry,
    preliminary: Option<&'a density::DensityFunction>,
    ctx: density::EvalContext,
    centers: Vec<((i32, i32, i32), (i32, i32, i32))>,
}

impl<'a> Aquifer<'a> {
    pub fn new(
        seed: i64,
        fallback_surface: i32,
        water_column: bool,
        noises: &'a NoiseRegistry,
        preliminary: Option<&'a density::DensityFunction>,
        ctx: density::EvalContext,
    ) -> Self {
        Self {
            seed,
            fallback_surface,
            water_column,
            noises,
            preliminary,
            ctx,
            centers: Vec::with_capacity(12),
        }
    }

    /// Vanilla returns null for solid; callers map null to stone.
    pub fn substance(&mut self, x: i32, y: i32, z: i32, density_value: f64) -> u32 {
        if density_value > 0.0 {
            return block::STONE;
        }
        let global = self.global(y);
        if y > 40 {
            return global.at(y);
        }
        if global.lava {
            return block::LAVA;
        }
        let (p1, p2, p3, p4) = self.nearest_four(x, y, z);
        let s1 = self.status(p1);
        let s2 = self.status(p2);
        let s3 = self.status(p3);
        let _s4 = self.status(p4);
        let d1 = dist(p1, x, y, z);
        let d2 = dist(p2, x, y, z);
        let d3 = dist(p3, x, y, z);
        let fluid = s1.at(y);
        let sim12 = similarity(d1, d2);
        if sim12 <= 0.0 {
            return fluid;
        }
        if density_value + sim12 * self.pressure(x, y, z, s1, s2) > 0.0 {
            return block::STONE;
        }
        let sim13 = similarity(d1, d3);
        if sim13 > 0.0 && density_value + sim12 * sim13 * self.pressure(x, y, z, s1, s3) > 0.0 {
            return block::STONE;
        }
        // Vanilla performs the third barrier check against centers 2 and 3.
        let sim23 = similarity(d2, d3);
        if sim23 > 0.0 && density_value + sim12 * sim23 * self.pressure(x, y, z, s2, s3) > 0.0 {
            return block::STONE;
        }
        fluid
    }

    fn global(&self, y: i32) -> FluidStatus {
        if y < -54 {
            FluidStatus {
                level: -54,
                lava: true,
            }
        } else {
            FluidStatus {
                level: crate::SEA_LEVEL,
                lava: false,
            }
        }
    }

    fn nearest_four(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
    ) -> (
        (i32, i32, i32),
        (i32, i32, i32),
        (i32, i32, i32),
        (i32, i32, i32),
    ) {
        let ax = (x - 5).div_euclid(16);
        let ay = (y + 1).div_euclid(12);
        let az = (z - 5).div_euclid(16);
        let mut v = Vec::with_capacity(12);
        for gx in 0..=1 {
            for gy in -1..=1 {
                for gz in 0..=1 {
                    let c = (ax + gx, ay + gy, az + gz);
                    let p = self.center(c);
                    v.push((c, dist(p, x, y, z)));
                }
            }
        }
        v.sort_unstable_by_key(|e| e.1);
        (v[0].0, v[1].0, v[2].0, v[3].0)
    }
    fn center(&mut self, c: (i32, i32, i32)) -> (i32, i32, i32) {
        if let Some(&(_, p)) = self.centers.iter().find(|(k, _)| *k == c) {
            return p;
        }
        let mut root = Xoroshiro::new(self.seed);
        let factory = root.fork_positional();
        // positional factory at(gridX,gridY,gridZ), then bounded draws.
        let mut rr = factory.at(c.0, c.1, c.2);
        let p = (
            c.0 * 16 + rr.next_int(X_RANGE) as i32,
            c.1 * 12 + rr.next_int(Y_RANGE) as i32,
            c.2 * 16 + rr.next_int(Z_RANGE) as i32,
        );
        self.centers.push((c, p));
        p
    }

    fn status(&self, c: (i32, i32, i32)) -> FluidStatus {
        let (x, y, z) = (c.0 * 16, c.1 * 12, c.2 * 16);
        let global = self.global(y);
        if global.lava {
            return global;
        }
        let offsets = [
            (0, 0),
            (-2, -1),
            (-1, -1),
            (0, -1),
            (1, -1),
            (-3, 0),
            (-2, 0),
            (-1, 0),
            (1, 0),
            (-2, 1),
            (-1, 1),
            (0, 1),
            (1, 1),
        ];
        let mut lowest = i32::MAX;
        for (ox, oz) in offsets {
            lowest = lowest.min(self.preliminary_level(x + ox * 16, z + oz * 16));
        }
        let center_surface = self.preliminary_level(x, z) + 8;
        // Vanilla: floodednessFactor = surfaceUnderWater ?
        //   clampedMap(lowest+8-y, 0, 64, 1.0, 0.0) : 0.0   = 1 - d/64 clamped.
        let factor = if center_surface < global.level {
            (1.0 - (lowest + 8 - y) as f64 / 64.0).clamp(0., 1.)
        } else {
            0.
        };
        let n = self
            .noises
            .sample(
                "aquifer_fluid_level_floodedness",
                self.seed,
                x as f64,
                y as f64 * 0.67,
                z as f64,
            )
            .clamp(-1., 1.);
        let fully = n - lerp(factor, 0.8, -0.3);
        let partial = n - lerp(factor, 0.4, -0.8);
        let level = if fully > 0. {
            global.level
        } else if partial > 0. {
            self.random_level(x, y, z, lowest)
        } else {
            NO_FLUID
        };
        let lava = level != NO_FLUID
            && level <= -10
            && !global.lava
            && self
                .noises
                .sample(
                    "aquifer_lava",
                    self.seed,
                    x as f64 / 64.,
                    y as f64 / 40.,
                    z as f64 / 64.,
                )
                .abs()
                > 0.3;
        FluidStatus { level, lava }
    }
    fn preliminary_level(&self, x: i32, z: i32) -> i32 {
        self.preliminary
            .map(|f| density::evaluate(f, x as f64, 0., z as f64, &self.ctx).floor() as i32)
            .unwrap_or(self.fallback_surface)
    }
    fn random_level(&self, x: i32, y: i32, z: i32, lowest: i32) -> i32 {
        let cx = x.div_euclid(16);
        let cy = y.div_euclid(40);
        let cz = z.div_euclid(16);
        let n = self.noises.sample(
            "aquifer_fluid_level_spread",
            self.seed,
            cx as f64,
            cy as f64,
            cz as f64,
        ) * 10.;
        (lowest.min(cy * 40 + 20 + (n / 3.).round() as i32 * 3))
    }
    fn pressure(&self, x: i32, y: i32, z: i32, a: FluidStatus, b: FluidStatus) -> f64 {
        let ta = a.at(y);
        let tb = b.at(y);
        if (ta == block::LAVA && tb == block::WATER) || (ta == block::WATER && tb == block::LAVA) {
            return 2.;
        }
        let diff = (a.level - b.level).abs() as f64;
        if diff == 0. {
            return 0.;
        }
        let above = y as f64 + 0.5 - (a.level + b.level) as f64 * 0.5;
        let edge = diff / 2. - above.abs();
        let gradient = if above > 0. {
            if edge > 0. {
                edge / 1.5
            } else {
                edge / 2.5
            }
        } else {
            let q = 3. + edge;
            if q > 0. {
                q / 3.
            } else {
                q / 10.
            }
        };
        let noise = if !(-2. ..=2.).contains(&gradient) {
            0.
        } else {
            self.noises.sample(
                "aquifer_barrier",
                self.seed,
                x as f64,
                y as f64 * 0.5,
                z as f64,
            )
        };
        2. * (noise + gradient)
    }
}
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + (b - a) * t
}
fn similarity(a: i32, b: i32) -> f64 {
    1. - (b - a) as f64 / 25.
}
fn dist(p: (i32, i32, i32), x: i32, y: i32, z: i32) -> i32 {
    (p.0 - x).pow(2) + (p.1 - y).pow(2) + (p.2 - z).pow(2)
}
