use bcore_core::ChunkPos;

use super::graph::ChunkStatus;

/// `Partial` is a finished attempt with known omissions, never a native completed
/// status. `Pending` and `Failed` are not promoted by the scheduler.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StageState {
    #[default]
    Pending,
    Running,
    Complete,
    Partial,
    Failed,
}

impl StageState {
    pub(crate) fn has_data(self) -> bool {
        matches!(self, Self::Complete | Self::Partial)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageProgress {
    pub state: StageState,
    pub attempts: u32,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkProgress {
    pub pos: ChunkPos,
    pub stages: [StageProgress; 12],
}

impl ChunkProgress {
    pub fn stage(&self, status: ChunkStatus) -> &StageProgress {
        &self.stages[status.index()]
    }

    /// Highest contiguous stage with complete implementation coverage.
    pub fn completed_status(&self) -> Option<ChunkStatus> {
        ChunkStatus::ALL
            .into_iter()
            .take_while(|&s| self.stage(s).state == StageState::Complete)
            .last()
    }

    pub(crate) fn available_status(&self) -> Option<ChunkStatus> {
        ChunkStatus::ALL
            .into_iter()
            .take_while(|&s| self.stage(s).state.has_data())
            .last()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureWork {
    pub step: usize,
    pub index: usize,
    pub feature: String,
    pub placed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFeature {
    pub source: ChunkPos,
    pub step: usize,
    pub index: usize,
    pub feature: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFeatureCoverage {
    pub source: ChunkPos,
    /// Monotonic within this world; records actual execution, not a global sort.
    pub sequence: u64,
    pub mineshafts_processed: bool,
    pub completed: Vec<FeatureWork>,
    pub missing: Vec<MissingFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MissingStage {
    pub status: ChunkStatus,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerCoverage {
    pub status: ChunkStatus,
    pub radius: usize,
    pub required_chunks: usize,
    pub complete: usize,
    pub partial: usize,
    pub pending: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationCoverage {
    pub requested_status: ChunkStatus,
    pub target: ChunkProgress,
    pub layers: Vec<LayerCoverage>,
    pub feature_sources: Vec<SourceFeatureCoverage>,
    pub missing_stages: Vec<MissingStage>,
    /// All potential direct incoming writers have finished their supported work.
    /// This is not FULL, lighting, or postProcessGeneration completion.
    pub incoming_sources_finished: bool,
}

impl GenerationCoverage {
    pub fn missing_features(&self) -> impl Iterator<Item = &MissingFeature> {
        self.feature_sources
            .iter()
            .flat_map(|source| &source.missing)
    }

    pub fn is_complete(&self) -> bool {
        self.missing_stages.is_empty()
            && self.missing_features().next().is_none()
            && self
                .layers
                .iter()
                .all(|layer| layer.complete == layer.required_chunks)
    }
}
