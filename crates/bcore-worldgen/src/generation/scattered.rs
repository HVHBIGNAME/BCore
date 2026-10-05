//! Retained scattered pieces with the live region's guards and ordered effects.
use super::{ChunkStatus, GenerationState};
use crate::feature_world::{FeatureError, FeatureHeightmap, FeatureWorld, Pos};
use crate::ore::OreWorld;
use crate::region::FeatureRegion;
use crate::simplex::WorldgenRandom;
use crate::structure::scattered::{
    self, ScatteredEffect, ScatteredKind, ScatteredStatus, ScatteredWorld,
};
use crate::structure::template::{
    BoundingBox, Mirror, Nbt, Rotation, TemplateBlockEntity, TemplateEffect, TemplateEntity,
};
use crate::structure::template_pool::StructureAssets;
use crate::tick_request::TickRequest;
use crate::ChunkPos;
use std::collections::BTreeMap;

impl GenerationState {
    pub(super) fn place_scattered_source(
        &mut self,
        source: ChunkPos,
        kind: ScatteredKind,
        random: &mut WorldgenRandom,
    ) -> Result<(bool, usize), String> {
        let references = self.holders[&(source.x, source.z)]
            .structures
            .scattered_references
            .get(kind.name())
            .cloned()
            .unwrap_or_default();
        let clip = BoundingBox {
            min: (source.x * 16, crate::MIN_Y + 1, source.z * 16),
            max: (source.x * 16 + 15, crate::MAX_Y, source.z * 16 + 15),
        };
        let mut placed = false;
        let mut pending = 0;
        for [x, z] in references {
            let holder = self
                .holders
                .get_mut(&(x, z))
                .expect("retained scattered start");
            let start = holder
                .structures
                .scattered_starts
                .get_mut(kind.name())
                .unwrap();
            let result = scattered::place_in_chunk(
                start,
                &mut ScatteredRegion {
                    region: &mut self.region,
                    source,
                },
                random,
                clip,
            );
            // Preserve piece flags and bounds even when a later callback fails.
            let start_pos = ChunkPos::new(x, z);
            if self.region.owned_chunk(start_pos).is_some() {
                self.region.owned_chunk_mut(start_pos).structures = holder.structures.clone();
            }
            let result = result.map_err(|error| error.to_string())?;
            placed |= result.status == ScatteredStatus::Processed;
            pending += result.mob_requests;
        }
        Ok((placed, pending))
    }
}

struct ScatteredRegion<'a> {
    region: &'a mut FeatureRegion,
    source: ChunkPos,
}

impl OreWorld for ScatteredRegion<'_> {
    fn get_block(&self, pos: Pos) -> Option<u32> {
        self.region.get_block(pos)
    }
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32 {
        self.region.ocean_floor_wg(x, z)
    }
    fn set_block(&mut self, pos: Pos, state: u32) -> bool {
        self.set_feature_block(pos, state, 2)
    }
}

impl FeatureWorld for ScatteredRegion<'_> {
    fn feature_biome(&self, pos: Pos) -> u32 {
        self.region.feature_biome(pos)
    }
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32 {
        self.region.feature_height(kind, x, z)
    }
    fn can_write_feature(&self, pos: Pos) -> bool {
        self.region.can_write_feature(pos)
    }
    fn set_feature_block(&mut self, pos @ (x, y, z): Pos, state: u32, flags: i32) -> bool {
        if !self.region.set_feature_block(pos, state, flags) {
            return false;
        }
        let local = ((x & 15) as usize, y, (z & 15) as usize);
        let owner = ChunkPos::new(x >> 4, z >> 4);
        let exists = {
            let chunk = self
                .region
                .owned_chunk(owner)
                .expect("accepted scattered write");
            chunk.feature_block_entities().contains_key(&local)
                || chunk.block_entities().contains_key(&local)
        };
        if !exists {
            if let Some(entity) = StructureAssets::bundled()
                .blocks
                .default_block_entity(state, pos)
                .expect("native scattered block-entity defaults")
            {
                self.region
                    .apply_template_effect(self.source, TemplateEffect::BlockEntity(entity))
                    .expect("native scattered block-entity factory");
            }
        }
        true
    }
    fn mark_feature_postprocessing(&mut self, pos: Pos) {
        self.region.mark_feature_postprocessing(pos);
    }
    fn schedule_feature_tick(&mut self, tick: TickRequest) -> bool {
        self.region.schedule_feature_tick(tick)
    }
}

impl ScatteredWorld for ScatteredRegion<'_> {
    fn structure_min_y(&self) -> i32 {
        crate::MIN_Y
    }
    fn has_structure_block_entity(&self, pos @ (x, y, z): Pos, id: &str) -> bool {
        let chunk = self
            .region
            .chunk_at_status(ChunkPos::new(x >> 4, z >> 4), ChunkStatus::Empty)
            .expect("scattered container within source dependencies");
        let local = ((x & 15) as usize, y, (z & 15) as usize);
        chunk
            .feature_block_entities()
            .get(&local)
            .is_some_and(|data| data.full_data["id"] == id)
            || chunk
                .block_entities()
                .get(&local)
                .is_some_and(|data| data.full_data(pos)["id"] == id)
    }
    fn apply_scattered_effect(&mut self, effect: ScatteredEffect) -> Result<(), FeatureError> {
        match effect {
            ScatteredEffect::LootTable {
                pos,
                block_entity,
                table,
                seed,
            } => {
                if !self.has_structure_block_entity(pos, &block_entity) {
                    return Err(FeatureError::MissingData(format!(
                        "scattered {block_entity} at {pos:?}"
                    )));
                }
                let state = self
                    .get_block(pos)
                    .ok_or_else(|| FeatureError::MissingData(format!("scattered block {pos:?}")))?;
                let chunk = self
                    .region
                    .owned_chunk(ChunkPos::new(pos.0 >> 4, pos.2 >> 4))
                    .unwrap();
                let local = ((pos.0 & 15) as usize, pos.1, (pos.2 & 15) as usize);
                let mut load = match chunk.feature_block_entities().get(&local) {
                    Some(data) => data.full_nbt()?.compound()?.clone(),
                    // The only core container is a generated dungeon chest with
                    // no custom fields; replacing its loot needs only these keys.
                    None => BTreeMap::new(),
                };
                load.insert("LootTable".into(), Nbt::String(table));
                load.insert("LootTableSeed".into(), Nbt::Long(seed));
                let entity = TemplateBlockEntity::from_load(
                    &StructureAssets::bundled().blocks,
                    state,
                    pos,
                    Nbt::Compound(load),
                )?;
                self.region
                    .apply_template_effect(self.source, TemplateEffect::BlockEntity(entity))
            }
            ScatteredEffect::MobRequest(request) => {
                let nbt = Nbt::Compound(BTreeMap::from([
                    ("id".into(), Nbt::String(request.mob.name().into())),
                    (
                        "Pos".into(),
                        Nbt::List {
                            element_type: 6,
                            values: request.position.into_iter().map(Nbt::Double).collect(),
                        },
                    ),
                    (
                        "Rotation".into(),
                        Nbt::List {
                            element_type: 5,
                            values: request.rotation.into_iter().map(Nbt::Float).collect(),
                        },
                    ),
                    (
                        "PersistenceRequired".into(),
                        Nbt::Byte(i8::from(request.persistence_required)),
                    ),
                ]));
                self.region.apply_template_effect(
                    self.source,
                    TemplateEffect::Entity(TemplateEntity {
                        pos: request.position,
                        block_pos: request.block_pos,
                        nbt,
                        finalize: request.finalize,
                        rotation: Rotation::None,
                        mirror: Mirror::None,
                    }),
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "scattered_tests.rs"]
mod tests;
