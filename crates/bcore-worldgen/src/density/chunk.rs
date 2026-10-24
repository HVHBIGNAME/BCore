//! Borrowed scalar plans with job-owned interpolation cells for full NOISE fills.

use super::{
    cache_once, interpolate_corners, squeeze, DensityFunction as D, EvalContext, EvaluationMode,
};
use crate::fast_hash::FastMap;
use crate::feature_world::Pos;
use crate::ore_vein::{OreVeinifier, VeinFunctions};
use crate::{ChunkPos, VanillaGraph, MAX_Y, MIN_Y};

const CELL_COUNT: usize = 4 * 48 * 4;
const MAX_INTERPOLATORS: usize = 16;
const MAX_SCALAR_NODES: usize = 4096;
const MAX_TRAVERSAL_DEPTH: usize = 128;

#[derive(Clone, Copy)]
struct Geometry {
    base_x: i32,
    base_z: i32,
}

#[derive(Clone, Copy)]
struct BlockPoint {
    pos: Pos,
    cell: usize,
    origin: [f64; 3],
    fractions: [f64; 3],
}

impl Geometry {
    fn new(pos: ChunkPos) -> Option<Self> {
        let base_x = pos.x.checked_mul(16)?;
        let base_z = pos.z.checked_mul(16)?;
        base_x.checked_add(16)?;
        base_z.checked_add(16)?;
        Some(Self { base_x, base_z })
    }

    fn point(self, pos: Pos) -> Option<BlockPoint> {
        let (x, y, z) = pos;
        let lx = x.checked_sub(self.base_x)?;
        let lz = z.checked_sub(self.base_z)?;
        if !(0..16).contains(&lx) || !(0..16).contains(&lz) || !(MIN_Y..=MAX_Y).contains(&y) {
            return None;
        }
        let dy = y - MIN_Y;
        let cx = lx >> 2;
        let cy = dy >> 3;
        let cz = lz >> 2;
        Some(BlockPoint {
            pos,
            cell: ((cx * 48 + cy) * 4 + cz) as usize,
            origin: [
                f64::from(self.base_x + cx * 4),
                f64::from(MIN_Y + cy * 8),
                f64::from(self.base_z + cz * 4),
            ],
            fractions: [
                f64::from(lx & 3) / 4.0,
                f64::from(dy & 7) / 8.0,
                f64::from(lz & 3) / 4.0,
            ],
        })
    }

    fn context(self, seed: i64, mode: EvaluationMode) -> EvalContext {
        EvalContext {
            seed,
            mode,
            ..Default::default()
        }
        .with_noise_bounds(self.base_x, self.base_z, 4)
    }
}

#[derive(Clone, Copy)]
enum Unary {
    Abs,
    Squeeze,
}

#[derive(Clone, Copy)]
enum Binary {
    Add,
    Mul,
    Min,
    Max,
}

#[derive(Clone, Copy)]
enum Scalar<'a> {
    Constant(f64),
    Unary(Unary, usize),
    Binary(Binary, usize, usize),
    Range {
        input: usize,
        min: f64,
        max: f64,
        when_in: usize,
        when_out: usize,
    },
    Interpolated(usize),
    Generic(&'a D),
}

struct Compiler<'a> {
    nodes: Vec<Scalar<'a>>,
    inner: Vec<&'a D>,
    seen: FastMap<usize, (usize, usize)>,
}

impl<'a> Compiler<'a> {
    fn node(
        &mut self,
        mut function: &'a D,
        remaining: &mut usize,
        mut depth: usize,
    ) -> Option<(usize, usize)> {
        let mut references = 0;
        while let D::Reference(inner) = function {
            depth = depth.checked_sub(1)?;
            references += 1;
            function = inner;
        }
        let child_depth = depth.checked_sub(1)?;
        let key = function as *const D as usize;
        if let Some(&(index, height)) = self.seen.get(&key) {
            return (height <= depth).then_some((index, height + references));
        }
        if self.nodes.len() >= MAX_SCALAR_NODES {
            return None;
        }
        // Reserve before descending so outstanding ancestors count toward admission.
        let index = self.nodes.len();
        self.nodes.push(Scalar::Generic(function));
        let (node, height) = match function {
            D::Constant(value) => (Scalar::Constant(*value), 1),
            D::Abs(a) | D::Squeeze(a) => {
                let op = if matches!(function, D::Abs(_)) {
                    Unary::Abs
                } else {
                    Unary::Squeeze
                };
                let (a, height) = self.node(a, remaining, child_depth)?;
                (Scalar::Unary(op, a), height + 1)
            }
            D::Add(a, b) | D::Mul(a, b) | D::Min(a, b) | D::Max(a, b) => {
                let op = match function {
                    D::Add(..) => Binary::Add,
                    D::Mul(..) => Binary::Mul,
                    D::Min(..) => Binary::Min,
                    _ => Binary::Max,
                };
                let (a, ah) = self.node(a, remaining, child_depth)?;
                let (b, bh) = self.node(b, remaining, child_depth)?;
                (Scalar::Binary(op, a, b), ah.max(bh) + 1)
            }
            D::Range {
                input,
                min,
                max,
                when_in,
                when_out,
            } => {
                let (input, ih) = self.node(input, remaining, child_depth)?;
                let (when_in, ah) = self.node(when_in, remaining, child_depth)?;
                let (when_out, bh) = self.node(when_out, remaining, child_depth)?;
                (
                    Scalar::Range {
                        input,
                        min: *min,
                        max: *max,
                        when_in,
                        when_out,
                    },
                    ih.max(ah).max(bh) + 1,
                )
            }
            D::Interpolated(inner) => {
                *remaining = remaining.checked_sub(1)?;
                let index = self.inner.len();
                self.inner.push(inner);
                (Scalar::Interpolated(index), 1)
            }
            _ => (Scalar::Generic(function), 1),
        };
        self.nodes[index] = node;
        self.seen.insert(key, (index, height));
        Some((index, height + references))
    }
}

struct CellInterpolator<'a> {
    inner: &'a D,
    corners: Vec<[f64; 8]>,
    valid: [u64; CELL_COUNT / 64],
}

impl CellInterpolator<'_> {
    #[inline(never)]
    fn sample(&mut self, point: &BlockPoint, ctx: &EvalContext) -> f64 {
        let word = point.cell >> 6;
        let bit = 1 << (point.cell & 63);
        if self.valid[word] & bit == 0 {
            let [x, y, z] = point.origin;
            let inner = self.inner;
            let corners = [
                cache_once(inner, x, y, z, ctx),
                cache_once(inner, x + 4.0, y, z, ctx),
                cache_once(inner, x, y + 8.0, z, ctx),
                cache_once(inner, x + 4.0, y + 8.0, z, ctx),
                cache_once(inner, x, y, z + 4.0, ctx),
                cache_once(inner, x + 4.0, y, z + 4.0, ctx),
                cache_once(inner, x, y + 8.0, z + 4.0, ctx),
                cache_once(inner, x + 4.0, y + 8.0, z + 4.0, ctx),
            ];
            self.corners[point.cell] = corners;
            self.valid[word] |= bit;
        }
        interpolate_corners(self.corners[point.cell], point.fractions, ctx.mode)
    }
}

pub(crate) struct RootDensity<'a> {
    original: &'a D,
    ctx: EvalContext,
    geometry: Geometry,
    nodes: Vec<Scalar<'a>>,
    root: usize,
    interpolators: Vec<CellInterpolator<'a>>,
}

impl<'a> RootDensity<'a> {
    fn new(
        original: &'a D,
        ctx: EvalContext,
        geometry: Geometry,
        remaining: &mut usize,
    ) -> Option<Self> {
        let mut compiler = Compiler {
            nodes: Vec::new(),
            inner: Vec::new(),
            seen: FastMap::default(),
        };
        let (root, _) = compiler.node(original, remaining, MAX_TRAVERSAL_DEPTH)?;
        let interpolators = compiler
            .inner
            .into_iter()
            .map(|inner| CellInterpolator {
                inner,
                corners: vec![[0.0; 8]; CELL_COUNT],
                valid: [0; CELL_COUNT / 64],
            })
            .collect();
        Some(Self {
            original,
            ctx,
            geometry,
            nodes: compiler.nodes,
            root,
            interpolators,
        })
    }

    pub(crate) fn sample(&mut self, pos: Pos) -> f64 {
        if !self.interpolators.is_empty() {
            if let Some(point) = self.geometry.point(pos) {
                return self.evaluate(self.root, &point);
            }
        }
        let (x, y, z) = pos;
        self.original
            .evaluate(f64::from(x), f64::from(y), f64::from(z), &self.ctx)
    }

    fn evaluate(&mut self, index: usize, point: &BlockPoint) -> f64 {
        match self.nodes[index] {
            Scalar::Constant(value) => value,
            Scalar::Unary(op, input) => {
                let value = self.evaluate(input, point);
                match op {
                    Unary::Abs => value.abs(),
                    Unary::Squeeze => squeeze(value),
                }
            }
            Scalar::Binary(op, a, b) => {
                let a = self.evaluate(a, point);
                let b = self.evaluate(b, point);
                match op {
                    Binary::Add => a + b,
                    Binary::Mul => a * b,
                    Binary::Min => a.min(b),
                    Binary::Max => a.max(b),
                }
            }
            Scalar::Range {
                input,
                min,
                max,
                when_in,
                when_out,
            } => {
                let value = self.evaluate(input, point);
                let branch = if value >= min && value < max {
                    when_in
                } else {
                    when_out
                };
                self.evaluate(branch, point)
            }
            Scalar::Interpolated(index) => self.interpolators[index].sample(point, &self.ctx),
            Scalar::Generic(function) => {
                let (x, y, z) = point.pos;
                function.evaluate(f64::from(x), f64::from(y), f64::from(z), &self.ctx)
            }
        }
    }
}

pub(crate) struct ChunkVeins<'a> {
    toggle: RootDensity<'a>,
    ridged: RootDensity<'a>,
    gap: RootDensity<'a>,
}

impl ChunkVeins<'_> {
    pub(crate) fn calculate(&mut self, ore: &OreVeinifier, pos: Pos) -> Option<u32> {
        let Self {
            toggle,
            ridged,
            gap,
        } = self;
        ore.calculate(
            pos,
            toggle.sample(pos),
            || ridged.sample(pos),
            || gap.sample(pos),
        )
    }
}

pub(crate) struct ChunkDensity<'a> {
    pub(crate) final_density: RootDensity<'a>,
    pub(crate) veins: Option<ChunkVeins<'a>>,
}

impl<'a> ChunkDensity<'a> {
    /// Called only by the full-NOISE job's default, empty-Blender context.
    pub(crate) fn new(graph: &'a VanillaGraph, seed: i64, pos: ChunkPos) -> Option<Self> {
        let geometry = Geometry::new(pos)?;
        let mut remaining = MAX_INTERPOLATORS;
        let final_density = RootDensity::new(
            &graph.final_density,
            geometry.context(seed, EvaluationMode::NoiseChunk),
            geometry,
            &mut remaining,
        )?;
        let veins = match graph.ore_veins.as_ref() {
            Some(VeinFunctions {
                toggle,
                ridged,
                gap,
            }) => {
                let ctx = geometry.context(seed, EvaluationMode::NoiseChunkMaterial);
                Some(ChunkVeins {
                    toggle: RootDensity::new(toggle, ctx, geometry, &mut remaining)?,
                    ridged: RootDensity::new(ridged, ctx, geometry, &mut remaining)?,
                    gap: RootDensity::new(gap, ctx, geometry, &mut remaining)?,
                })
            }
            None => None,
        };
        Some(Self {
            final_density,
            veins,
        })
    }
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
