//! Native 26.1 generation lighting and climate observations.
//!
//! FEATURES reads the world's light storage, not a heightmap approximation. An
//! untouched sky storage returns 15 (even underground); an allocated, unlit
//! section returns 0. INITIALIZE_LIGHT creates sky sources/section storage; LIGHT
//! enables sources and propagates them. Neither operation executes SPAWN/FULL.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::OnceLock;

use bcore_core::ChunkPos;
use serde::{Deserialize, Serialize};

use crate::block_predicate::{catalog, read_block, Direction, FeatureResult};
use crate::feature_world::{FeatureError, FeatureWorld, Pos};
use crate::{GeneratedChunk, MAX_Y, MIN_Y, SEA_LEVEL};

mod updates;
pub use updates::has_different_light_properties;

#[derive(Clone, Copy, Debug)]
pub struct StateLight {
    pub emission: u8,
    pub dampening: u8,
    pub snow_support: bool,
    faces: [usize; 6],
}

#[derive(Deserialize)]
struct Climate {
    has_precipitation: bool,
}

struct LightCatalog {
    states: Vec<StateLight>,
    occludes: Vec<Vec<bool>>,
    climates: BTreeMap<u32, Climate>,
}

fn light_catalog() -> &'static LightCatalog {
    static DATA: OnceLock<LightCatalog> = OnceLock::new();
    DATA.get_or_init(|| {
        #[derive(Deserialize)]
        struct Capture {
            state_count: usize,
            state_ranges: Vec<[usize; 11]>,
            occludes: Vec<Vec<usize>>,
            climates: BTreeMap<String, Climate>,
        }
        let capture: Capture =
            serde_json::from_str(include_str!("../data/lighting_catalog_26_1.json"))
                .expect("native 26.1 light catalog");
        let mut states = Vec::with_capacity(capture.state_count);
        for row in capture.state_ranges {
            assert_eq!(row[0], states.len());
            assert!(row[0] < row[1] && row[1] <= capture.state_count);
            assert!(row[2] <= 15 && row[3] <= 15 && row[4] <= 1);
            let faces = std::array::from_fn(|i| row[5 + i]);
            assert!(faces.iter().all(|&id| id < capture.occludes.len()));
            states.resize(
                row[1],
                StateLight {
                    emission: row[2] as u8,
                    dampening: row[3] as u8,
                    snow_support: row[4] != 0,
                    faces,
                },
            );
        }
        assert_eq!(states.len(), capture.state_count);
        let mut occludes = vec![vec![false; capture.occludes.len()]; capture.occludes.len()];
        for (i, row) in capture.occludes.into_iter().enumerate() {
            for j in row {
                occludes[i][j] = true;
            }
        }
        let climates = capture
            .climates
            .into_iter()
            .map(|(name, climate)| {
                (
                    crate::biome::id(&name).expect("native biome resource identity"),
                    climate,
                )
            })
            .collect();
        LightCatalog {
            states,
            occludes,
            climates,
        }
    })
}

pub fn state_light(state: u32) -> FeatureResult<StateLight> {
    light_catalog()
        .states
        .get(state as usize)
        .copied()
        .ok_or_else(|| {
            FeatureError::MissingData(format!("26.1 light properties for block state {state}"))
        })
}

/// Union of the two native face occlusion shapes. A sturdy face is NOT an
/// equivalent query (e.g. slabs, stairs, snow, pistons and leaves).
pub fn face_occludes(from: u32, to: u32, direction: Direction) -> FeatureResult<bool> {
    let from = state_light(from)?;
    let to = state_light(to)?;
    Ok(light_catalog().occludes[from.faces[direction as usize]][to.faces[direction as usize ^ 1]])
}

/// Reuses the bit-exact, native-verified biome temperature calculation. The
/// identity check prevents a 26.2-only wire biome or an invalid ID from aliasing
/// a native 26.1 climate (or indexing the wire registry out of bounds).
pub fn biome_temperature(biome: u32, pos: Pos, sea_level: i32) -> FeatureResult<f32> {
    if !light_catalog().climates.contains_key(&biome) {
        return Err(FeatureError::MissingData(format!(
            "26.1 climate for wire biome {biome}"
        )));
    }
    Ok(crate::surface_rules::SurfaceContext {
        biome,
        stone_depth_above: 0,
        stone_depth_below: 0,
        water_height: 0,
        surface_depth: 0,
        preliminary_surface_level: 0,
        sea_level,
        x: pos.0,
        y: pos.1,
        z: pos.2,
        seed: 0, // Biome's climate noises have fixed native seeds.
        noise: None,
    }
    .temperature())
}

pub fn snow_survives(world: &dyn FeatureWorld, pos: Pos) -> FeatureResult<bool> {
    Ok(state_light(read_block(world, Direction::Down.step(pos))?)?.snow_support)
}

/// `Biome.shouldFreeze` for the overworld, with an explicit biome and observed
/// BLOCK light. Freeze-top-layer passes the biome at the upper position, while
/// the test itself is performed at the block below it.
pub fn should_freeze(
    world: &dyn FeatureWorld,
    biome: u32,
    pos: Pos,
    must_be_at_edge: bool,
    block_light: u8,
) -> FeatureResult<bool> {
    if biome_temperature(biome, pos, SEA_LEVEL)? >= 0.15_f32
        || !(MIN_Y..=MAX_Y).contains(&pos.1)
        || block_light >= 10
    {
        return Ok(false);
    }
    let data = catalog();
    let state = read_block(world, pos)?;
    // Only WATER, not FLOWING_WATER, and only a LiquidBlock, not waterlogging.
    if data.info(state)?.fluid != data.fluids["minecraft:water"]
        || data.block(state)?.1.class != "LiquidBlock"
    {
        return Ok(false);
    }
    if !must_be_at_edge {
        return Ok(true);
    }
    // Preserve Java short-circuit access order and its water TAG check.
    for direction in [
        Direction::West,
        Direction::East,
        Direction::North,
        Direction::South,
    ] {
        if !data.in_fluid_tag(read_block(world, direction.step(pos))?, "water")? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn should_snow(
    world: &dyn FeatureWorld,
    biome: u32,
    pos: Pos,
    block_light: u8,
) -> FeatureResult<bool> {
    let temperature = biome_temperature(biome, pos, SEA_LEVEL)?;
    if !light_catalog().climates[&biome].has_precipitation
        || temperature >= 0.15_f32
        || !(MIN_Y..=MAX_Y).contains(&pos.1)
        || block_light >= 10
    {
        return Ok(false);
    }
    let state = read_block(world, pos)?;
    if !catalog().info(state)?.is_air() && !catalog().is_block(state, "snow")? {
        return Ok(false);
    }
    snow_survives(world, pos)
}

/// `ChunkSkyLightSources`: first unobstructed source Y, x + 16*z order. Fully
/// open columns extend below the world and use Java's NEGATIVE_INFINITY sentinel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkyLightSources {
    pub lowest_source_y: Vec<i32>,
}

impl SkyLightSources {
    pub fn get(&self, x: usize, z: usize) -> i32 {
        self.lowest_source_y[x + z * 16]
    }
}

/// A null layer means no native section storage, distinct from 2048 zero bytes.
/// Nibbles are x-fastest, then z, then y, low nibble first (native DataLayer).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightSection {
    pub sky: Option<Vec<u8>>,
    pub block: Option<Vec<u8>>,
    /// Native DataLayer.isEmpty(): present, lazy default-zero data. Materialized
    /// zero arrays are NOT empty and must be sent using the packet's DATA mask.
    #[serde(default)]
    pub sky_empty: bool,
    #[serde(default)]
    pub block_empty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkLight {
    pub min_section_y: i32,
    pub sections: Vec<LightSection>,
}

impl ChunkLight {
    pub fn validate(&self, min_y: i32, height: i32, has_sky: bool) -> bool {
        min_y % 16 == 0
            && height > 0
            && height % 16 == 0
            && self.min_section_y == min_y / 16 - 1
            && self.sections.len() == height as usize / 16 + 2
            && self.sections.iter().all(|s| {
                (has_sky || s.sky.is_none())
                    && (!s.sky_empty || s.sky.as_ref().is_some_and(|v| v.iter().all(|&b| b == 0)))
                    && (!s.block_empty
                        || s.block.as_ref().is_some_and(|v| v.iter().all(|&b| b == 0)))
                    && [&s.sky, &s.block]
                        .into_iter()
                        .all(|v| v.as_ref().is_none_or(|v| v.len() == 2048))
            })
    }
}

type SectionKey = (i32, i32, i32);
type ColumnKey = (i32, i32);

#[derive(Clone)]
struct StoredSection {
    sky: Vec<u8>,
    block: Vec<u8>,
    sky_empty: bool,
    block_empty: bool,
}

struct Column {
    states: Vec<u32>,
    section_counts: Vec<u16>,
    sources: SkyLightSources,
    initialized: bool,
}

#[derive(Clone, Copy)]
struct Increase {
    pos: Pos,
    level: u8,
    directions: u8,
    from_empty: bool,
    from_emission: bool,
}

/// A synchronous native-style generation light storage. Supply block snapshots
/// at INITIALIZE_LIGHT, then enable each source column at LIGHT. Later proto
/// writes use `apply_block_updates`, preserving their order within each batch. All
/// reads use this shared storage, including light received by unlit neighbours.
/// Missing LightChunk data is BEDROCK, never guessed AIR.
pub struct GenerationLight {
    min_y: i32,
    height: i32,
    has_sky: bool,
    columns: BTreeMap<ColumnKey, Column>,
    sections: BTreeMap<SectionKey, StoredSection>,
    enabled: BTreeSet<ColumnKey>,
    nonempty_sections: BTreeSet<SectionKey>,
    removed_sections: BTreeSet<SectionKey>,
    changed_columns: BTreeSet<ColumnKey>,
}

impl Default for GenerationLight {
    fn default() -> Self {
        Self::new(MIN_Y, crate::WORLD_HEIGHT, true).expect("overworld light dimensions")
    }
}

impl GenerationLight {
    pub fn new(min_y: i32, height: i32, has_sky: bool) -> FeatureResult<Self> {
        if min_y % 16 != 0
            || height <= 0
            || height % 16 != 0
            || min_y.checked_sub(16).is_none()
            || height
                .checked_add(16)
                .and_then(|h| min_y.checked_add(h))
                .is_none()
        {
            return Err(FeatureError::InvalidConfig(
                "light dimensions must be section aligned".into(),
            ));
        }
        Ok(Self {
            min_y,
            height,
            has_sky,
            columns: BTreeMap::new(),
            sections: BTreeMap::new(),
            enabled: BTreeSet::new(),
            nonempty_sections: BTreeSet::new(),
            removed_sections: BTreeSet::new(),
            changed_columns: BTreeSet::new(),
        })
    }

    pub fn initialize_generated_chunk(&mut self, chunk: &GeneratedChunk) -> FeatureResult<()> {
        if self.min_y != MIN_Y || self.height != crate::WORLD_HEIGHT {
            return Err(FeatureError::InvalidConfig(
                "GeneratedChunk is an overworld column".into(),
            ));
        }
        self.initialize_chunk(chunk.pos, chunk.states())
    }

    /// `ServerChunkCache.getChunkForLighting` exposes FEATURES holders, even
    /// before INITIALIZE_LIGHT. Registration alone creates no light sections;
    /// sky source heights still have the native constructor's -infinity values.
    pub fn register_chunk(&mut self, pos: ChunkPos, states: &[u32]) -> FeatureResult<()> {
        if states.len() != self.height as usize * 256 {
            return Err(FeatureError::InvalidConfig(
                "light block snapshot length".into(),
            ));
        }
        if [pos.x, pos.z].into_iter().any(|c| {
            i64::from(c) * 16 - 16 < i64::from(i32::MIN)
                || i64::from(c) * 16 + 31 > i64::from(i32::MAX)
        }) {
            return Err(FeatureError::InvalidConfig(
                "light chunk coordinates overflow padding".into(),
            ));
        }
        for &state in states {
            state_light(state)?;
        }
        if let Some(old) = self.columns.get_mut(&(pos.x, pos.z)) {
            if old.states == states {
                return Ok(());
            }
            if old.initialized {
                let unchanged = old.states.iter().zip(states).all(|(&before, &after)| {
                    if before == after {
                        return true;
                    }
                    let a = state_light(before).expect("validated stored light state");
                    let b = state_light(after).expect("validated incoming light state");
                    crate::is_air(before) == crate::is_air(after)
                        && a.emission == b.emission
                        && a.dampening == b.dampening
                        && a.faces == b.faces
                });
                if !unchanged {
                    return Err(FeatureError::Unsupported(
                        "replacing initialized light blocks requires invalidation".into(),
                    ));
                }
            }
            old.states.copy_from_slice(states);
            old.section_counts = section_counts(states);
        } else {
            self.changed_columns.insert((pos.x, pos.z));
            self.columns.insert(
                (pos.x, pos.z),
                Column {
                    states: states.to_vec(),
                    section_counts: section_counts(states),
                    sources: SkyLightSources {
                        lowest_source_y: vec![i32::MIN; 256],
                    },
                    initialized: false,
                },
            );
        }
        Ok(())
    }

    /// Registers native nonempty sections and all 26 neighbouring padding
    /// sections; computes the exact face/opacity sky-source heightmap.
    pub fn initialize_chunk(&mut self, pos: ChunkPos, states: &[u32]) -> FeatureResult<()> {
        self.register_chunk(pos, states)?;
        if self.columns[&(pos.x, pos.z)].initialized {
            return Ok(());
        }
        let mut sources = SkyLightSources {
            lowest_source_y: vec![i32::MIN; 256],
        };
        let occupied: Vec<_> = states
            .chunks_exact(4096)
            .map(|section| section.iter().any(|&state| !crate::is_air(state)))
            .collect();
        for z in 0..16 {
            for x in 0..16 {
                let mut above = crate::block::AIR;
                'column: for section in (0..occupied.len()).rev() {
                    if !occupied[section] {
                        // Native fillFrom skips WHOLE empty sections and resets
                        // the preceding state to AIR. In particular it does not
                        // test the bottom face of a slab at the preceding edge.
                        above = crate::block::AIR;
                        continue;
                    }
                    for ry in (0..16).rev() {
                        let dy = section * 16 + ry;
                        let state = states[dy * 256 + z * 16 + x];
                        if state_light(state)?.dampening != 0
                            || face_occludes(above, state, Direction::Down)?
                        {
                            sources.lowest_source_y[z * 16 + x] = self.min_y + dy as i32 + 1;
                            break 'column;
                        }
                        above = state;
                    }
                }
            }
        }
        for (index, &occupied) in occupied.iter().enumerate() {
            if !occupied {
                continue;
            }
            let sy = self.min_y / 16 + index as i32;
            // Native updateSectionStatus allocates the center first, then x/y/z.
            // Sky storage creation can copy an already-stored plane above it.
            self.set_section_nonempty((pos.x, sy, pos.z), true);
        }
        let column = self.columns.get_mut(&(pos.x, pos.z)).unwrap();
        column.sources = sources;
        column.initialized = true;
        self.changed_columns.insert((pos.x, pos.z));
        Ok(())
    }

    fn initialize_section(&mut self, key: SectionKey) {
        if self.sections.contains_key(&key) {
            return;
        }
        let (cx, sy, cz) = key;
        self.changed_columns.insert((cx, cz));
        let above = self
            .sections
            .range((cx, sy, i32::MIN)..=(cx, i32::MAX, i32::MAX))
            .find(|((_, y, z), _)| *z == cz && *y > sy);
        let (sky, sky_empty) = if let Some((_, section)) = above {
            (
                (0..4096).map(|i| section.sky[i & 255]).collect(),
                section.sky_empty,
            )
        } else {
            let value = if self.has_sky && self.enabled.contains(&(cx, cz)) {
                15
            } else {
                0
            };
            (vec![value; 4096], value == 0)
        };
        self.sections.insert(
            key,
            StoredSection {
                sky,
                block: vec![0; 4096],
                sky_empty,
                block_empty: true,
            },
        );
    }

    pub fn is_initialized(&self, pos: ChunkPos) -> bool {
        self.columns
            .get(&(pos.x, pos.z))
            .is_some_and(|c| c.initialized)
    }

    pub fn sky_sources(&self, pos: ChunkPos) -> Option<&SkyLightSources> {
        self.columns
            .get(&(pos.x, pos.z))
            .map(|column| &column.sources)
    }

    pub fn is_enabled(&self, pos: ChunkPos) -> bool {
        self.enabled.contains(&(pos.x, pos.z))
    }

    pub(crate) fn take_changed_columns(&mut self) -> Vec<ChunkPos> {
        std::mem::take(&mut self.changed_columns)
            .into_iter()
            .map(|(x, z)| ChunkPos::new(x, z))
            .collect()
    }

    fn key(pos: Pos) -> SectionKey {
        (pos.0 >> 4, pos.1 >> 4, pos.2 >> 4)
    }
    fn index(pos: Pos) -> usize {
        ((pos.1 & 15) * 256 + (pos.2 & 15) * 16 + (pos.0 & 15)) as usize
    }
    fn state(&self, pos: Pos) -> u32 {
        let Some(column) = self.columns.get(&(pos.0 >> 4, pos.2 >> 4)) else {
            return crate::block::BEDROCK;
        };
        let dy = pos.1 - self.min_y;
        if !(0..self.height).contains(&dy) {
            return crate::block::AIR;
        }
        column.states[(dy * 256 + (pos.2 & 15) * 16 + (pos.0 & 15)) as usize]
    }

    pub fn block_brightness(&self, pos: Pos) -> u8 {
        self.sections
            .get(&Self::key(pos))
            .map_or(0, |s| s.block[Self::index(pos)])
    }

    /// Missing sky sections repeat the bottom plane of the next stored section
    /// above, or 15 above all storage. No build-height clamp is applied here.
    pub fn sky_brightness(&self, pos: Pos) -> u8 {
        if !self.has_sky {
            return 0;
        }
        let (cx, sy, cz) = Self::key(pos);
        if let Some(section) = self.sections.get(&(cx, sy, cz)) {
            return section.sky[Self::index(pos)];
        }
        self.sections
            .range((cx, sy, i32::MIN)..=(cx, i32::MAX, i32::MAX))
            .find(|((_, y, z), _)| *z == cz && *y > sy)
            .map_or(15, |(_, section)| {
                section.sky[((pos.2 & 15) * 16 + (pos.0 & 15)) as usize]
            })
    }

    pub fn raw_brightness(&self, pos: Pos, sky_darken: i32) -> i32 {
        i32::from(self.block_brightness(pos)).max(i32::from(self.sky_brightness(pos)) - sky_darken)
    }

    pub fn max_local_raw_brightness(&self, pos: Pos) -> i32 {
        if pos.0 < -30_000_000 || pos.0 >= 30_000_000 || pos.2 < -30_000_000 || pos.2 >= 30_000_000
        {
            15
        } else {
            self.raw_brightness(pos, 0)
        }
    }

    /// Enable a column's emission and sky sources and drain propagation before
    /// returning. Already-lit columns are idempotent. This does not light-enable
    /// its neighbours (they may receive propagated light in their padding).
    pub fn propagate_chunk(&mut self, pos: ChunkPos) -> FeatureResult<()> {
        if !self.is_initialized(pos) {
            return Err(FeatureError::MissingData(format!(
                "INITIALIZE_LIGHT at {pos:?}"
            )));
        }
        if !self.enabled.insert((pos.x, pos.z)) {
            return Ok(());
        }
        self.changed_columns.insert((pos.x, pos.z));
        let mut block_queue = VecDeque::new();
        let mut sky_queue = VecDeque::new();
        let column = &self.columns[&(pos.x, pos.z)];
        for (&(cx, sy, cz), section) in &mut self.sections {
            if (cx, cz) != (pos.x, pos.z) {
                continue;
            }
            for i in 0..4096 {
                let p = (
                    cx * 16 + (i & 15) as i32,
                    sy * 16 + (i >> 8) as i32,
                    cz * 16 + ((i >> 4) & 15) as i32,
                );
                let dy = p.1 - self.min_y;
                if (0..self.height).contains(&dy) {
                    let emission =
                        state_light(column.states[dy as usize * 256 + (i & 255)])?.emission;
                    if emission > section.block[i] {
                        section.block[i] = emission;
                        section.block_empty = false;
                        block_queue.push_back(Increase {
                            pos: p,
                            level: emission,
                            directions: 63,
                            from_empty: false,
                            from_emission: false,
                        });
                    }
                }
            }
        }
        if self.has_sky {
            let keys: Vec<_> = self
                .sections
                .keys()
                .copied()
                .filter(|&(x, _, z)| (x, z) == (pos.x, pos.z))
                .rev()
                .collect();
            for (cx, sy, cz) in keys {
                for z in 0..16 {
                    for x in 0..16 {
                        let wx = cx * 16 + x;
                        let wz = cz * 16 + z;
                        let lowest = self.lowest_source(wx, wz);
                        for y in (sy * 16..sy * 16 + 16).rev() {
                            if y < lowest {
                                break;
                            }
                            let p = (wx, y, wz);
                            let section = self.sections.get_mut(&(cx, sy, cz)).unwrap();
                            section.sky[Self::index(p)] = 15;
                            section.sky_empty = false;
                            let mut directions = u8::from(y == lowest);
                            for direction in [
                                Direction::North,
                                Direction::South,
                                Direction::West,
                                Direction::East,
                            ] {
                                let q = direction.step(p);
                                if y < self.lowest_source(q.0, q.2) {
                                    directions |= 1 << direction as u8;
                                }
                            }
                            if directions != 0 {
                                sky_queue.push_back(Increase {
                                    pos: p,
                                    level: 15,
                                    directions,
                                    from_empty: false,
                                    from_emission: false,
                                });
                            }
                        }
                    }
                }
            }
        }
        self.propagate(&mut block_queue, false)?;
        self.propagate(&mut sky_queue, true)?;
        Ok(())
    }

    fn lowest_source(&self, x: i32, z: i32) -> i32 {
        self.columns.get(&(x >> 4, z >> 4)).map_or(i32::MIN, |c| {
            c.sources.get((x & 15) as usize, (z & 15) as usize)
        })
    }

    fn propagate(&mut self, queue: &mut VecDeque<Increase>, sky: bool) -> FeatureResult<()> {
        while let Some(Increase {
            pos,
            level: value,
            directions,
            from_empty,
            from_emission,
        }) = queue.pop_front()
        {
            let mut current = if sky {
                self.sky_brightness(pos)
            } else {
                self.block_brightness(pos)
            };
            if from_emission && current < value {
                self.set_stored_level(pos, sky, value);
                current = value;
            }
            if current != value || value <= 1 {
                continue;
            }
            let from = if from_empty {
                crate::block::AIR
            } else {
                self.state(pos)
            };
            let skipped = if sky {
                self.empty_sections_below(pos)
            } else {
                0
            };
            for direction in Direction::ALL {
                if directions & (1 << direction as u8) == 0 {
                    continue;
                }
                let next = direction.step(pos);
                let Some(section) = self.sections.get(&Self::key(next)) else {
                    continue;
                };
                let old = if sky {
                    section.sky[Self::index(next)]
                } else {
                    section.block[Self::index(next)]
                };
                if old >= value - 1 {
                    continue;
                }
                let to = self.state(next);
                let level = value.saturating_sub(state_light(to)?.dampening.max(1));
                if level <= old || face_occludes(from, to, direction)? {
                    continue;
                }
                let section = self.sections.get_mut(&Self::key(next)).unwrap();
                self.changed_columns.insert((next.0 >> 4, next.2 >> 4));
                if sky {
                    section.sky[Self::index(next)] = level;
                    section.sky_empty = false;
                } else {
                    section.block[Self::index(next)] = level;
                    section.block_empty = false;
                }
                let directions = 63 ^ (1 << (direction as u8 ^ 1));
                if level > 1 {
                    queue.push_back(Increase {
                        pos: next,
                        level,
                        directions,
                        from_empty: false,
                        from_emission: false,
                    });
                }
                if sky {
                    self.propagate_empty_sections(next, direction, level, skipped, queue);
                }
            }
        }
        Ok(())
    }

    fn empty_sections_below(&self, pos: Pos) -> i32 {
        if pos.1 & 15 != 0
            || ![pos.0 & 15, pos.2 & 15]
                .into_iter()
                .any(|v| v == 0 || v == 15)
        {
            return 0;
        }
        let bottom = self.sections.keys().map(|k| k.1).min().unwrap_or(i32::MAX);
        let (cx, sy, cz) = Self::key(pos);
        let mut count = 0;
        while sy - count - 1 >= bottom && !self.sections.contains_key(&(cx, sy - count - 1, cz)) {
            count += 1;
        }
        count
    }

    fn propagate_empty_sections(
        &mut self,
        pos: Pos,
        direction: Direction,
        level: u8,
        count: i32,
        queue: &mut VecDeque<Increase>,
    ) {
        if count == 0 {
            return;
        }
        let crossed = match direction {
            Direction::North => pos.2 & 15 == 15,
            Direction::South => pos.2 & 15 == 0,
            Direction::West => pos.0 & 15 == 15,
            Direction::East => pos.0 & 15 == 0,
            _ => false,
        };
        if !crossed {
            return;
        }
        let (cx, sy, cz) = Self::key(pos);
        for y_section in (sy - count..sy).rev() {
            let Some(section) = self.sections.get_mut(&(cx, y_section, cz)) else {
                continue;
            };
            self.changed_columns.insert((cx, cz));
            for y in (y_section * 16..y_section * 16 + 16).rev() {
                let p = (pos.0, y, pos.2);
                section.sky[Self::index(p)] = level;
                section.sky_empty = false;
                if level > 1 {
                    queue.push_back(Increase {
                        pos: p,
                        level,
                        directions: 63 ^ (1 << (direction as u8 ^ 1)),
                        from_empty: true,
                        from_emission: false,
                    });
                }
            }
        }
    }

    /// Packet/persistence handoff includes both native padding sections. The
    /// caller must separately retain the stage claim and initialized sources.
    pub fn chunk_light(&self, pos: ChunkPos) -> ChunkLight {
        fn pack(values: &[u8]) -> Vec<u8> {
            values.chunks_exact(2).map(|v| v[0] | v[1] << 4).collect()
        }
        let min_section_y = self.min_y / 16 - 1;
        let sections = (min_section_y..min_section_y + self.height / 16 + 2)
            .map(|y| match self.sections.get(&(pos.x, y, pos.z)) {
                Some(section) => LightSection {
                    sky: self.has_sky.then(|| pack(&section.sky)),
                    block: Some(pack(&section.block)),
                    sky_empty: self.has_sky && section.sky_empty,
                    block_empty: section.block_empty,
                },
                None => LightSection::default(),
            })
            .collect();
        ChunkLight {
            min_section_y,
            sections,
        }
    }
}

fn section_counts(states: &[u32]) -> Vec<u16> {
    states
        .chunks_exact(4096)
        .map(|s| s.iter().filter(|&&state| !crate::is_air(state)).count() as u16)
        .collect()
}
