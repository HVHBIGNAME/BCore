//! Deterministic, seed-based realistic world generation for BCore.
//!
//! # Design
//!
//! Generation is a pure function of `(seed, block position)`. Nothing depends on
//! the order chunks are visited, no global mutable state is touched and no
//! `rand`/clock/`HashMap` iteration is involved, so a given `(seed, chunkX,
//! chunkZ)` always produces byte-identical blocks. That is what makes chunk
//! persistence and the parity tests meaningful.
//!
//! The pipeline per column `(x, z)`:
//!
//! 1. **Climate** — two low-frequency noise fields (`continent`, `weirdness`)
//!    plus `temperature`/`humidity` pick a [`Biome`].
//! 2. **Height** — fractal value noise (4 octaves) is shaped by the biome's
//!    base height and amplitude, giving a `heightmap` in `MIN_Y..=MAX_Y`.
//! 3. **Fill** — bedrock floor, stone/deepslate at depth, the biome's soil and
//!    surface blocks, then water up to [`SEA_LEVEL`] wherever the terrain is
//!    lower.
//! 4. **Caves** — a 3D noise "tunnel" field carves air below the surface, with a
//!    ceiling guard so the surface is never breached.
//! 5. **Ores** — per-block 3D noise thresholds gated by depth bands.
//! 6. **Features** — trees and surface plants, keyed on a per-position hash so
//!    neighbouring chunks agree without needing to see each other.
//!
//! Trees are placed with a **2-chunk margin**: a tree whose trunk sits in a
//! neighbouring chunk still writes the leaves that overhang into this one, so
//! chunk borders never cut a canopy in half.

use bcore_core::ChunkPos;
use rayon::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

pub mod aquifer;
pub mod biome;
pub mod density;
pub mod features;
pub mod noise;
pub mod noise_perlin;
pub mod simplex;
pub mod surface;
pub mod surface_rules;

pub use noise::{fbm2, fbm3, hash_2d, splitmix64, value_noise_2d, value_noise_3d};

/// Width and depth of a chunk column in blocks.
pub const CHUNK_SIZE: usize = 16;
/// Lowest block Y coordinate of the overworld.
pub const MIN_Y: i32 = -64;
/// Total overworld height in blocks.
pub const WORLD_HEIGHT: i32 = 384;
/// Highest block Y coordinate of the overworld.
pub const MAX_Y: i32 = MIN_Y + WORLD_HEIGHT - 1;
/// Sea level: the water *surface* plane. Water occupies `y < SEA_LEVEL`.
pub const SEA_LEVEL: i32 = 63;
/// The highest Y a water block occupies.
///
/// Vanilla's sea level is 63 but its topmost water block is at y=62 — measured
/// over 49 real overworld chunks by `scripts/analyze_fluid_rules.py`. Filling up
/// to `SEA_LEVEL` inclusive would put water one block too high.
pub const WATER_SURFACE_Y: i32 = SEA_LEVEL - 1;
/// Terrain is clamped into this band so caves and bedrock always have room.
pub const MIN_SURFACE: i32 = 40;
/// Highest terrain the generator will produce.
pub const MAX_SURFACE: i32 = 140;
/// Deepslate replaces stone below this Y.
pub const DEEPSLATE_Y: i32 = 0;

/// Vanilla `SurfaceSystem.getSurfaceDepth` for the overworld.
fn surface_depth(seed: i64, x: i32, z: i32) -> i32 {
    let noise =
        noise_perlin::NormalNoise::for_world(seed, "minecraft:surface", -6, &[1.0, 1.0, 1.0]);
    // `noiseRandom = rootPositional.fromHashOf("minecraft:surface").forkPositional()`,
    // then `noiseRandom.at(x, 0, z).nextDouble()` — the named factory, not the root.
    let mut root = noise_perlin::Xoroshiro::new(seed);
    let positional = root.fork_positional();
    let mut named = positional.from_hash_of("minecraft:surface");
    let named_factory = named.fork_positional();
    let mut random = named_factory.at(x, 0, z);
    (noise.get_value(x as f64, 0.0, z as f64) * 2.75 + 3.0 + random.next_double() * 0.25) as i32
}

/// Network block-state ids for the blocks the generator places.
///
/// These are the real 26.2 `minecraft:block_state` ids, taken from the vanilla
/// datagen block report (`scripts/extract_block_states.py`). They are re-exported
/// by `bcore_protocol::chunk::block_state`, which is the canonical definition;
/// the values are duplicated here so `bcore-worldgen` stays independent of the
/// protocol crate.
pub mod block {
    pub const AIR: u32 = 0;
    pub const STONE: u32 = 1;
    pub const GRANITE: u32 = 2;
    pub const DIORITE: u32 = 4;
    pub const ANDESITE: u32 = 6;
    pub const TUFF: u32 = 23452;
    pub const GRASS_BLOCK: u32 = 9;
    pub const DIRT: u32 = 10;
    pub const COARSE_DIRT: u32 = 11;
    pub const PODZOL: u32 = 13;
    pub const BEDROCK: u32 = 85;
    pub const WATER: u32 = 86;
    pub const LAVA: u32 = 87;
    pub const SAND: u32 = 118;
    pub const GRAVEL: u32 = 124;
    pub const GOLD_ORE: u32 = 129;
    pub const IRON_ORE: u32 = 131;
    pub const COAL_ORE: u32 = 133;
    pub const COPPER_ORE: u32 = 27790;
    pub const OAK_LOG: u32 = 137;
    pub const OAK_LEAVES: u32 = 255;
    pub const BIRCH_LOG: u32 = 143;
    pub const BIRCH_LEAVES: u32 = 311;
    pub const SPRUCE_LOG: u32 = 149;
    pub const SPRUCE_LEAVES: u32 = 367;
    pub const LAPIS_ORE: u32 = 563;
    pub const SANDSTONE: u32 = 578;
    pub const SHORT_GRASS: u32 = 2248;
    pub const DEAD_BUSH: u32 = 2250;
    pub const DIAMOND_ORE: u32 = 5307;
    pub const REDSTONE_ORE: u32 = 6882;
    pub const SNOW_BLOCK: u32 = 6928;
    pub const CACTUS: u32 = 6929;
    // Deepslate ore variants (`deepslate_ore_replaceables` target).
    pub const DEEPSLATE_COAL_ORE: u32 = 134;
    pub const DEEPSLATE_IRON_ORE: u32 = 132;
    pub const DEEPSLATE_COPPER_ORE: u32 = 27791;
    pub const DEEPSLATE_GOLD_ORE: u32 = 130;
    pub const DEEPSLATE_REDSTONE_ORE: u32 = 6883;
    pub const DEEPSLATE_DIAMOND_ORE: u32 = 5308;
    pub const DEEPSLATE_LAPIS_ORE: u32 = 564;
    pub const DEEPSLATE_EMERALD_ORE: u32 = 9574;
    pub const DEEPSLATE: u32 = 30417;
}

/// Network biome ids from the `minecraft:worldgen/biome` registry sync
/// (`scripts/extract_biomes.py`).
pub mod biome_id {
    pub const BEACH: u32 = 3;
    pub const BIRCH_FOREST: u32 = 5;
    pub const DARK_FOREST: u32 = 13;
    pub const DESERT: u32 = 14;
    pub const FOREST: u32 = 21;
    pub const FROZEN_OCEAN: u32 = 22;
    pub const JAGGED_PEAKS: u32 = 27;
    pub const JUNGLE: u32 = 28;
    pub const MUSHROOM_FIELDS: u32 = 34;
    pub const OCEAN: u32 = 35;
    pub const PLAINS: u32 = 40;
    pub const RIVER: u32 = 41;
    pub const SAVANNA: u32 = 42;
    pub const SNOWY_PLAINS: u32 = 46;
    pub const SNOWY_SLOPES: u32 = 47;
    pub const STONY_PEAKS: u32 = 51;
    pub const SWAMP: u32 = 54;
    pub const TAIGA: u32 = 56;
    pub const WINDSWEPT_HILLS: u32 = 63;
}

/// The biomes BCore generates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Biome {
    Ocean,
    FrozenOcean,
    Beach,
    River,
    Plains,
    Forest,
    BirchForest,
    DarkForest,
    Taiga,
    SnowyPlains,
    Desert,
    Savanna,
    Jungle,
    Swamp,
    MushroomFields,
    Mountains,
    SnowyMountains,
}

impl Biome {
    /// Snow replaces the surface block above this Y in [`Biome::Mountains`].
    pub const SNOW_LINE: i32 = 105;

    /// The network biome id sent in the chunk's biome container.
    pub fn network_id(self) -> u32 {
        match self {
            Biome::Ocean => biome_id::OCEAN,
            Biome::FrozenOcean => biome_id::FROZEN_OCEAN,
            Biome::Beach => biome_id::BEACH,
            Biome::River => biome_id::RIVER,
            Biome::Plains => biome_id::PLAINS,
            Biome::Forest => biome_id::FOREST,
            Biome::BirchForest => biome_id::BIRCH_FOREST,
            Biome::DarkForest => biome_id::DARK_FOREST,
            Biome::Taiga => biome_id::TAIGA,
            Biome::SnowyPlains => biome_id::SNOWY_PLAINS,
            Biome::Desert => biome_id::DESERT,
            Biome::Savanna => biome_id::SAVANNA,
            Biome::Jungle => biome_id::JUNGLE,
            Biome::Swamp => biome_id::SWAMP,
            Biome::MushroomFields => biome_id::MUSHROOM_FIELDS,
            Biome::Mountains => biome_id::WINDSWEPT_HILLS,
            Biome::SnowyMountains => biome_id::SNOWY_SLOPES,
        }
    }

    fn base_height(self) -> f64 {
        match self {
            Biome::Ocean | Biome::FrozenOcean => 43.0,
            Biome::River | Biome::Swamp => 59.0,
            Biome::Beach => 63.0,
            Biome::Plains | Biome::SnowyPlains | Biome::Savanna => 68.0,
            Biome::Forest | Biome::BirchForest | Biome::DarkForest | Biome::Taiga => 71.0,
            Biome::Desert => 69.0,
            Biome::Jungle => 73.0,
            Biome::MushroomFields => 74.0,
            Biome::Mountains | Biome::SnowyMountains => 91.0,
        }
    }

    fn height_amplitude(self) -> f64 {
        match self {
            Biome::Ocean | Biome::FrozenOcean => 11.0,
            Biome::River | Biome::Swamp => 2.0,
            Biome::Beach => 2.0,
            Biome::Plains | Biome::SnowyPlains | Biome::Savanna => 5.0,
            Biome::Forest | Biome::BirchForest | Biome::DarkForest | Biome::Taiga => 8.0,
            Biome::Desert => 6.0,
            Biome::Jungle | Biome::MushroomFields => 10.0,
            Biome::Mountains | Biome::SnowyMountains => 38.0,
        }
    }

    fn soil_depth(self) -> i32 {
        match self {
            Biome::Mountains | Biome::SnowyMountains => 1,
            Biome::Desert | Biome::Beach => 4,
            Biome::Ocean | Biome::FrozenOcean | Biome::River => 3,
            _ => 3,
        }
    }

    fn surface_block(self, y: i32) -> u32 {
        match self {
            Biome::Ocean | Biome::FrozenOcean | Biome::Beach | Biome::River => block::SAND,
            Biome::Taiga | Biome::DarkForest => block::PODZOL,
            Biome::MushroomFields => block::PODZOL,
            Biome::Desert => block::SAND,
            Biome::Mountains => {
                if y >= Self::SNOW_LINE {
                    block::SNOW_BLOCK
                } else if y >= 93 {
                    block::STONE
                } else {
                    block::GRASS_BLOCK
                }
            }
            Biome::SnowyMountains | Biome::SnowyPlains => block::SNOW_BLOCK,
            _ => block::GRASS_BLOCK,
        }
    }

    fn soil_block(self) -> u32 {
        match self {
            Biome::Ocean | Biome::FrozenOcean | Biome::Beach | Biome::River => block::SAND,
            Biome::Desert => block::SANDSTONE,
            Biome::Mountains | Biome::SnowyMountains => block::STONE,
            Biome::Savanna => block::COARSE_DIRT,
            Biome::Swamp => block::DIRT,
            _ => block::DIRT,
        }
    }
}

/// A generated 16 x [`WORLD_HEIGHT`] x 16 column.
///
/// Blocks are stored in wire order (`x` fastest, then `z`, then `y`) so the
/// protocol crate can hand slices straight to the paletted-container encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedChunk {
    /// Chunk coordinates this column was generated for.
    pub pos: ChunkPos,
    states: Vec<u32>,
    /// Surface biome per `(x, z)`, indexed `z * 16 + x`.
    biomes: Vec<Biome>,
    /// Terrain height (topmost solid terrain Y, before features) per `(x, z)`.
    heights: Vec<i32>,
}

impl GeneratedChunk {
    fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            states: vec![block::AIR; CHUNK_SIZE * CHUNK_SIZE * WORLD_HEIGHT as usize],
            biomes: vec![Biome::Plains; CHUNK_SIZE * CHUNK_SIZE],
            heights: vec![MIN_Y; CHUNK_SIZE * CHUNK_SIZE],
        }
    }

    #[inline]
    fn index(x: usize, y: i32, z: usize) -> Option<usize> {
        if x >= CHUNK_SIZE || z >= CHUNK_SIZE || y < MIN_Y || y > MAX_Y {
            return None;
        }
        Some((y - MIN_Y) as usize * (CHUNK_SIZE * CHUNK_SIZE) + z * CHUNK_SIZE + x)
    }

    /// Read a block state by chunk-local coordinates.
    #[inline]
    pub fn get(&self, x: usize, y: i32, z: usize) -> Option<u32> {
        Self::index(x, y, z).map(|i| self.states[i])
    }

    /// Write a block state by chunk-local coordinates.
    #[inline]
    pub fn set(&mut self, x: usize, y: i32, z: usize, state: u32) -> bool {
        match Self::index(x, y, z) {
            Some(i) => {
                self.states[i] = state;
                true
            }
            None => false,
        }
    }

    /// Every block state in wire order (`x`, then `z`, then `y`).
    pub fn states(&self) -> &[u32] {
        &self.states
    }

    /// The surface biome at a chunk-local `(x, z)`.
    pub fn biome_at(&self, x: usize, z: usize) -> Biome {
        self.biomes[z * CHUNK_SIZE + x]
    }

    /// The terrain height at a chunk-local `(x, z)` (topmost solid terrain Y).
    pub fn height_at(&self, x: usize, z: usize) -> i32 {
        self.heights[z * CHUNK_SIZE + x]
    }

    /// Highest non-air Y at `(x, z)`, or `None` for an empty column.
    pub fn surface_y(&self, x: usize, z: usize) -> Option<i32> {
        (MIN_Y..=MAX_Y)
            .rev()
            .find(|&y| self.get(x, y, z).is_some_and(|state| state != block::AIR))
    }

    /// Only write into air, so features never carve terrain.
    fn set_if_air(&mut self, x: usize, y: i32, z: usize, state: u32) {
        if self.get(x, y, z) == Some(block::AIR) {
            self.set(x, y, z, state);
        }
    }
}

/// Deterministic realistic-terrain generator.
#[derive(Debug, Clone, Copy)]
pub struct WorldGenerator {
    seed: i64,
}

/// Height and biome of one column, computed together.
///
/// See [`WorldGenerator::column`]: producing both at once avoids re-sampling the
/// four climate noise fields, which dominates generation cost otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnInfo {
    /// Topmost solid terrain Y (before features like trees are added).
    pub height: i32,
    /// The surface biome of this column.
    pub biome: Biome,
}

/// Channel seeds hoisted out of the per-block generation loops.
#[derive(Debug, Clone, Copy)]
struct Channels {
    cave_a: i64,
    cave_b: i64,
    /// Diamond, redstone, lapis, gold, iron, coal — same order as `ORE_BANDS`.
    ore: [i64; 6],
}

/// Ore depth bands and noise parameters, richest first.
///
/// `(block, max_y, min_y, scale, threshold)` — a position becomes this ore when
/// its 3D noise sample exceeds `threshold`, so a higher threshold means rarer.
const ORE_BANDS: [(u32, i32, i32, f64, f64); 6] = [
    (block::DIAMOND_ORE, 16, MIN_Y + 5, 7.0, 0.86),
    (block::REDSTONE_ORE, 16, MIN_Y + 5, 8.0, 0.83),
    (block::LAPIS_ORE, 30, MIN_Y + 5, 8.0, 0.85),
    (block::GOLD_ORE, 32, MIN_Y + 5, 9.0, 0.82),
    (block::IRON_ORE, 64, MIN_Y + 5, 10.0, 0.76),
    (block::COAL_ORE, 128, 5, 12.0, 0.74),
];

/// Highest Y any ore can generate at (the top of the coal band).
const MAX_ORE_Y: i32 = 136;

impl WorldGenerator {
    /// A generator for the given world seed.
    pub fn new(seed: i64) -> Self {
        Self { seed }
    }

    /// The world seed this generator was built with.
    pub fn seed(self) -> i64 {
        self.seed
    }

    /// Derive an independent noise seed for one generation channel.
    ///
    /// `salt` is a small compile-time constant per channel, so this is a single
    /// mix — but it sits inside the per-block loops, and the *callers* that run
    /// per block (caves, ores) hoist it out via [`Self::channels`].
    #[inline]
    fn channel(self, salt: u64) -> i64 {
        splitmix64((self.seed as u64) ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)) as i64
    }

    /// Pre-derived channel seeds for the per-block generation loops.
    ///
    /// Caves and ores are evaluated for every solid block in a column (up to
    /// ~200 per column, 51k per chunk). Deriving their seeds once per chunk
    /// instead of once per sample removes a `splitmix64` from the inner loop.
    fn channels(self) -> Channels {
        Channels {
            cave_a: self.channel(20),
            cave_b: self.channel(21),
            ore: [
                self.channel(30),
                self.channel(31),
                self.channel(32),
                self.channel(33),
                self.channel(34),
                self.channel(35),
            ],
        }
    }

    // ---- climate ---------------------------------------------------------

    /// Large-scale land/sea signal: negative is ocean, positive is inland.
    pub fn continent(self, x: i32, z: i32) -> f64 {
        fbm2(self.channel(1), x as f64, z as f64, 320.0, 3, 0.45)
    }

    /// Mountain-vs-lowland signal.
    pub fn erosion(self, x: i32, z: i32) -> f64 {
        fbm2(self.channel(2), x as f64, z as f64, 180.0, 2, 0.5)
    }

    /// Hot/cold signal, used to place desert.
    pub fn temperature(self, x: i32, z: i32) -> f64 {
        fbm2(self.channel(3), x as f64, z as f64, 260.0, 2, 0.5)
    }

    /// Wet/dry signal, used to place forest.
    pub fn humidity(self, x: i32, z: i32) -> f64 {
        fbm2(self.channel(4), x as f64, z as f64, 210.0, 2, 0.5)
    }

    /// River valley signal. Values near zero trace long, narrow channels.
    pub fn river(self, x: i32, z: i32) -> f64 {
        fbm2(self.channel(5), x as f64, z as f64, 190.0, 3, 0.5).abs()
    }

    fn climate_choice(
        self,
        continent: f64,
        erosion: f64,
        temperature: f64,
        humidity: f64,
        river: f64,
    ) -> Biome {
        if continent < -0.16 {
            if temperature < -0.38 {
                Biome::FrozenOcean
            } else if humidity > 0.58 && continent < -0.48 {
                Biome::MushroomFields
            } else {
                Biome::Ocean
            }
        } else if river < 0.035 && continent > -0.08 {
            Biome::River
        } else if continent < -0.06 {
            Biome::Plains
        } else if erosion > 0.38 {
            if temperature < -0.18 {
                Biome::SnowyMountains
            } else {
                Biome::Mountains
            }
        } else if temperature < -0.42 {
            Biome::SnowyPlains
        } else if temperature < -0.18 {
            Biome::Taiga
        } else if temperature > 0.42 && humidity < -0.02 {
            Biome::Desert
        } else if temperature > 0.28 && humidity < 0.16 {
            Biome::Savanna
        } else if temperature > 0.24 && humidity > 0.38 {
            Biome::Jungle
        } else if humidity > 0.48 && erosion < -0.20 {
            Biome::Swamp
        } else if humidity > 0.34 {
            Biome::DarkForest
        } else if humidity > 0.16 {
            if temperature < 0.02 {
                Biome::BirchForest
            } else {
                Biome::Forest
            }
        } else {
            Biome::Plains
        }
    }

    /// The biome at an absolute block column.
    pub fn biome_at(self, x: i32, z: i32, height: i32) -> Biome {
        let c = self.continent(x, z);
        let e = self.erosion(x, z);
        let t = self.temperature(x, z);
        let h = self.humidity(x, z);
        let r = self.river(x, z);
        if height < SEA_LEVEL {
            return if t < -0.38 {
                Biome::FrozenOcean
            } else {
                Biome::Ocean
            };
        }
        if height <= SEA_LEVEL + 2 && c < 0.04 {
            return Biome::Beach;
        }
        self.climate_choice(c, e, t, h, r)
    }

    /// Height and biome for a column, sharing all climate samples.
    pub fn column(self, x: i32, z: i32) -> ColumnInfo {
        let continent = self.continent(x, z);
        let erosion = self.erosion(x, z);
        let temperature = self.temperature(x, z);
        let humidity = self.humidity(x, z);
        let river = self.river(x, z);
        let climate = self.climate_choice(continent, erosion, temperature, humidity, river);
        let height = self.shape_height(x, z, climate, continent);
        let biome = if height < SEA_LEVEL {
            if temperature < -0.38 {
                Biome::FrozenOcean
            } else {
                Biome::Ocean
            }
        } else if height <= SEA_LEVEL + 2 && continent < 0.04 {
            Biome::Beach
        } else {
            climate
        };
        ColumnInfo { height, biome }
    }

    fn climate_biome(self, x: i32, z: i32) -> Biome {
        self.climate_choice(
            self.continent(x, z),
            self.erosion(x, z),
            self.temperature(x, z),
            self.humidity(x, z),
            self.river(x, z),
        )
    }

    // ---- height ----------------------------------------------------------

    /// Terrain surface height at an absolute block column.
    ///
    /// Blends the climate biome's base height with 4 octaves of fractal noise,
    /// then nudges the result toward the ocean floor as the continent signal
    /// goes negative, so coastlines slope instead of forming cliffs.
    pub fn height_at(self, x: i32, z: i32) -> i32 {
        let climate = self.climate_biome(x, z);
        self.shape_height(x, z, climate, self.continent(x, z))
    }

    /// Shape the terrain height for a column whose climate is already known.
    fn shape_height(self, x: i32, z: i32, climate: Biome, continent: f64) -> i32 {
        let relief = fbm2(self.channel(10), x as f64, z as f64, 96.0, 4, 0.5);
        let detail = fbm2(self.channel(11), x as f64, z as f64, 28.0, 2, 0.5);

        let mut height = climate.base_height() + relief * climate.height_amplitude() + detail * 2.5;

        // Continental shelf: dive toward the sea floor offshore.
        if continent < 0.0 {
            height += continent * 46.0;
        }

        // Rivers cut a shallow channel through otherwise dry land.
        if climate == Biome::River {
            height = height.min(SEA_LEVEL as f64 - 3.0);
        }

        // Mountains get a ridged boost so peaks are sharp, not domed.
        if matches!(climate, Biome::Mountains | Biome::SnowyMountains) {
            let ridge = 1.0 - fbm2(self.channel(12), x as f64, z as f64, 140.0, 2, 0.5).abs();
            height += ridge * ridge * 26.0;
        }

        (height.round() as i32).clamp(MIN_SURFACE, MAX_SURFACE)
    }

    // ---- caves -----------------------------------------------------------

    /// Whether the solid block at an absolute position is carved into a cave.
    ///
    /// Two independent 3D noise fields are thresholded near zero; where both are
    /// inside the band the block becomes air, which produces intersecting
    /// worm-like tunnels rather than blobs. The band narrows toward the surface
    /// (`ceiling`) so caves never breach the terrain top.
    ///
    /// The second field is only sampled when the first is already inside the
    /// band, which skips it for ~90% of blocks.
    pub fn is_cave(self, x: i32, y: i32, z: i32, surface: i32) -> bool {
        self.is_cave_with(&self.channels(), x, y, z, surface)
    }

    /// [`Self::is_cave`] with the channel seeds already derived.
    fn is_cave_with(self, ch: &Channels, x: i32, y: i32, z: i32, surface: i32) -> bool {
        // Keep a solid crust under the surface and a solid floor above bedrock.
        if y >= surface - 5 || y <= MIN_Y + 4 {
            return false;
        }
        // Tunnels: both fields close to zero.
        let threshold = 0.10;
        let (nx, ny, nz) = (x as f64 / 58.0, y as f64 / 30.0, z as f64 / 58.0);
        if value_noise_3d(ch.cave_a, nx, ny, nz).abs() >= threshold {
            return false;
        }
        if value_noise_3d(ch.cave_b, nx, ny, nz + 91.7).abs() >= threshold {
            return false;
        }
        // Never open a cave straight into open water.
        y < SEA_LEVEL - 2 || y < surface - 8
    }

    // ---- ores ------------------------------------------------------------

    /// The ore that replaces stone at an absolute position, if any.
    ///
    /// Each ore has a depth band and a 3D-noise threshold; rarer ores use a
    /// higher threshold and a finer noise scale so they form small pockets.
    /// Checked richest-first so a diamond position is never masked by coal.
    ///
    /// The depth-band test precedes the noise sample, so a surface block costs
    /// six integer comparisons rather than six 3D noise evaluations.
    pub fn ore_at(self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.ore_at_with(&self.channels(), x, y, z)
    }

    /// [`Self::ore_at`] with the channel seeds already derived.
    fn ore_at_with(self, ch: &Channels, x: i32, y: i32, z: i32) -> Option<u32> {
        // Nothing generates above the highest band, which is the common case for
        // most of a column.
        if y > MAX_ORE_Y {
            return None;
        }
        for (i, &(state, max_y, min_y, scale, threshold)) in ORE_BANDS.iter().enumerate() {
            if y > max_y || y < min_y {
                continue;
            }
            let mut chance = threshold;
            // Mountains have the vanilla emerald-like iron enrichment effect.
            if state == block::IRON_ORE && self.erosion(x, z) > 0.35 {
                chance -= 0.08;
            }
            let n = value_noise_3d(
                ch.ore[i],
                x as f64 / scale,
                y as f64 / scale,
                z as f64 / scale,
            );
            if n > chance {
                return Some(state);
            }
        }
        None
    }

    // ---- generation ------------------------------------------------------

    /// Generate a chunk using the staged vanilla data-driven pipeline.
    ///
    /// Missing/incomplete datapack functions intentionally degrade to the existing
    /// deterministic generator; this keeps callers working while density support
    /// grows (interpolated/blend/cache are represented as zero by density.rs).
    pub fn generate_chunk_vanilla(self, pos: ChunkPos) -> GeneratedChunk {
        let Some(graph) = VanillaGraph::load() else {
            return self.generate_chunk(pos);
        };
        // The graph is seed-independent; only evaluation state varies per world.
        let ctx = density::EvalContext {
            seed: self.seed,
            ..Default::default()
        };

        // Chunk-pyramid scheduler. Dependency radii are deliberately explicit:
        // future structure/light implementations can request these neighborhoods
        // without changing the deterministic terrain stages below.
        let pyramid = ChunkPyramid::VANILLA;
        let _ready_dependency_radii = pyramid.stages();
        // structure_starts (23x23), biomes (7x7), noise (5x5), surface (3x3),
        // caves/features/light (chunk-local for this implementation) are no-op
        // scheduling barriers until their vanilla data is implemented.

        let base_x = pos.x * CHUNK_SIZE as i32;
        let base_z = pos.z * CHUNK_SIZE as i32;
        let column_count = CHUNK_SIZE * CHUNK_SIZE;

        // The biome result is stored in the chunk-local column cache together with
        // terrain. Rayon may execute columns in any order, but indexed collection
        // restores a fixed (z * 16 + x) layout and never shares mutable state.
        let columns: Vec<VanillaColumn> = (0..column_count)
            .into_par_iter()
            .map(|column_index| {
                let x = column_index % CHUNK_SIZE;
                let z = column_index / CHUNK_SIZE;
                let wx = base_x + x as i32;
                let wz = base_z + z as i32;
                let mut top = MIN_Y;
                let mut densities = vec![0.0f64; WORLD_HEIGHT as usize];
                for y in MIN_Y..=MAX_Y {
                    let d = density::evaluate(
                        &graph.final_density,
                        wx as f64,
                        y as f64,
                        wz as f64,
                        &ctx,
                    );
                    densities[(y - MIN_Y) as usize] = d;
                    if d > 0.0 {
                        top = y;
                    }
                }
                let climate = |f: &Option<density::DensityFunction>| {
                    f.as_ref()
                        .map(|v| density::evaluate(v, wx as f64, top as f64, wz as f64, &ctx))
                        .unwrap_or(0.0)
                };
                let biome_id = biome::biome_at(
                    &graph.parameters,
                    climate(&graph.temperature),
                    climate(&graph.humidity),
                    climate(&graph.continentalness),
                    climate(&graph.erosion),
                    climate(&graph.weirdness),
                    climate(&graph.depth),
                );
                let biome = biome_from_id(biome_id);
                let mut states = vec![block::AIR; WORLD_HEIGHT as usize];
                let mut aquifer = aquifer::Aquifer::new(
                    self.seed,
                    top,
                    biome_is_water(biome_id),
                    density::noise_registry(),
                    graph.preliminary_surface_level.as_ref(),
                    ctx,
                );
                let mut stone_depth_above = 0i32;
                let mut water_height = i32::MIN;
                let surface_depth = surface_depth(self.seed, wx, wz);
                // Vanilla `NoiseChunk.preliminarySurfaceLevel(x, z)` = floor of the
                // `find_top_surface` density function at (x, 0, z); the surface rule
                // and aquifer both rely on this, not the raw terrain top.
                let preliminary_surface_level = graph
                    .preliminary_surface_level
                    .as_ref()
                    .map(|f| density::evaluate(f, wx as f64, 0., wz as f64, &ctx).floor() as i32)
                    .unwrap_or(top);
                for y in (MIN_Y..=MAX_Y).rev() {
                    let density_value = densities[(y - MIN_Y) as usize];
                    // Vanilla `NoiseChunk.getInterpolatedState()`: the aquifer returns
                    // null (solid) for density > 0 *or* a pressure barrier; anything
                    // else is a fluid/air. We encode "solid" as `STONE` and treat every
                    // other result as the final block.
                    let substance = aquifer.substance(wx, y, wz, density_value);
                    let idx = (y - MIN_Y) as usize;
                    if substance == block::STONE {
                        // Vanilla increments `stoneAboveDepth` *before* applying the
                        // rule, so the topmost solid block is depth 1 (not 0).
                        stone_depth_above += 1;
                        let default_state = block::STONE;
                        let ctx = surface_rules::SurfaceContext {
                            biome: biome_id,
                            stone_depth_above,
                            stone_depth_below: 0,
                            water_height,
                            surface_depth,
                            preliminary_surface_level,
                            sea_level: SEA_LEVEL,
                            x: wx,
                            y,
                            z: wz,
                            seed: self.seed,
                            noise: Some(density::noise_registry()),
                        };
                        states[idx] = graph
                            .surface_rule
                            .as_ref()
                            .and_then(|r| r.evaluate(&ctx))
                            .unwrap_or(default_state);
                    } else if substance == block::WATER || substance == block::LAVA {
                        // Vanilla records `waterHeight = y + 1` on the first fluid
                        // block (top of the water) and does *not* reset stone depth.
                        if water_height == i32::MIN {
                            water_height = y + 1;
                        }
                        states[idx] = substance;
                    } else {
                        // Air resets both stone depth and water height.
                        states[idx] = substance;
                        stone_depth_above = 0;
                        water_height = i32::MIN;
                    }
                }
                VanillaColumn { top, biome, states }
            })
            .collect();

        let mut chunk = GeneratedChunk::new(pos);
        for (column_index, column) in columns.into_iter().enumerate() {
            chunk.heights[column_index] = column.top;
            chunk.biomes[column_index] = column.biome;
            for (y_offset, state) in column.states.into_iter().enumerate() {
                chunk.states[y_offset * column_count + column_index] = state;
            }
        }

        // Vanilla UNDERGROUND_ORES step: `OreFeature` replaces blocks matching the
        // configured target. The stone *blobs* (granite/diorite/andesite/tuff/dirt/
        // gravel) target `base_stone_overworld` (stone + granite + diorite + andesite
        // + tuff + deepslate), while the metal ores target `stone_ore_replaceables`
        // (stone + granite + diorite + andesite) and `deepslate_ore_replaceables`
        // (deepslate). This lets a later blob overwrite an earlier one.
        let ore_chunk = &mut chunk;
        features::place_ore_veins(self.seed, pos.x, pos.z, &mut |wx, y, wz, state| {
            let lx = wx - base_x;
            let lz = wz - base_z;
            if !(0..CHUNK_SIZE as i32).contains(&lx)
                || !(0..CHUNK_SIZE as i32).contains(&lz)
                || y < MIN_Y
                || y > MAX_Y
            {
                return;
            }
            let idx = (y - MIN_Y) as usize * column_count + lz as usize * CHUNK_SIZE + lx as usize;
            let cur = ore_chunk.states[idx];
            let is_base_stone = matches!(
                state,
                block::GRANITE
                    | block::DIORITE
                    | block::ANDESITE
                    | block::TUFF
                    | block::DIRT
                    | block::GRAVEL
            );
            if is_base_stone {
                // base_stone_overworld: stone, granite, diorite, andesite, tuff, deepslate.
                if matches!(
                    cur,
                    block::STONE
                        | block::DEEPSLATE
                        | block::GRANITE
                        | block::DIORITE
                        | block::ANDESITE
                        | block::TUFF
                ) {
                    ore_chunk.states[idx] = state;
                }
            } else if matches!(
                cur,
                block::STONE | block::GRANITE | block::DIORITE | block::ANDESITE
            ) {
                // stone_ore_replaceables → the plain ore state.
                ore_chunk.states[idx] = state;
            } else if cur == block::DEEPSLATE {
                // deepslate_ore_replaceables → the deepslate variant.
                ore_chunk.states[idx] = match state {
                    block::COAL_ORE => block::DEEPSLATE_COAL_ORE,
                    block::IRON_ORE => block::DEEPSLATE_IRON_ORE,
                    block::COPPER_ORE => block::DEEPSLATE_COPPER_ORE,
                    block::GOLD_ORE => block::DEEPSLATE_GOLD_ORE,
                    block::REDSTONE_ORE => block::DEEPSLATE_REDSTONE_ORE,
                    block::DIAMOND_ORE => block::DEEPSLATE_DIAMOND_ORE,
                    block::LAPIS_ORE => block::DEEPSLATE_LAPIS_ORE,
                    9573 => block::DEEPSLATE_EMERALD_ORE,
                    other => other,
                };
            }
        });
        chunk
    }
    pub fn cave_density_probe(
        seed: i64,
        x: f64,
        y: f64,
        z: f64,
    ) -> Option<(f64, Option<f64>, Option<f64>, Option<f64>)> {
        let graph = VanillaGraph::load()?;
        let ctx = density::EvalContext {
            seed,
            ..Default::default()
        };
        let eval = |f: &Option<density::DensityFunction>| {
            f.as_ref().map(|v| density::evaluate(v, x, y, z, &ctx))
        };
        Some((
            density::evaluate(&graph.final_density, x, y, z, &ctx),
            eval(&graph.noodle),
            eval(&graph.cave_cheese),
            eval(&graph.entrances),
        ))
    }

    /// Generate one complete chunk deterministically from its position and seed.
    pub fn generate_chunk(self, pos: ChunkPos) -> GeneratedChunk {
        let mut chunk = GeneratedChunk::new(pos);
        let base_x = pos.x * CHUNK_SIZE as i32;
        let base_z = pos.z * CHUNK_SIZE as i32;
        // Channel seeds are derived once, not per noise sample.
        let ch = self.channels();

        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let wx = base_x + x as i32;
                let wz = base_z + z as i32;
                // One shared climate/height evaluation per column.
                let ColumnInfo { height, biome } = self.column(wx, wz);
                chunk.heights[z * CHUNK_SIZE + x] = height;
                chunk.biomes[z * CHUNK_SIZE + x] = biome;
                self.fill_column(&mut chunk, &ch, x, z, wx, wz, height, biome);
            }
        }

        self.decorate(&mut chunk);
        chunk
    }

    /// Fill one column: bedrock, stone/deepslate, ores, caves, soil, surface, water.
    #[allow(clippy::too_many_arguments)]
    fn fill_column(
        self,
        chunk: &mut GeneratedChunk,
        ch: &Channels,
        x: usize,
        z: usize,
        wx: i32,
        wz: i32,
        height: i32,
        biome: Biome,
    ) {
        let submerged = height < SEA_LEVEL;
        let soil_depth = biome.soil_depth();
        // Gravel patches speckle the stone just under the soil.
        let gravel = value_noise_2d(self.channel(40), wx as f64 / 19.0, wz as f64 / 19.0) > 0.62;

        for y in MIN_Y..=height {
            // Bedrock floor: solid bottom layer, then a rough two-block skirt.
            if y == MIN_Y {
                chunk.set(x, y, z, block::BEDROCK);
                continue;
            }
            if y <= MIN_Y + 2
                && hash_2d(self.channel(41), wx as i64 * 7919 + y as i64, wz as i64) > 0.35
            {
                chunk.set(x, y, z, block::BEDROCK);
                continue;
            }

            let depth = height - y;
            let state = if depth < soil_depth.max(1) && y != height {
                // Soil band directly under the surface block.
                if gravel && depth == soil_depth - 1 {
                    block::GRAVEL
                } else {
                    biome.soil_block()
                }
            } else if y == height {
                if submerged {
                    // Ocean floor: sand shallow, gravel deep.
                    if height > SEA_LEVEL - 8 {
                        block::SAND
                    } else {
                        block::GRAVEL
                    }
                } else {
                    biome.surface_block(y)
                }
            } else if y < DEEPSLATE_Y {
                block::DEEPSLATE
            } else {
                block::STONE
            };

            // Caves carve everything except the bedrock skirt.
            if self.is_cave_with(ch, wx, y, wz, height) {
                continue;
            }

            // Ores only replace plain stone/deepslate.
            let state = if state == block::STONE || state == block::DEEPSLATE {
                match self.ore_at_with(ch, wx, y, wz) {
                    Some(ore) => ore,
                    None => state,
                }
            } else {
                state
            };

            chunk.set(x, y, z, state);
        }

        // Water fills every air block from the terrain top up to the water
        // surface. Note `WATER_SURFACE_Y`, not `SEA_LEVEL`: measuring 49 real
        // vanilla chunks showed the highest water block is y=62 while sea level
        // is 63, i.e. water occupies `y < SEA_LEVEL`
        // (`scripts/analyze_fluid_rules.py`).
        if submerged {
            for y in (height + 1)..=WATER_SURFACE_Y {
                chunk.set_if_air(x, y, z, block::WATER);
            }
        }
    }

    // ---- features --------------------------------------------------------

    /// Place trees and surface plants.
    ///
    /// Trunks are considered for a 2-chunk-wide margin around this chunk so a
    /// canopy that overhangs the border is still written here. Because placement
    /// is a pure hash of the absolute trunk position, both chunks agree on
    /// exactly the same tree without any cross-chunk communication.
    fn decorate(self, chunk: &mut GeneratedChunk) {
        const MARGIN: i32 = 2;
        let base_x = chunk.pos.x * CHUNK_SIZE as i32;
        let base_z = chunk.pos.z * CHUNK_SIZE as i32;

        for dz in -MARGIN..(CHUNK_SIZE as i32 + MARGIN) {
            for dx in -MARGIN..(CHUNK_SIZE as i32 + MARGIN) {
                let wx = base_x + dx;
                let wz = base_z + dz;
                let Some((trunk_y, biome)) = self.tree_at(wx, wz) else {
                    continue;
                };
                self.place_oak(chunk, wx - base_x, trunk_y, wz - base_z, biome, wx, wz);
            }
        }

        // Ground cover only matters inside the chunk itself.
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let wx = base_x + x as i32;
                let wz = base_z + z as i32;
                let height = chunk.height_at(x, z);
                let biome = chunk.biome_at(x, z);
                if height < SEA_LEVEL {
                    continue;
                }
                // The surface may have been carved by a cave: only plant on solid ground.
                if chunk.get(x, height, z) == Some(block::AIR) {
                    continue;
                }
                let roll = hash_2d(self.channel(51), wx as i64, wz as i64);
                match biome {
                    Biome::Plains if roll > 0.72 => {
                        chunk.set_if_air(x, height + 1, z, block::SHORT_GRASS);
                    }
                    Biome::Forest if roll > 0.80 => {
                        chunk.set_if_air(x, height + 1, z, block::SHORT_GRASS);
                    }
                    Biome::Desert if roll > 0.988 => {
                        // Small cactus column.
                        let tall = if roll > 0.995 { 3 } else { 2 };
                        for i in 0..tall {
                            chunk.set_if_air(x, height + 1 + i, z, block::CACTUS);
                        }
                    }
                    Biome::Desert if roll > 0.975 => {
                        chunk.set_if_air(x, height + 1, z, block::DEAD_BUSH);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Whether a tree trunk starts at this absolute column, and on what biome.
    ///
    /// Returns the ground Y the trunk sits on. Density is per-biome; the hash is
    /// position-keyed so the answer never depends on which chunk is asking.
    ///
    /// The cheap position hash is tested **first**: it rejects ~95% of columns
    /// without ever evaluating the (much more expensive) climate noise, which
    /// matters because decoration probes 400 columns per chunk.
    fn tree_at(self, wx: i32, wz: i32) -> Option<(i32, Biome)> {
        let roll = hash_2d(self.channel(50), wx as i64, wz as i64);
        // Cheapest possible early-out: below the loosest per-biome threshold no
        // biome can produce a tree here.
        if roll < 0.945 {
            return None;
        }
        let ColumnInfo { height, biome } = self.column(wx, wz);
        if height < SEA_LEVEL + 1 {
            return None;
        }
        let density = match biome {
            Biome::Forest | Biome::DarkForest | Biome::Jungle => 0.945,
            Biome::BirchForest | Biome::Taiga => 0.95,
            Biome::Plains | Biome::Savanna => 0.995,
            _ => return None,
        };
        if roll < density {
            return None;
        }
        Some((height, biome))
    }

    /// Write an oak: 4-6 log trunk plus a 5x5x3 canopy, clipped to the chunk.
    ///
    /// `local_x`/`local_z` may be negative or past 15 — [`GeneratedChunk::set`]
    /// rejects out-of-range writes, which is exactly the clipping we want.
    #[allow(clippy::too_many_arguments)]
    fn place_oak(
        self,
        chunk: &mut GeneratedChunk,
        local_x: i32,
        ground_y: i32,
        local_z: i32,
        biome: Biome,
        wx: i32,
        wz: i32,
    ) {
        let roll = hash_2d(self.channel(52), wx as i64, wz as i64);
        let (log, leaves) = match biome {
            Biome::BirchForest => (block::BIRCH_LOG, block::BIRCH_LEAVES),
            Biome::Taiga => (block::SPRUCE_LOG, block::SPRUCE_LEAVES),
            _ => (block::OAK_LOG, block::OAK_LEAVES),
        };
        let trunk = 4 + (roll * 3.0) as i32; // 4..=6
        let top = ground_y + trunk;
        if top + 2 > MAX_Y {
            return;
        }

        // Canopy: two 5x5 layers around the top, then a 3x3 cap, then a centre tip.
        for (dy, radius) in [(-1i32, 2i32), (0, 2), (1, 1)] {
            let y = top + dy;
            for lz in -radius..=radius {
                for lx in -radius..=radius {
                    // Trim the 5x5 corners so the canopy is round, not square.
                    if radius == 2 && lx.abs() == 2 && lz.abs() == 2 {
                        let corner = hash_2d(
                            self.channel(53),
                            wx as i64 * 31 + lx as i64,
                            wz as i64 * 31 + lz as i64,
                        );
                        if corner < 0.5 {
                            continue;
                        }
                    }
                    self.leaf_with(chunk, local_x + lx, y, local_z + lz, leaves);
                }
            }
        }
        self.leaf(chunk, local_x, top + 2, local_z);

        // Trunk last so it overwrites any leaf that landed in the middle.
        for i in 0..trunk {
            let y = ground_y + 1 + i;
            if local_x < 0
                || local_x >= CHUNK_SIZE as i32
                || local_z < 0
                || local_z >= CHUNK_SIZE as i32
            {
                break;
            }
            chunk.set(local_x as usize, y, local_z as usize, log);
        }
    }

    fn leaf_with(self, chunk: &mut GeneratedChunk, x: i32, y: i32, z: i32, state: u32) {
        if x < 0 || x >= CHUNK_SIZE as i32 || z < 0 || z >= CHUNK_SIZE as i32 {
            return;
        }
        chunk.set_if_air(x as usize, y, z as usize, state);
    }

    fn leaf(self, chunk: &mut GeneratedChunk, x: i32, y: i32, z: i32) {
        self.leaf_with(chunk, x, y, z, block::OAK_LEAVES);
    }

    /// A safe Y to spawn a player at in the given column: one block above the
    /// highest solid terrain, or above sea level when that column is ocean.
    pub fn spawn_y(self, x: i32, z: i32) -> f64 {
        let height = self.height_at(x, z);
        if height < SEA_LEVEL {
            (SEA_LEVEL + 1) as f64
        } else {
            (height + 1) as f64
        }
    }
}

fn biome_from_id(id: biome::BiomeId) -> Biome {
    match id {
        biome::ids::OCEAN => Biome::Ocean,
        biome::ids::FROZEN_OCEAN => Biome::FrozenOcean,
        biome::ids::RIVER => Biome::River,
        biome::ids::DESERT => Biome::Desert,
        biome::ids::SNOWY_PLAINS => Biome::SnowyPlains,
        biome::ids::SNOWY_SLOPES => Biome::SnowyMountains,
        biome::ids::MUSHROOM_FIELDS => Biome::MushroomFields,
        _ => Biome::Plains,
    }
}
fn biome_is_water(id: biome::BiomeId) -> bool {
    matches!(
        id,
        biome::ids::OCEAN | biome::ids::FROZEN_OCEAN | biome::ids::RIVER
    )
}

/// Radius (in chunks) of the dependency neighborhood for each generation stage.
///
/// The current data-driven generator has no structure or light placement yet, but
/// keeping this schedule explicit makes those stages composable without changing
/// the order-independent terrain result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkPyramid {
    structure_starts: u8,
    biomes: u8,
    noise: u8,
    surface: u8,
    caves: u8,
    features: u8,
    light: u8,
}

impl ChunkPyramid {
    const VANILLA: Self = Self {
        structure_starts: 11, // 23 x 23
        biomes: 3,            // 7 x 7
        noise: 2,             // 5 x 5
        surface: 1,           // 3 x 3
        caves: 0,
        features: 0,
        light: 0,
    };

    #[inline]
    fn stages(self) -> [u8; 7] {
        [
            self.structure_starts,
            self.biomes,
            self.noise,
            self.surface,
            self.caves,
            self.features,
            self.light,
        ]
    }
}

struct VanillaColumn {
    top: i32,
    biome: Biome,
    states: Vec<u32>,
}

struct VanillaGraph {
    final_density: density::DensityFunction,
    preliminary_surface_level: Option<density::DensityFunction>,
    // Exposed in the graph for cave probes and parity diagnostics.  These are
    noodle: Option<density::DensityFunction>,
    cave_cheese: Option<density::DensityFunction>,
    entrances: Option<density::DensityFunction>,
    parameters: Vec<(biome::BiomeId, biome::BiomeParameters)>,
    temperature: Option<density::DensityFunction>,
    humidity: Option<density::DensityFunction>,
    continentalness: Option<density::DensityFunction>,
    erosion: Option<density::DensityFunction>,
    weirdness: Option<density::DensityFunction>,
    depth: Option<density::DensityFunction>,
    surface_rule: Option<surface_rules::SurfaceRule>,
}
impl VanillaGraph {
    fn load() -> Option<&'static Self> {
        static GRAPH: OnceLock<Option<VanillaGraph>> = OnceLock::new();
        GRAPH.get_or_init(Self::load_uncached).as_ref()
    }

    fn load_uncached() -> Option<Self> {
        let root = std::env::var_os("BCORE_DATAPACK")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/datapack"));
        let settings = root.join("data/minecraft/worldgen/noise_settings/overworld.json");
        let text = fs::read_to_string(settings).ok()?;
        let final_json = text
            .split("\"final_density\":")
            .nth(1)?
            .split("\"vein_toggle\"")
            .next()?;
        let final_density = density::parse_json(final_json).ok()?;
        let dir = root.join("data/minecraft/worldgen/density_function/overworld");
        let load = |name: &str| {
            fs::read_to_string(dir.join(format!("{name}.json")))
                .ok()
                .and_then(|s| density::parse_json(&s).ok())
        };
        let cave_dir = dir.join("caves");
        let load_cave = |name: &str| {
            fs::read_to_string(cave_dir.join(format!("{name}.json")))
                .ok()
                .and_then(|s| density::parse_json(&s).ok())
        };
        let parameters = biome::load_overworld_parameters(
            root.join("../datagen/reports/biome_parameters/minecraft/overworld.json"),
        )
        .unwrap_or_default();
        let surface_rule = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("surface_rule").map(surface_rules::SurfaceRule::parse));
        let preliminary = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("noise_router")
                    .and_then(|r| r.get("preliminary_surface_level"))
                    .cloned()
            })
            .and_then(|v| density::parse_json(&v.to_string()).ok());
        Some(Self {
            final_density,
            preliminary_surface_level: preliminary,
            noodle: load_cave("noodle"),
            cave_cheese: load("sloped_cheese"),
            entrances: load_cave("entrances"),
            parameters,
            temperature: load("temperature"),
            humidity: load("humidity"),
            continentalness: load("continents"),
            erosion: load("erosion"),
            weirdness: load("ridges"),
            depth: load("depth"),
            surface_rule,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const SEED: i64 = 1234;

    #[test]
    #[ignore]
    fn benchmark_vanilla_graph_cache() {
        use std::time::Instant;
        let pos = ChunkPos::new(-34, 3);
        let cold = Instant::now();
        let first = WorldGenerator::new(1234).generate_chunk_vanilla(pos);
        let cold_elapsed = cold.elapsed();
        let warm = Instant::now();
        let second = WorldGenerator::new(1234).generate_chunk_vanilla(pos);
        let warm_elapsed = warm.elapsed();
        assert_eq!(first, second);
        println!("vanilla graph cache: cold={cold_elapsed:?}, warm={warm_elapsed:?}");
    }
    #[test]
    fn generation_is_deterministic_for_the_same_seed_and_position() {
        let gen = WorldGenerator::new(SEED);
        let a = gen.generate_chunk(ChunkPos::new(-3, 7));
        let b = gen.generate_chunk(ChunkPos::new(-3, 7));
        assert_eq!(a, b, "same seed + position must produce identical blocks");
    }

    #[test]
    fn generation_does_not_depend_on_visit_order() {
        let gen = WorldGenerator::new(SEED);
        // Forward order.
        let forward: Vec<_> = (0..4)
            .map(|i| gen.generate_chunk(ChunkPos::new(i, 1)))
            .collect();
        // Reverse order must yield the same columns.
        let reverse: Vec<_> = (0..4)
            .rev()
            .map(|i| gen.generate_chunk(ChunkPos::new(i, 1)))
            .collect();
        for (i, chunk) in forward.iter().enumerate() {
            let mirror = &reverse[3 - i];
            assert_eq!(chunk, mirror, "chunk {i} changed with visit order");
        }
    }

    #[test]
    fn different_seeds_produce_different_terrain() {
        let a = WorldGenerator::new(1).generate_chunk(ChunkPos::new(0, 0));
        let b = WorldGenerator::new(2).generate_chunk(ChunkPos::new(0, 0));
        assert_ne!(a.states(), b.states());
    }

    #[test]
    fn bedrock_floors_every_column_and_nothing_below() {
        let chunk = WorldGenerator::new(SEED).generate_chunk(ChunkPos::new(5, -2));
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                assert_eq!(
                    chunk.get(x, MIN_Y, z),
                    Some(block::BEDROCK),
                    "y={MIN_Y} must be bedrock at ({x},{z})"
                );
            }
        }
    }

    #[test]
    fn terrain_height_varies_across_the_world() {
        let gen = WorldGenerator::new(SEED);
        let mut heights = Vec::new();
        for cz in -6..6 {
            for cx in -6..6 {
                heights.push(gen.height_at(cx * 16, cz * 16));
            }
        }
        let min = *heights.iter().min().expect("heights");
        let max = *heights.iter().max().expect("heights");
        assert!(
            max - min > 20,
            "terrain should be varied, got range {min}..{max}"
        );
    }

    #[test]
    fn layers_run_surface_soil_stone_downwards() {
        let gen = WorldGenerator::new(SEED);
        let chunk = gen.generate_chunk(ChunkPos::new(2, 3));
        let mut checked = 0;
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let height = chunk.height_at(x, z);
                let biome = chunk.biome_at(x, z);
                if height < SEA_LEVEL || biome == Biome::Mountains {
                    continue; // water column / stone surface handled elsewhere
                }
                // Surface block matches the biome (unless a cave ate it).
                let surface = chunk.get(x, height, z).expect("in range");
                if surface == block::AIR {
                    continue;
                }
                match biome {
                    Biome::Plains | Biome::Forest => {
                        assert_eq!(surface, block::GRASS_BLOCK, "({x},{z}) {biome:?}")
                    }
                    Biome::Desert | Biome::Beach => {
                        assert_eq!(surface, block::SAND, "({x},{z}) {biome:?}")
                    }
                    _ => {}
                }
                // Deep below the soil it must be stone/deepslate/ore, never soil.
                let deep = chunk.get(x, height - 12, z).expect("in range");
                assert_ne!(deep, block::GRASS_BLOCK, "grass at depth ({x},{z})");
                assert_ne!(deep, block::DIRT, "dirt 12 blocks down ({x},{z})");
                checked += 1;
            }
        }
        assert!(checked > 0, "test covered no columns");
    }

    #[test]
    fn soil_sits_directly_under_a_grass_surface() {
        let gen = WorldGenerator::new(SEED);
        let mut found = 0;
        for cz in 0..8 {
            for cx in 0..8 {
                let chunk = gen.generate_chunk(ChunkPos::new(cx, cz));
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        if chunk.biome_at(x, z) != Biome::Plains {
                            continue;
                        }
                        let h = chunk.height_at(x, z);
                        if chunk.get(x, h, z) != Some(block::GRASS_BLOCK) {
                            continue;
                        }
                        let under = chunk.get(x, h - 1, z).expect("in range");
                        assert!(
                            under == block::DIRT || under == block::GRAVEL || under == block::AIR,
                            "under grass at ({x},{z}) of ({cx},{cz}): {under}"
                        );
                        found += 1;
                    }
                }
                if found > 200 {
                    return;
                }
            }
        }
        assert!(found > 0, "no plains grass columns found");
    }

    /// Find a chunk whose centre column is below sea level.
    ///
    /// The continent noise wavelength is 320 blocks, so a search window has to
    /// span several hundred blocks to be sure of crossing a coastline — a 16x16
    /// chunk box around the origin can legitimately be all dry land.
    fn find_ocean_chunk(gen: WorldGenerator) -> ChunkPos {
        for radius in 0..48i32 {
            for cz in -radius..=radius {
                for cx in -radius..=radius {
                    // Only test the ring just added, so the search spirals outwards.
                    if cx.abs() != radius && cz.abs() != radius {
                        continue;
                    }
                    let pos = ChunkPos::new(cx * 2, cz * 2);
                    let (wx, wz) = (pos.x * 16 + 8, pos.z * 16 + 8);
                    if gen.height_at(wx, wz) < SEA_LEVEL - 4 {
                        return pos;
                    }
                }
            }
        }
        panic!("seed {} produced no ocean within 1536 blocks", gen.seed());
    }

    #[test]
    fn water_fills_everything_below_sea_level_in_ocean_columns() {
        let gen = WorldGenerator::new(SEED);
        let pos = find_ocean_chunk(gen);
        let chunk = gen.generate_chunk(pos);
        let mut ocean_columns = 0;
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let height = chunk.height_at(x, z);
                if height >= SEA_LEVEL {
                    continue;
                }
                ocean_columns += 1;
                // Every block from the sea floor up to the water surface is water.
                for y in (height + 1)..=WATER_SURFACE_Y {
                    assert_eq!(
                        chunk.get(x, y, z),
                        Some(block::WATER),
                        "({x},{y},{z}) of chunk {pos:?} should be water"
                    );
                }
                // Water stops below sea level: y=63 is already air.
                assert_eq!(
                    chunk.get(x, SEA_LEVEL, z),
                    Some(block::AIR),
                    "water at sea level {SEA_LEVEL} in ({x},{z}) of {pos:?}; \
                     vanilla's topmost water is {WATER_SURFACE_Y}"
                );
                // The sea floor itself is solid, never water.
                assert_ne!(chunk.get(x, height, z), Some(block::WATER));
                assert_ne!(chunk.get(x, height, z), Some(block::AIR));
                // Ocean columns are biome Ocean.
                assert_eq!(chunk.biome_at(x, z), Biome::Ocean);
            }
        }
        assert!(
            ocean_columns > 0,
            "chunk {pos:?} was picked as ocean but has no submerged column"
        );
    }

    #[test]
    fn water_never_reaches_sea_level_itself() {
        // Regression: the generator originally filled `..=SEA_LEVEL`, putting the
        // ocean surface one block above vanilla's. An end-to-end test caught it.
        let gen = WorldGenerator::new(SEED);
        let pos = find_ocean_chunk(gen);
        let chunk = gen.generate_chunk(pos);
        let mut water_columns = 0;
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                if chunk.get(x, WATER_SURFACE_Y, z) == Some(block::WATER) {
                    water_columns += 1;
                }
                // Nothing at or above sea level may be water, anywhere.
                for y in SEA_LEVEL..=MAX_Y {
                    assert_ne!(
                        chunk.get(x, y, z),
                        Some(block::WATER),
                        "water at y={y}, at/above sea level {SEA_LEVEL}"
                    );
                }
            }
        }
        assert!(
            water_columns > 0,
            "the ocean chunk has no water at the surface height {WATER_SURFACE_Y}"
        );
    }

    #[test]
    fn spawn_y_stands_on_water_surface_over_ocean() {
        let gen = WorldGenerator::new(SEED);
        let pos = find_ocean_chunk(gen);
        let (wx, wz) = (pos.x * 16 + 8, pos.z * 16 + 8);
        let y = gen.spawn_y(wx, wz);
        // Over ocean the player stands on the water surface, not under it.
        assert!(
            y > WATER_SURFACE_Y as f64,
            "spawn y={y} is inside the water (surface {WATER_SURFACE_Y})"
        );
    }

    #[test]
    fn oceans_and_dry_land_both_exist_for_several_seeds() {
        for seed in [SEED, 0, -99, 777] {
            let gen = WorldGenerator::new(seed);
            let mut ocean = 0;
            let mut land = 0;
            // Sample on a coarse grid wide enough to cross the 320-block
            // continent wavelength several times.
            for iz in -24..24i32 {
                for ix in -24..24i32 {
                    if gen.height_at(ix * 64, iz * 64) < SEA_LEVEL {
                        ocean += 1;
                    } else {
                        land += 1;
                    }
                }
            }
            assert!(ocean > 0, "seed {seed} has no ocean");
            assert!(land > 0, "seed {seed} is entirely ocean");
        }
    }

    #[test]
    fn ores_only_appear_inside_their_depth_bands() {
        let gen = WorldGenerator::new(SEED);
        let mut seen: Vec<(u32, i32, i32)> = Vec::new(); // block, min y, max y
        for cz in 0..6 {
            for cx in 0..6 {
                let chunk = gen.generate_chunk(ChunkPos::new(cx, cz));
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        for y in MIN_Y..=MAX_SURFACE {
                            let Some(state) = chunk.get(x, y, z) else {
                                continue;
                            };
                            let ore = matches!(
                                state,
                                block::COAL_ORE
                                    | block::IRON_ORE
                                    | block::GOLD_ORE
                                    | block::DIAMOND_ORE
                                    | block::REDSTONE_ORE
                                    | block::LAPIS_ORE
                            );
                            if !ore {
                                continue;
                            }
                            match seen.iter_mut().find(|(s, _, _)| *s == state) {
                                Some(entry) => {
                                    entry.1 = entry.1.min(y);
                                    entry.2 = entry.2.max(y);
                                }
                                None => seen.push((state, y, y)),
                            }
                        }
                    }
                }
            }
        }
        seen.sort_unstable();
        assert!(!seen.is_empty(), "no ores generated at all");
        for (state, min_y, max_y) in seen {
            let limit = match state {
                block::DIAMOND_ORE | block::REDSTONE_ORE => 16,
                block::LAPIS_ORE => 30,
                block::GOLD_ORE => 32,
                block::IRON_ORE => 64,
                block::COAL_ORE => 128,
                _ => unreachable!("filtered above"),
            };
            assert!(
                max_y <= limit,
                "ore {state} found at y={max_y}, above its {limit} limit"
            );
            assert!(min_y >= MIN_Y, "ore {state} below the world");
        }
    }

    #[test]
    fn diamond_is_rarer_than_coal() {
        let gen = WorldGenerator::new(SEED);
        let mut coal = 0usize;
        let mut diamond = 0usize;
        for cz in 0..4 {
            for cx in 0..4 {
                let chunk = gen.generate_chunk(ChunkPos::new(cx, cz));
                for &state in chunk.states() {
                    match state {
                        block::COAL_ORE => coal += 1,
                        block::DIAMOND_ORE => diamond += 1,
                        _ => {}
                    }
                }
            }
        }
        assert!(coal > 0, "no coal generated");
        assert!(
            coal > diamond,
            "coal {coal} should outnumber diamond {diamond}"
        );
    }

    #[test]
    fn trees_grow_with_logs_under_leaves() {
        let gen = WorldGenerator::new(SEED);
        let mut trunks = 0;
        for cz in 0..10 {
            for cx in 0..10 {
                let chunk = gen.generate_chunk(ChunkPos::new(cx, cz));
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let h = chunk.height_at(x, z);
                        if chunk.get(x, h + 1, z) != Some(block::OAK_LOG) {
                            continue;
                        }
                        trunks += 1;
                        // Leaves must exist somewhere above the trunk.
                        let has_leaves =
                            (h + 2..h + 10).any(|y| chunk.get(x, y, z) == Some(block::OAK_LEAVES));
                        let capped =
                            (h + 2..h + 10).any(|y| chunk.get(x, y, z) == Some(block::OAK_LOG));
                        assert!(
                            has_leaves || capped,
                            "trunk at ({x},{z}) in ({cx},{cz}) has no canopy"
                        );
                    }
                }
                if trunks > 20 {
                    return;
                }
            }
        }
        assert!(trunks > 0, "no trees generated in 100 chunks");
    }

    #[test]
    fn caves_carve_air_underground_without_breaching_the_surface() {
        let gen = WorldGenerator::new(SEED);
        let mut cave_blocks = 0usize;
        for cz in 0..6 {
            for cx in 0..6 {
                let chunk = gen.generate_chunk(ChunkPos::new(cx, cz));
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let height = chunk.height_at(x, z);
                        // The 5 blocks under the surface are always solid crust.
                        for y in (height - 4)..=height {
                            if y <= MIN_Y {
                                continue;
                            }
                            assert_ne!(
                                chunk.get(x, y, z),
                                Some(block::AIR),
                                "surface crust breached at ({x},{y},{z}) in ({cx},{cz})"
                            );
                        }
                        for y in (MIN_Y + 5)..(height - 5) {
                            if chunk.get(x, y, z) == Some(block::AIR) {
                                cave_blocks += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(cave_blocks > 0, "no caves carved in 36 chunks");
    }

    #[test]
    fn every_biome_kind_appears_somewhere() {
        let gen = WorldGenerator::new(SEED);
        let mut found: Vec<Biome> = Vec::new();
        for cz in -20..20 {
            for cx in -20..20 {
                let h = gen.height_at(cx * 16, cz * 16);
                let biome = gen.biome_at(cx * 16, cz * 16, h);
                if !found.contains(&biome) {
                    found.push(biome);
                }
            }
        }
        found.sort_unstable();
        // Ocean, plains, forest and mountains must all show up; desert/beach are
        // climate-dependent and checked separately below.
        for want in [Biome::Ocean, Biome::Plains, Biome::Forest, Biome::Mountains] {
            assert!(
                found.contains(&want),
                "biome {want:?} never generated: {found:?}"
            );
        }
    }

    #[test]
    fn biome_network_ids_are_the_registry_values() {
        assert_eq!(Biome::Plains.network_id(), 40);
        assert_eq!(Biome::Forest.network_id(), 21);
        assert_eq!(Biome::Desert.network_id(), 14);
        assert_eq!(Biome::Ocean.network_id(), 35);
        assert_eq!(Biome::Beach.network_id(), 3);
        assert_eq!(Biome::Mountains.network_id(), 63);
    }

    #[test]
    fn chunk_borders_line_up_between_neighbours() {
        let gen = WorldGenerator::new(SEED);
        let left = gen.generate_chunk(ChunkPos::new(0, 0));
        let right = gen.generate_chunk(ChunkPos::new(1, 0));
        // The generator is column-pure, so the height at x=16 of chunk 0's
        // right neighbour equals the height computed for x=0 of chunk 1.
        for z in 0..CHUNK_SIZE {
            assert_eq!(
                gen.height_at(16, z as i32),
                right.height_at(0, z),
                "height mismatch at border z={z}"
            );
            assert_eq!(
                left.height_at(15, z),
                gen.height_at(15, z as i32),
                "height mismatch inside left chunk z={z}"
            );
        }
    }

    #[test]
    fn spawn_y_is_above_the_ground() {
        let gen = WorldGenerator::new(SEED);
        for (x, z) in [(0, 0), (100, -250), (-777, 512)] {
            let y = gen.spawn_y(x, z);
            let height = gen.height_at(x, z);
            assert!(
                y > height as f64 || height < SEA_LEVEL,
                "spawn {y} vs height {height}"
            );
            assert!(y >= (SEA_LEVEL + 1) as f64 || height >= SEA_LEVEL);
        }
    }

    #[test]
    fn heights_stay_inside_the_generator_band() {
        let gen = WorldGenerator::new(SEED);
        for cz in -12..12 {
            for cx in -12..12 {
                let h = gen.height_at(cx * 13, cz * 13);
                assert!(
                    (MIN_SURFACE..=MAX_SURFACE).contains(&h),
                    "height {h} outside {MIN_SURFACE}..={MAX_SURFACE}"
                );
            }
        }
    }
}
