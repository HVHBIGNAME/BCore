//! Vanilla 26.2 configured carvers — exact port of `WorldCarver`,
//! `CaveWorldCarver` and `CanyonWorldCarver` (overworld: `minecraft:cave`,
//! `minecraft:cave_extra_underground`, `minecraft:canyon`).
//!
//! Chunk-status order: density fill → buildSurface → CARVERS → features.
//! Every target chunk is carved from each source chunk in a 17×17
//! neighborhood (`NoiseBasedChunkGenerator.applyCarvers`): each source
//! chunk's biome contributes its configured carver list; every overworld
//! biome lists exactly `[cave, cave_extra_underground, canyon]`, so the
//! list is hardcoded here (index = position in the list seeds the random).
//!
//! Random semantics: the per-carver random is a legacy 48-bit LCG
//! (`LegacyRandomSource`, i.e. `JavaRandom`), re-seeded per carver with
//! `WorldgenRandom.setLargeFeatureSeed(seed + index, sx, sz)` — the 26.2
//! formula `setSeed(seed); xs=nextLong(); zs=nextLong(); setSeed(cx*xs ^ cz*zs ^ seed)`.
//! Tunnel branches fork a fresh LCG seeded with `random.nextLong()`
//! (`RandomSource.createThreadLocalInstance(seed)` = `SingleThreadedRandomSource`,
//! which is the same 48-bit LCG).

use crate::{
    aquifer::Aquifer, block, density, simplex::JavaRandom, ChunkPos, GeneratedChunk, VanillaGraph,
    MAX_Y, MIN_Y, WORLD_HEIGHT,
};

/// `WorldCarver.getRange()` — the carver reach in chunks.
const RANGE: i32 = 4;
/// `SectionPos.sectionToBlockCoord(RANGE * 2 - 1)` = 112 blocks.
const MAX_DISTANCE: i32 = (RANGE * 2 - 1) * 16;
/// `VerticalAnchor.aboveBottom(8)` resolved against the overworld min Y (−64).
const LAVA_LEVEL: i32 = MIN_Y + 8;
/// Carver y-range min: `UniformHeight(aboveBottom(8), …)`.
const CAVE_Y_MIN: i32 = MIN_Y + 8;
/// `minecraft:cave` y max.
const CAVE_Y_MAX: i32 = 180;
/// `minecraft:cave_extra_underground` y max.
const EXTRA_Y_MAX: i32 = 47;
/// Overworld cave-carver list: (probability, y max).
const CAVE_CARVERS: [(f32, i32); 2] = [(0.15, CAVE_Y_MAX), (0.07, EXTRA_Y_MAX)];
const CANYON_PROB: f32 = 0.01;

// ── Vanilla `Mth.sin/cos` — 65536-entry lookup table ──────────────────────
#[inline]
fn mth_sin(x: f64) -> f32 {
    const SIN_SCALE: f64 = 10430.378350470453;
    let idx = ((x * SIN_SCALE) as i64 & 0xFFFF) as usize;
    (idx as f64 / SIN_SCALE).sin() as f32
}

#[inline]
fn mth_cos(x: f64) -> f32 {
    const SIN_SCALE: f64 = 10430.378350470453;
    let idx = ((x * SIN_SCALE + 16384.0) as i64 & 0xFFFF) as usize;
    (idx as f64 / SIN_SCALE).sin() as f32
}

// ── Carving mask (one per target chunk, shared across carvers) ────────────
struct Mask {
    bits: Vec<u64>,
}

impl Mask {
    fn new() -> Self {
        Self {
            bits: vec![0; (16 * 16 * WORLD_HEIGHT as usize + 63) / 64],
        }
    }

    #[inline]
    fn bit(x: usize, y: i32, z: usize) -> usize {
        x | (z << 4) | (((y - MIN_Y) as usize) << 8)
    }

    /// Returns true if this cell was not carved yet (and marks it carved).
    #[inline]
    fn set(&mut self, x: usize, y: i32, z: usize) -> bool {
        let i = Self::bit(x, y, z);
        let (word, bit) = (i >> 6, i & 63);
        let mask = 1u64 << bit;
        if self.bits[word] & mask != 0 {
            false
        } else {
            self.bits[word] |= mask;
            true
        }
    }
}

// ── Random helpers ────────────────────────────────────────────────────────
/// `WorldgenRandom.setLargeFeatureSeed` (26.2 formula).
fn set_large_feature_seed(r: &mut JavaRandom, seed: i64, cx: i32, cz: i32) {
    r.set_seed(seed);
    let xs = r.next_long();
    let zs = r.next_long();
    let result = (cx as i64).wrapping_mul(xs) ^ (cz as i64).wrapping_mul(zs) ^ seed;
    r.set_seed(result);
}

/// `UniformFloat.sample` = min + nextFloat·(max−min), float arithmetic.
#[inline]
fn uniform_float(r: &mut JavaRandom, min: f32, max_exclusive: f32) -> f32 {
    min + r.next_float() * (max_exclusive - min)
}

/// `Mth.randomBetweenInclusive`.
#[inline]
fn random_between(r: &mut JavaRandom, min: i32, max: i32) -> i32 {
    min + r.next_int((max - min + 1) as usize) as i32
}

/// `#minecraft:overworld_carver_replaceables` restricted to the block ids
/// BCore emits before the carver stage (stone family, surface substrates,
/// sand/gravel, water). Ore ids do not exist yet (features run after).
#[inline]
fn replaceable(s: u32) -> bool {
    matches!(
        s,
        block::STONE
            | block::DEEPSLATE
            | block::GRANITE
            | block::DIORITE
            | block::ANDESITE
            | block::TUFF
            | block::DIRT
            | block::GRASS_BLOCK
            | block::COARSE_DIRT
            | block::PODZOL
            | block::SAND
            | block::GRAVEL
            | block::SANDSTONE
            | block::SNOW_BLOCK
            | block::WATER
    )
}

/// `WorldCarver.carveEllipsoid` — carves the target chunk at `pos`.
///
/// `floor` carries the cave `floorLevel` (cells at `yd <= floor` are kept as
/// the cave floor); `canyon_width` switches to the canyon skip rule
/// `(xd²+zd²)·w + yd²/6 ≥ 1` with the per-y width factors.
#[allow(clippy::too_many_arguments)]
fn carve_ellipsoid(
    pos: ChunkPos,
    chunk: &mut GeneratedChunk,
    mask: &mut Mask,
    aquifer: &mut Aquifer,
    cx: f64,
    cy: f64,
    cz: f64,
    hr: f64,
    vr: f64,
    floor: f64,
    canyon_width: Option<&[f32]>,
) {
    let bx = pos.x * 16;
    let bz = pos.z * 16;
    // Vanilla: bail out when the sphere cannot reach this chunk.
    let (center_x, center_z) = (bx as f64 + 8.0, bz as f64 + 8.0);
    if (cx - center_x).abs() > 16.0 + hr * 2.0 || (cz - center_z).abs() > 16.0 + hr * 2.0 {
        return;
    }
    // Integer bounds relative to the chunk; bail when the ellipsoid does not
    // overlap it (keeping the math in i32 avoids usize wrap of negatives).
    let lo_x = (cx - hr).floor() as i32 - bx - 1;
    let hi_x = (cx + hr).floor() as i32 - bx;
    let lo_z = (cz - hr).floor() as i32 - bz - 1;
    let hi_z = (cz + hr).floor() as i32 - bz;
    if hi_x < 0 || lo_x > 15 || hi_z < 0 || lo_z > 15 {
        return;
    }
    let min_x = lo_x.max(0) as usize;
    let max_x = hi_x.min(15) as usize;
    let min_z = lo_z.max(0) as usize;
    let max_z = hi_z.min(15) as usize;
    let min_y = ((cy - vr).floor() as i32 - 1).max(MIN_Y + 1);
    let max_y = ((cy + vr).floor() as i32 + 1).min(MAX_Y - 7);
    if max_y < min_y {
        return;
    }
    for lz in min_z..=max_z {
        let wz = (bz + lz as i32) as f64 + 0.5;
        let zd = (wz - cz) / hr;
        for lx in min_x..=max_x {
            let wx = (bx + lx as i32) as f64 + 0.5;
            let xd = (wx - cx) / hr;
            if xd * xd + zd * zd >= 1.0 {
                continue;
            }
            for wy in (min_y..=max_y).rev() {
                let yd = (wy as f64 - 0.5 - cy) / vr;
                let skip = match canyon_width {
                    None => yd <= floor || xd * xd + yd * yd + zd * zd >= 1.0,
                    Some(width) => {
                        let yi = (wy - MIN_Y) as usize;
                        let w = if yi == 0 { 1.0 } else { width[yi - 1] as f64 };
                        (xd * xd + zd * zd) * w + yd * yd / 6.0 >= 1.0
                    }
                };
                if skip || !mask.set(lx, wy, lz) {
                    continue;
                }
                let Some(old) = chunk.get(lx, wy, lz) else {
                    continue;
                };
                if !replaceable(old) {
                    continue;
                }
                // `WorldCarver.getCarveState`: lava below the configured level,
                // else the aquifer's substance at density 0.0.
                let state = if wy <= LAVA_LEVEL {
                    block::LAVA
                } else {
                    aquifer.substance(bx + lx as i32, wy, bz + lz as i32, 0.0)
                };
                if state != block::STONE {
                    chunk.set(lx, wy, lz, state);
                }
            }
        }
    }
}

/// `WorldCarver.canReach` — stop carving when the tunnel drifts too far.
#[inline]
fn can_reach(pos: ChunkPos, x: f64, z: f64, step: i32, dist: i32, thickness: f32) -> bool {
    let (x_mid, z_mid) = ((pos.x * 16 + 8) as f64, (pos.z * 16 + 8) as f64);
    let dx = x - x_mid;
    let dz = z - z_mid;
    let remaining = (dist - step) as f64;
    let reach = thickness + 18.0;
    dx * dx + dz * dz - remaining * remaining <= (reach * reach) as f64
}

/// `CaveWorldCarver.createTunnel` recursion (tunnels + branch splits).
#[allow(clippy::too_many_arguments)]
fn carve_cave(
    pos: ChunkPos,
    chunk: &mut GeneratedChunk,
    mask: &mut Mask,
    aquifer: &mut Aquifer,
    tunnel_seed: i64,
    mut cx: f64,
    mut cy: f64,
    mut cz: f64,
    hm: f64,
    vm: f64,
    thickness: f32,
    mut hrot: f32,
    mut vrot: f32,
    step: i32,
    dist: i32,
    floor: f64,
) {
    let mut r = JavaRandom::new(tunnel_seed);
    let split_point = r.next_int((dist / 2) as usize) as i32 + dist / 4;
    let steep = r.next_int(6) == 0;
    let mut y_rota = 0.0f32;
    let mut x_rota = 0.0f32;
    for current_step in step..dist {
        let hr = 1.5
            + mth_sin(std::f64::consts::PI * current_step as f64 / dist as f64) as f64
                * thickness as f64;
        let vr = hr;
        let cos_x = mth_cos(vrot as f64);
        cx += (mth_cos(hrot as f64) * cos_x) as f64;
        cy += mth_sin(vrot as f64) as f64;
        cz += (mth_sin(hrot as f64) * cos_x) as f64;
        vrot *= if steep { 0.92 } else { 0.7 };
        vrot += x_rota * 0.1;
        hrot += y_rota * 0.1;
        x_rota *= 0.9;
        y_rota *= 0.75;
        x_rota += (r.next_float() - r.next_float()) * r.next_float() * 2.0;
        y_rota += (r.next_float() - r.next_float()) * r.next_float() * 4.0;
        if current_step == split_point && thickness > 1.0 {
            // Vanilla evaluates call arguments left-to-right: the branch seed
            // (nextLong) is drawn *before* the branch thickness (nextFloat).
            let s1 = r.next_long();
            let t1 = r.next_float() * 0.5 + 0.5;
            let s2 = r.next_long();
            let t2 = r.next_float() * 0.5 + 0.5;
            let v3 = vrot / 3.0;
            carve_cave(
                pos,
                chunk,
                mask,
                aquifer,
                s1,
                cx,
                cy,
                cz,
                hm,
                vm,
                t1,
                hrot - std::f32::consts::FRAC_PI_2,
                v3,
                current_step,
                dist,
                floor,
            );
            carve_cave(
                pos,
                chunk,
                mask,
                aquifer,
                s2,
                cx,
                cy,
                cz,
                hm,
                vm,
                t2,
                hrot + std::f32::consts::FRAC_PI_2,
                v3,
                current_step,
                dist,
                floor,
            );
            return;
        }
        if r.next_int(4) != 0 {
            if !can_reach(pos, cx, cz, current_step, dist, thickness) {
                return;
            }
            carve_ellipsoid(
                pos,
                chunk,
                mask,
                aquifer,
                cx,
                cy,
                cz,
                hr * hm,
                vr * vm,
                floor,
                None,
            );
        }
    }
}

/// `CaveWorldCarver.carve` for one source chunk (one cave run).
#[allow(clippy::too_many_arguments)]
fn carve_cave_chunk(
    pos: ChunkPos,
    chunk: &mut GeneratedChunk,
    mask: &mut Mask,
    aquifer: &mut Aquifer,
    r: &mut JavaRandom,
    sx: i32,
    sz: i32,
    y_max: i32,
) {
    // caveCount = nextInt(nextInt(nextInt(15)+1)+1); getCaveBound() = 15.
    let cave_count = {
        let a = r.next_int(15);
        let b = r.next_int(a + 1);
        r.next_int(b + 1)
    };
    for _ in 0..cave_count {
        let cx = (sx * 16 + r.next_int(16) as i32) as f64;
        let cy = random_between(r, CAVE_Y_MIN, y_max) as f64;
        let cz = (sz * 16 + r.next_int(16) as i32) as f64;
        let hm = uniform_float(r, 0.7, 1.4) as f64;
        let vm = uniform_float(r, 0.8, 1.3) as f64;
        let floor = uniform_float(r, -1.0, -0.4) as f64;
        let mut tunnels = 1;
        if r.next_int(4) == 0 {
            // createRoom: ellipsoid with the sampled y scale.
            let y_scale = uniform_float(r, 0.1, 0.9) as f64;
            let thickness = 1.0 + r.next_float() as f64 * 6.0;
            let hr = 1.5 + mth_sin(std::f64::consts::FRAC_PI_2) as f64 * thickness;
            let vr = hr * y_scale;
            carve_ellipsoid(
                pos,
                chunk,
                mask,
                aquifer,
                cx + 1.0,
                cy,
                cz,
                hr,
                vr,
                floor,
                None,
            );
            tunnels += r.next_int(4);
        }
        for _ in 0..tunnels {
            let hrot = r.next_float() * std::f32::consts::TAU;
            let vrot = (r.next_float() - 0.5) / 4.0;
            // getThickness.
            let mut thickness = r.next_float() * 2.0 + r.next_float();
            if r.next_int(10) == 0 {
                thickness *= r.next_float() * r.next_float() * 3.0 + 1.0;
            }
            let dist = MAX_DISTANCE - r.next_int((MAX_DISTANCE / 4) as usize) as i32;
            carve_cave(
                pos,
                chunk,
                mask,
                aquifer,
                r.next_long(),
                cx,
                cy,
                cz,
                hm,
                vm,
                thickness,
                hrot,
                vrot,
                0,
                dist,
                floor,
            );
        }
    }
}

/// `CanyonWorldCarver.carve` + `doCarve` for one source chunk.
#[allow(clippy::too_many_arguments)]
fn carve_canyon_chunk(
    pos: ChunkPos,
    chunk: &mut GeneratedChunk,
    mask: &mut Mask,
    aquifer: &mut Aquifer,
    r: &mut JavaRandom,
    sx: i32,
    sz: i32,
) {
    let cx = (sx * 16 + r.next_int(16) as i32) as f64;
    // canyon y: UniformHeight(absolute 10, absolute 67).
    let cy = random_between(r, 10, 67) as f64;
    let cz = (sz * 16 + r.next_int(16) as i32) as f64;
    let hrot0 = r.next_float() * std::f32::consts::TAU;
    let vrot0 = uniform_float(r, -0.125, 0.125);
    let y_scale = 3.0f64;
    // shape.thickness = TrapezoidFloat(0, 6, plateau 2): nextFloat·4 + nextFloat·2.
    let thickness = r.next_float() * 4.0 + r.next_float() * 2.0;
    // distanceFactor uniform(0.75, 1.0); maxDistance = 112.
    let distance = (MAX_DISTANCE as f64 * uniform_float(r, 0.75, 1.0) as f64) as i32;
    // doCarve: fork the tunnel random, then initWidthFactors (smoothness 3).
    let mut r2 = JavaRandom::new(r.next_long());
    let depth = WORLD_HEIGHT as usize;
    let mut width = vec![0.0f32; depth];
    let mut wf = 1.0f32;
    for (i, slot) in width.iter_mut().enumerate() {
        if i == 0 || r2.next_int(3) == 0 {
            wf = 1.0 + r2.next_float() * r2.next_float();
        }
        *slot = wf * wf;
    }
    let (mut x, mut y, mut z) = (cx, cy, cz);
    let (mut hrot, mut vrot) = (hrot0, vrot0);
    let (mut y_rota, mut x_rota) = (0.0f32, 0.0f32);
    for current_step in 0..distance {
        let mut hr = 1.5
            + mth_sin(std::f64::consts::PI * current_step as f64 / distance as f64) as f64
                * thickness as f64;
        let mut vr = hr * y_scale;
        hr *= uniform_float(&mut r2, 0.75, 1.0) as f64;
        // updateVerticalRadius: vrd=1.0, vrc=0.0 → factor=1.0.
        vr *= uniform_float(&mut r2, 0.75, 1.0) as f64;
        let cos_x = mth_cos(vrot as f64);
        x += (mth_cos(hrot as f64) * cos_x) as f64;
        y += mth_sin(vrot as f64) as f64;
        z += (mth_sin(hrot as f64) * cos_x) as f64;
        vrot *= 0.7;
        vrot += x_rota * 0.05;
        hrot += y_rota * 0.05;
        x_rota *= 0.8;
        y_rota *= 0.5;
        x_rota += (r2.next_float() - r2.next_float()) * r2.next_float() * 2.0;
        y_rota += (r2.next_float() - r2.next_float()) * r2.next_float() * 4.0;
        if r2.next_int(4) != 0 {
            if !can_reach(pos, x, z, current_step, distance, thickness) {
                return;
            }
            carve_ellipsoid(
                pos,
                chunk,
                mask,
                aquifer,
                x,
                y,
                z,
                hr,
                vr,
                0.0,
                Some(&width),
            );
        }
    }
}

/// `NoiseBasedChunkGenerator.applyCarvers` — the 17×17 source neighborhood.
pub(crate) fn apply(
    seed: i64,
    pos: ChunkPos,
    chunk: &mut GeneratedChunk,
    graph: &VanillaGraph,
    ctx: density::EvalContext,
) {
    let mut mask = Mask::new();
    let mut aquifer = Aquifer::new(
        seed,
        chunk.height_at(8, 8),
        false,
        density::noise_registry(),
        graph.preliminary_surface_level.as_ref(),
        ctx,
    );
    let mut r = JavaRandom::new(0);
    for dx in -RANGE * 2..=RANGE * 2 {
        for dz in -RANGE * 2..=RANGE * 2 {
            let sx = pos.x + dx;
            let sz = pos.z + dz;
            for (index, (prob, y_max)) in CAVE_CARVERS.iter().enumerate() {
                set_large_feature_seed(&mut r, seed + index as i64, sx, sz);
                if r.next_float() <= *prob {
                    carve_cave_chunk(pos, chunk, &mut mask, &mut aquifer, &mut r, sx, sz, *y_max);
                }
            }
            set_large_feature_seed(&mut r, seed + 2, sx, sz);
            if r.next_float() <= CANYON_PROB {
                carve_canyon_chunk(pos, chunk, &mut mask, &mut aquifer, &mut r, sx, sz);
            }
        }
    }
}
