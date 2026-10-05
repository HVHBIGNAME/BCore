//! Minecraft 26.1 `OreVeinifier`: a material rule after the aquifer, before the
//! generator's default stone. This is independent of the placed OreFeature.

use crate::block;
use crate::density::{self, DensityFunction, EvalContext, EvaluationMode};
use crate::feature_world::Pos;
use crate::noise_perlin::{clamped_lerp, Xoroshiro, XoroshiroPositional};

/// The three seeded router functions, sampled in the live material phase.
#[derive(Debug, Clone)]
pub struct VeinFunctions {
    pub toggle: DensityFunction,
    pub ridged: DensityFunction,
    pub gap: DensityFunction,
}

impl VeinFunctions {
    pub fn calculate(&self, ore: &OreVeinifier, (x, y, z): Pos, ctx: &EvalContext) -> Option<u32> {
        let ctx = EvalContext {
            mode: EvaluationMode::NoiseChunkMaterial,
            ..*ctx
        };
        let sample = |f: &DensityFunction| density::evaluate(f, x as f64, y as f64, z as f64, &ctx);
        ore.calculate(
            (x, y, z),
            sample(&self.toggle),
            || sample(&self.ridged),
            || sample(&self.gap),
        )
    }
}

#[derive(Clone, Copy)]
pub struct OreVeinifier {
    random: XoroshiroPositional,
}

impl OreVeinifier {
    /// RandomState.oreRandom, not the decoration WorldgenRandom or noise seed.
    pub fn new(seed: i64) -> Self {
        Self {
            random: Xoroshiro::new(seed)
                .fork_positional()
                .from_hash_of("minecraft:ore")
                .fork_positional(),
        }
    }

    /// Return null/None for the next material rule. Ridge and gap callbacks are
    /// deliberately lazy: native short-circuiting determines both their reads
    /// and the number of draws from the position's fresh Xoroshiro stream.
    pub fn calculate(
        &self,
        (x, y, z): Pos,
        toggle: f64,
        ridged: impl FnOnce() -> f64,
        gap: impl FnOnce() -> f64,
    ) -> Option<u32> {
        calculate_with_random(y, toggle, ridged, gap, || {
            let mut random = self.random.at(x, y, z);
            move || random.next_float()
        })
    }
}

fn calculate_with_random<R: FnMut() -> f32>(
    y: i32,
    toggle: f64,
    ridged: impl FnOnce() -> f64,
    gap: impl FnOnce() -> f64,
    random_at: impl FnOnce() -> R,
) -> Option<u32> {
    let (min_y, max_y, ore, raw, filler) = if toggle > 0.0 {
        (
            0i32,
            50i32,
            block::COPPER_ORE,
            block::RAW_COPPER_BLOCK,
            block::GRANITE,
        )
    } else {
        (
            -60i32,
            -8i32,
            block::DEEPSLATE_IRON_ORE,
            block::RAW_IRON_BLOCK,
            block::TUFF,
        )
    };
    let above_bottom = y.wrapping_sub(min_y);
    let below_top = max_y.wrapping_sub(y);
    if above_bottom < 0 || below_top < 0 {
        return None;
    }
    let edge = clamped_lerp(below_top.min(above_bottom) as f64 / 20.0, -0.2, 0.0);
    let veininess = toggle.abs();
    // These constants are Java floats widened to doubles, not decimal doubles.
    if veininess + edge < f64::from(0.4f32) {
        return None;
    }
    let mut random = random_at();
    if random() > 0.7f32 || ridged() >= 0.0 {
        return None;
    }
    let richness = clamped_lerp(
        (veininess - f64::from(0.4f32)) / (f64::from(0.6f32) - f64::from(0.4f32)),
        f64::from(0.1f32),
        f64::from(0.3f32),
    );
    Some(
        if f64::from(random()) < richness && gap() > f64::from(-0.3f32) {
            if random() < 0.02f32 {
                raw
            } else {
                ore
            }
        } else {
            filler
        },
    )
}

#[cfg(test)]
#[path = "ore_vein_tests.rs"]
mod tests;
