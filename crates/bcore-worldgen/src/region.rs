//! Mutable feature region backed by lazily generated terrain/carver chunks.
use std::cell::{Ref, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;

use bcore_core::ChunkPos;

use crate::ore::OreWorld;
use crate::{block, GeneratedChunk, WorldGenerator, MAX_Y, MIN_Y};

const UPDATE_KNOWN_SHAPE: i32 = 16;

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
    pub(crate) tree_effects: crate::tree::standing::TreeEffects,
}

impl FeatureRegion {
    pub fn new(generator: WorldGenerator, center: Arc<GeneratedChunk>) -> Self {
        Self {
            generator,
            biome_zoom: crate::biome_zoom::BiomeZoom::new(generator.seed),
            chunks: RefCell::new(BTreeMap::from([((center.pos.x, center.pos.z), center)])),
            tree_effects: Default::default(),
        }
    }

    fn chunk(&self, x: i32, z: i32) -> Ref<'_, GeneratedChunk> {
        if !self.chunks.borrow().contains_key(&(x, z)) {
            let chunk = crate::terrain_cache::get(self.generator, ChunkPos::new(x, z));
            self.chunks.borrow_mut().insert((x, z), chunk);
        }
        Ref::map(self.chunks.borrow(), |chunks| chunks[&(x, z)].as_ref())
    }

    pub fn biome_at(&self, pos: (i32, i32, i32)) -> u32 {
        let (qx, qy, qz) = self.biome_zoom.quart_at(pos);
        let (x, y, z) = (qx * 4, qy.clamp(MIN_Y >> 2, MAX_Y >> 2) * 4, qz * 4);
        self.chunk(x >> 4, z >> 4).noise_biome_at(
            (x & 15) as usize,
            y.clamp(MIN_Y, MAX_Y),
            (z & 15) as usize,
        )
    }

    fn chunk_mut(&mut self, x: i32, z: i32) -> &mut GeneratedChunk {
        drop(self.chunk(x, z));
        Arc::make_mut(self.chunks.get_mut().get_mut(&(x, z)).unwrap())
    }

    /// Proto-chunk feature writes. WorldGenRegion tests bit 16 before asking the
    /// new state for a postprocessing position; it does not run live neighbour
    /// updates or send client packets for bits 1/2 at this generation stage.
    fn set_block_with_flags(&mut self, pos: (i32, i32, i32), state: u32, flags: i32) -> bool {
        if !OreWorld::set_block(self, pos, state) {
            return false;
        }
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

    pub fn place_mineshafts(&mut self, sources: impl IntoIterator<Item = ChunkPos>) {
        use crate::structure::mineshaft::{region::RegionPlan, MineshaftLayout};
        let generator = self.generator;
        let mut plan = RegionPlan::new(sources, |pos| {
            MineshaftLayout::for_chunk(
                generator.seed(),
                pos,
                |p| generator.noise_biome_vanilla(p),
                |x, z| generator.base_height_vanilla(x, z),
            )
        });
        plan.place(generator.seed(), self);
        for pos in plan.chunks() {
            let data = plan.structure_data(pos);
            if !data.is_empty() {
                self.chunk_mut(pos.x, pos.z).structures = data;
            }
        }
    }

    pub fn finish_chunk(mut self, pos: ChunkPos) -> GeneratedChunk {
        assert!(
            self.tree_effects.beehives.is_empty() && self.tree_effects.tick_requests.is_empty(),
            "transfer standing-tree bee/tick effects before finishing the region"
        );
        let generator = self.generator;
        generator.decorate_vanilla(self.chunk_mut(pos.x, pos.z));
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
            self.set_block((p[0], p[1], p[2]), state);
        }
        Arc::unwrap_or_clone(
            self.chunks
                .into_inner()
                .remove(&(pos.x, pos.z))
                .expect("feature target chunk is present"),
        )
    }
}

/// Live fallen-tree world backed by the same lazy, copy-on-write region cache as
/// ores and structures. Call `tree::fallen::place(&mut region, &mut random, ...)`
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
        // entities. This region currently represents only chest/spawner data.
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

    fn can_place_tree(&self, _pos: crate::tree::fallen::Pos) -> bool {
        // This region lazily supplies every horizontal neighbour. Source status
        // dependencies and vanilla's write radius belong to the future scheduler.
        true
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
    fn has_beehive(&mut self, pos: crate::tree::fallen::Pos) -> bool {
        if OreWorld::get_block(self, pos) != Some(crate::tree::standing::bee_nest_state()) {
            return false;
        }
        self.tree_effects.beehives.entry(pos).or_default();
        true
    }

    fn store_bee(&mut self, pos: crate::tree::fallen::Pos, ticks_in_hive: i32) {
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
        // Scan the actual column, including changes made by previous features.
        crate::heightmap::placement_heights(
            &self.chunk(x >> 4, z >> 4),
            (x & 15) as usize,
            (z & 15) as usize,
        )
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
        if !(MIN_Y..=MAX_Y).contains(&y) {
            return false;
        }
        self.tree_effects.beehives.remove(&(x, y, z));
        self.chunk_mut(x >> 4, z >> 4)
            .set((x & 15) as usize, y, (z & 15) as usize, state);
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
        self.chunk_mut(x >> 4, z >> 4).postprocessing.push((
            (x & 15) as usize,
            y,
            (z & 15) as usize,
        ));
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
                tree_effects: Default::default(),
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
            tree_effects: Default::default(),
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
