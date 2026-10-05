//! Selected feature boundaries for complete, matched-input differential replay.
use super::{graph::layer_positions, GenerationError, GenerationState, GenerationWorld, Key};
use crate::simplex::WorldgenRandom;
use crate::{ChunkPos, GeneratedChunk};

#[derive(Debug)]
pub struct FeatureTrace {
    pub source: ChunkPos,
    pub feature: String,
    pub boundary: &'static str,
    /// Read from a copied RNG, leaving the source's exact continuation intact.
    pub next_i64: i64,
    pub chunks: Vec<GeneratedChunk>,
}

#[derive(Default)]
pub(super) struct FeatureTraceState {
    selected: Option<(Key, String)>,
    observations: Vec<FeatureTrace>,
}

impl GenerationWorld {
    pub fn trace_feature(&self, source: ChunkPos, feature: &str) -> Result<(), GenerationError> {
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        state.feature_trace = FeatureTraceState {
            selected: Some((
                (source.x, source.z),
                feature.trim_start_matches("minecraft:").into(),
            )),
            observations: Vec::new(),
        };
        Ok(())
    }

    pub fn take_feature_traces(&self) -> Result<Vec<FeatureTrace>, GenerationError> {
        let mut state = self.state.lock().map_err(|_| GenerationError::Poisoned)?;
        Ok(std::mem::take(&mut state.feature_trace.observations))
    }
}

impl GenerationState {
    pub(super) fn record_feature_trace(
        &mut self,
        source: ChunkPos,
        feature: &str,
        random: &WorldgenRandom,
        boundary: &'static str,
    ) {
        let name = feature.trim_start_matches("minecraft:");
        if !self
            .feature_trace
            .selected
            .as_ref()
            .is_some_and(|(pos, feature)| *pos == (source.x, source.z) && feature == name)
        {
            return;
        }
        let chunks = layer_positions(source, 1)
            .filter_map(|pos| self.region.owned_chunk(pos).map(|chunk| (*chunk).clone()))
            .collect();
        let mut copy = WorldgenRandom::new(0);
        copy.source = random.source.clone();
        self.feature_trace.observations.push(FeatureTrace {
            source,
            feature: format!("minecraft:{name}"),
            boundary,
            next_i64: copy.next_long(),
            chunks,
        });
    }
}
