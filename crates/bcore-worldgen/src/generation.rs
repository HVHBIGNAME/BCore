//! Persistent generation for one fresh, unblended overworld.
//!
//! A world mutex coalesces status claims and serializes source feature writes.
//! Completed work and neighbouring effects belong to this context, not to the
//! target snapshot returned to the caller. Request histories may affect features.
#[cfg(feature = "diagnostics")]
pub mod benchmark;
mod coverage;
#[cfg(feature = "diagnostics")]
mod diagnostics;
mod driver;
mod effects;
pub mod graph;
mod light;
mod placed;
mod scattered;
mod spawn;
mod structures;
mod terrain;
mod tree_world;

pub use coverage::*;
#[cfg(feature = "diagnostics")]
pub use diagnostics::FeatureTrace;
pub(crate) use effects::sculk_block_entity;
pub use effects::{FeatureBlockEntity, StructureEntityRequest};
pub use graph::{ChunkPyramid, ChunkStatus};
pub use spawn::{SpawnInputs, SpawnSettings, WorldSpawnInputs};

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use bcore_core::ChunkPos;

use crate::region::FeatureRegion;
use crate::structure::mineshaft::region::StructureData;
use crate::{GeneratedChunk, WorldGenerator};
use graph::layer_positions;

type Key = (i32, i32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationError {
    CoordinateOutOfRange(ChunkPos),
    StageFailed {
        pos: ChunkPos,
        status: ChunkStatus,
        detail: String,
    },
    Poisoned,
}

impl std::fmt::Display for GenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoordinateOutOfRange(pos) => write!(
                f,
                "generation dependencies exceed block coordinate bounds for {pos:?}"
            ),
            Self::StageFailed {
                pos,
                status,
                detail,
            } => write!(f, "{status} at {pos:?}: {detail}"),
            Self::Poisoned => f.write_str("generation context mutex poisoned"),
        }
    }
}
impl std::error::Error for GenerationError {}

#[derive(Debug, Clone)]
#[must_use = "generation coverage identifies missing features and unrun native stages"]
pub struct GenerationResult {
    pub chunk: GeneratedChunk,
    pub coverage: GenerationCoverage,
}

/// Share one instance (or an `Arc<GenerationWorld>`) for the lifetime of a world.
/// Constructing another instance deliberately creates an independent history,
/// even when its seed is the same. This context does not load persisted chunks.
pub struct GenerationWorld {
    generator: WorldGenerator,
    state: Mutex<GenerationState>,
}

impl std::fmt::Debug for GenerationWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenerationWorld")
            .field("seed", &self.seed())
            .finish_non_exhaustive()
    }
}

impl GenerationWorld {
    pub fn new(seed: i64) -> Self {
        Self::with_spawn_inputs(seed, Arc::new(WorldSpawnInputs::default()))
    }

    pub fn with_spawn_inputs(seed: i64, inputs: Arc<dyn SpawnInputs>) -> Self {
        let generator = WorldGenerator::new(seed);
        let mut state = GenerationState::new(generator);
        state.spawn_inputs = inputs;
        Self {
            generator,
            state: Mutex::new(state),
        }
    }

    pub fn seed(&self) -> i64 {
        self.generator.seed()
    }

    /// Run the supported work in a fresh FULL target's native dependency envelope.
    /// Unsupported work is reported, never promoted to completed FEATURES/FULL.
    /// Repeated and concurrent calls reuse the same claims and source effects.
    pub fn generate_chunk(&self, pos: ChunkPos) -> Result<GenerationResult, GenerationError> {
        self.generate_to_status(pos, ChunkStatus::Full)
    }

    pub fn generate_to_status(
        &self,
        pos: ChunkPos,
        target: ChunkStatus,
    ) -> Result<GenerationResult, GenerationError> {
        // Include the largest pyramid envelope and structure-piece horizontal reach.
        // Validate before any holder is claimed or any coordinate multiplication.
        const MARGIN: i64 = 16 * (11 + 9);
        if [pos.x, pos.z].into_iter().any(|v| {
            let block = i64::from(v) * 16;
            block - MARGIN < i64::from(i32::MIN) || block + MARGIN + 15 > i64::from(i32::MAX)
        }) {
            return Err(GenerationError::CoordinateOutOfRange(pos));
        }
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        state.generate(self.generator, pos, target)
    }

    pub fn progress(&self, pos: ChunkPos) -> Result<Option<ChunkProgress>, GenerationError> {
        let state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        Ok(state
            .holders
            .get(&(pos.x, pos.z))
            .map(|holder| holder.progress.clone()))
    }

    /// Snapshot of the current live data, without requesting more generation.
    pub fn chunk_snapshot(&self, pos: ChunkPos) -> Result<Option<GeneratedChunk>, GenerationError> {
        let state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        Ok(state.region.owned_chunk(pos).map(|chunk| (*chunk).clone()))
    }
}

struct ChunkHolder {
    progress: ChunkProgress,
    structures: StructureData,
    features: Option<SourceFeatureCoverage>,
    spawn: Option<crate::spawn::SpawnReport>,
    failure: Option<GenerationError>,
    /// Failed validation must not discard effects or contaminate another source.
    pending_tree_effects: crate::tree::standing::TreeEffects,
}

impl ChunkHolder {
    fn new(pos: ChunkPos) -> Self {
        Self {
            progress: ChunkProgress {
                pos,
                stages: std::array::from_fn(|_| StageProgress::default()),
            },
            structures: StructureData::default(),
            features: None,
            spawn: None,
            failure: None,
            pending_tree_effects: Default::default(),
        }
    }
}

struct GenerationState {
    holders: BTreeMap<Key, ChunkHolder>,
    region: FeatureRegion,
    source_sequence: u64,
    graph: Option<Arc<crate::VanillaGraph>>,
    light: crate::lighting::GenerationLight,
    light_missing: Option<String>,
    spawn_inputs: Arc<dyn SpawnInputs>,
    #[cfg(feature = "diagnostics")]
    feature_trace: diagnostics::FeatureTraceState,
    #[cfg(test)]
    test_stage: Option<fn(ChunkStatus, ChunkPos, &mut FeatureRegion) -> Result<(), String>>,
}

impl GenerationState {
    fn new(generator: WorldGenerator) -> Self {
        Self {
            holders: BTreeMap::new(),
            region: FeatureRegion::shared(generator),
            source_sequence: 0,
            graph: None,
            light: crate::lighting::GenerationLight::default(),
            light_missing: None,
            spawn_inputs: Arc::new(WorldSpawnInputs::default()),
            #[cfg(feature = "diagnostics")]
            feature_trace: Default::default(),
            #[cfg(test)]
            test_stage: None,
        }
    }

    fn generate(
        &mut self,
        generator: WorldGenerator,
        pos: ChunkPos,
        target: ChunkStatus,
    ) -> Result<GenerationResult, GenerationError> {
        let step = ChunkPyramid::Generation.step(target);
        for status in ChunkStatus::ALL {
            let Some(radius) = step.layer_radius(status) else {
                continue;
            };
            if status > ChunkStatus::Spawn {
                continue;
            }
            for source in layer_positions(pos, radius) {
                self.run_stage(generator, source, status)?;
            }
        }
        let coverage = self.coverage(pos, target);
        let structures = self.holders[&(pos.x, pos.z)].structures.clone();
        let chunk = self.region.owned_chunk_mut(pos);
        chunk.structures = structures;
        if target == ChunkStatus::Full {
            // Execute the verified BE-only boundary. Native FULL conversion
            // (including ticks, postprocessing and WG retirement) remains pending.
            chunk
                .materialize_block_entities()
                .map_err(|error| GenerationError::StageFailed {
                    pos,
                    status: ChunkStatus::Full,
                    detail: format!("block-entity materialization: {error}"),
                })?;
        }
        Ok(GenerationResult {
            chunk: chunk.clone(),
            coverage,
        })
    }

    fn run_stage(
        &mut self,
        generator: WorldGenerator,
        pos: ChunkPos,
        status: ChunkStatus,
    ) -> Result<(), GenerationError> {
        let key = (pos.x, pos.z);
        let holder = self
            .holders
            .entry(key)
            .or_insert_with(|| ChunkHolder::new(pos));
        let progress = &mut holder.progress.stages[status.index()];
        if progress.state.has_data() {
            return Ok(());
        }
        if let Some(error) = &holder.failure {
            return Err(error.clone());
        }
        assert_eq!(
            progress.state,
            StageState::Pending,
            "recursive generation claim"
        );
        progress.state = StageState::Running;
        progress.attempts += 1;

        let result = catch_unwind(AssertUnwindSafe(|| {
            self.validate_dependencies(pos, status)?;
            if matches!(status, ChunkStatus::Features | ChunkStatus::Spawn) {
                let available = self
                    .holders
                    .iter()
                    .filter_map(|(&key, holder)| {
                        holder.progress.available_status().map(|s| (key, s))
                    })
                    .collect();
                self.region.begin_source(pos, status, available);
            }
            let missing = self.execute_stage(generator, pos, status)?;
            if matches!(
                status,
                ChunkStatus::Noise | ChunkStatus::Surface | ChunkStatus::Carvers
            ) {
                self.region
                    .owned_chunk_mut(pos)
                    .capture_worldgen_heightmaps(status == ChunkStatus::Carvers);
            }
            Ok(missing)
        }));
        let mut result = match result {
            Ok(result) => result,
            Err(payload) => Err(panic_detail(payload)),
        };
        if status == ChunkStatus::Features {
            // Flush on success AND failure. A failed child can already have made
            // valid writes, hive snapshots and tick requests into neighbours.
            let transfer = catch_unwind(AssertUnwindSafe(|| self.region.transfer_tree_effects()));
            let transfer_error = match transfer {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(payload) => Some(panic_detail(payload)),
            };
            if let Some(error) = transfer_error {
                result = Err(error);
            }
            self.region.end_source();
            self.holders.get_mut(&key).unwrap().pending_tree_effects =
                std::mem::take(&mut self.region.tree_effects);
            let lighting = catch_unwind(AssertUnwindSafe(|| self.refresh_light_inputs(Some(pos))));
            let lighting = match lighting {
                Ok(result) => result,
                Err(payload) => Err(panic_detail(payload)),
            };
            if let Err(error) = lighting {
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
        if status == ChunkStatus::Spawn {
            self.region.end_source();
        }
        let holder = self.holders.get_mut(&key).unwrap();
        let progress = &mut holder.progress.stages[status.index()];
        match result {
            Ok(missing) => {
                progress.state = if missing.is_empty() {
                    StageState::Complete
                } else {
                    StageState::Partial
                };
                progress.missing = missing;
                Ok(())
            }
            Err(detail) => {
                progress.state = StageState::Failed;
                progress.missing = vec![detail.clone()];
                let error = GenerationError::StageFailed {
                    pos,
                    status,
                    detail,
                };
                holder.failure = Some(error.clone());
                Err(error)
            }
        }
    }

    fn validate_dependencies(&self, source: ChunkPos, status: ChunkStatus) -> Result<(), String> {
        let deps = &ChunkPyramid::Generation.step(status).direct;
        if deps.by_radius.is_empty() {
            return Ok(());
        }
        for pos in layer_positions(source, deps.radius()) {
            let required = deps.at(source, pos).unwrap();
            let available = self.holders.get(&(pos.x, pos.z)).and_then(|h| {
                // The source's new claim is Running, but its preceding stages remain available.
                h.progress.available_status()
            });
            if !available.is_some_and(|s| s >= required) {
                return Err(format!(
                    "dependency {pos:?} requires {required}, available {available:?}"
                ));
            }
        }
        Ok(())
    }

    fn execute_stage(
        &mut self,
        generator: WorldGenerator,
        pos: ChunkPos,
        status: ChunkStatus,
    ) -> Result<Vec<String>, String> {
        #[cfg(test)]
        if let Some(stage) = self.test_stage {
            stage(status, pos, &mut self.region)?;
            return Ok(Vec::new());
        }
        use ChunkStatus::*;
        let key = (pos.x, pos.z);
        let graph = if matches!(
            status,
            StructureStarts | Biomes | Noise | Surface | Carvers | Spawn
        ) {
            Some(
                self.graph
                    .get_or_insert_with(|| {
                        Arc::new(
                            crate::VanillaGraph::load()
                                .expect("complete vanilla worldgen assets")
                                .fork(),
                        )
                    })
                    .clone(),
            )
        } else {
            None
        };
        match status {
            Empty => Ok(Vec::new()),
            StructureStarts => {
                self.generate_structure_starts(generator, graph.as_ref().unwrap(), pos)?;
                Ok(vec![structures::unsupported_structures()])
            }
            StructureReferences => {
                let references = self.mineshaft_references(pos);
                let jigsaw_references = self.jigsaw_references(pos);
                let scattered_references = self.scattered_references(pos);
                let data = &mut self.holders.get_mut(&key).unwrap().structures;
                data.references = references;
                data.jigsaw_references = jigsaw_references;
                data.scattered_references = scattered_references;
                Ok(vec![structures::unsupported_structures()])
            }
            Biomes => {
                let structure_data = self.holders[&key].structures.clone();
                let chunk = self.region.owned_chunk_mut(pos);
                chunk.structures = structure_data;
                generator.generate_biomes(chunk, graph.as_ref().unwrap());
                Ok(Vec::new())
            }
            Noise => {
                let beardifier =
                    crate::beardifier::Beardifier::for_chunk(pos, self.retained_jigsaw_starts(pos))
                        .map_err(|error| error.to_string())?;
                generator.generate_noise_with_structures(
                    self.region.owned_chunk_mut(pos),
                    graph.as_ref().unwrap(),
                    &beardifier,
                );
                Ok(Vec::new())
            }
            Surface => {
                // BIOMES is immutable once filled. Copy only the nine palettes,
                // retaining all live block/effect storage in the world region.
                let palettes: BTreeMap<_, _> = layer_positions(pos, 1)
                    .map(|p| {
                        let chunk = self
                            .region
                            .owned_chunk(p)
                            .expect("surface biome dependency");
                        (
                            (p.x, p.z),
                            chunk
                                .noise_biomes
                                .clone()
                                .expect("filled surface biome palette"),
                        )
                    })
                    .collect();
                generator.generate_surface_with_biomes(
                    self.region.owned_chunk_mut(pos),
                    graph.as_ref().unwrap(),
                    |_, qx, qy, qz| {
                        palettes[&(qx >> 2, qz >> 2)]
                            [((qy - (crate::MIN_Y >> 2)) * 16 + (qz & 3) * 4 + (qx & 3)) as usize]
                    },
                );
                Ok(Vec::new())
            }
            Carvers => {
                generator
                    .generate_carvers(self.region.owned_chunk_mut(pos), graph.as_ref().unwrap());
                Ok(Vec::new())
            }
            Features => {
                self.source_sequence += 1;
                let coverage = self.decorate_source(generator.seed(), pos, self.source_sequence);
                let mut missing = vec![structures::unsupported_structures()];
                if !coverage.missing.is_empty() {
                    missing.push(format!(
                        "{} placed features have incomplete coverage; see feature_sources",
                        coverage.missing.len()
                    ));
                }
                self.holders.get_mut(&key).unwrap().features = Some(coverage);
                Ok(missing)
            }
            InitializeLight => self.initialize_light(pos),
            Light => self.propagate_light(pos),
            Spawn => self.spawn_source(
                generator,
                pos,
                graph.as_ref().unwrap().disable_mob_generation,
            ),
            _ => Err(format!("no generation implementation for {status}")),
        }
    }

    fn coverage(&self, pos: ChunkPos, target: ChunkStatus) -> GenerationCoverage {
        let mut layers = Vec::new();
        let mut missing_stages = BTreeSet::new();
        let mut feature_sources = Vec::new();
        for status in ChunkStatus::ALL {
            let Some(radius) = ChunkPyramid::Generation.step(target).layer_radius(status) else {
                continue;
            };
            let mut layer = LayerCoverage {
                status,
                radius,
                required_chunks: (2 * radius + 1).pow(2),
                complete: 0,
                partial: 0,
                pending: 0,
                failed: 0,
            };
            for source in layer_positions(pos, radius) {
                let holder = self.holders.get(&(source.x, source.z));
                let progress = holder.map(|h| h.progress.stage(status));
                match progress.map_or(StageState::Pending, |p| p.state) {
                    StageState::Complete => layer.complete += 1,
                    StageState::Partial => layer.partial += 1,
                    StageState::Failed => layer.failed += 1,
                    _ => layer.pending += 1,
                }
                if let Some(progress) = progress {
                    missing_stages.extend(progress.missing.iter().map(|reason| MissingStage {
                        status,
                        reason: reason.clone(),
                    }));
                }
                if status == ChunkStatus::Features {
                    if let Some(report) = holder.and_then(|h| h.features.as_ref()) {
                        feature_sources.push(report.clone());
                    }
                }
            }
            if status > ChunkStatus::Spawn {
                missing_stages.insert(MissingStage {
                    status,
                    reason: "Native FULL conversion, postprocessing and tick-container lifecycle remain pending; only the supported block-entity materialization boundary is executed".into(),
                });
            }
            layers.push(layer);
        }
        feature_sources.sort_by_key(|source| source.sequence);
        let incoming_sources_finished = layer_positions(pos, 1).all(|source| {
            self.holders
                .get(&(source.x, source.z))
                .is_some_and(|holder| {
                    holder
                        .progress
                        .stage(ChunkStatus::Features)
                        .state
                        .has_data()
                })
        });
        GenerationCoverage {
            requested_status: target,
            target: self.holders[&(pos.x, pos.z)].progress.clone(),
            layers,
            feature_sources,
            missing_stages: missing_stages.into_iter().collect(),
            incoming_sources_finished,
        }
    }
}

fn panic_detail(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "generation stage panicked with a non-string payload".into()
    }
}

#[cfg(test)]
#[path = "generation/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "generation/structure_runtime_tests.rs"]
mod structure_runtime_tests;
