//! Retained starts and per-source clipping, without target-local spider replay.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::dripstone::CaveRandomState;
use crate::feature_world::FeatureHeightmap;
use crate::simplex::WorldgenRandom;
use crate::structure::jigsaw::{self, HeightContext, JigsawStart};
use crate::structure::mineshaft::{region::writable_area, MineType};
use crate::structure::scattered::{self, ScatteredKind};
use crate::structure::template::{BoundingBox, PlacementResult};
use crate::structure::template_pool::StructureAssets;
use crate::ChunkPos;

use super::{graph::layer_positions, GenerationState, Key};

impl GenerationState {
    pub(super) fn generate_structure_starts(
        &mut self,
        generator: crate::WorldGenerator,
        graph: &crate::VanillaGraph,
        pos: ChunkPos,
    ) -> Result<(), String> {
        let ctx = crate::density::EvalContext {
            seed: generator.seed(),
            mode: crate::density::EvaluationMode::Raw,
            ..Default::default()
        };
        let height_cache = RefCell::new(BTreeMap::new());
        let first_free = |kind: FeatureHeightmap, x, z| {
            if let Some(&y) = height_cache.borrow().get(&(kind as u8, x, z)) {
                return y;
            }
            let y = generator.base_height_for_vanilla(kind, x, z);
            height_cache.borrow_mut().insert((kind as u8, x, z), y);
            y
        };
        let noise_biome =
            |p: (i32, i32, i32)| graph.noise_biome_at(p.0 >> 2, p.1 >> 2, p.2 >> 2, &ctx);
        let mineshaft = crate::structure::mineshaft::MineshaftLayout::for_chunk(
            generator.seed(),
            pos,
            |p| noise_biome((p[0], p[1], p[2])),
            |x, z| first_free(FeatureHeightmap::WorldSurfaceWg, x, z),
        );
        let heights = HeightContext {
            min_y: crate::MIN_Y,
            max_y: crate::MAX_Y,
            first_free: &first_free,
        };
        let mut starts = BTreeMap::new();
        for set in [
            "ancient_cities",
            "villages",
            "trail_ruins",
            "trial_chambers",
        ] {
            if let Some(named) = jigsaw::for_chunk(
                StructureAssets::bundled(),
                set,
                generator.seed(),
                pos,
                &heights,
                |p| native_biome(noise_biome(p)),
            )
            .map_err(|error| error.to_string())?
            {
                starts.insert(named.structure, named.start);
            }
        }
        let mut scattered_starts = BTreeMap::new();
        for kind in ScatteredKind::ALL {
            if let Some(start) = scattered::for_chunk(kind, generator.seed(), pos, &heights, |p| {
                native_biome(noise_biome(p))
            })
            .map_err(|error| error.to_string())?
            {
                scattered_starts.insert(kind.name().into(), start);
            }
        }
        crate::density::clear_density_caches();
        let data = &mut self.holders.get_mut(&(pos.x, pos.z)).unwrap().structures;
        data.mineshaft_start = mineshaft;
        data.jigsaw_starts = starts;
        data.scattered_starts = scattered_starts;
        Ok(())
    }

    pub(super) fn jigsaw_references(&self, pos: ChunkPos) -> BTreeMap<String, Vec<[i32; 2]>> {
        let mut inserted: BTreeMap<String, Vec<Key>> = BTreeMap::new();
        for source in layer_positions(pos, 8) {
            for (name, start) in &self.holders[&(source.x, source.z)].structures.jigsaw_starts {
                if start
                    .reference_bounds()
                    .is_some_and(|bb| intersects_chunk(bb, pos))
                {
                    inserted
                        .entry(name.clone())
                        .or_default()
                        .push((source.x, source.z));
                }
            }
        }
        inserted
            .into_iter()
            .map(|(name, sources)| {
                (
                    name,
                    reference_order(sources)
                        .into_iter()
                        .map(|(x, z)| [x, z])
                        .collect(),
                )
            })
            .collect()
    }

    pub(super) fn retained_jigsaw_starts(&self, pos: ChunkPos) -> Vec<&JigsawStart> {
        self.holders[&(pos.x, pos.z)]
            .structures
            .jigsaw_references
            .iter()
            .flat_map(|(name, references)| {
                references
                    .iter()
                    .map(move |[x, z]| &self.holders[&(*x, *z)].structures.jigsaw_starts[name])
            })
            .collect()
    }

    pub(super) fn scattered_references(
        &mut self,
        pos: ChunkPos,
    ) -> BTreeMap<String, Vec<[i32; 2]>> {
        let mut inserted: BTreeMap<String, Vec<Key>> = BTreeMap::new();
        for source in layer_positions(pos, 8) {
            let data = &mut self
                .holders
                .get_mut(&(source.x, source.z))
                .unwrap()
                .structures;
            for (name, start) in &mut data.scattered_starts {
                // Cache before any piece moves in FEATURES. Serde retains this
                // separately from the piece's current native saved bounding box.
                if start.references_chunk(pos) {
                    inserted
                        .entry(name.clone())
                        .or_default()
                        .push((source.x, source.z));
                }
            }
        }
        inserted
            .into_iter()
            .map(|(name, sources)| {
                (
                    name,
                    reference_order(sources)
                        .into_iter()
                        .map(|(x, z)| [x, z])
                        .collect(),
                )
            })
            .collect()
    }

    pub(super) fn mineshaft_references(&self, pos: ChunkPos) -> Vec<([i32; 2], MineType)> {
        let clip = writable_area(pos);
        let mut result = Vec::new();
        for mine_type in [MineType::Normal, MineType::Mesa] {
            let starts = layer_positions(pos, 8).filter_map(|source| {
                let start = self.holders[&(source.x, source.z)]
                    .structures
                    .mineshaft_start
                    .as_ref()?;
                let bb = start.bounds();
                (start.mine_type == mine_type
                    && bb.max[0] >= clip.min[0]
                    && bb.min[0] <= clip.max[0]
                    && bb.max[2] >= clip.min[2]
                    && bb.min[2] <= clip.max[2])
                    .then_some((source.x, source.z))
            });
            result.extend(
                reference_order(starts)
                    .into_iter()
                    .map(|(x, z)| ([x, z], mine_type)),
            );
        }
        result
    }

    pub(super) fn place_mineshaft_source(
        &mut self,
        source: ChunkPos,
        random: &mut WorldgenRandom,
        mine_type: MineType,
    ) {
        let references = self.holders[&(source.x, source.z)]
            .structures
            .references
            .clone();
        let clip = writable_area(source);
        for &(start_pos, ty) in &references {
            if ty != mine_type {
                continue;
            }
            let holder = self
                .holders
                .get_mut(&(start_pos[0], start_pos[1]))
                .expect("retained structure start holder");
            let start = holder
                .structures
                .mineshaft_start
                .as_mut()
                .expect("referenced mineshaft start");
            for piece in &mut start.pieces {
                if piece.bounds.intersects(clip) {
                    piece.post_process(mine_type, &mut self.region, random, clip);
                }
            }
            let pos = ChunkPos::new(start_pos[0], start_pos[1]);
            if self.region.owned_chunk(pos).is_some() {
                self.region.owned_chunk_mut(pos).structures = holder.structures.clone();
            }
        }
    }

    pub(super) fn place_jigsaw_source(
        &mut self,
        seed: i64,
        source: ChunkPos,
        name: &str,
        random: &mut WorldgenRandom,
        cave_random: &mut CaveRandomState,
    ) -> Result<(bool, usize), String> {
        let assets = StructureAssets::bundled();
        let references = self.holders[&(source.x, source.z)]
            .structures
            .jigsaw_references
            .get(name)
            .cloned()
            .unwrap_or_default();
        let clip = BoundingBox {
            min: (source.x * 16, crate::MIN_Y + 1, source.z * 16),
            max: (source.x * 16 + 15, crate::MAX_Y, source.z * 16 + 15),
        };
        let mut placed = false;
        let mut entity_requests = 0;
        let environment = super::placed::GenerationEnvironment::with_light_or_missing(
            &self.light,
            self.light_missing.as_deref(),
        );
        for [x, z] in references {
            let start = &self.holders[&(x, z)].structures.jigsaw_starts[name];
            let result = jigsaw::place_in_chunk_with_callbacks(
                assets,
                start,
                &mut self.region,
                random,
                clip,
                seed,
                &mut |name, world, random, origin| {
                    let placed = super::placed::place_placed(
                        &serde_json::Value::String(name.into()),
                        world,
                        random,
                        origin,
                        cave_random,
                        &environment,
                    )?;
                    Ok(PlacementResult {
                        placed,
                        ..Default::default()
                    })
                },
                &mut |world, effect| world.apply_template_effect(source, effect),
            )
            .map_err(|error| error.to_string())?;
            placed |= result.placed;
            entity_requests += result.entities.len();
        }
        Ok((placed, entity_requests))
    }
}

fn intersects_chunk(bb: BoundingBox, pos: ChunkPos) -> bool {
    bb.max.0 >= pos.x * 16
        && bb.min.0 <= pos.x * 16 + 15
        && bb.max.2 >= pos.z * 16
        && bb.min.2 <= pos.z * 16 + 15
}

fn native_biome(wire_id: u32) -> u32 {
    static IDS: OnceLock<BTreeMap<u32, u32>> = OnceLock::new();
    IDS.get_or_init(|| {
        crate::block_predicate::catalog().documents["biome_ids"]
            .as_object()
            .expect("native biome IDs")
            .iter()
            .map(|(name, id)| {
                (
                    crate::biome::id(name).expect("native biome identity"),
                    id.as_u64().unwrap() as u32,
                )
            })
            .collect()
    })[&wire_id]
}

pub(super) fn unsupported_structures() -> String {
    "Unsupported structure types: igloo, mansion, monument, ocean_ruin_cold/warm, pillager_outpost, ruined_portal variants, shipwreck/beached, stronghold (Nether/End structures require their dimension pipelines)".into()
}

/// 26.1's insertion-only LongOpenHashSet reference iteration. This is the order
/// within one structure type's source task, not a sort of feature source tasks.
fn reference_order(positions: impl IntoIterator<Item = Key>) -> Vec<Key> {
    fn insert(slots: &mut [u64], key: u64) {
        let h = key.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let h = h ^ (h >> 32);
        let mut index = (h ^ (h >> 16)) as usize & (slots.len() - 1);
        while slots[index] != 0 {
            index = (index + 1) & (slots.len() - 1);
        }
        slots[index] = key;
    }
    let mut slots = vec![0; 32];
    let mut seen = BTreeSet::new();
    for (x, z) in positions {
        let packed = u64::from(x as u32) | (u64::from(z as u32) << 32);
        if !seen.insert(packed) {
            continue;
        }
        if packed != 0 {
            insert(&mut slots, packed);
        }
        if seen.len() > slots.len() * 3 / 4 {
            let mut larger = vec![0; slots.len() * 2];
            for value in slots.into_iter().rev().filter(|&p| p != 0) {
                insert(&mut larger, value);
            }
            slots = larger;
        }
    }
    let mut result = Vec::new();
    if seen.contains(&0) {
        result.push((0, 0));
    }
    result.extend(
        slots
            .into_iter()
            .rev()
            .filter(|&p| p != 0)
            .map(|p| (p as u32 as i32, (p >> 32) as u32 as i32)),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_reference_order_matches_the_native_set_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../data/mineshaft_starts_26_1.json")).unwrap();
        for row in fixture["reference_orders"].as_array().unwrap() {
            let positions = |v: &serde_json::Value| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                reference_order(positions(&row["inserted"])),
                positions(&row["iterated"])
            );
        }
    }
}
