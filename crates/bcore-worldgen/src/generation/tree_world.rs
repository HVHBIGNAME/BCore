//! Tree capabilities across the placed dispatcher's object-safe world boundary.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::block_entity::BlockEntity;
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::ore::OreWorld;
use crate::region::FeatureRegion;
use crate::simplex::WorldgenRandom;
use crate::tick_request::TickRequest;
use crate::tree::fallen::FallenTreeWorld;
use crate::tree::standing::StandingTreeWorld;

#[derive(Default)]
pub(super) struct TreeEffects {
    hives: BTreeMap<Pos, Vec<i32>>,
    changed: BTreeSet<Pos>,
    ticks: Vec<[i32; 6]>,
}
pub(super) type SharedTreeEffects = Rc<RefCell<TreeEffects>>;

/// Every block write passes this view, so removing and recreating a hive cannot
/// resurrect a snapshot of its old occupants. Drop also flushes during unwinding.
pub(super) struct EffectWorld<'a> {
    region: &'a mut FeatureRegion,
    pub effects: SharedTreeEffects,
}

impl<'a> EffectWorld<'a> {
    pub fn new(region: &'a mut FeatureRegion) -> Self {
        let hives = region.beehive_snapshots();
        Self {
            region,
            effects: Rc::new(RefCell::new(TreeEffects {
                hives,
                ..Default::default()
            })),
        }
    }

    fn written(&self, pos: Pos, state: u32, accepted: bool) -> bool {
        if accepted
            && !(BlockEntity::Beehive {
                ticks_in_hive: Vec::new(),
            })
            .matches_state(state)
        {
            let mut effects = self.effects.borrow_mut();
            effects.hives.remove(&pos);
            effects.changed.remove(&pos);
        }
        accepted
    }
}

impl Drop for EffectWorld<'_> {
    fn drop(&mut self) {
        let mut effects = self.effects.borrow_mut();
        for pos in std::mem::take(&mut effects.changed) {
            if let Some(bees) = effects.hives.remove(&pos) {
                self.region.tree_effects.beehives.insert(pos, bees);
            }
        }
        self.region
            .tree_effects
            .tick_requests
            .append(&mut effects.ticks);
    }
}

impl OreWorld for EffectWorld<'_> {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.region.ocean_floor_wg(x, z)
    }
    fn get_block(&self, pos: Pos) -> Option<u32> {
        OreWorld::get_block(self.region, pos)
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        let accepted = OreWorld::set_block(self.region, pos, state);
        self.written(pos, state, accepted)
    }
}

impl FeatureWorld for EffectWorld<'_> {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.region.feature_biome(pos)
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.region.feature_height(kind, x, z)
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        self.region.can_write_feature(pos)
    }
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        let accepted = self.region.set_feature_block(pos, state, flags);
        self.written(pos, state, accepted)
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        self.region.mark_feature_postprocessing(pos);
    }
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool {
        self.region.schedule_feature_tick(request)
    }
}

pub(super) fn configured_name(
    document: &serde_json::Value,
    name: Option<&str>,
) -> Result<String, FeatureError> {
    let name = name
        .map(str::to_owned)
        .or_else(|| {
            crate::block_predicate::catalog().documents["configured_feature"]
                .as_object()?
                .iter()
                .find(|(_, known)| *known == document)
                .map(|(name, _)| name.clone())
        })
        .ok_or_else(|| {
            FeatureError::Unsupported("unregistered inline tree configuration".into())
        })?;
    let supported = match document["type"].as_str() {
        Some("minecraft:tree") => crate::tree::standing::config_for(&name).is_some(),
        Some("minecraft:fallen_tree") => crate::tree::fallen::config_for(&name).is_some(),
        _ => false,
    };
    if supported {
        Ok(name)
    } else {
        Err(FeatureError::Unsupported(format!("configured tree {name}")))
    }
}

pub(super) fn place(
    document: &serde_json::Value,
    name: Option<&str>,
    world: &mut dyn FeatureWorld,
    random: &mut WorldgenRandom,
    origin: Pos,
    effects: SharedTreeEffects,
) -> Result<bool, FeatureError> {
    let name = configured_name(document, name)?;
    let mut tree = TreeWorld { world, effects };
    if let Some(config) = crate::tree::fallen::config_for(&name) {
        return Ok(crate::tree::fallen::place(
            &mut tree, random, &config, origin,
        ));
    }
    crate::tree::standing::place(&mut tree, random, &name, origin)
        .map_err(|error| FeatureError::Unsupported(error.to_string()))?
        .ok_or_else(|| FeatureError::Unsupported(format!("configured tree {name}")))
}

struct TreeWorld<'a> {
    world: &'a mut dyn FeatureWorld,
    effects: SharedTreeEffects,
}

pub(super) fn update_template_shapes(
    world: &mut dyn FeatureWorld,
    effects: SharedTreeEffects,
    positions: &[Pos],
    flags: i32,
) -> Result<(), FeatureError> {
    crate::tree::standing::update_template_shapes(
        &mut TreeWorld { world, effects },
        positions,
        flags,
    )
    .map_err(|error| FeatureError::Unsupported(error.to_string()))
}

impl FallenTreeWorld for TreeWorld<'_> {
    fn get_block(&self, pos: Pos) -> u32 {
        self.world.get_block(pos).expect("guarded tree block read")
    }
    fn set_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool {
        self.world.set_feature_block(pos, state, flags)
    }
    fn is_face_sturdy_up(&self, state: u32, _pos: Pos) -> bool {
        crate::structure::mineshaft::blocks::is_face_sturdy_up(state)
    }
    fn mark_for_postprocessing(&mut self, pos: Pos) {
        self.world.mark_feature_postprocessing(pos);
    }
}

impl StandingTreeWorld for TreeWorld<'_> {
    fn has_beehive(&mut self, pos: Pos) -> bool {
        let hive = BlockEntity::Beehive {
            ticks_in_hive: Vec::new(),
        };
        if !self
            .world
            .get_block(pos)
            .is_some_and(|state| hive.matches_state(state))
        {
            return false;
        }
        let mut effects = self.effects.borrow_mut();
        effects.hives.entry(pos).or_default();
        effects.changed.insert(pos);
        true
    }
    fn store_bee(&mut self, pos: Pos, ticks: i32) {
        assert!(self.has_beehive(pos), "generated hive exists");
        self.effects
            .borrow_mut()
            .hives
            .get_mut(&pos)
            .unwrap()
            .push(ticks);
    }
    fn schedule_tree_tick(&mut self, request: [i32; 6]) {
        self.effects.borrow_mut().ticks.push(request);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{block, ChunkPos, GeneratedChunk, WorldGenerator};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::Arc;

    #[test]
    fn erased_tree_world_preserves_hive_transitions_and_ticks_through_unwinding() {
        let mut region = FeatureRegion::new(
            WorldGenerator::new(0),
            Arc::new(GeneratedChunk::new(ChunkPos::new(0, 0))),
        );
        let pos = (1, 80, 1);
        assert!(region.set_feature_block(pos, 21774, 19));
        region.store_bee(pos, 7);
        region.transfer_tree_effects().unwrap();
        {
            let mut world = EffectWorld::new(&mut region);
            let effects = world.effects.clone();
            let mut tree = TreeWorld {
                world: &mut world,
                effects,
            };
            tree.store_bee(pos, 9);
            assert!(tree.set_block(pos, 21775, 19));
            tree.store_bee(pos, 11);
            tree.schedule_tree_tick([1, 80, 1, 21768, 1, 0]);
        }
        region.transfer_tree_effects().unwrap();
        let chunk = region.owned_chunk(ChunkPos::new(0, 0)).unwrap();
        assert_eq!(
            chunk.block_entities()[&(1, 80, 1)],
            BlockEntity::Beehive {
                ticks_in_hive: vec![7, 9, 11]
            }
        );
        assert_eq!(chunk.tick_requests().len(), 1);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut world = EffectWorld::new(&mut region);
            let effects = world.effects.clone();
            let mut tree = TreeWorld {
                world: &mut world,
                effects,
            };
            assert!(tree.set_block(pos, block::AIR, 19));
            assert!(tree.set_block(pos, 21774, 19));
            tree.store_bee(pos, -5);
            tree.schedule_tree_tick([1, 80, 1, 21768, 1, 0]);
            panic!("interrupted configured tree");
        }));
        assert!(result.is_err());
        region.transfer_tree_effects().unwrap();
        let chunk = region.owned_chunk(ChunkPos::new(0, 0)).unwrap();
        assert_eq!(
            chunk.block_entities()[&(1, 80, 1)],
            BlockEntity::Beehive {
                ticks_in_hive: vec![-5]
            }
        );
        assert_eq!(chunk.tick_requests().len(), 2);
    }

    #[test]
    fn configured_dispatch_matches_direct_standing_tree_blocks_effects_and_rng() {
        let mut base = GeneratedChunk::new(ChunkPos::new(0, 0));
        for x in 0..16 {
            for z in 0..16 {
                base.set(x, 64, z, block::GRASS_BLOCK);
            }
        }
        let base = Arc::new(base);
        for name in ["birch_bees_0002", "oak_bees_005", "spruce"] {
            let mut direct = FeatureRegion::new(WorldGenerator::new(0), base.clone());
            let mut dispatched = FeatureRegion::new(WorldGenerator::new(0), base.clone());
            let mut a = WorldgenRandom::new(97);
            let mut b = WorldgenRandom::new(97);
            let expected = crate::tree::standing::place(&mut direct, &mut a, name, (8, 65, 8))
                .unwrap()
                .unwrap();
            let actual = {
                let mut world = EffectWorld::new(&mut dispatched);
                let effects = world.effects.clone();
                place(
                    crate::block_predicate::catalog().configured(name).unwrap(),
                    Some(name),
                    &mut world,
                    &mut b,
                    (8, 65, 8),
                    effects,
                )
                .unwrap()
            };
            assert_eq!(actual, expected, "{name}");
            assert_eq!(a.next_long(), b.next_long(), "{name}");
            direct.transfer_tree_effects().unwrap();
            dispatched.transfer_tree_effects().unwrap();
            assert_eq!(
                direct.owned_chunk(ChunkPos::new(0, 0)),
                dispatched.owned_chunk(ChunkPos::new(0, 0)),
                "{name}"
            );
        }
    }
}
