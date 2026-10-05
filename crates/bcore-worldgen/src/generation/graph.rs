//! Fresh-generation and loading dependencies from the pinned 26.1 pyramid.
use std::sync::OnceLock;

use bcore_core::ChunkPos;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ChunkStatus {
    Empty,
    StructureStarts,
    StructureReferences,
    Biomes,
    Noise,
    Surface,
    Carvers,
    Features,
    InitializeLight,
    Light,
    Spawn,
    Full,
}

impl ChunkStatus {
    pub const ALL: [Self; 12] = [
        Self::Empty,
        Self::StructureStarts,
        Self::StructureReferences,
        Self::Biomes,
        Self::Noise,
        Self::Surface,
        Self::Carvers,
        Self::Features,
        Self::InitializeLight,
        Self::Light,
        Self::Spawn,
        Self::Full,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub fn parent(self) -> Self {
        Self::ALL[self.index().saturating_sub(1)]
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Empty => "minecraft:empty",
            Self::StructureStarts => "minecraft:structure_starts",
            Self::StructureReferences => "minecraft:structure_references",
            Self::Biomes => "minecraft:biomes",
            Self::Noise => "minecraft:noise",
            Self::Surface => "minecraft:surface",
            Self::Carvers => "minecraft:carvers",
            Self::Features => "minecraft:features",
            Self::InitializeLight => "minecraft:initialize_light",
            Self::Light => "minecraft:light",
            Self::Spawn => "minecraft:spawn",
            Self::Full => "minecraft:full",
        }
    }

    pub const fn chunk_type(self) -> &'static str {
        if matches!(self, Self::Full) {
            "LEVELCHUNK"
        } else {
            "PROTOCHUNK"
        }
    }

    pub const fn heightmaps_after(self) -> &'static [&'static str] {
        if self.index() < Self::Carvers.index() {
            &["WORLD_SURFACE_WG", "OCEAN_FLOOR_WG"]
        } else {
            &[
                "WORLD_SURFACE",
                "OCEAN_FLOOR",
                "MOTION_BLOCKING",
                "MOTION_BLOCKING_NO_LEAVES",
            ]
        }
    }
}

impl std::fmt::Display for ChunkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Each entry applies at the exact chessboard distance, including ring zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDependencies {
    pub by_radius: Vec<ChunkStatus>,
}

impl ChunkDependencies {
    pub fn radius(&self) -> usize {
        self.by_radius.len().saturating_sub(1)
    }

    pub fn at_radius(&self, radius: usize) -> Option<ChunkStatus> {
        self.by_radius.get(radius).copied()
    }

    /// `None` corresponds to native getRadiusOf's absent-dependency exception.
    pub fn radius_of(&self, status: ChunkStatus) -> Option<usize> {
        self.by_radius
            .iter()
            .rposition(|&required| required >= status)
    }

    pub fn at(&self, source: ChunkPos, destination: ChunkPos) -> Option<ChunkStatus> {
        self.at_radius(chessboard_distance(source, destination) as usize)
    }
}

pub fn chessboard_distance(a: ChunkPos, b: ChunkPos) -> u32 {
    a.x.abs_diff(b.x).max(a.z.abs_diff(b.z))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkStep {
    pub target: ChunkStatus,
    pub direct: ChunkDependencies,
    pub accumulated: ChunkDependencies,
    pub block_state_write_radius: i32,
}

impl ChunkStep {
    pub fn layer_radius(&self, layer: ChunkStatus) -> Option<usize> {
        if layer == self.target {
            Some(0)
        } else {
            self.accumulated.radius_of(layer)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkPyramid {
    Generation,
    Loading,
}

impl ChunkPyramid {
    pub fn step(self, target: ChunkStatus) -> &'static ChunkStep {
        static GENERATION: OnceLock<Vec<ChunkStep>> = OnceLock::new();
        static LOADING: OnceLock<Vec<ChunkStep>> = OnceLock::new();
        let steps = match self {
            Self::Generation => GENERATION.get_or_init(|| Self::Generation.build()),
            Self::Loading => LOADING.get_or_init(|| Self::Loading.build()),
        };
        &steps[target.index()]
    }

    fn build(self) -> Vec<ChunkStep> {
        use ChunkStatus::*;
        let mut steps: Vec<ChunkStep> = Vec::with_capacity(12);
        for target in ChunkStatus::ALL {
            let direct = match (self, target) {
                (_, Empty) => vec![],
                (_, StructureStarts) => vec![Empty],
                (_, Light) => vec![InitializeLight; 2],
                (Self::Loading, _) => vec![target.parent()],
                (_, StructureReferences) => vec![StructureStarts; 9],
                (_, Biomes) => rings(&[StructureReferences], StructureStarts, 8),
                (_, Noise) => rings(&[Biomes, Biomes], StructureStarts, 8),
                (_, Surface) => rings(&[Noise, Biomes], StructureStarts, 8),
                (_, Carvers) => rings(&[Surface], StructureStarts, 8),
                (_, Features) => rings(&[Carvers, Carvers], StructureStarts, 8),
                (_, InitializeLight) => vec![Features],
                (_, Spawn) => vec![Light, Biomes],
                (_, Full) => vec![Spawn],
            };
            // A requirement S at distance r brings S's own accumulated footprint.
            // Taking the maximum status in every covered ring composes the pyramids.
            let mut accumulated = direct.clone();
            for (r, &required) in direct.iter().enumerate() {
                for (dr, &inherited) in steps[required.index()]
                    .accumulated
                    .by_radius
                    .iter()
                    .enumerate()
                {
                    accumulated.resize(accumulated.len().max(r + dr + 1), Empty);
                    for status in &mut accumulated[..=r + dr] {
                        *status = (*status).max(inherited);
                    }
                }
            }
            let block_state_write_radius = match (self, target) {
                (Self::Generation, Noise | Surface | Carvers) => 0,
                (Self::Generation, Features) => 1,
                _ => -1,
            };
            steps.push(ChunkStep {
                target,
                direct: ChunkDependencies { by_radius: direct },
                accumulated: ChunkDependencies {
                    by_radius: accumulated,
                },
                block_state_write_radius,
            });
        }
        steps
    }
}

fn rings(prefix: &[ChunkStatus], outer: ChunkStatus, radius: usize) -> Vec<ChunkStatus> {
    let mut result = prefix.to_vec();
    result.resize(radius + 1, outer);
    result
}

/// Order is X then Z within this task layer only. Separate requests keep their
/// actual execution history; this iterator does not impose a global source order.
pub fn layer_positions(center: ChunkPos, radius: usize) -> impl Iterator<Item = ChunkPos> {
    let r = radius as i32;
    (-r..=r).flat_map(move |dx| (-r..=r).map(move |dz| ChunkPos::new(center.x + dx, center.z + dz)))
}

pub const DECORATION_STEPS: [&str; 11] = [
    "RAW_GENERATION",
    "LAKES",
    "LOCAL_MODIFICATIONS",
    "UNDERGROUND_STRUCTURES",
    "SURFACE_STRUCTURES",
    "STRONGHOLDS",
    "UNDERGROUND_ORES",
    "UNDERGROUND_DECORATION",
    "FLUID_SPRINGS",
    "VEGETAL_DECORATION",
    "TOP_LAYER_MODIFICATION",
];

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
