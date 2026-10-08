//! Vanilla `NoiseBasedAquifer` substance computation.
use crate::fast_hash::FastMap;
use crate::{
    block, density,
    noise_perlin::{Xoroshiro, XoroshiroPositional},
    simplex::NoiseRegistry,
    VanillaGraph,
};

const NO_FLUID: i32 = -32512;
const X_SPACING: i32 = 16;
const Y_SPACING: i32 = 12;
const Z_SPACING: i32 = 16;
const X_RANGE: u32 = 10;
const Y_RANGE: u32 = 9;
const Z_RANGE: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Aquifer samples and statuses are cached independently of terrain writes.
pub struct Aquifer<'a> {
    seed: i64,
    graph: &'a VanillaGraph,
    random: XoroshiroPositional,
    noises: &'a NoiseRegistry,
    ctx: density::EvalContext,
    centers: FastMap<(i32, i32, i32), (i32, i32, i32)>,
    statuses: FastMap<(i32, i32, i32), FluidStatus>,
    preliminary_levels: FastMap<(i32, i32), i32>,
    schedule_fluid_update: bool,
}

impl<'a> Aquifer<'a> {
    pub(crate) fn new(seed: i64, graph: &'a VanillaGraph, ctx: density::EvalContext) -> Self {
        let random = Xoroshiro::new(seed)
            .fork_positional()
            .from_hash_of("minecraft:aquifer")
            .fork_positional();
        Self {
            seed,
            graph,
            random,
            noises: density::noise_registry(),
            ctx,
            centers: FastMap::default(),
            statuses: FastMap::default(),
            preliminary_levels: FastMap::default(),
            schedule_fluid_update: false,
        }
    }

    /// Vanilla returns null for solid; callers map null to stone.
    pub fn substance(&mut self, x: i32, y: i32, z: i32, density_value: f64) -> u32 {
        self.schedule_fluid_update = false;
        if density_value > 0.0 {
            return block::STONE;
        }
        let global = self.global(y);
        // Vanilla does not use a fixed Y cutoff here.  The global fluid picker
        // is only the fallback for cells whose density is already non-solid;
        // aquifer barriers still decide whether underground air is water or air
        // at every Y (including above y=40).  The former shortcut flooded every
        // below-sea-level cavity and made land columns differ from vanilla.
        if global.lava {
            return block::LAVA;
        }
        let [(p1, d1), (p2, d2), (p3, d3), (p4, d4)] = self.nearest_four(x, y, z);
        let s1 = self.status(p1);
        let fluid = s1.at(y);
        let sim12 = similarity(d1, d2);
        if sim12 <= 0.0 {
            self.schedule_fluid_update = sim12 >= similarity(100, 144) && s1 != self.status(p2);
            return fluid;
        }
        if fluid == block::WATER && self.global(y - 1).at(y - 1) == block::LAVA {
            self.schedule_fluid_update = true;
            return fluid;
        }
        let s2 = self.status(p2);
        if density_value + sim12 * self.pressure(x, y, z, s1, s2) > 0.0 {
            return block::STONE;
        }
        let sim13 = similarity(d1, d3);
        let s3 = self.status(p3);
        if sim13 > 0.0 && density_value + sim12 * sim13 * self.pressure(x, y, z, s1, s3) > 0.0 {
            return block::STONE;
        }
        // Vanilla performs the third barrier check against centers 2 and 3.
        let sim23 = similarity(d2, d3);
        if sim23 > 0.0 && density_value + sim12 * sim23 * self.pressure(x, y, z, s2, s3) > 0.0 {
            return block::STONE;
        }
        let threshold = similarity(100, 144);
        self.schedule_fluid_update = s1 != s2
            || (sim23 >= threshold && s2 != s3)
            || (sim13 >= threshold && s1 != s3)
            || (sim13 >= threshold && similarity(d1, d4) >= threshold && s1 != self.status(p4));
        fluid
    }

    pub fn should_schedule_fluid_update(&self) -> bool {
        self.schedule_fluid_update
    }

    pub(crate) fn seed(&self) -> i64 {
        self.seed
    }

    pub(crate) fn context(&self) -> density::EvalContext {
        self.ctx
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

    fn nearest_four(&mut self, x: i32, y: i32, z: i32) -> [((i32, i32, i32), i32); 4] {
        let ax = (x - 5).div_euclid(X_SPACING);
        let ay = (y + 1).div_euclid(Y_SPACING);
        let az = (z - 5).div_euclid(Z_SPACING);
        let mut nearest = [((0, 0, 0), i32::MAX); 4];
        for gx in 0..=1 {
            for gy in -1..=1 {
                for gz in 0..=1 {
                    let c = (ax + gx, ay + gy, az + gz);
                    let p = self.center(c);
                    let distance = dist(p, x, y, z);
                    // Ties favour the last visited center in the native loops.
                    if let Some(i) = nearest.iter().position(|&(_, d)| distance <= d) {
                        nearest.copy_within(i..3, i + 1);
                        nearest[i] = (p, distance);
                    }
                }
            }
        }
        nearest
    }
    fn center(&mut self, c: (i32, i32, i32)) -> (i32, i32, i32) {
        if let Some(&p) = self.centers.get(&c) {
            return p;
        }
        let mut rr = self.random.at(c.0, c.1, c.2);
        let p = (
            c.0 * X_SPACING + rr.next_int(X_RANGE) as i32,
            c.1 * Y_SPACING + rr.next_int(Y_RANGE) as i32,
            c.2 * Z_SPACING + rr.next_int(Z_RANGE) as i32,
        );
        self.centers.insert(c, p);
        p
    }

    fn status(&mut self, pos: (i32, i32, i32)) -> FluidStatus {
        if let Some(status) = self.statuses.get(&pos) {
            return *status;
        }
        let status = self.compute_status(pos);
        self.statuses.insert(pos, status);
        status
    }

    fn compute_status(&mut self, (x, y, z): (i32, i32, i32)) -> FluidStatus {
        let global = self.global(y);
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
        let mut surface_under_water = false;
        for (ox, oz) in offsets {
            let preliminary = self.preliminary_level(x + ox * 16, z + oz * 16);
            let adjusted = preliminary + 8;
            let center = ox == 0 && oz == 0;
            if center && y - 12 > adjusted {
                return global;
            }
            let near_surface = y + 12 > adjusted;
            if near_surface || center {
                let surface_fluid = self.global(adjusted);
                if surface_fluid.at(adjusted) != block::AIR {
                    if center {
                        surface_under_water = true;
                    }
                    if near_surface {
                        return surface_fluid;
                    }
                }
            }
            lowest = lowest.min(preliminary);
        }
        let evaluate = |f: &Option<density::DensityFunction>| {
            density::evaluate(
                f.as_ref().expect("aquifer climate router"),
                x as f64,
                y as f64,
                z as f64,
                &self.ctx,
            )
        };
        if evaluate(&self.graph.erosion) < f64::from(-0.225f32)
            && evaluate(&self.graph.depth) > f64::from(0.9f32)
        {
            return FluidStatus {
                level: NO_FLUID,
                lava: global.lava,
            };
        }
        // Vanilla: floodednessFactor = surfaceUnderWater ?
        //   clampedMap(lowest+8-y, 0, 64, 1.0, 0.0) : 0.0   = 1 - d/64 clamped.
        let factor = if surface_under_water {
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
        let fully = n - lerp(1.0 - factor, -0.3, 0.8);
        let partial = n - lerp(1.0 - factor, -0.8, 0.4);
        let level = if fully > 0. {
            global.level
        } else if partial > 0. {
            self.random_level(x, y, z, lowest)
        } else {
            NO_FLUID
        };
        let lava = global.lava
            || (level != NO_FLUID
                && level <= -10
                && !global.lava
                && self
                    .noises
                    .sample(
                        "aquifer_lava",
                        self.seed,
                        x.div_euclid(64) as f64,
                        y.div_euclid(40) as f64,
                        z.div_euclid(64) as f64,
                    )
                    .abs()
                    > 0.3);
        FluidStatus { level, lava }
    }
    fn preliminary_level(&mut self, x: i32, z: i32) -> i32 {
        // NoiseChunk.preliminarySurfaceLevel samples the flat cache at quart
        // coordinates, not at the arbitrary block coordinates of the aquifer
        // cell.  Without this quantization the 13-cell scan uses different
        // surfaces from vanilla, shifting floodedness and ocean boundaries.
        let qx = (x >> 2) << 2;
        let qz = (z >> 2) << 2;
        // NoiseChunk memoizes the returned height, not just density markers
        // inside its scan. This aquifer owns a fixed graph and full context.
        if let Some(&level) = self.preliminary_levels.get(&(qx, qz)) {
            return level;
        }
        let level = density::evaluate(
            self.graph
                .preliminary_surface_level
                .as_ref()
                .expect("aquifer preliminary surface"),
            qx as f64,
            0.,
            qz as f64,
            &self.ctx,
        )
        .floor() as i32;
        self.preliminary_levels.insert((qx, qz), level);
        level
    }
    fn random_level(&self, x: i32, y: i32, z: i32, lowest: i32) -> i32 {
        let cx = x.div_euclid(16);
        let cy = y.div_euclid(40);
        let cz = z.div_euclid(16);
        let n = self.noises.sample(
            "aquifer_fluid_level_spread",
            self.seed,
            cx as f64,
            cy as f64 * (1.0 / 1.4),
            cz as f64,
        ) * 10.;
        lowest.min(cy * 40 + 20 + (n / 3.).floor() as i32 * 3)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_and_substances_match_native_26_1() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../data/aquifers_26_1.json")).unwrap();
        let graph = VanillaGraph::load().unwrap();
        assert_eq!(NO_FLUID as i64, fixture["no_fluid"].as_i64().unwrap());
        for sample in fixture["centers"].as_array().unwrap() {
            let seed = sample["seed"].as_i64().unwrap();
            let mut aquifer = Aquifer::new(
                seed,
                graph,
                density::EvalContext {
                    seed,
                    ..Default::default()
                },
            );
            let point = |key: &str| {
                (
                    sample[key][0].as_i64().unwrap() as i32,
                    sample[key][1].as_i64().unwrap() as i32,
                    sample[key][2].as_i64().unwrap() as i32,
                )
            };
            assert_eq!(aquifer.center(point("grid")), point("center"), "{sample}");
        }
        let mut differences = Vec::new();
        for sample in fixture["samples"].as_array().unwrap() {
            density::clear_density_caches();
            let seed = sample["seed"].as_i64().unwrap();
            let x = sample["x"].as_i64().unwrap() as i32;
            let z = sample["z"].as_i64().unwrap() as i32;
            let input = sample["density"].as_f64().unwrap();
            let mut aquifer = Aquifer::new(
                seed,
                graph,
                density::EvalContext {
                    seed,
                    ..Default::default()
                },
            );
            for (offset, expected) in sample["states"].as_array().unwrap().iter().enumerate() {
                let y = crate::MIN_Y + offset as i32;
                let actual = aquifer.substance(x, y, z, input);
                let expected = expected.as_u64().unwrap() as u32;
                let update = sample["updates"][offset].as_bool().unwrap();
                if actual != expected || aquifer.should_schedule_fluid_update() != update {
                    differences.push((
                        seed,
                        x,
                        y,
                        z,
                        input,
                        actual,
                        expected,
                        aquifer.should_schedule_fluid_update(),
                        update,
                    ));
                }
            }
            density::clear_density_caches();
        }
        assert!(
            differences.is_empty(),
            "{} mismatches; first {:?}",
            differences.len(),
            &differences[..differences.len().min(12)]
        );
    }
}
