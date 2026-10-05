//! World-owned feature storage with source-relative dependency and write guards.
use std::cell::{Ref, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;

use bcore_core::ChunkPos;

use crate::block_entity::BlockEntity;
use crate::feature_world::{FeatureHeightmap, FeatureWorld};
use crate::generation::graph::{chessboard_distance, ChunkPyramid, ChunkStatus};
use crate::ore::OreWorld;
use crate::tick_request::{TickRequest, TickTarget};
use crate::{block, GeneratedChunk, WorldGenerator, MAX_Y, MIN_Y};

const UPDATE_KNOWN_SHAPE: i32 = 16;

#[cfg(test)]
thread_local! {
    static STRUCTURE_WRITE_TRACE: RefCell<Option<Vec<[i32; 5]>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn begin_structure_write_trace() {
    STRUCTURE_WRITE_TRACE.with(|trace| *trace.borrow_mut() = Some(Vec::new()));
}

#[cfg(test)]
pub(crate) fn take_structure_write_trace() -> Vec<[i32; 5]> {
    STRUCTURE_WRITE_TRACE.with(|trace| trace.borrow_mut().take().unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeEffectError {
    InvalidTickRequest([i32; 6]),
    InvalidBeehive { pos: (i32, i32, i32), state: u32 },
}

impl std::fmt::Display for TreeEffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTickRequest(request) => {
                write!(f, "invalid standing-tree tick request {request:?}")
            }
            Self::InvalidBeehive { pos, state } => write!(
                f,
                "standing-tree bee data at {pos:?} is incompatible with block state {state}"
            ),
        }
    }
}

impl std::error::Error for TreeEffectError {}

fn tree_tick_request(row: [i32; 6]) -> Result<TickRequest, TreeEffectError> {
    let [x, y, z, value, delay, fluid] = row;
    let target = match fluid {
        0 => TickTarget::Block(value as u32),
        1 => TickTarget::Fluid(value as u32),
        _ => return Err(TreeEffectError::InvalidTickRequest(row)),
    };
    let request = TickRequest {
        block_pos: [x, y, z],
        target,
        delay,
    };
    if !request.valid_for(ChunkPos::new(x >> 4, z >> 4)) {
        return Err(TreeEffectError::InvalidTickRequest(row));
    }
    Ok(request)
}

// Native BlockState.getPostProcessPos, checked for all 29,873 states without
// allowing world reads. The pinned ranges live in fallen_tree_configs_26_1.json.
fn postprocess_on_write(state: u32, (x, y, z): (i32, i32, i32)) -> Option<(i32, i32, i32)> {
    assert!(
        state < 29_873,
        "block state outside the native 26.1 registry"
    );
    match state {
        2336 | 2337 => Some((x, y, z)),
        6998 | 14845 => Some((x, y + 1, z)),
        _ => None,
    }
}

pub(crate) struct FeatureRegion {
    generator: WorldGenerator,
    biome_zoom: crate::biome_zoom::BiomeZoom,
    chunks: RefCell<BTreeMap<(i32, i32), Arc<GeneratedChunk>>>,
    access: RegionAccess,
    pub(crate) tree_effects: crate::tree::standing::TreeEffects,
    /// Ordered source writes for refreshing readable light inputs and initialized columns.
    pub(crate) light_updates: Vec<((i32, i32, i32), u32)>,
}

enum RegionAccess {
    /// Isolated component fixtures may supply terrain lazily. Live worlds never do.
    #[cfg(test)]
    Fixture,
    Inactive,
    Source {
        source: ChunkPos,
        status: ChunkStatus,
        available: BTreeMap<(i32, i32), ChunkStatus>,
    },
}

#[cfg(test)]
impl Default for RegionAccess {
    fn default() -> Self {
        Self::Fixture
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegionError {
    pub destination: ChunkPos,
    pub requested: ChunkStatus,
    pub allowed: Option<ChunkStatus>,
    pub available: Option<ChunkStatus>,
}

impl FeatureRegion {
    pub(crate) fn apply_template_effect(
        &mut self,
        source: ChunkPos,
        effect: crate::structure::template::TemplateEffect,
    ) -> Result<(), crate::feature_world::FeatureError> {
        use crate::feature_world::FeatureError;
        use crate::structure::template::TemplateEffect;
        match effect {
            TemplateEffect::ClearBlockEntity((x, y, z)) => {
                self.check_access(ChunkPos::new(x >> 4, z >> 4), ChunkStatus::Empty)
                    .map_err(|error| {
                        FeatureError::MissingData(format!("structure entity access: {error:?}"))
                    })?;
                self.tree_effects.beehives.remove(&(x, y, z));
                let chunk = self.chunk_mut(x >> 4, z >> 4);
                let local = ((x & 15) as usize, y, (z & 15) as usize);
                chunk.block_entities.remove(&local);
                chunk.feature_block_entities.remove(&local);
            }
            TemplateEffect::BlockEntity(entity) => {
                let (x, y, z) = entity.pos;
                let data = crate::generation::FeatureBlockEntity::from_template(&entity)?;
                let state = OreWorld::get_block(self, entity.pos).ok_or_else(|| {
                    FeatureError::MissingData(format!("structure block entity at {:?}", entity.pos))
                })?;
                if !data.matches_state(state) {
                    return Err(FeatureError::InvalidConfig(format!(
                        "structure block entity incompatible with {state} at {:?}",
                        entity.pos
                    )));
                }
                let chunk = self.chunk_mut(x >> 4, z >> 4);
                let local = ((x & 15) as usize, y, (z & 15) as usize);
                chunk.block_entities.remove(&local);
                chunk.feature_block_entities.insert(local, data);
            }
            TemplateEffect::Entity(entity) => {
                let request =
                    crate::generation::StructureEntityRequest::from_template(entity, source);
                let p = request.position();
                let owner = ChunkPos::new(p[0].floor() as i32 >> 4, p[2].floor() as i32 >> 4);
                if !request.valid_for(owner) {
                    return Err(FeatureError::InvalidConfig(
                        "invalid STRUCTURE entity request".into(),
                    ));
                }
                self.check_access(owner, ChunkStatus::Empty)
                    .map_err(|error| {
                        FeatureError::MissingData(format!("structure entity access: {error:?}"))
                    })?;
                self.chunk_mut(owner.x, owner.z)
                    .structure_entities
                    .push(request);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn new(generator: WorldGenerator, center: Arc<GeneratedChunk>) -> Self {
        Self {
            generator,
            biome_zoom: crate::biome_zoom::BiomeZoom::new(generator.seed),
            chunks: RefCell::new(BTreeMap::from([((center.pos.x, center.pos.z), center)])),
            access: RegionAccess::Fixture,
            tree_effects: Default::default(),
            light_updates: Vec::new(),
        }
    }

    pub(crate) fn shared(generator: WorldGenerator) -> Self {
        Self {
            generator,
            biome_zoom: crate::biome_zoom::BiomeZoom::new(generator.seed()),
            chunks: RefCell::new(BTreeMap::new()),
            access: RegionAccess::Inactive,
            tree_effects: Default::default(),
            light_updates: Vec::new(),
        }
    }

    pub(crate) fn world_seed(&self) -> i64 {
        self.generator.seed()
    }

    pub(crate) fn begin_source(
        &mut self,
        source: ChunkPos,
        status: ChunkStatus,
        available: BTreeMap<(i32, i32), ChunkStatus>,
    ) {
        assert!(
            matches!(self.access, RegionAccess::Inactive),
            "source writes already claimed"
        );
        self.access = RegionAccess::Source {
            source,
            status,
            available,
        };
    }

    pub(crate) fn end_source(&mut self) {
        self.access = RegionAccess::Inactive;
    }

    pub(crate) fn owned_chunk(&self, pos: ChunkPos) -> Option<Arc<GeneratedChunk>> {
        self.chunks.borrow().get(&(pos.x, pos.z)).cloned()
    }

    pub(crate) fn beehive_snapshots(&self) -> BTreeMap<(i32, i32, i32), Vec<i32>> {
        let mut hives = BTreeMap::new();
        for (&(cx, cz), chunk) in self.chunks.borrow().iter() {
            for (&(x, y, z), entity) in chunk.block_entities() {
                if let BlockEntity::Beehive { ticks_in_hive } = entity {
                    hives.insert(
                        (cx * 16 + x as i32, y, cz * 16 + z as i32),
                        ticks_in_hive.clone(),
                    );
                }
            }
        }
        hives.extend(
            self.tree_effects
                .beehives
                .iter()
                .map(|(&pos, bees)| (pos, bees.clone())),
        );
        hives
    }

    pub(crate) fn owned_chunk_mut(&mut self, pos: ChunkPos) -> &mut GeneratedChunk {
        Arc::make_mut(
            self.chunks
                .get_mut()
                .entry((pos.x, pos.z))
                .or_insert_with(|| Arc::new(GeneratedChunk::new(pos))),
        )
    }

    fn check_access(
        &self,
        destination: ChunkPos,
        requested: ChunkStatus,
    ) -> Result<(), RegionError> {
        let (allowed, available) = match &self.access {
            #[cfg(test)]
            RegionAccess::Fixture => return Ok(()),
            RegionAccess::Inactive => (None, None),
            RegionAccess::Source {
                source,
                status,
                available,
            } => (
                ChunkPyramid::Generation
                    .step(*status)
                    .direct
                    .at(*source, destination),
                available.get(&(destination.x, destination.z)).copied(),
            ),
        };
        if allowed.is_some_and(|s| s >= requested) && available.is_some_and(|s| s >= requested) {
            Ok(())
        } else {
            Err(RegionError {
                destination,
                requested,
                allowed,
                available,
            })
        }
    }

    pub(crate) fn chunk_at_status(
        &self,
        pos: ChunkPos,
        requested: ChunkStatus,
    ) -> Result<Ref<'_, GeneratedChunk>, RegionError> {
        self.check_access(pos, requested)?;
        if !self.chunks.borrow().contains_key(&(pos.x, pos.z)) {
            let chunk = match self.access {
                #[cfg(test)]
                RegionAccess::Fixture => crate::terrain_cache::get(self.generator, pos),
                // A retained EMPTY/STARTS holder has no terrain. Never replace it
                // with a fully carved chunk merely because a feature reads it.
                _ => Arc::new(GeneratedChunk::new(pos)),
            };
            self.chunks.borrow_mut().insert((pos.x, pos.z), chunk);
        }
        Ok(Ref::map(self.chunks.borrow(), |chunks| {
            chunks[&(pos.x, pos.z)].as_ref()
        }))
    }

    fn chunk(&self, x: i32, z: i32) -> Ref<'_, GeneratedChunk> {
        self.chunk_at_status(ChunkPos::new(x, z), ChunkStatus::Empty)
            .expect("feature read outside the source's direct dependencies")
    }

    pub fn biome_at(&self, pos: (i32, i32, i32)) -> u32 {
        let (qx, qy, qz) = self.biome_zoom.quart_at(pos);
        let (x, y, z) = (qx * 4, qy.clamp(MIN_Y >> 2, MAX_Y >> 2) * 4, qz * 4);
        self.chunk_at_status(ChunkPos::new(x >> 4, z >> 4), ChunkStatus::Biomes)
            .expect("biome read outside the source's BIOMES dependencies")
            .noise_biome_at((x & 15) as usize, y.clamp(MIN_Y, MAX_Y), (z & 15) as usize)
    }

    fn chunk_mut(&mut self, x: i32, z: i32) -> &mut GeneratedChunk {
        drop(self.chunk(x, z));
        self.owned_chunk_mut(ChunkPos::new(x, z))
    }

    /// Proto-chunk feature writes. WorldGenRegion tests bit 16 before asking the
    /// new state for a postprocessing position; it does not run live neighbour
    /// updates or send client packets for bits 1/2 at this generation stage.
    fn set_block_with_flags(&mut self, pos: (i32, i32, i32), state: u32, flags: i32) -> bool {
        if !OreWorld::set_block(self, pos, state) {
            return false;
        }
        #[cfg(test)]
        STRUCTURE_WRITE_TRACE.with(|trace| {
            if let Some(writes) = &mut *trace.borrow_mut() {
                writes.push([pos.0, pos.1, pos.2, state as i32, flags]);
            }
        });
        if flags & UPDATE_KNOWN_SHAPE == 0 {
            if let Some(pos) = postprocess_on_write(state, pos) {
                self.mark_postprocessing(pos);
            }
        }
        true
    }

    fn mark_postprocessing(&mut self, (x, y, z): (i32, i32, i32)) {
        if (MIN_Y..=MAX_Y).contains(&y) {
            self.chunk_mut(x >> 4, z >> 4).postprocessing.push((
                (x & 15) as usize,
                y,
                (z & 15) as usize,
            ));
        }
    }

    /// Transfer staged snapshots and raw requests without executing or deduplicating
    /// ticks. Invalid batches leave all staged and stored effects intact for retry.
    pub(crate) fn transfer_tree_effects(&mut self) -> Result<(), TreeEffectError> {
        let requests = self
            .tree_effects
            .tick_requests
            .iter()
            .copied()
            .map(tree_tick_request)
            .collect::<Result<Vec<_>, _>>()?;
        let hive = BlockEntity::Beehive {
            ticks_in_hive: Vec::new(),
        };
        for &pos in self.tree_effects.beehives.keys() {
            let state =
                OreWorld::get_block(self, pos).expect("tree-effect region supplies every block");
            if !hive.matches_state(state) {
                return Err(TreeEffectError::InvalidBeehive { pos, state });
            }
        }

        // Validation precedes draining: a malformed late request must not consume
        // an earlier hive or append a request that would be duplicated on retry.
        for ((x, y, z), ticks_in_hive) in std::mem::take(&mut self.tree_effects.beehives) {
            assert!(
                self.chunk_mut(x >> 4, z >> 4).set_block_entity(
                    (x & 15) as usize,
                    y,
                    (z & 15) as usize,
                    BlockEntity::Beehive { ticks_in_hive },
                ),
                "validated standing-tree hive must remain compatible"
            );
        }
        for request in requests {
            let [x, _, z] = request.block_pos;
            assert!(
                self.chunk_mut(x >> 4, z >> 4).add_tick_request(request),
                "validated standing-tree request must belong to its destination chunk"
            );
        }
        self.tree_effects.tick_requests.clear();
        Ok(())
    }

    #[cfg(test)]
    pub fn finish_chunk(mut self, pos: ChunkPos) -> GeneratedChunk {
        self.transfer_tree_effects()
            .expect("invalid staged standing-tree effects");
        let mut marked = std::mem::take(&mut self.chunk_mut(pos.x, pos.z).postprocessing);
        // Native sections are processed bottom-up; marks within a section keep
        // their insertion order. Neighbours already contain structure writes.
        // The current consumer updates only the supported mineshaft fences and
        // wall torches in this target chunk. It does not execute fluid/block
        // ticks or general neighbour updates for the other queued positions.
        marked.sort_by_key(|&(_, y, _)| y >> 4);
        for (x, y, z) in marked {
            let p = [pos.x * 16 + x as i32, y, pos.z * 16 + z as i32];
            let state = crate::structure::mineshaft::blocks::updated_shape(&self, p);
            if self.get_block((p[0], p[1], p[2])) != Some(state) {
                self.set_block((p[0], p[1], p[2]), state);
            }
        }
        Arc::unwrap_or_clone(
            self.chunks
                .into_inner()
                .remove(&(pos.x, pos.z))
                .expect("feature target chunk is present"),
        )
    }
}

/// Live fallen-tree world backed by the same world-owned chunks as ores and
/// structures. Call `tree::fallen::place(&mut region, &mut random, ...)`
/// with the caller's existing feature RNG; there is no adapter-owned random state.
impl crate::tree::fallen::FallenTreeWorld for FeatureRegion {
    fn get_block(&self, pos: (i32, i32, i32)) -> u32 {
        OreWorld::get_block(self, pos).expect("fallen-tree region missing a required block")
    }

    fn set_block(&mut self, pos: (i32, i32, i32), state: u32, flags: i32) -> bool {
        self.set_block_with_flags(pos, state, flags)
    }

    fn is_face_sturdy_up(&self, state: u32, _at: (i32, i32, i32)) -> bool {
        // Also checked natively at three positions with no shape-changing block
        // entities; the generated chest, spawner and hive data do not change it.
        crate::structure::mineshaft::blocks::is_face_sturdy_up(state)
    }

    fn mark_for_postprocessing(&mut self, pos: (i32, i32, i32)) {
        self.mark_postprocessing(pos);
    }
}

impl crate::decoration::TreeFeatureWorld for FeatureRegion {
    fn tree_height(&self, heightmap: crate::decoration::TreeHeightmap, x: i32, z: i32) -> i32 {
        let heights = crate::heightmap::placement_heights(
            &self.chunk(x >> 4, z >> 4),
            (x & 15) as usize,
            (z & 15) as usize,
        );
        match heightmap {
            crate::decoration::TreeHeightmap::OceanFloor => heights.ocean_floor,
            crate::decoration::TreeHeightmap::WorldSurface => heights.world_surface,
        }
    }

    fn tree_biome(&self, pos: crate::tree::fallen::Pos) -> u32 {
        self.biome_at(pos)
    }

    fn can_place_tree(&self, pos: crate::tree::fallen::Pos) -> bool {
        self.can_write_feature(pos)
    }

    fn place_standing_tree<R: crate::tree::TreeRandom + ?Sized>(
        &mut self,
        random: &mut R,
        configured_feature: &'static str,
        origin: crate::tree::fallen::Pos,
    ) -> Result<bool, crate::decoration::UnsupportedTree> {
        crate::tree::standing::place(self, random, configured_feature, origin)
            .map_err(|shape| crate::decoration::UnsupportedTree {
                configured_feature,
                origin,
                unsupported_shape: Some(shape),
            })?
            .ok_or(crate::decoration::UnsupportedTree {
                configured_feature,
                origin,
                unsupported_shape: None,
            })
    }
}

impl crate::tree::standing::StandingTreeWorld for FeatureRegion {
    fn has_beehive(&mut self, pos @ (x, y, z): crate::tree::fallen::Pos) -> bool {
        let hive = BlockEntity::Beehive {
            ticks_in_hive: Vec::new(),
        };
        if !OreWorld::get_block(self, pos).is_some_and(|state| hive.matches_state(state)) {
            return false;
        }
        if !self.tree_effects.beehives.contains_key(&pos) {
            // A staged hive is a complete snapshot, including occupants already
            // persisted by an earlier transfer in the same region.
            let ticks = match self.chunk(x >> 4, z >> 4).block_entities().get(&(
                (x & 15) as usize,
                y,
                (z & 15) as usize,
            )) {
                Some(BlockEntity::Beehive { ticks_in_hive }) => ticks_in_hive.clone(),
                None => Vec::new(),
                Some(_) => panic!("incompatible block entity at a standing-tree hive"),
            };
            self.tree_effects.beehives.insert(pos, ticks);
        }
        true
    }

    fn store_bee(&mut self, pos: crate::tree::fallen::Pos, ticks_in_hive: i32) {
        assert!(self.has_beehive(pos), "native bee nest exists");
        self.tree_effects
            .beehives
            .get_mut(&pos)
            .expect("native bee nest exists")
            .push(ticks_in_hive);
    }

    fn schedule_tree_tick(&mut self, request: [i32; 6]) {
        self.tree_effects.tick_requests.push(request);
    }
}

impl OreWorld for FeatureRegion {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        let chunk = self.chunk(x >> 4, z >> 4);
        if let Some(heights) = chunk.worldgen_heightmaps() {
            return heights.ocean_floor[(z & 15) as usize * 16 + (x & 15) as usize];
        }
        // Explicit fixture worlds without staged WG maps keep their live contract.
        crate::heightmap::placement_heights(&chunk, (x & 15) as usize, (z & 15) as usize)
            .ocean_floor
    }

    fn get_block(&self, (x, y, z): (i32, i32, i32)) -> Option<u32> {
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return Some(block::AIR);
        }
        self.chunk(x >> 4, z >> 4)
            .get((x & 15) as usize, y, (z & 15) as usize)
    }

    fn set_block(&mut self, (x, y, z): (i32, i32, i32), state: u32) -> bool {
        if !self.can_write_feature((x, y, z)) {
            return false;
        }
        let track_light = matches!(self.access, RegionAccess::Source { .. });
        // Native proto writes retain instantiated hives across compatible state
        // changes. Staged occupants represent that same live entity.
        let hive = BlockEntity::Beehive {
            ticks_in_hive: Vec::new(),
        };
        if !hive.matches_state(state) {
            self.tree_effects.beehives.remove(&(x, y, z));
        }
        let local = ((x & 15) as usize, y, (z & 15) as usize);
        let chunk = self.chunk_mut(x >> 4, z >> 4);
        if !chunk.set(local.0, y, local.2, state) {
            return false;
        }
        if !chunk.feature_block_entities.contains_key(&local) {
            if let Some(data) = crate::generation::sculk_block_entity(state, (x, y, z))
                .expect("native generated sculk entity data")
            {
                chunk.feature_block_entities.insert(local, data);
            }
        }
        if track_light {
            self.light_updates.push(((x, y, z), state));
        }
        true
    }
}

impl crate::dungeon::DungeonWorld for FeatureRegion {
    fn set_block_entity(
        &mut self,
        (x, y, z): (i32, i32, i32),
        data: crate::block_entity::BlockEntity,
    ) {
        let chunk = self.chunk_mut(x >> 4, z >> 4);
        assert!(data.matches_state(
            chunk
                .get((x & 15) as usize, y, (z & 15) as usize)
                .expect("block entity inside world")
        ));
        chunk
            .block_entities
            .insert(((x & 15) as usize, y, (z & 15) as usize), data);
    }
}

impl crate::structure::mineshaft::blocks::MineshaftWorld for FeatureRegion {
    fn mineshaft_blocked_biome(&self, [x, y, z]: [i32; 3]) -> bool {
        crate::biome::name(self.biome_at((x, y, z))) == "deep_dark"
    }

    fn add_entity(&mut self, entity: crate::generated_entity::GeneratedEntity) {
        let [x, _, z] = entity.block_pos();
        self.chunk_mut(x >> 4, z >> 4).entities.push(entity);
    }

    fn mark_for_postprocessing(&mut self, [x, y, z]: [i32; 3]) {
        self.mark_postprocessing((x, y, z));
    }
}

impl FeatureWorld for FeatureRegion {
    fn feature_biome(&self, pos: (i32, i32, i32)) -> u32 {
        self.biome_at(pos)
    }

    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        let chunk = self.chunk(x >> 4, z >> 4);
        if let Some(heights) = chunk.worldgen_heightmaps() {
            let index = (z & 15) as usize * 16 + (x & 15) as usize;
            match kind {
                FeatureHeightmap::WorldSurfaceWg => return heights.world_surface[index],
                FeatureHeightmap::OceanFloorWg => return heights.ocean_floor[index],
                _ => {}
            }
        }
        let heights =
            crate::heightmap::placement_heights(&chunk, (x & 15) as usize, (z & 15) as usize);
        match kind {
            FeatureHeightmap::WorldSurface | FeatureHeightmap::WorldSurfaceWg => {
                heights.world_surface
            }
            FeatureHeightmap::OceanFloor | FeatureHeightmap::OceanFloorWg => heights.ocean_floor,
            FeatureHeightmap::MotionBlockingNoLeaves => {
                crate::tree::standing::StandingTreeWorld::motion_no_leaves_height(self, x, z)
            }
            FeatureHeightmap::MotionBlocking => heights
                .ocean_floor
                .max(crate::tree::standing::StandingTreeWorld::motion_no_leaves_height(self, x, z)),
        }
    }

    fn can_write_feature(&self, (x, y, z): (i32, i32, i32)) -> bool {
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return false;
        }
        match &self.access {
            #[cfg(test)]
            RegionAccess::Fixture => true,
            RegionAccess::Inactive => false,
            RegionAccess::Source {
                source,
                status,
                available,
            } => {
                let radius = ChunkPyramid::Generation
                    .step(*status)
                    .block_state_write_radius;
                radius >= 0
                    && chessboard_distance(*source, ChunkPos::new(x >> 4, z >> 4)) <= radius as u32
                    && available.get(&(x >> 4, z >> 4)).is_some_and(|&actual| {
                        ChunkPyramid::Generation
                            .step(*status)
                            .direct
                            .at(*source, ChunkPos::new(x >> 4, z >> 4))
                            .is_some_and(|required| actual >= required)
                    })
            }
        }
    }

    fn set_feature_block(&mut self, pos: (i32, i32, i32), state: u32, flags: i32) -> bool {
        self.set_block_with_flags(pos, state, flags)
    }

    fn mark_feature_postprocessing(&mut self, pos: (i32, i32, i32)) {
        self.mark_postprocessing(pos);
    }

    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        let [x, _, z] = request.block_pos;
        let owner = ChunkPos::new(x >> 4, z >> 4);
        if !request.valid_for(owner) || self.check_access(owner, ChunkStatus::Empty).is_err() {
            return false;
        }
        self.chunk_mut(owner.x, owner.z).add_tick_request(request)
    }
}

#[cfg(test)]
#[path = "region/fallen_tests.rs"]
mod fallen_tests;

#[cfg(test)]
#[path = "region/vegetation_tests.rs"]
mod vegetation_tests;

#[cfg(test)]
#[path = "region/standing_tests.rs"]
mod standing_tests;

#[cfg(test)]
#[path = "region/tree_effects_tests.rs"]
mod tree_effects_tests;

#[cfg(test)]
#[path = "region/access_tests.rs"]
mod access_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_mineshaft_entities_reach_their_owning_chunk_without_changing_cached_terrain() {
        use crate::structure::mineshaft::{region::RegionPlan, MineshaftLayout};
        use serde_json::json;
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../data/mineshaft_regions_26_1.json")).unwrap();
        let sample = &data["samples"][0];
        let seed = sample["seed"].as_i64().unwrap();
        let source_keys: std::collections::BTreeSet<_> = sample["starts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["source"][0].as_i64().unwrap() as i32,
                    s["source"][1].as_i64().unwrap() as i32,
                )
            })
            .collect();
        for field in ["minecarts", "block_entities"] {
            let expected = sample["chunks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| !c[field].as_array().unwrap().is_empty())
                .unwrap();
            let target = ChunkPos::new(
                expected["chunk"][0].as_i64().unwrap() as i32,
                expected["chunk"][1].as_i64().unwrap() as i32,
            );
            let generator = WorldGenerator::new(seed);
            let mut plan = RegionPlan::new([target], |p| {
                if !source_keys.contains(&(p.x, p.z)) {
                    return None;
                }
                MineshaftLayout::for_chunk(
                    seed,
                    p,
                    |_| crate::biome::ids::PLAINS,
                    |x, z| generator.base_height_vanilla(x, z),
                )
            });
            let mut chunks = BTreeMap::new();
            for pos in plan.chunks() {
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        chunks.entry((pos.x + dx, pos.z + dz)).or_insert_with(|| {
                            let mut chunk =
                                GeneratedChunk::new(ChunkPos::new(pos.x + dx, pos.z + dz));
                            chunk.states.fill(block::STONE);
                            Arc::new(chunk)
                        });
                    }
                }
            }
            let bases = chunks.clone();
            let mut region = FeatureRegion {
                generator,
                biome_zoom: crate::biome_zoom::BiomeZoom::new(seed),
                chunks: RefCell::new(chunks),
                access: Default::default(),
                tree_effects: Default::default(),
                light_updates: Vec::new(),
            };
            plan.place(seed, &mut region);
            region.chunk_mut(target.x, target.z).structures = plan.structure_data(target);
            let chunk = region.finish_chunk(target);
            let carts: Vec<_> = chunk
                .entities()
                .iter()
                .map(|entity| match entity {
                    crate::generated_entity::GeneratedEntity::ChestMinecart {
                        loot_seed, ..
                    } => json!({"pos": entity.position(),"loot_seed":loot_seed}),
                })
                .collect();
            assert_eq!(json!(carts), expected["minecarts"]);
            let bes: Vec<_> = chunk
                .block_entities()
                .iter()
                .map(|(&(x, y, z), be)| {
                    be.full_data((target.x * 16 + x as i32, y, target.z * 16 + z as i32))
                })
                .collect();
            assert_eq!(json!(bes), expected["block_entities"]);
            assert!(chunk.structures().valid_for(target));
            assert!(bases.values().all(|c| c.entities().is_empty()
                && c.block_entities().is_empty()
                && c.states().iter().all(|&s| s == block::STONE)));
        }
    }

    #[test]
    fn native_dungeon_entities_cross_chunk_edges_without_mutating_base_chunks() {
        let mut chunks = BTreeMap::new();
        for cx in -1..=0 {
            for cz in 0..=1 {
                let mut chunk = GeneratedChunk::new(ChunkPos::new(cx, cz));
                chunk.states.fill(block::STONE);
                if cz == 1 {
                    for x in 0..16 {
                        for y in [-63, -62] {
                            chunk.set(x, y, 0, block::AIR);
                        }
                    }
                }
                chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
        let bases = chunks.clone();
        let mut region = FeatureRegion {
            generator: WorldGenerator::new(0),
            biome_zoom: crate::biome_zoom::BiomeZoom::new(0),
            chunks: RefCell::new(chunks),
            access: Default::default(),
            tree_effects: Default::default(),
            light_updates: Vec::new(),
        };
        assert!(crate::dungeon::place(
            &mut region,
            &mut crate::simplex::WorldgenRandom::new(0),
            (-1, -63, 16)
        ));
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../data/monster_rooms_26_1.json")).unwrap();
        let sample = reference["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| {
                s["seed"] == 0
                    && s["terrain"] == "tunnel"
                    && s["origin"] == serde_json::json!([-1, -63, 16])
            })
            .unwrap();
        let expected: BTreeMap<_, _> = sample["block_entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|be| {
                let p = &be["pos"];
                (
                    (
                        p[0].as_i64().unwrap() as i32,
                        p[1].as_i64().unwrap() as i32,
                        p[2].as_i64().unwrap() as i32,
                    ),
                    be["nbt"].clone(),
                )
            })
            .collect();
        let mut actual = BTreeMap::new();
        for ((cx, cz), chunk) in region.chunks.into_inner() {
            for (&(lx, y, lz), data) in chunk.block_entities() {
                let pos = (cx * 16 + lx as i32, y, cz * 16 + lz as i32);
                actual.insert(pos, data.full_data(pos));
            }
        }
        assert_eq!(actual, expected);
        assert!(bases
            .values()
            .all(|chunk| chunk.block_entities().is_empty()));
        assert_eq!(bases[&(-1, 1)].get(15, -63, 0), Some(block::AIR));
    }
}
