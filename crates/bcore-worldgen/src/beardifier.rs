//! Minecraft 26.1 structure-density adaptation at the NOISE material boundary.
//!
//! Starts must be supplied in the StructureManager's reference traversal order.
//! All rigid pieces are summed first, then all junctions; neither list is sorted
//! or deduplicated. Terrain-matching pool pieces contribute junctions only.

use crate::feature_world::FeatureError;
use crate::noise_perlin::clamped_lerp;
use crate::structure::jigsaw::{JigsawStart, Junction};
use crate::structure::template::BoundingBox;
use crate::structure::template_pool::Projection;
use bcore_core::ChunkPos;
use std::sync::OnceLock;

pub const KERNEL_RADIUS: i32 = 12;
const KERNEL_SIZE: usize = 24;
const KERNEL_LENGTH: usize = KERNEL_SIZE * KERNEL_SIZE * KERNEL_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainAdjustment {
    None,
    Bury,
    BeardThin,
    BeardBox,
    Encapsulate,
}

impl TryFrom<&str> for TerrainAdjustment {
    type Error = FeatureError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        match name {
            "none" => Ok(Self::None),
            "bury" => Ok(Self::Bury),
            "beard_thin" => Ok(Self::BeardThin),
            "beard_box" => Ok(Self::BeardBox),
            "encapsulate" => Ok(Self::Encapsulate),
            _ => Err(FeatureError::Unsupported(format!(
                "structure terrain adaptation {name}"
            ))),
        }
    }
}

/// A selected native `Beardifier.Rigid`. Ordinary (non-pool) pieces use delta 0.
/// The box is the original piece box, including jigsaw's expansion reservation,
/// not the structure's inflated reference box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rigid {
    pub bounds: BoundingBox,
    pub terrain_adjustment: TerrainAdjustment,
    pub ground_level_delta: i32,
}

#[derive(Debug, Default, Clone)]
pub struct Beardifier {
    pieces: Vec<Rigid>,
    junctions: Vec<Junction>,
    affected_bounds: Option<BoundingBox>,
}

impl Beardifier {
    /// Select from admitted starts/references without changing their order.
    /// StructureStart's reference box is expanded by 12 by the structure owner;
    /// here native `isCloseToChunk` tests each original piece against that margin.
    pub fn for_chunk<'a>(
        chunk: ChunkPos,
        starts: impl IntoIterator<Item = &'a JigsawStart>,
    ) -> Result<Self, FeatureError> {
        let min_x = chunk.x.wrapping_mul(16).wrapping_sub(KERNEL_RADIUS);
        let min_z = chunk.z.wrapping_mul(16).wrapping_sub(KERNEL_RADIUS);
        let max_x = chunk.x.wrapping_mul(16).wrapping_add(15 + KERNEL_RADIUS);
        let max_z = chunk.z.wrapping_mul(16).wrapping_add(15 + KERNEL_RADIUS);
        let mut pieces = Vec::new();
        let mut junctions = Vec::new();
        for start in starts {
            let adjustment = TerrainAdjustment::try_from(start.terrain_adaptation.as_str())?;
            if adjustment == TerrainAdjustment::None {
                continue;
            }
            for piece in &start.pieces {
                let bb = piece.bounds;
                if bb.max.0 < min_x || bb.min.0 > max_x || bb.max.2 < min_z || bb.min.2 > max_z {
                    continue;
                }
                if piece.projection() == Projection::Rigid {
                    pieces.push(Rigid {
                        bounds: bb,
                        terrain_adjustment: adjustment,
                        ground_level_delta: piece.ground_level_delta,
                    });
                }
                // Both projections contribute junctions. Destination projection
                // and delta_y are retained metadata but are not density inputs.
                junctions.extend(
                    piece
                        .junctions
                        .iter()
                        .filter(|j| {
                            j.source.0 > min_x
                                && j.source.0 < max_x
                                && j.source.2 > min_z
                                && j.source.2 < max_z
                        })
                        .cloned(),
                );
            }
        }
        Ok(Self::from_parts(pieces, junctions))
    }

    /// Construct from already selected, ordered pieces (also supports non-pool
    /// structures). The affected-box guard is native: union original boxes and
    /// junction source points, then inflate by 24, without ground-delta shifts.
    pub fn from_parts(pieces: Vec<Rigid>, junctions: Vec<Junction>) -> Self {
        let affected_bounds = pieces
            .iter()
            .map(|p| p.bounds)
            .chain(
                junctions
                    .iter()
                    .map(|j| BoundingBox::new(j.source, j.source)),
            )
            .reduce(BoundingBox::union)
            .map(|bb| {
                let inflated = bb.inflated(24);
                // BoundingBox's native constructor normalizes overflowed ends.
                BoundingBox::new(inflated.min, inflated.max)
            });
        Self {
            pieces,
            junctions,
            affected_bounds,
        }
    }

    pub fn affected_bounds(&self) -> Option<BoundingBox> {
        self.affected_bounds
    }

    pub fn pieces(&self) -> &[Rigid] {
        &self.pieces
    }

    pub fn junctions(&self) -> &[Junction] {
        &self.junctions
    }

    pub fn compute(&self, x: i32, y: i32, z: i32) -> f64 {
        if !self
            .affected_bounds
            .is_some_and(|bb| bb.contains((x, y, z)))
        {
            return 0.0;
        }
        let mut density = 0.0;
        for piece in &self.pieces {
            let bb = piece.bounds;
            let dx = 0.max(bb.min.0.wrapping_sub(x).max(x.wrapping_sub(bb.max.0)));
            let dz = 0.max(bb.min.2.wrapping_sub(z).max(z.wrapping_sub(bb.max.2)));
            let ground = bb.min.1.wrapping_add(piece.ground_level_delta);
            let ground_y = y.wrapping_sub(ground);
            let dy = match piece.terrain_adjustment {
                TerrainAdjustment::None => 0,
                TerrainAdjustment::Bury | TerrainAdjustment::BeardThin => ground_y,
                TerrainAdjustment::BeardBox => {
                    0.max(ground.wrapping_sub(y).max(y.wrapping_sub(bb.max.1)))
                }
                TerrainAdjustment::Encapsulate => {
                    0.max(bb.min.1.wrapping_sub(y).max(y.wrapping_sub(bb.max.1)))
                }
            };
            density += match piece.terrain_adjustment {
                TerrainAdjustment::None => 0.0,
                TerrainAdjustment::Bury => bury(dx as f64, dy as f64 / 2.0, dz as f64),
                TerrainAdjustment::BeardThin | TerrainAdjustment::BeardBox => {
                    beard(dx, dy, dz, ground_y) * 0.8
                }
                TerrainAdjustment::Encapsulate => {
                    bury(dx as f64 / 2.0, dy as f64 / 2.0, dz as f64 / 2.0) * 0.8
                }
            };
        }
        for junction in &self.junctions {
            let dx = x.wrapping_sub(junction.source.0);
            let dy = y.wrapping_sub(junction.source.1);
            let dz = z.wrapping_sub(junction.source.2);
            density += beard(dx, dy, dz, dy) * 0.4;
        }
        density
    }
}

fn bury(x: f64, y: f64, z: f64) -> f64 {
    clamped_lerp((x * x + y * y + z * z).sqrt() / 6.0, 1.0, 0.0)
}

fn beard(x: i32, y: i32, z: i32, ground_y: i32) -> f64 {
    let ix = x.wrapping_add(KERNEL_RADIUS);
    let iy = y.wrapping_add(KERNEL_RADIUS);
    let iz = z.wrapping_add(KERNEL_RADIUS);
    if !(0..24).contains(&ix) || !(0..24).contains(&iy) || !(0..24).contains(&iz) {
        return 0.0;
    }
    let dy = ground_y as f64 + 0.5;
    let length_squared = (x as f64) * (x as f64) + dy * dy + (z as f64) * (z as f64);
    let factor = -dy * fast_inv_sqrt(length_squared / 2.0) / 2.0;
    factor * f64::from(kernel()[(iz as usize * 24 + ix as usize) * 24 + iy as usize])
}

fn fast_inv_sqrt(value: f64) -> f64 {
    let half = 0.5 * value;
    let bits = 6910469410427058090i64.wrapping_sub((value.to_bits() as i64) >> 1);
    let estimate = f64::from_bits(bits as u64);
    estimate * (1.5 - half * estimate * estimate)
}

fn kernel() -> &'static [f32] {
    static KERNEL: OnceLock<Vec<f32>> = OnceLock::new();
    KERNEL.get_or_init(|| {
        let mut values = Vec::with_capacity(KERNEL_LENGTH);
        for z in -12..12 {
            for x in -12..12 {
                for y in -12..12 {
                    let (x, y, z) = (x as f64, y as f64 + 0.5, z as f64);
                    let length_squared = x * x + y * y + z * z;
                    // Math.pow(E, -lengthSquared / 16), rounded to float once.
                    values.push(std::f64::consts::E.powf(-length_squared / 16.0) as f32);
                }
            }
        }
        values
    })
}

#[cfg(test)]
#[path = "beardifier_tests.rs"]
mod tests;
