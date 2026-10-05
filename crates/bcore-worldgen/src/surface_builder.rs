//! Minecraft 26.1 overworld `SurfaceSystem.buildSurface`, after noise filling.
//!
//! Inputs are the rule tree, seeded noise registry, preliminary-height sampler and
//! quart-biome lookup. The builder does not own a density graph or neighboring chunks.
use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::biome::BiomeId;
use crate::biome_zoom::BiomeZoom;
use crate::noise_perlin::Xoroshiro;
use crate::simplex::NoiseRegistry;
use crate::surface::BlockState;
use crate::surface_rules::{self, SurfaceContext, SurfaceRule};
use crate::{block, is_air, GeneratedChunk, MAX_Y, MIN_Y, SEA_LEVEL};

const PACKED_ICE: BlockState = 12914;

/// Apply the complete overworld surface pass in native X-then-Z visitation order.
///
/// `preliminary_surface` receives absolute block coordinates at the four 16-block
/// cell corners, and must return `NoiseChunk.preliminarySurfaceLevel` there.
/// `noise_biome` receives the current chunk and absolute **quart** coordinates;
/// the builder performs biome zoom and clamps quart Y to the chunk's height first.
/// The callback can read the chunk palette or a neighbor's BIOMES-stage palette.
///
/// Returns fluid postprocessing marks as `(local_x, absolute_y, local_z)`, in
/// insertion order. The stage owner must append these to the chunk's existing marks.
pub fn build_surface(
    chunk: &mut GeneratedChunk,
    seed: i64,
    rule: &SurfaceRule,
    noise: &NoiseRegistry,
    preliminary_surface: impl FnMut(i32, i32) -> i32,
    noise_biome: impl FnMut(&GeneratedChunk, i32, i32, i32) -> BiomeId,
) -> Vec<(usize, i32, usize)> {
    build_surface_observed(
        chunk,
        seed,
        rule,
        noise,
        preliminary_surface,
        noise_biome,
        |_, _| {},
    )
}

pub(crate) fn build_surface_observed(
    chunk: &mut GeneratedChunk,
    seed: i64,
    rule: &SurfaceRule,
    noise: &NoiseRegistry,
    mut preliminary_surface: impl FnMut(i32, i32) -> i32,
    mut noise_biome: impl FnMut(&GeneratedChunk, i32, i32, i32) -> BiomeId,
    mut observe: impl FnMut(&SurfaceContext<'_>, Option<BlockState>),
) -> Vec<(usize, i32, usize)> {
    let base_x = chunk.pos.x * 16;
    let base_z = chunk.pos.z * 16;
    let corners = std::array::from_fn(|i| {
        preliminary_surface(base_x + (i as i32 & 1) * 16, base_z + (i as i32 >> 1) * 16)
    });
    let zoom = BiomeZoom::new(seed);
    let mut biomes = HashMap::new();
    let mut biome_at = |chunk: &GeneratedChunk, x, y, z| {
        let (qx, qy, qz) = zoom.quart_at((x, y, z));
        let qy = qy.clamp(MIN_Y >> 2, MAX_Y >> 2);
        *biomes
            .entry((qx, qy, qz))
            .or_insert_with(|| noise_biome(chunk, qx, qy, qz))
    };
    let mut postprocessing = Vec::new();
    for x in 0..16 {
        for z in 0..16 {
            let mut column = Column {
                chunk,
                seed,
                noise,
                x,
                z,
                wx: base_x + x as i32,
                wz: base_z + z as i32,
                postprocessing: &mut postprocessing,
            };
            let initial_height = column.height();
            let extension_biome = biome_at(column.chunk, column.wx, initial_height, column.wz);
            if crate::biome::name(extension_biome) == "eroded_badlands" {
                column.eroded_badlands(initial_height);
            }
            let depth = surface_rules::surface_depth_with_noise(noise, seed, column.wx, column.wz);
            let preliminary = interpolate_preliminary(corners, column.wx, column.wz);
            column.apply_rules(rule, depth, preliminary, &mut biome_at, &mut observe);
            if matches!(
                crate::biome::name(extension_biome),
                "frozen_ocean" | "deep_frozen_ocean"
            ) {
                column.frozen_ocean(extension_biome, initial_height, preliminary + depth - 8);
            }
        }
    }
    postprocessing
}

/// `SurfaceRules.Context` bilinear interpolation, before surface-depth adjustment.
pub fn interpolate_preliminary(corners: [i32; 4], x: i32, z: i32) -> i32 {
    let [nw, ne, sw, se] = corners.map(f64::from);
    let tx = f64::from((x & 15) as f32 / 16.0);
    let tz = f64::from((z & 15) as f32 / 16.0);
    let north = nw + tx * (ne - nw);
    let south = sw + tx * (se - sw);
    (north + tz * (south - north)).floor() as i32
}

/// Includes waterlogged and other fluid-bearing states, not just water and lava.
pub fn has_fluid(state: BlockState) -> bool {
    static FLUIDS: OnceLock<Vec<bool>> = OnceLock::new();
    let fluids = FLUIDS.get_or_init(|| {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../data/surface_builder_blocks_26_1.json"))
                .expect("native surface block predicates");
        let mut flags = vec![false; data["state_count"].as_u64().unwrap() as usize];
        for range in data["fluid_ranges"].as_array().unwrap() {
            let start = range[0].as_u64().unwrap() as usize;
            let end = range[1].as_u64().unwrap() as usize;
            flags[start..end].fill(true);
        }
        flags
    });
    fluids[state as usize]
}

struct Column<'a> {
    chunk: &'a mut GeneratedChunk,
    seed: i64,
    noise: &'a NoiseRegistry,
    x: usize,
    z: usize,
    wx: i32,
    wz: i32,
    postprocessing: &'a mut Vec<(usize, i32, usize)>,
}

impl Column<'_> {
    fn get(&self, y: i32) -> BlockState {
        self.chunk.get(self.x, y, self.z).unwrap_or(block::AIR)
    }

    fn set(&mut self, y: i32, state: BlockState) {
        if self.chunk.set(self.x, y, self.z, state) && has_fluid(state) {
            self.postprocessing.push((self.x, y, self.z));
        }
    }

    fn height(&self) -> i32 {
        self.chunk
            .surface_y(self.x, self.z)
            .map_or(MIN_Y, |y| y + 1)
    }

    fn noise(&self, name: &str, scale: f64) -> f64 {
        self.noise.sample(
            name,
            self.seed,
            self.wx as f64 * scale,
            0.0,
            self.wz as f64 * scale,
        )
    }

    fn apply_rules(
        &mut self,
        rule: &SurfaceRule,
        surface_depth: i32,
        preliminary_surface_level: i32,
        biome_at: &mut impl FnMut(&GeneratedChunk, i32, i32, i32) -> BiomeId,
        observe: &mut impl FnMut(&SurfaceContext<'_>, Option<BlockState>),
    ) {
        let mut stone_depth_above = 0;
        let mut water_height = i32::MIN;
        let mut stone_floor = i32::MAX;
        let steep = OnceCell::new();
        for y in (MIN_Y..=self.height()).rev() {
            let state = self.get(y);
            if is_air(state) {
                stone_depth_above = 0;
                water_height = i32::MIN;
                continue;
            }
            if has_fluid(state) {
                if water_height == i32::MIN {
                    water_height = y + 1;
                }
                continue;
            }
            if stone_floor >= y {
                // Every dry non-air state contributes to stone depth. Look below
                // the build floor as well: the out-of-height air terminates the run.
                stone_floor = (MIN_Y - 1..y)
                    .rev()
                    .find(|&below| {
                        let state = self.get(below);
                        is_air(state) || has_fluid(state)
                    })
                    .expect("air below the build floor")
                    + 1;
            }
            stone_depth_above += 1;
            if state != block::STONE {
                continue;
            }
            let context = SurfaceContext {
                biome: biome_at(self.chunk, self.wx, y, self.wz),
                stone_depth_above,
                stone_depth_below: y - stone_floor + 1,
                water_height,
                surface_depth,
                preliminary_surface_level,
                sea_level: SEA_LEVEL,
                x: self.wx,
                y,
                z: self.wz,
                seed: self.seed,
                noise: Some(self.noise),
            };
            let result = rule.evaluate_with_steep(&context, &mut || {
                *steep.get_or_init(|| surface_rules::steep(self.chunk, self.wx, self.wz))
            });
            observe(&context, result);
            if let Some(state) = result {
                self.set(y, state);
            }
        }
    }

    fn eroded_badlands(&mut self, initial_height: i32) {
        let strength = (self.noise("minecraft:badlands_surface", 1.0) * 8.25)
            .abs()
            .min(self.noise("minecraft:badlands_pillar", 0.2) * 15.0);
        if strength <= 0.0 {
            return;
        }
        let roof = (self.noise("minecraft:badlands_pillar_roof", 0.75) * 1.5).abs();
        let top =
            (64.0 + (strength * strength * 2.5).min((roof * 50.0).ceil() + 24.0)).floor() as i32;
        if initial_height > top {
            return;
        }
        for y in (MIN_Y..=top).rev() {
            let state = self.get(y);
            if state == block::STONE {
                break;
            }
            // The native veto is WATER specifically, not any nonempty fluid.
            if is_water(state) {
                return;
            }
        }
        for y in (MIN_Y..=top).rev() {
            if !is_air(self.get(y)) {
                break;
            }
            self.set(y, block::STONE);
        }
    }

    fn frozen_ocean(&mut self, biome: BiomeId, initial_height: i32, min_surface_level: i32) {
        let strength = (self.noise("minecraft:iceberg_surface", 1.0) * 8.25)
            .abs()
            .min(self.noise("minecraft:iceberg_pillar", 1.28) * 15.0);
        if strength <= 1.8 {
            return;
        }
        let roof = (self.noise("minecraft:iceberg_pillar_roof", 1.17) * 1.5).abs();
        let mut top = (strength * strength * 1.2).min((roof * 40.0).ceil() + 14.0);
        let temperature = SurfaceContext {
            biome,
            stone_depth_above: 0,
            stone_depth_below: 0,
            water_height: i32::MIN,
            surface_depth: 0,
            preliminary_surface_level: 0,
            sea_level: SEA_LEVEL,
            x: self.wx,
            y: SEA_LEVEL,
            z: self.wz,
            seed: self.seed,
            noise: Some(self.noise),
        }
        .temperature();
        if temperature > 0.1_f32 {
            top -= 2.0;
        }
        let bottom = if top > 2.0 {
            let bottom = SEA_LEVEL as f64 - top - 7.0;
            top += SEA_LEVEL as f64;
            bottom
        } else {
            top = 0.0;
            0.0
        };
        // Start a fresh root positional stream; the depth jitter did not consume it.
        let mut random = Xoroshiro::new(self.seed)
            .fork_positional()
            .at(self.wx, 0, self.wz);
        let snow_limit = 2 + random.next_int(4);
        let snow_line = SEA_LEVEL + 18 + random.next_int(10) as i32;
        let mut snow_count = 0;
        for y in (min_surface_level..=initial_height.max(top as i32 + 1)).rev() {
            let state = self.get(y);
            if (is_air(state) && y < top as i32 && random.next_double() > 0.01)
                || (is_water(state)
                    && y > bottom as i32
                    && y < SEA_LEVEL
                    && bottom != 0.0
                    && random.next_double() > 0.15)
            {
                if snow_count <= snow_limit && y > snow_line {
                    self.set(y, block::SNOW_BLOCK);
                    snow_count += 1;
                } else {
                    self.set(y, PACKED_ICE);
                }
            }
        }
    }
}

fn is_water(state: BlockState) -> bool {
    (block::WATER..block::WATER + 16).contains(&state)
}
