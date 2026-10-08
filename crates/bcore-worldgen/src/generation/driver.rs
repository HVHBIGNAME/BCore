//! One live source FEATURES task, ordered by native decoration step and index.
use std::collections::BTreeSet;

use crate::feature_sorter::{sorter, POSSIBLE_BIOMES};
use crate::region::FeatureRegion;
use crate::simplex::WorldgenRandom;
use crate::structure::mineshaft::MineType;
use crate::structure::scattered::{ScatteredCatalog, ScatteredKind};
use crate::ChunkPos;

use super::placed::Outcome;
use super::{
    graph::layer_positions, ChunkStatus, FeatureWork, GenerationState, MissingFeature,
    SourceFeatureCoverage,
};

#[derive(Clone, Copy)]
enum SourceStructure {
    Mineshaft(MineType),
    Jigsaw(&'static str),
    Scattered(ScatteredKind),
}

impl SourceStructure {
    fn name(self) -> &'static str {
        match self {
            Self::Mineshaft(kind) => kind.name(),
            Self::Jigsaw(name) => name,
            Self::Scattered(kind) => kind.name(),
        }
    }
}

fn structure_slots() -> Vec<(usize, usize, SourceStructure)> {
    let mut slots: Vec<_> = crate::structure::template_pool::StructureAssets::bundled()
        .structure_metadata
        .iter()
        .map(|(name, metadata)| {
            (
                metadata.step as usize,
                metadata.feature_index as usize,
                SourceStructure::Jigsaw(name),
            )
        })
        .collect();
    slots.extend([MineType::Normal, MineType::Mesa].into_iter().map(|kind| {
        (
            3,
            kind.feature_index() as usize,
            SourceStructure::Mineshaft(kind),
        )
    }));
    slots.extend(ScatteredKind::ALL.into_iter().map(|kind| {
        let config = ScatteredCatalog::bundled().config(kind);
        (
            config.decoration_step as usize,
            config.structure_index as usize,
            SourceStructure::Scattered(kind),
        )
    }));
    slots.sort_by_key(|&(step, index, _)| (step, index));
    slots
}

impl GenerationState {
    fn source_features(&self, source: ChunkPos) -> Vec<(usize, usize, &'static str)> {
        let mut biomes = BTreeSet::new();
        for pos in layer_positions(source, 1) {
            let chunk = self
                .region
                .chunk_at_status(pos, ChunkStatus::Carvers)
                .expect("source palette dependency");
            biomes.extend(
                chunk
                    .noise_biomes
                    .as_ref()
                    .expect("BIOMES stage supplies quart cells")
                    .iter()
                    .copied(),
            );
        }
        let mut features = BTreeSet::new();
        for biome in biomes {
            let biome = crate::biome::name(biome);
            if !POSSIBLE_BIOMES.contains(&biome) {
                continue;
            }
            for step in 0..11 {
                features.extend(
                    sorter()
                        .indices_for_biome(step, biome)
                        .iter()
                        .map(|&index| (step, index)),
                );
            }
        }
        features
            .into_iter()
            .map(|(step, index)| {
                (
                    step,
                    index,
                    sorter()
                        .feature_name(step, index)
                        .expect("native feature index"),
                )
            })
            .collect()
    }

    pub(super) fn decorate_source(
        &mut self,
        seed: i64,
        source: ChunkPos,
        sequence: u64,
    ) -> SourceFeatureCoverage {
        let features = self.source_features(source);
        let mut report = SourceFeatureCoverage {
            source,
            sequence,
            mineshafts_processed: false,
            completed: Vec::new(),
            missing: Vec::new(),
        };
        let mut random = WorldgenRandom::new(seed);
        let decoration_seed = random.set_decoration_seed(seed, source.x * 16, source.z * 16);
        let mut region_random =
            crate::structure::scattered::desert_pyramid::region_random(seed, source);
        let mut cave_random = crate::dripstone::CaveRandomState::default();
        let mut interrupted: Option<String> = None;
        let structures = structure_slots();
        let mut mineshafts_processed = 0;
        for step in 0..11 {
            for &(_, index, structure) in structures.iter().filter(|&&(s, _, _)| s == step) {
                let name = structure.name();
                if let Some(reason) = &interrupted {
                    report
                        .missing
                        .push(missing(source, step, index, name, reason.clone()));
                    continue;
                }
                random.set_feature_seed(decoration_seed, index as i32, step as i32);
                let result = match structure {
                    SourceStructure::Mineshaft(kind) => {
                        self.place_mineshaft_source(source, &mut random, kind);
                        mineshafts_processed += 1;
                        report.mineshafts_processed = mineshafts_processed == 2;
                        continue;
                    }
                    SourceStructure::Jigsaw(name) => {
                        self.place_jigsaw_source(seed, source, name, &mut random, &mut cave_random)
                    }
                    SourceStructure::Scattered(kind) => self.place_scattered_source(
                        source,
                        kind,
                        &mut random,
                        &mut region_random,
                        seed,
                    ),
                };
                match result {
                    Ok((placed, pending_entities)) => {
                        report.completed.push(FeatureWork {
                            step,
                            index,
                            feature: name.into(),
                            placed,
                        });
                        if pending_entities != 0 {
                            report.missing.push(missing(source, step, index, name, format!(
                                "{pending_entities} retained STRUCTURE entity requests need native factory, identity, orientation/passengers and mob finalization"
                            )));
                        }
                    }
                    Err(reason) => {
                        report
                            .missing
                            .push(missing(source, step, index, name, reason.clone()));
                        interrupted =
                            Some(format!("Not executed after incomplete {name}: {reason}"));
                    }
                }
            }
            for &(_, index, name) in features.iter().filter(|&&(s, _, _)| s == step) {
                if let Some(reason) = &interrupted {
                    report
                        .missing
                        .push(missing(source, step, index, name, reason.clone()));
                    continue;
                }
                random.set_feature_seed(decoration_seed, index as i32, step as i32);
                #[cfg(feature = "diagnostics")]
                self.record_feature_trace(source, name, &random, "before");
                let result = place_feature(
                    &mut self.region,
                    &mut random,
                    source,
                    name,
                    &mut cave_random,
                    &super::placed::GenerationEnvironment::with_light_or_missing(
                        &self.light,
                        self.light_missing.as_deref(),
                    ),
                );
                #[cfg(feature = "diagnostics")]
                self.record_feature_trace(source, name, &random, "after");
                match result {
                    Ok(Outcome::Complete(placed)) => report.completed.push(FeatureWork {
                        step,
                        index,
                        feature: name.into(),
                        placed,
                    }),
                    Ok(Outcome::Unavailable(reason)) => report
                        .missing
                        .push(missing(source, step, index, name, reason)),
                    Err(reason) => {
                        report
                            .missing
                            .push(missing(source, step, index, name, reason.clone()));
                        // Keep preceding writes and RNG effects. Never restart the
                        // source, skip a child draw, or run its remaining attempts.
                        interrupted =
                            Some(format!("Not executed after incomplete {name}: {reason}"));
                    }
                }
            }
        }
        report
    }
}

fn missing(
    source: ChunkPos,
    step: usize,
    index: usize,
    feature: &str,
    reason: String,
) -> MissingFeature {
    MissingFeature {
        source,
        step,
        index,
        feature: feature.into(),
        reason,
    }
}

fn place_feature(
    world: &mut FeatureRegion,
    random: &mut WorldgenRandom,
    source: ChunkPos,
    name: &str,
    cave_random: &mut crate::dripstone::CaveRandomState,
    environment: &dyn crate::block_predicate::FeatureEnvironment,
) -> Result<Outcome, String> {
    if let Some(result) =
        crate::features::place_ore_feature(world, random, source, name, |world, pos| {
            sorter().feature_in_biome(crate::biome::name(world.biome_at(pos)), name)
        })
    {
        return Ok(Outcome::Complete(result));
    }
    if let Some((count, low, high)) = match name {
        "monster_room" => Some((10, 0, 319)),
        "monster_room_deep" => Some((4, -58, -1)),
        _ => None,
    } {
        let mut placed = false;
        for _ in 0..count {
            let x = source.x * 16 + random.next_int(16) as i32;
            let z = source.z * 16 + random.next_int(16) as i32;
            let y = low + random.next_int((high - low + 1) as usize) as i32;
            if sorter().feature_in_biome(crate::biome::name(world.biome_at((x, y, z))), name) {
                placed |= crate::dungeon::place(world, random, (x, y, z));
            }
        }
        return Ok(Outcome::Complete(placed));
    }
    if let Some(placed) = crate::decoration::place_tree_feature(world, random, source, name)
        .map_err(|error| error.to_string())?
    {
        return Ok(Outcome::Complete(placed));
    }
    super::placed::place_named_with_environment(
        name,
        world,
        random,
        source,
        cave_random,
        environment,
    )
}
