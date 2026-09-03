//! Vanilla-style noise aquifers.
//!
//! The real generator evaluates this after final density: positive density is
//! stone, while negative density is resolved by nearby 16x12x16 aquifer cells.
//! This implementation keeps the same cell geometry and positional seeding and
//! uses the four 26.2 aquifer noises from the datapack.

use crate::{
    block,
    simplex::{fork_seed_for, JavaRandom, NoiseRegistry},
};

const NO_FLUID: i32 = -1_000_000;
const XZ_SPACING: i32 = 16;
const Y_SPACING: i32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FluidStatus {
    level: i32,
    lava: bool,
}

/// Per-column aquifer resolver. It is deliberately local to a generated column:
/// no mutable global cache is needed and rayon order cannot affect generation.
pub struct Aquifer<'a> {
    seed: i64,
    top: i32,
    water_column: bool,
    noises: &'a NoiseRegistry,
    locations: [((i32, i32, i32), (i32, i32, i32)); 12],
}

impl<'a> Aquifer<'a> {
    pub fn new(seed: i64, top: i32, water_column: bool, noises: &'a NoiseRegistry) -> Self {
        Self {
            seed,
            top,
            water_column,
            noises,
            locations: [((0, 0, 0), (0, 0, 0)); 12],
        }
    }

    /// Resolve a density into the block substance (never returns `None`).
    pub fn substance(&mut self, x: i32, y: i32, z: i32, density: f64) -> u32 {
        if density > 0.0 {
            return if y < crate::DEEPSLATE_Y {
                block::DEEPSLATE
            } else {
                block::STONE
            };
        }
        // Aquifers do not affect the five-block surface crust or the bedrock floor.
        if y >= self.top - 5 || y <= crate::MIN_Y + 4 {
            return block::AIR;
        }
        let (best, second, third) = self.nearest(x, y, z);
        let a = self.status(best);
        let b = self.status(second);
        let c = self.status(third);
        let d1 = dist(best, x, y, z);
        let d2 = dist(second, x, y, z);
        let d3 = dist(third, x, y, z);
        let sim12 = 1.0 - (d2 - d1) as f64 / 25.0;
        let fluid = if sim12 <= 0.0 {
            a
        } else {
            // Barrier noise is pressure between unlike fluid levels. This is the
            // important vanilla distinction from simply testing density > 0.
            let pressure = self.pressure(x, y, z, a, b);
            if density + sim12 * pressure > 0.0 {
                return if y < crate::DEEPSLATE_Y {
                    block::DEEPSLATE
                } else {
                    block::STONE
                };
            }
            let sim13 = 1.0 - (d3 - d1) as f64 / 25.0;
            if sim13 > 0.0 && density + sim12 * sim13 * self.pressure(x, y, z, a, c) > 0.0 {
                return if y < crate::DEEPSLATE_Y {
                    block::DEEPSLATE
                } else {
                    block::STONE
                };
            }
            a
        };
        if fluid.level != NO_FLUID && y < fluid.level {
            if fluid.lava {
                block::LAVA
            } else {
                block::WATER
            }
        } else {
            block::AIR
        }
    }

    fn nearest(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
    ) -> ((i32, i32, i32), (i32, i32, i32), (i32, i32, i32)) {
        let ax = x.div_euclid(16);
        let ay = (y + 1).div_euclid(12);
        let az = z.div_euclid(16);
        let mut v: Vec<((i32, i32, i32), i32)> = Vec::with_capacity(12);
        for dx in 0..=1 {
            for dy in -1..=1 {
                for dz in 0..=1 {
                    let cell = (ax + dx, ay + dy, az + dz);
                    let p = self.location(cell);
                    v.push((cell, dist(p, x, y, z)));
                }
            }
        }
        v.sort_by_key(|x| x.1);
        (v[0].0, v[1].0, v[2].0)
    }

    fn location(&mut self, cell: (i32, i32, i32)) -> (i32, i32, i32) {
        // Tiny fixed cache covers the 12 cells queried for a column.
        for &((cx, cy, cz), p) in &self.locations {
            if (cx, cy, cz) == cell && (cx != 0 || cy != 0 || cz != 0) {
                return p;
            }
        }
        let named = crate::noise_perlin::java_string_hash("minecraft: aquifer");
        let mut r = JavaRandom::new(
            named as i64
                ^ fork_seed_for(self.seed)
                ^ (cell.0 as i64).wrapping_mul(0x9e3779b97f4a7c15u64 as i64)
                ^ (cell.1 as i64).wrapping_mul(0xc2b2ae3d27d4eb4fu64 as i64)
                ^ (cell.2 as i64).wrapping_mul(0x165667b19e3779f9),
        );
        let p = (
            cell.0 * 16 + r.next_int(10) as i32,
            cell.1 * 12 + r.next_int(9) as i32,
            cell.2 * 16 + r.next_int(10) as i32,
        );
        for slot in &mut self.locations {
            if slot.0 == (0, 0, 0) {
                *slot = (cell, p);
                break;
            }
        }
        p
    }

    fn status(&self, cell: (i32, i32, i32)) -> FluidStatus {
        let (x, y, z) = (cell.0 * 16, cell.1 * 12, cell.2 * 16);
        let global_level = if self.water_column {
            crate::SEA_LEVEL
        } else {
            NO_FLUID
        };
        let flooded = self.noises.sample(
            "aquifer_fluid_level_floodedness",
            self.seed,
            x as f64,
            y as f64 * 0.67,
            z as f64,
        );
        if flooded < -0.15 {
            return FluidStatus {
                level: NO_FLUID,
                lava: false,
            };
        }
        let spread = self.noises.sample(
            "aquifer_fluid_level_spread",
            self.seed,
            (cell.0 as f64) / 1.0,
            (cell.1 as f64) / 1.0,
            (cell.2 as f64) / 1.0,
        ) * 10.0;
        let spread = (spread / 3.0).round() as i32 * 3;
        let level = (cell.1 * 12 + 6 + spread)
            .min(self.top - 8)
            .min(global_level.max(NO_FLUID));
        let lava = self
            .noises
            .sample(
                "aquifer_lava",
                self.seed,
                (cell.0 as f64) / 4.0,
                (cell.1 as f64) / 4.0,
                (cell.2 as f64) / 4.0,
            )
            .abs()
            > 0.3
            && level <= -10;
        FluidStatus { level, lava }
    }

    fn pressure(&self, x: i32, y: i32, z: i32, a: FluidStatus, b: FluidStatus) -> f64 {
        if a.lava != b.lava && a.level != NO_FLUID && b.level != NO_FLUID {
            return 2.0;
        }
        let diff = (a.level - b.level).abs();
        if diff == 0 {
            return 0.0;
        }
        let avg = (a.level + b.level) as f64 * 0.5;
        let h = y as f64 + 0.5 - avg;
        let g = (diff as f64 / 2.0 - h.abs()) / if h > 0.0 { 1.5 } else { 3.0 };
        2.0 * (g + self.noises.sample(
            "aquifer_barrier",
            self.seed,
            x as f64,
            y as f64 * 0.5,
            z as f64,
        ))
    }
}

fn dist(p: (i32, i32, i32), x: i32, y: i32, z: i32) -> i32 {
    (p.0 - x).pow(2) + (p.1 - y).pow(2) + (p.2 - z).pow(2)
}
