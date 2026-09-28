//! Vanilla 26.1 configured carver geometry from `WorldCarver`,
//! `CaveWorldCarver` and `CanyonWorldCarver` (overworld: `minecraft:cave`,
//! `minecraft:cave_extra_underground`, `minecraft:canyon`).
//!
//! Chunk-status order: density fill → buildSurface → CARVERS → features.
//! Every target chunk is carved from each source chunk in a 17×17
//! neighborhood (`NoiseBasedChunkGenerator.applyCarvers`): each source
//! chunk's biome contributes its configured carver list; default overworld
//! generation uses `[cave, cave_extra_underground, canyon]`, so the
//! list is hardcoded here (index = position in the list seeds the random).
//!
//! Random semantics: the per-carver random is a legacy 48-bit LCG
//! (`LegacyRandomSource`, i.e. `JavaRandom`), re-seeded per carver with
//! `WorldgenRandom.setLargeFeatureSeed(seed + index, sx, sz)` — the 26.1
//! formula `setSeed(seed); xs=nextLong(); zs=nextLong(); setSeed(cx*xs ^ cz*zs ^ seed)`.
//! Tunnel branches fork a fresh LCG seeded with `random.nextLong()`
//! (`RandomSource.createThreadLocalInstance(seed)` = `SingleThreadedRandomSource`,
//! which is the same 48-bit LCG).

use crate::mth::{cos as mth_cos, sin as mth_sin};
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

// ── Carving mask (one per target chunk, shared across carvers) ────────────
struct Mask {
    bits: Vec<u64>,
}

trait CarvingMask {
    fn get(&mut self, x: usize, y: i32, z: usize) -> bool;
    fn set(&mut self, x: usize, y: i32, z: usize);
}

impl Mask {
    fn new() -> Self {
        Self {
            bits: vec![0; (16 * 16 * WORLD_HEIGHT as usize + 63) / 64],
        }
    }

    #[inline]
    fn bit(x: usize, y: i32, z: usize) -> usize {
        (x & 15) | ((z & 15) << 4) | (((y - MIN_Y) as usize) << 8)
    }
}

impl CarvingMask for Mask {
    #[inline]
    fn get(&mut self, x: usize, y: i32, z: usize) -> bool {
        let i = Self::bit(x, y, z);
        self.bits[i >> 6] & (1u64 << (i & 63)) != 0
    }

    #[inline]
    fn set(&mut self, x: usize, y: i32, z: usize) {
        let i = Self::bit(x, y, z);
        self.bits[i >> 6] |= 1u64 << (i & 63);
    }
}

// ── Random helpers ────────────────────────────────────────────────────────
/// `WorldgenRandom.setLargeFeatureSeed(worldSeed + carverIndex, cx, cz)`.
fn set_carver_seed(r: &mut JavaRandom, seed: i64, index: usize, cx: i32, cz: i32) {
    let seed = seed.wrapping_add(index as i64);
    r.set_seed(seed);
    let xs = r.next_long();
    let zs = r.next_long();
    let result = (cx as i64).wrapping_mul(xs) ^ (cz as i64).wrapping_mul(zs) ^ seed;
    r.set_seed(result);
}

fn is_start_chunk(r: &mut JavaRandom, probability: f32) -> bool {
    r.next_float() <= probability
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

/// `CaveWorldCarver.getThickness`, including the occasional wider tunnel.
fn cave_thickness(r: &mut JavaRandom) -> f32 {
    let mut thickness = r.next_float() * 2.0 + r.next_float();
    if r.next_int(10) == 0 {
        thickness *= r.next_float() * r.next_float() * 3.0 + 1.0;
    }
    thickness
}

/// `CanyonWorldCarver.initWidthFactors` for the overworld's width smoothness 3.
fn canyon_width_factors(r: &mut JavaRandom) -> Vec<f32> {
    let mut width = vec![0.0f32; WORLD_HEIGHT as usize];
    let mut factor = 1.0f32;
    for (i, slot) in width.iter_mut().enumerate() {
        if i == 0 || r.next_int(3) == 0 {
            factor = 1.0 + r.next_float() * r.next_float();
        }
        *slot = factor * factor;
    }
    width
}

/// All states accepted by the native 26.1 overworld carver configurations.
/// `carvers_26_1.json` checks every registered state against `canReplaceBlock`.
/// Includes noise-fill ore veins, which precede the carver stage.
#[inline]
fn replaceable(s: u32) -> bool {
    matches!(
        s,
        block::STONE
            | block::GRANITE
            | block::DIORITE
            | block::ANDESITE
            | 8..=13 // grass (both snowy states), dirt, coarse dirt, podzol
            | 86..=101 // every water level
            | 118..=128 // sand, suspicious sand, red sand, gravel, suspicious gravel
            | 131..=132 // iron and deepslate iron ore
            | 165..=167 // muddy mangrove roots, all axes
            | block::SANDSTONE
            | 6919..=6926 // snow layers
            | block::SNOW_BLOCK
            | 8918..=8919 // mycelium, both snowy states
            | 11444..=11459 // dyed terracotta
            | 12912 // terracotta
            | 12914 // packed ice
            | 13247 // red sandstone
            | block::TUFF
            | 24687 // calcite
            | 24689 // powder snow
            | 25313..=25314 // copper and deepslate copper ore
            | 27862 // moss block
            | 27921..=27925 // rooted dirt, mud, deepslate (all axes)
            | 29577..=29578 // raw iron and raw copper blocks
            | 29703 // pale moss block
    )
}

#[derive(Clone, Copy)]
struct CarveState {
    id: u32,
    has_fluid: bool,
}

/// Storage and environment boundary used by native-style block carving.
trait CarvingWorld {
    fn block(&mut self, pos: [i32; 3]) -> u32;
    fn set_block(&mut self, pos: [i32; 3], state: u32);
    fn substance(&mut self, pos: [i32; 3], density: f64) -> Option<CarveState>;
    fn should_schedule_fluid_update(&mut self) -> bool;
    fn mark_for_postprocessing(&mut self, pos: [i32; 3]);
    fn top_material(&mut self, pos: [i32; 3], under_fluid: bool) -> Option<CarveState>;
}

struct ChunkCarvingWorld<'a, 'graph> {
    chunk: &'a mut GeneratedChunk,
    aquifer: &'a mut Aquifer<'graph>,
}

impl CarvingWorld for ChunkCarvingWorld<'_, '_> {
    fn block(&mut self, [x, y, z]: [i32; 3]) -> u32 {
        // Native ProtoChunk returns VOID_AIR outside its build height.
        self.chunk
            .get((x & 15) as usize, y, (z & 15) as usize)
            .unwrap_or(15292)
    }

    fn set_block(&mut self, [x, y, z]: [i32; 3], state: u32) {
        self.chunk
            .set((x & 15) as usize, y, (z & 15) as usize, state);
    }

    fn substance(&mut self, [x, y, z]: [i32; 3], density: f64) -> Option<CarveState> {
        let id = self.aquifer.substance(x, y, z, density);
        // The aquifer encodes native null as STONE; its non-null outputs are air/water/lava.
        (id != block::STONE).then_some(CarveState {
            id,
            has_fluid: matches!(id, block::WATER | block::LAVA),
        })
    }

    fn should_schedule_fluid_update(&mut self) -> bool {
        self.aquifer.should_schedule_fluid_update()
    }

    fn mark_for_postprocessing(&mut self, [x, y, z]: [i32; 3]) {
        if (MIN_Y..=MAX_Y).contains(&y) {
            self.chunk
                .postprocessing
                .push(((x & 15) as usize, y, (z & 15) as usize));
        }
    }

    fn top_material(&mut self, pos: [i32; 3], under_fluid: bool) -> Option<CarveState> {
        let graph = VanillaGraph::load().expect("overworld generation graph");
        let context = carving_surface_context(
            graph,
            self.aquifer.seed(),
            self.aquifer.context(),
            pos,
            under_fluid,
        );
        graph
            .surface_rule
            .as_ref()
            .expect("overworld surface rule")
            .evaluate_in_chunk(&context, self.chunk)
            .map(|id| CarveState {
                id,
                has_fluid: matches!(id, 86..=117),
            })
    }
}

/// `SurfaceRules.Context.getMinSurfaceLevel` samples the four 16-block cell corners.
fn preliminary_surface_corners(
    graph: &VanillaGraph,
    ctx: &density::EvalContext,
    x: i32,
    z: i32,
) -> [i32; 4] {
    let function = graph
        .preliminary_surface_level
        .as_ref()
        .expect("preliminary surface density");
    std::array::from_fn(|i| {
        let cx = (x & !15) + (i as i32 & 1) * 16;
        let cz = (z & !15) + (i as i32 >> 1) * 16;
        density::evaluate(function, cx as f64, 0.0, cz as f64, ctx).floor() as i32
    })
}

fn carving_surface_context(
    graph: &VanillaGraph,
    seed: i64,
    ctx: density::EvalContext,
    [x, y, z]: [i32; 3],
    under_fluid: bool,
) -> crate::surface_rules::SurfaceContext<'static> {
    let [a, b, c, d] = preliminary_surface_corners(graph, &ctx, x, z).map(f64::from);
    let fx = (x & 15) as f64 / 16.0;
    let fz = (z & 15) as f64 / 16.0;
    let north = a + fx * (b - a);
    let south = c + fx * (d - c);
    let preliminary = (north + fz * (south - north)).floor() as i32;
    // applyCarvers installs the generator's raw climate sampler in BiomeManager.
    // It does not use the chunk palette or clamp the selected quart Y.
    let (qx, qy, qz) = crate::biome_zoom::BiomeZoom::new(seed).quart_at((x, y, z));
    let biome_ctx = density::EvalContext {
        mode: density::EvaluationMode::Raw,
        ..ctx
    };
    crate::surface_rules::SurfaceContext {
        biome: graph.noise_biome_at(qx, qy, qz, &biome_ctx),
        stone_depth_above: 1,
        stone_depth_below: 1,
        water_height: if under_fluid { y + 1 } else { i32::MIN },
        surface_depth: crate::surface_rules::surface_depth(seed, x, z),
        preliminary_surface_level: preliminary,
        sea_level: crate::SEA_LEVEL,
        x,
        y,
        z,
        seed,
        noise: Some(density::noise_registry()),
    }
}

/// Native `WorldCarver.carveBlock`, with a caller-owned, sticky per-column surface flag.
fn carve_block(world: &mut impl CarvingWorld, pos: [i32; 3], found_surface: &mut bool) -> bool {
    let old = world.block(pos);
    if matches!(old, 8..=9 | 8918..=8919) {
        *found_surface = true;
    }
    if !replaceable(old) {
        return false;
    }
    let state = if pos[1] <= LAVA_LEVEL {
        // This path must leave the aquifer's previous scheduling flag intact.
        Some(CarveState {
            id: block::LAVA,
            has_fluid: true,
        })
    } else {
        world.substance(pos, 0.0)
    };
    let Some(state) = state else { return false };
    world.set_block(pos, state.id);
    if world.should_schedule_fluid_update() && state.has_fluid {
        world.mark_for_postprocessing(pos);
    }
    if *found_surface {
        let below = [pos[0], pos[1] - 1, pos[2]];
        if world.block(below) == block::DIRT {
            if let Some(top) = world.top_material(below, state.has_fluid) {
                world.set_block(below, top.id);
                if top.has_fluid {
                    world.mark_for_postprocessing(below);
                }
            }
        }
    }
    true
}

struct Ellipsoid<'a> {
    center: [f64; 3],
    horizontal_radius: f64,
    vertical_radius: f64,
    floor: f64,
    canyon_width: Option<&'a [f32]>,
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
    // Intercept the same geometry boundary as the native recording subclass.
    #[cfg(test)]
    if tests::record_ellipsoid([cx, cy, cz, hr, vr]) {
        return;
    }
    let mut world = ChunkCarvingWorld { chunk, aquifer };
    rasterize_ellipsoid(
        pos,
        mask,
        Ellipsoid {
            center: [cx, cy, cz],
            horizontal_radius: hr,
            vertical_radius: vr,
            floor,
            canyon_width,
        },
        |point, surface| carve_block(&mut world, point, surface),
    );
}

/// Native ellipsoid visitation and mask ownership; the block callback is invoked
/// for every unmasked candidate, even after a previous callback succeeded.
fn rasterize_ellipsoid(
    pos: ChunkPos,
    mask: &mut impl CarvingMask,
    shape: Ellipsoid<'_>,
    mut carve: impl FnMut([i32; 3], &mut bool) -> bool,
) -> bool {
    let Ellipsoid {
        center: [cx, cy, cz],
        horizontal_radius: hr,
        vertical_radius: vr,
        floor,
        canyon_width,
    } = shape;
    let bx = pos.x * 16;
    let bz = pos.z * 16;
    // Vanilla: bail out when the sphere cannot reach this chunk.
    let (center_x, center_z) = (bx as f64 + 8.0, bz as f64 + 8.0);
    if (cx - center_x).abs() > 16.0 + hr * 2.0 || (cz - center_z).abs() > 16.0 + hr * 2.0 {
        return false;
    }
    // Integer bounds relative to the chunk; bail when the ellipsoid does not
    // overlap it (keeping the math in i32 avoids usize wrap of negatives).
    let lo_x = (cx - hr).floor() as i32 - bx - 1;
    let hi_x = (cx + hr).floor() as i32 - bx;
    let lo_z = (cz - hr).floor() as i32 - bz - 1;
    let hi_z = (cz + hr).floor() as i32 - bz;
    if hi_x < 0 || lo_x > 15 || hi_z < 0 || lo_z > 15 {
        return false;
    }
    let min_x = lo_x.max(0) as usize;
    let max_x = hi_x.min(15) as usize;
    let min_z = lo_z.max(0) as usize;
    let max_z = hi_z.min(15) as usize;
    let min_y = ((cy - vr).floor() as i32 - 1).max(MIN_Y + 1);
    let max_y = ((cy + vr).floor() as i32 + 1).min(MAX_Y - 7);
    if max_y <= min_y {
        return false;
    }
    let mut carved = false;
    for lx in min_x..=max_x {
        let wx = (bx + lx as i32) as f64 + 0.5;
        let xd = (wx - cx) / hr;
        for lz in min_z..=max_z {
            let wz = (bz + lz as i32) as f64 + 0.5;
            let zd = (wz - cz) / hr;
            if xd * xd + zd * zd >= 1.0 {
                continue;
            }
            let mut found_surface = false;
            for wy in (min_y + 1..=max_y).rev() {
                let yd = (wy as f64 - 0.5 - cy) / vr;
                let skip = match canyon_width {
                    None => yd <= floor || xd * xd + yd * yd + zd * zd >= 1.0,
                    Some(width) => {
                        let yi = (wy - MIN_Y) as usize;
                        let w = if yi == 0 { 1.0 } else { width[yi - 1] as f64 };
                        (xd * xd + zd * zd) * w + yd * yd / 6.0 >= 1.0
                    }
                };
                if skip || mask.get(lx, wy, lz) {
                    continue;
                }
                mask.set(lx, wy, lz);
                carved |= carve([bx + lx as i32, wy, bz + lz as i32], &mut found_surface);
            }
        }
    }
    carved
}

/// `WorldCarver.canReach` — stop carving when the tunnel drifts too far.
#[inline]
fn can_reach(pos: ChunkPos, x: f64, z: f64, step: i32, dist: i32, thickness: f32) -> bool {
    let (x_mid, z_mid) = ((pos.x * 16 + 8) as f64, (pos.z * 16 + 8) as f64);
    let dx = x - x_mid;
    let dz = z - z_mid;
    let remaining = (dist - step) as f64;
    // Native performs two float additions, then widens before the square.
    let reach = (thickness + 2.0 + 16.0) as f64;
    dx * dx + dz * dz - remaining * remaining <= reach * reach
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
        // Vanilla `Mth.PI` is a *float* ((float)Math.PI), so the whole
        // `Mth.PI * currentStep / dist` expression is float arithmetic
        // before the SIN-table lookup (which takes a double).
        let progress_arg = std::f32::consts::PI * current_step as f32 / dist as f32;
        let sin_val = mth_sin(progress_arg as f64);
        let hr = 1.5 + (sin_val * thickness) as f64;
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
            // createRoom: ellipsoid with the sampled y scale. Vanilla room
            // thickness = 1.0F + nextFloat()*6.0F (float), radius =
            // 1.5 + sin((float)(π/2))·thickness (sin table → 1.0f).
            let y_scale = uniform_float(r, 0.1, 0.9) as f64;
            let thickness: f32 = 1.0 + r.next_float() * 6.0;
            let sin_val = mth_sin(std::f32::consts::FRAC_PI_2 as f64);
            let hr = 1.5 + (sin_val * thickness) as f64;
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
            let thickness = cave_thickness(r);
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
    let distance = (MAX_DISTANCE as f32 * uniform_float(r, 0.75, 1.0)) as i32;
    // doCarve: fork the tunnel random, then initWidthFactors (smoothness 3).
    let mut r2 = JavaRandom::new(r.next_long());
    let width = canyon_width_factors(&mut r2);
    let (mut x, mut y, mut z) = (cx, cy, cz);
    let (mut hrot, mut vrot) = (hrot0, vrot0);
    let (mut y_rota, mut x_rota) = (0.0f32, 0.0f32);
    for current_step in 0..distance {
        // Vanilla `Mth.PI` is float ((float)Math.PI): float arithmetic.
        let progress_arg = std::f32::consts::PI * current_step as f32 / distance as f32;
        let sin_val = mth_sin(progress_arg as f64);
        let mut hr = 1.5 + (sin_val * thickness) as f64;
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
    let mut aquifer = Aquifer::new(seed, graph, ctx);
    let mut r = JavaRandom::new(0);
    for dx in -RANGE * 2..=RANGE * 2 {
        for dz in -RANGE * 2..=RANGE * 2 {
            let sx = pos.x + dx;
            let sz = pos.z + dz;
            for (index, (prob, y_max)) in CAVE_CARVERS.iter().enumerate() {
                set_carver_seed(&mut r, seed, index, sx, sz);
                if is_start_chunk(&mut r, *prob) {
                    carve_cave_chunk(pos, chunk, &mut mask, &mut aquifer, &mut r, sx, sz, *y_max);
                }
            }
            set_carver_seed(&mut r, seed, 2, sx, sz);
            if is_start_chunk(&mut r, CANYON_PROB) {
                carve_canyon_chunk(pos, chunk, &mut mask, &mut aquifer, &mut r, sx, sz);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::{
        cell::RefCell, collections::BTreeMap, collections::BTreeSet, rc::Rc, sync::OnceLock,
    };

    thread_local! {
        static TRACE: RefCell<Option<Vec<[u64; 5]>>> = const { RefCell::new(None) };
    }

    pub(super) fn record_ellipsoid(values: [f64; 5]) -> bool {
        TRACE.with_borrow_mut(|trace| {
            if let Some(trace) = trace {
                trace.push(values.map(f64::to_bits));
                true
            } else {
                false
            }
        })
    }

    fn capture(f: impl FnOnce()) -> Vec<[u64; 5]> {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                TRACE.with_borrow_mut(|trace| *trace = None);
            }
        }
        TRACE.with_borrow_mut(|trace| {
            assert!(trace.is_none(), "nested carver trace");
            *trace = Some(Vec::new());
        });
        let _reset = Reset;
        f();
        TRACE.with_borrow_mut(|trace| trace.take().unwrap())
    }

    fn fixture() -> &'static Value {
        static FIXTURE: OnceLock<Value> = OnceLock::new();
        FIXTURE.get_or_init(|| {
            serde_json::from_str(include_str!("../data/carvers_26_1.json")).unwrap()
        })
    }

    fn samples(op: &str) -> impl Iterator<Item = &Value> {
        fixture()["samples"]
            .as_array()
            .unwrap()
            .iter()
            .filter(move |sample| sample["op"] == op)
    }

    fn bits(value: &Value) -> u64 {
        u64::from_str_radix(value.as_str().unwrap(), 16).unwrap()
    }

    fn float(value: &Value) -> f32 {
        assert_eq!(value.as_str().unwrap().len(), 8);
        f32::from_bits(bits(value) as u32)
    }

    fn double(value: &Value) -> f64 {
        assert_eq!(value.as_str().unwrap().len(), 16);
        f64::from_bits(bits(value))
    }

    fn int(value: &Value) -> i32 {
        value.as_i64().unwrap().try_into().unwrap()
    }

    fn position(value: &Value) -> ChunkPos {
        ChunkPos::new(int(&value[0]), int(&value[1]))
    }

    fn probability(index: usize) -> f32 {
        match index {
            0 | 1 => CAVE_CARVERS[index].0,
            2 => CANYON_PROB,
            _ => panic!("unknown carver index {index}"),
        }
    }

    #[test]
    fn original_geometry_payload_is_preserved() {
        let fixture = fixture();
        let baseline = serde_json::json!({
            "samples": &fixture["samples"].as_array().unwrap()[..432],
            "configurations": fixture["configurations"],
            "replaceability": fixture["replaceability"],
            "geometry_fields": fixture["geometry_fields"],
        });
        let hash = Sha256::digest(serde_json::to_vec(&baseline).unwrap());
        assert_eq!(
            format!("{hash:x}"),
            "466f906d701f6b687bb47b125385822a7d575dced4579f8fd450e5a2643257f6"
        );
    }

    #[test]
    fn original_voxel_payload_is_preserved() {
        let fixture = fixture();
        let baseline = json!({
            "samples": &fixture["samples"].as_array().unwrap()[..498],
            "configurations": fixture["configurations"],
            "replaceability": fixture["replaceability"],
            "geometry_fields": fixture["geometry_fields"],
            "voxel_metadata": fixture["voxel_metadata"],
        });
        let hash = Sha256::digest(serde_json::to_vec(&baseline).unwrap());
        assert_eq!(
            format!("{hash:x}"),
            "fbede4af5752c8216e1fcb7127c77575d5891f91f023491c9959f0bb14aa1b47"
        );
    }

    #[test]
    fn native_fixture_provenance_and_coverage() {
        let fixture = fixture();
        assert_eq!(fixture["minecraft"], "26.1");
        assert_eq!(
            fixture["jar_sha256"],
            "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52"
        );
        let mut digest = Sha256::new();
        for source in [
            include_bytes!("../../../scripts/TreeReference.java").as_slice(),
            include_bytes!("../../../scripts/OreReference.java").as_slice(),
            include_bytes!("../../../scripts/NativeWorldgenRegistries.java").as_slice(),
            include_bytes!("../../../scripts/NativeEntityLevel.java").as_slice(),
            include_bytes!("../../../scripts/CarverReference.java").as_slice(),
        ] {
            digest.update(source);
        }
        assert_eq!(fixture["probe_sha256"], format!("{:x}", digest.finalize()));
        let mut counts = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for sample in fixture["samples"].as_array().unwrap() {
            *counts.entry(sample["op"].as_str().unwrap()).or_insert(0) += 1;
            assert!(ids.insert(sample["id"].as_str().unwrap()));
        }
        assert_eq!(
            counts,
            BTreeMap::from([
                ("can_reach", 296),
                ("source_rng", 96),
                ("cave_thickness", 8),
                ("canyon_widths", 8),
                ("carve_trace", 16),
                ("tunnel_trace", 8),
                ("voxel", 66),
                ("top_material", 899),
                ("surface_voxel", 26),
                ("surface_bands", 8),
            ])
        );
        for (i, name) in ["cave", "cave_extra_underground", "canyon"]
            .iter()
            .enumerate()
        {
            let config = &fixture["configurations"][name]["config"];
            assert_eq!(
                config["probability"].as_f64().unwrap() as f32,
                probability(i)
            );
            assert_eq!(config["lava_level"]["above_bottom"], LAVA_LEVEL - MIN_Y);
            assert_eq!(
                config["replaceable"],
                "#minecraft:overworld_carver_replaceables"
            );
        }
        let ellipsoids: usize = samples("carve_trace")
            .chain(samples("tunnel_trace"))
            .map(|s| s["ellipsoids"].as_array().unwrap().len())
            .sum();
        assert_eq!(ellipsoids, 3398);
        eprintln!("1431 native carver samples; 498 preserved cases, {ellipsoids} geometry ellipsoids, 899 evaluated surface cases, 26 surface voxel cases, 8 band palettes; 89619 state predicates");
    }

    #[test]
    fn can_reach_matches_native_boundaries() {
        let mut differences = Vec::new();
        for sample in samples("can_reach") {
            let actual = can_reach(
                position(&sample["target"]),
                double(&sample["xz_bits"][0]),
                double(&sample["xz_bits"][1]),
                int(&sample["step"]),
                int(&sample["distance"]),
                float(&sample["thickness_bits"]),
            );
            if actual != sample["result"].as_bool().unwrap() {
                differences.push(sample["id"].as_str().unwrap());
            }
        }
        assert!(
            differences.is_empty(),
            "{} reach differences: {:?}",
            differences.len(),
            differences
        );
    }

    #[test]
    fn source_seeds_and_admission_match_native() {
        for sample in samples("source_rng") {
            let seed = sample["seed"].as_i64().unwrap();
            let index = sample["index"].as_u64().unwrap() as usize;
            let source = position(&sample["source"]);
            let mut random = JavaRandom::new(0);
            set_carver_seed(&mut random, seed, index, source.x, source.z);
            assert_eq!(
                is_start_chunk(&mut random, probability(index)),
                sample["start"].as_bool().unwrap(),
                "{}",
                sample["id"]
            );
            assert_eq!(
                random.next_long(),
                sample["next_i64"].as_i64().unwrap(),
                "{}",
                sample["id"]
            );
        }
    }

    #[test]
    fn thickness_and_width_rng_kernels_match_native() {
        for sample in samples("cave_thickness").chain(samples("canyon_widths")) {
            let mut random = JavaRandom::new(sample["seed"].as_i64().unwrap());
            let expected = sample["bits"].as_array().unwrap();
            let actual = if sample["op"] == "cave_thickness" {
                assert_eq!(expected.len(), 16);
                (0..16)
                    .map(|_| cave_thickness(&mut random))
                    .collect::<Vec<_>>()
            } else {
                assert_eq!(expected.len(), WORLD_HEIGHT as usize);
                canyon_width_factors(&mut random)
            };
            for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(
                    actual.to_bits(),
                    float(expected).to_bits(),
                    "{} value {i}",
                    sample["id"]
                );
            }
            assert_eq!(
                random.next_long(),
                sample["next_i64"].as_i64().unwrap(),
                "{}",
                sample["id"]
            );
        }
    }

    #[test]
    fn replaceability_matches_every_native_block_state() {
        let table = &fixture()["replaceability"];
        let count = table["state_count"].as_u64().unwrap() as u32;
        assert_eq!(count, 29873);
        for (name, states) in table["accepted"].as_object().unwrap() {
            let expected: BTreeSet<_> = states
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_u64().unwrap() as u32)
                .collect();
            assert_eq!(expected.len(), 87);
            let differences: Vec<_> = (0..count)
                .filter(|s| replaceable(*s) != expected.contains(s))
                .collect();
            assert!(
                differences.is_empty(),
                "{name}: {} predicate differences: {differences:?}",
                differences.len()
            );
        }
        assert!(!replaceable(count));
        assert!(!replaceable(u32::MAX));
    }

    #[test]
    fn geometry_matches_native_carves_and_tunnel_branches() {
        let graph = VanillaGraph::load().unwrap();
        for sample in samples("carve_trace").chain(samples("tunnel_trace")) {
            let seed = sample["seed"].as_i64().unwrap();
            let target = position(&sample["target"]);
            let mut chunk = GeneratedChunk::new(target);
            let mut mask = Mask::new();
            let mut aquifer = Aquifer::new(
                seed,
                graph,
                density::EvalContext {
                    seed,
                    ..Default::default()
                },
            );
            let actual = capture(|| {
                if sample["op"] == "carve_trace" {
                    let source = position(&sample["source"]);
                    let index = sample["index"].as_u64().unwrap() as usize;
                    let mut random = JavaRandom::new(0);
                    set_carver_seed(&mut random, seed, index, source.x, source.z);
                    let start = is_start_chunk(&mut random, probability(index));
                    assert_eq!(
                        start,
                        sample["start"].as_bool().unwrap(),
                        "{}",
                        sample["id"]
                    );
                    if start || sample["force"].as_bool().unwrap() {
                        if index < 2 {
                            carve_cave_chunk(
                                target,
                                &mut chunk,
                                &mut mask,
                                &mut aquifer,
                                &mut random,
                                source.x,
                                source.z,
                                CAVE_CARVERS[index].1,
                            );
                        } else {
                            carve_canyon_chunk(
                                target,
                                &mut chunk,
                                &mut mask,
                                &mut aquifer,
                                &mut random,
                                source.x,
                                source.z,
                            );
                        }
                    }
                    assert_eq!(
                        random.next_long(),
                        sample["next_i64"].as_i64().unwrap(),
                        "{}",
                        sample["id"]
                    );
                } else {
                    let origin = &sample["origin_bits"];
                    let multipliers = &sample["multiplier_bits"];
                    carve_cave(
                        target,
                        &mut chunk,
                        &mut mask,
                        &mut aquifer,
                        seed,
                        double(&origin[0]),
                        double(&origin[1]),
                        double(&origin[2]),
                        double(&multipliers[0]),
                        double(&multipliers[1]),
                        float(&sample["thickness_bits"]),
                        float(&sample["yaw_bits"]),
                        float(&sample["pitch_bits"]),
                        int(&sample["step"]),
                        int(&sample["distance"]),
                        -0.7,
                    );
                }
            });
            let expected = sample["ellipsoids"].as_array().unwrap();
            assert_eq!(
                actual.len(),
                expected.len(),
                "{} ellipsoid count",
                sample["id"]
            );
            for (row, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(expected.as_array().unwrap().len(), 5);
                for (column, &actual) in actual.iter().enumerate() {
                    assert_eq!(
                        actual,
                        double(&expected[column]).to_bits(),
                        "{} ellipsoid {row}, field {column}",
                        sample["id"]
                    );
                }
            }
        }
    }

    type Events = Rc<RefCell<Vec<Value>>>;

    fn point(value: &Value) -> [i32; 3] {
        [int(&value[0]), int(&value[1]), int(&value[2])]
    }

    fn mask_indices(mask: &Mask) -> Vec<u32> {
        let mut result = Vec::new();
        for (word, &value) in mask.bits.iter().enumerate() {
            let mut value = value;
            while value != 0 {
                result.push(word as u32 * 64 + value.trailing_zeros());
                value &= value - 1;
            }
        }
        result
    }

    fn premarked_mask(sample: &Value) -> Mask {
        let mut mask = Mask::new();
        for value in sample["premarked"].as_array().unwrap() {
            let [x, y, z] = point(value);
            // Native CarvingMask accepts raw X/Z values and masks them to four bits.
            mask.set(x as usize, y, z as usize);
        }
        mask
    }

    struct VoxelMask {
        mask: Mask,
        additional: BTreeSet<[i32; 3]>,
        events: Events,
    }

    impl CarvingMask for VoxelMask {
        fn get(&mut self, x: usize, y: i32, z: usize) -> bool {
            let result =
                self.additional.contains(&[x as i32, y, z as i32]) || self.mask.get(x, y, z);
            self.events
                .borrow_mut()
                .push(json!(["mask_get", x, y, z, result]));
            result
        }

        fn set(&mut self, x: usize, y: i32, z: usize) {
            self.mask.set(x, y, z);
            self.events.borrow_mut().push(json!(["mask_set", x, y, z]));
        }
    }

    fn fixture_state(id: i32) -> Option<CarveState> {
        if id == -1 {
            None
        } else {
            Some(CarveState {
                id: id.try_into().unwrap(),
                has_fluid: fixture()["voxel_metadata"]["fluid_states"][id.to_string()]
                    .as_bool()
                    .unwrap(),
            })
        }
    }

    struct VoxelWorld<'a, 'graph> {
        storage: ChunkCarvingWorld<'a, 'graph>,
        events: Events,
        written: BTreeSet<[i32; 3]>,
        replies: Vec<(Option<CarveState>, bool)>,
        draws: usize,
        schedule: bool,
        top: Option<CarveState>,
        native_surface: bool,
        native_aquifer: bool,
    }

    impl CarvingWorld for VoxelWorld<'_, '_> {
        fn block(&mut self, pos: [i32; 3]) -> u32 {
            self.storage.block(pos)
        }

        fn set_block(&mut self, [x, y, z]: [i32; 3], state: u32) {
            let old = self.storage.block([x, y, z]);
            self.storage.set_block([x, y, z], state);
            self.written.insert([x, y, z]);
            // Native ChunkAccess's two-argument overload passes update flags 3.
            self.events
                .borrow_mut()
                .push(json!(["write", x, y, z, old, state, 3]));
        }

        fn substance(&mut self, [x, y, z]: [i32; 3], density: f64) -> Option<CarveState> {
            let (reply, schedule) = if self.native_aquifer {
                let reply = self.storage.substance([x, y, z], density);
                (reply, self.storage.should_schedule_fluid_update())
            } else {
                self.replies[self.draws % self.replies.len()]
            };
            self.draws += 1;
            self.schedule = schedule;
            self.events.borrow_mut().push(json!([
                "aquifer",
                x,
                y,
                z,
                format!("{:016x}", density.to_bits()),
                reply.map_or(-1, |s| s.id as i32),
                schedule
            ]));
            reply
        }

        fn should_schedule_fluid_update(&mut self) -> bool {
            self.events
                .borrow_mut()
                .push(json!(["schedule", self.schedule]));
            self.schedule
        }

        fn mark_for_postprocessing(&mut self, [x, y, z]: [i32; 3]) {
            self.storage.mark_for_postprocessing([x, y, z]);
            self.events.borrow_mut().push(json!(["post", x, y, z]));
        }

        fn top_material(&mut self, [x, y, z]: [i32; 3], under_fluid: bool) -> Option<CarveState> {
            let top = if self.native_surface {
                self.storage.top_material([x, y, z], under_fluid)
            } else {
                self.top
            };
            self.events.borrow_mut().push(json!([
                "top_material",
                x,
                y,
                z,
                under_fluid,
                top.map_or(-1, |s| s.id as i32)
            ]));
            top
        }
    }

    fn run_voxel_case(sample: &Value, graph: &VanillaGraph) {
        let native_surface = sample["surface"]["mode"] == "native_overworld";
        let native_aquifer = sample["aquifer"]["mode"] == "native_noise_chunk";
        assert!(native_surface || sample["surface"]["mode"] == "scripted_constant");
        assert!(native_aquifer || sample["aquifer"]["mode"] == "scripted_cycle");
        let target = position(&sample["target"]);
        let mut chunk = GeneratedChunk::new(target);
        chunk.states.fill(sample["fill"].as_u64().unwrap() as u32);
        for value in sample["overrides"].as_array().unwrap() {
            let [x, y, z] = point(value);
            chunk.set(x as usize, y, z as usize, value[3].as_u64().unwrap() as u32);
        }
        let seed = sample["surface"]["seed"].as_i64().unwrap_or(0);
        let ctx = density::EvalContext {
            seed,
            ..Default::default()
        }
        .with_noise_bounds(target.x * 16, target.z * 16, 4);
        let mut aquifer = Aquifer::new(seed, graph, ctx);
        let events: Events = Rc::default();
        let mut world = VoxelWorld {
            storage: ChunkCarvingWorld {
                chunk: &mut chunk,
                aquifer: &mut aquifer,
            },
            events: events.clone(),
            written: BTreeSet::new(),
            replies: sample["aquifer"]["replies"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| (fixture_state(int(&r[0])), r[1].as_bool().unwrap()))
                .collect(),
            draws: 0,
            schedule: sample["aquifer"]["initial_schedule"].as_bool().unwrap(),
            top: sample["surface"]["state"]
                .as_i64()
                .and_then(|id| fixture_state(id as i32)),
            native_surface,
            native_aquifer,
        };
        assert_eq!(
            world.block([target.x * 16, MIN_Y - 1, target.z * 16]),
            fixture()["voxel_metadata"]["void_air_state"]
                .as_u64()
                .unwrap() as u32
        );
        assert!(native_aquifer || !world.replies.is_empty());
        let mut mask = VoxelMask {
            mask: premarked_mask(sample),
            additional: sample["additional_mask"]
                .as_array()
                .unwrap()
                .iter()
                .map(point)
                .collect(),
            events: events.clone(),
        };
        assert_eq!(
            json!(mask_indices(&mask.mask)),
            sample["initial_mask"],
            "{} initial mask",
            sample["id"]
        );
        let mut results = Vec::new();
        let mut masks = Vec::new();
        let mut caller_surface = sample["initial_surface"].as_bool().unwrap();
        for (pass, step) in sample["steps"].as_array().unwrap().iter().enumerate() {
            events.borrow_mut().push(json!(["pass", pass]));
            let mut carve = |pos, surface: &mut bool| {
                let result = carve_block(&mut world, pos, surface);
                let [x, y, z] = pos;
                events
                    .borrow_mut()
                    .push(json!(["block_result", x, y, z, result, *surface]));
                result
            };
            let result = match step["kind"].as_str().unwrap() {
                "block" => carve(point(&step["position"]), &mut caller_surface),
                "cave" | "canyon" => {
                    let width = if step["kind"] == "canyon" {
                        Some(canyon_width_factors(&mut JavaRandom::new(
                            step["width_seed"].as_i64().unwrap(),
                        )))
                    } else {
                        None
                    };
                    rasterize_ellipsoid(
                        target,
                        &mut mask,
                        Ellipsoid {
                            center: std::array::from_fn(|i| double(&step["center_bits"][i])),
                            horizontal_radius: double(&step["radius_bits"][0]),
                            vertical_radius: double(&step["radius_bits"][1]),
                            floor: double(&step["floor_bits"]),
                            canyon_width: width.as_deref(),
                        },
                        carve,
                    )
                }
                other => panic!("unknown voxel step {other}"),
            };
            results.push(result);
            events.borrow_mut().push(json!(["return", pass, result]));
            masks.push(mask_indices(&mask.mask));
        }
        assert_eq!(
            json!(results),
            sample["returns"],
            "{} returns",
            sample["id"]
        );
        let actual_events = events.borrow();
        let expected_events = sample["events"].as_array().unwrap();
        for (i, (actual, expected)) in actual_events.iter().zip(expected_events).enumerate() {
            assert_eq!(actual, expected, "{} event {i}", sample["id"]);
        }
        assert_eq!(
            actual_events.len(),
            expected_events.len(),
            "{} event count",
            sample["id"]
        );
        assert_eq!(json!(masks), sample["masks"], "{} masks", sample["id"]);
        assert_eq!(
            world.draws as u64,
            sample["aquifer_calls"].as_u64().unwrap(),
            "{} aquifer draws",
            sample["id"]
        );
        assert_eq!(
            world.schedule,
            sample["final_schedule"].as_bool().unwrap(),
            "{} scheduling flag",
            sample["id"]
        );
        let mut checked = world.written.clone();
        for value in sample["overrides"].as_array().unwrap() {
            let [x, y, z] = point(value);
            checked.insert([target.x * 16 + x, y, target.z * 16 + z]);
        }
        for index in mask_indices(&mask.mask) {
            checked.insert([
                target.x * 16 + (index & 15) as i32,
                MIN_Y + (index >> 8) as i32,
                target.z * 16 + ((index >> 4) & 15) as i32,
            ]);
        }
        let final_states: Vec<_> = checked
            .into_iter()
            .map(|p| json!([p[0], p[1], p[2], world.block(p)]))
            .collect();
        assert_eq!(
            json!(final_states),
            sample["final_states"],
            "{} final states",
            sample["id"]
        );
        let mut postprocessing = world.storage.chunk.postprocessing.clone();
        postprocessing.sort_by_key(|&(_, y, _)| y >> 4);
        let postprocessing: Vec<_> = postprocessing
            .into_iter()
            .map(|(x, y, z)| [target.x * 16 + x as i32, y, target.z * 16 + z as i32])
            .collect();
        assert_eq!(
            json!(postprocessing),
            sample["postprocessing"],
            "{} postprocessing",
            sample["id"]
        );
        if native_surface {
            for expected in sample["surface"]["contexts"].as_array().unwrap() {
                let p = [
                    int(&expected["blockX"]),
                    int(&expected["blockY"]),
                    int(&expected["blockZ"]),
                ];
                let c = carving_surface_context(
                    graph,
                    seed,
                    ctx,
                    p,
                    int(&expected["waterHeight"]) != i32::MIN,
                );
                assert_surface_context(sample, &c, ctx, world.storage.chunk, expected);
            }
        }
    }

    fn assert_surface_context(
        sample: &Value,
        c: &crate::surface_rules::SurfaceContext<'_>,
        ctx: density::EvalContext,
        chunk: &GeneratedChunk,
        expected: &Value,
    ) {
        let id = &sample["id"];
        assert_eq!(
            c.stone_depth_above,
            int(&expected["stoneDepthAbove"]),
            "{id} depth above"
        );
        assert_eq!(
            c.stone_depth_below,
            int(&expected["stoneDepthBelow"]),
            "{id} depth below"
        );
        assert_eq!(
            c.water_height,
            int(&expected["waterHeight"]),
            "{id} water height"
        );
        assert_eq!(
            c.surface_depth,
            int(&expected["surfaceDepth"]),
            "{id} surface depth"
        );
        assert_eq!(
            c.preliminary_surface_level + c.surface_depth - 8,
            int(&expected["min_surface_level"]),
            "{id} minimum surface"
        );
        assert_eq!(
            json!(preliminary_surface_corners(
                VanillaGraph::load().unwrap(),
                &ctx,
                c.x,
                c.z
            )),
            expected["preliminary_corners"],
            "{id} preliminary corners"
        );
        assert_eq!(
            crate::biome::id(expected["biome"].as_str().unwrap()),
            Some(c.biome),
            "{id} biome"
        );
        let (qx, qy, qz) = crate::biome_zoom::BiomeZoom::new(c.seed).quart_at((c.x, c.y, c.z));
        assert_eq!(json!([qx, qy, qz]), expected["quart"], "{id} biome zoom");
        let secondary = c.noise.unwrap().sample(
            "minecraft:surface_secondary",
            c.seed,
            c.x as f64,
            0.0,
            c.z as f64,
        );
        assert_eq!(
            secondary.to_bits(),
            bits(&expected["surface_secondary_bits"]),
            "{id} secondary depth noise"
        );
        assert_eq!(
            c.temperature().to_bits(),
            float(&expected["temperature_bits"]).to_bits(),
            "{id} biome temperature"
        );
        assert_eq!(
            crate::surface_rules::steep(chunk, c.x, c.z),
            expected["steep"].as_bool().unwrap(),
            "{id} steepness"
        );
    }

    fn surface_fixture_chunk(sample: &Value) -> GeneratedChunk {
        let [x, _, z] = point(&sample["position"]);
        let mut chunk = GeneratedChunk::new(ChunkPos::new(x >> 4, z >> 4));
        for x in 0..16 {
            for z in 0..16 {
                let y = match sample["height_profile"].as_str().unwrap() {
                    "flat" => 80,
                    "south_up" => 64 + z as i32 * 2,
                    "north_up" => 94 - z as i32 * 2,
                    "west_up" => 94 - x as i32 * 2,
                    "east_up" => 64 + x as i32 * 2,
                    other => panic!("unknown height profile {other}"),
                };
                chunk.set(x, y, z, block::STONE);
            }
        }
        chunk
    }

    #[test]
    fn evaluated_top_material_and_production_adapter_match_native() {
        let graph = VanillaGraph::load().unwrap();
        let mut results = BTreeMap::new();
        for sample in samples("top_material") {
            let seed = sample["seed"].as_i64().unwrap();
            let p @ [x, _, z] = point(&sample["position"]);
            let ctx = density::EvalContext {
                seed,
                ..Default::default()
            }
            .with_noise_bounds(x & !15, z & !15, 4);
            let wet = sample["under_fluid"].as_bool().unwrap();
            let mut c = carving_surface_context(graph, seed, ctx, p, wet);
            let mut chunk = surface_fixture_chunk(sample);
            let source = sample["source"].as_str().unwrap();
            if source != "overworld" {
                c.biome = crate::biome::id(source).unwrap();
            }
            assert_surface_context(sample, &c, ctx, &chunk, &sample["context"]);
            let actual = graph
                .surface_rule
                .as_ref()
                .unwrap()
                .evaluate_in_chunk(&c, &chunk);
            assert_eq!(
                actual.map_or(-1, |s| s as i32),
                int(&sample["result"]),
                "{} surface rule",
                sample["id"]
            );
            if source == "overworld" {
                // Deliberately stale palette/heights: carvers use the raw generator biome source.
                chunk.noise_biomes = Some(vec![crate::biome::ids::DESERT; 4 * 4 * 96]);
                chunk.heights.fill(-64);
                let mut aquifer = Aquifer::new(seed, graph, ctx);
                let result = ChunkCarvingWorld {
                    chunk: &mut chunk,
                    aquifer: &mut aquifer,
                }
                .top_material(p, wet);
                assert_eq!(
                    result.map(|s| s.id),
                    actual,
                    "{} production adapter",
                    sample["id"]
                );
                assert_eq!(
                    result.is_some_and(|s| s.has_fluid),
                    sample["has_fluid"].as_bool().unwrap(),
                    "{} fluid state",
                    sample["id"]
                );
            }
            *results.entry(actual).or_insert(0) += 1;
        }
        eprintln!("899 native top-material cases, 451 through production adapter; result distribution={results:?}");
    }

    #[test]
    fn evaluated_surface_voxel_operations_match_native() {
        let graph = VanillaGraph::load().unwrap();
        for sample in samples("surface_voxel") {
            run_voxel_case(sample, graph);
        }
        for sample in
            samples("surface_voxel").filter(|s| s["id"].as_str().unwrap().contains("water_witness"))
        {
            let events = sample["events"].as_array().unwrap();
            let top = events
                .iter()
                .find(|e| e[0] == "top_material")
                .expect("native water repair callback");
            assert_eq!(top[5], block::WATER);
            let p = [int(&top[1]), int(&top[2]), int(&top[3])];
            assert!(sample["postprocessing"]
                .as_array()
                .unwrap()
                .contains(&json!(p)));
            let index = Mask::bit(p[0] as usize, p[1], p[2] as usize);
            assert!(!sample["masks"][0]
                .as_array()
                .unwrap()
                .contains(&json!(index)));
        }
    }

    #[test]
    fn premarked_masks_match_native_bit_indices() {
        for sample in samples("voxel") {
            assert_eq!(
                json!(mask_indices(&premarked_mask(sample))),
                sample["initial_mask"],
                "{}",
                sample["id"]
            );
        }
    }

    #[test]
    fn bounded_voxel_operations_match_native() {
        let graph = VanillaGraph::load().unwrap();
        assert_eq!(fixture()["voxel_metadata"]["min_y"], MIN_Y);
        assert_eq!(fixture()["voxel_metadata"]["height"], WORLD_HEIGHT);
        assert_eq!(fixture()["voxel_metadata"]["lava_state"], block::LAVA);
        let mut counts = BTreeMap::new();
        let mut passes = 0;
        let mut successes = 0;
        let mut mask_cells = 0;
        let mut block_passes = 0;
        let mut successful_blocks = 0;
        let mut changed_writes = 0;
        let mut schedule_true = 0;
        let mut surface_flag_true = 0;
        let mut final_state_cells = 0;
        for sample in samples("voxel") {
            run_voxel_case(sample, graph);
            passes += sample["returns"].as_array().unwrap().len();
            successes += sample["returns"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v.as_bool().unwrap())
                .count();
            mask_cells += sample["masks"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_array()
                .unwrap()
                .len();
            block_passes += sample["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["kind"] == "block")
                .count();
            final_state_cells += sample["final_states"].as_array().unwrap().len();
            for event in sample["events"].as_array().unwrap() {
                *counts.entry(event[0].as_str().unwrap()).or_insert(0) += 1;
                if event[0] == "write" && event[4] != event[5] {
                    changed_writes += 1;
                }
                if event[0] == "schedule" && event[1] == true {
                    schedule_true += 1;
                }
                if event[0] == "block_result" {
                    if event[4] == true {
                        successful_blocks += 1;
                    }
                    if event[5] == true {
                        surface_flag_true += 1;
                    }
                }
            }
        }
        eprintln!("66 native voxel cases: {passes} passes, {successes} true returns, {mask_cells} final mask bits; events={counts:?}");
        eprintln!("{} ellipsoid calls, {block_passes} direct block calls, {successful_blocks} successful blocks, {changed_writes} changed writes, {schedule_true} true schedule queries, {surface_flag_true} true surface flags, {final_state_cells} final state checks", passes - block_passes);
    }

    #[test]
    fn native_voxel_evidence_covers_mask_and_fluid_edge_contracts() {
        let case = |id: &str| samples("voxel").find(|sample| sample["id"] == id).unwrap();
        let blocked = case("voxel/mask/barrier_is_not_retried");
        assert_eq!(blocked["returns"], json!([false, false, true]));
        assert_eq!(blocked["aquifer_calls"], 2);
        assert_eq!(blocked["masks"][0], blocked["masks"][1]);

        let repeated = case("voxel/block/same_state_and_premark");
        assert_eq!(repeated["returns"], json!([true, true]));
        assert_eq!(repeated["postprocessing"], json!([[8, 32, 8], [8, 32, 8]]));
        assert_eq!(repeated["masks"][1], repeated["initial_mask"]);
        let writes: Vec<_> = repeated["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] == "write")
            .collect();
        assert_eq!(writes.len(), 2);
        assert!(writes
            .iter()
            .all(|e| e[4] == block::WATER && e[5] == block::WATER && e[6] == 3));

        let carried = case("voxel/block/lava_carried_flag");
        assert_eq!(carried["returns"], json!([true, true, true, true, false]));
        assert_eq!(carried["aquifer_calls"], 1);
        assert_eq!(carried["final_schedule"], false);
        assert_eq!(carried["postprocessing"], json!([[8, -57, 8], [8, -56, 8]]));

        let surface = case("voxel/surface/wet_callback");
        assert_eq!(
            surface["postprocessing"],
            json!([[8, 31, 8], [8, 33, 8], [8, 32, 8]])
        );
        let below_mask = Mask::bit(8, 31, 8) as u64;
        assert!(!surface["masks"][0]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_u64() == Some(below_mask)));
        assert!(surface["final_states"]
            .as_array()
            .unwrap()
            .contains(&json!([8, 31, 8, block::WATER])));

        let flag = case("voxel/block/grass_barrier_flag")["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e[0] == "block_result")
            .unwrap();
        assert_eq!(flag, &json!(["block_result", 8, 34, 8, false, true]));
        for sample in samples("voxel") {
            for event in sample["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[0] == "mask_set")
            {
                assert!((0..16).contains(&int(&event[1])));
                assert!((MIN_Y + 2..=MAX_Y - 7).contains(&int(&event[2])));
                assert!((0..16).contains(&int(&event[3])));
            }
        }
    }
}
