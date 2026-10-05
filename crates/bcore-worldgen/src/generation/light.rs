//! Shared native light storage and real INITIALIZE_LIGHT / LIGHT stage execution.
use super::{ChunkStatus, GenerationState, StageState};
use crate::feature_world::FeatureError;
use crate::ChunkPos;

impl GenerationState {
    pub(super) fn refresh_light_inputs(&mut self, current: Option<ChunkPos>) -> Result<(), String> {
        let writes = std::mem::take(&mut self.region.light_updates);
        if self.light_missing.is_some() {
            return Ok(());
        }
        let mut touched: std::collections::BTreeSet<_> =
            writes.iter().map(|&(p, _)| (p.0 >> 4, p.2 >> 4)).collect();
        if let Some(pos) = current {
            touched.insert((pos.x, pos.z));
        }
        let positions: Vec<_> = touched
            .into_iter()
            .filter_map(|(x, z)| {
                let holder = self.holders.get(&(x, z))?;
                let pos = ChunkPos::new(x, z);
                ((Some(pos) == current
                    || holder
                        .progress
                        .stage(ChunkStatus::Features)
                        .state
                        .has_data())
                    && self.region.owned_chunk(pos).is_some())
                .then_some(pos)
            })
            .collect();
        // Make newly completed FEATURES neighbours readable before draining
        // checks. Initialized columns retain their old states until each ordered
        // write updates its section occupancy and incremental sky-source map.
        for &pos in &positions {
            if !self.light.is_initialized(pos) {
                let chunk = self.region.owned_chunk(pos).unwrap();
                self.light
                    .register_chunk(pos, chunk.states())
                    .map_err(|e| e.to_string())?;
            }
        }
        let writes: Vec<_> = writes
            .into_iter()
            .filter(|&(p, _)| self.light.is_initialized(ChunkPos::new(p.0 >> 4, p.2 >> 4)))
            .collect();
        if let Err(error) = self.light.apply_block_updates(&writes) {
            self.invalidate_light(error.to_string());
            return Err(error.to_string());
        }
        for pos in positions {
            if !self.light.is_initialized(pos) {
                continue;
            }
            let result = {
                let chunk = self
                    .region
                    .owned_chunk(pos)
                    .expect("FEATURES block data exists");
                self.light.register_chunk(pos, chunk.states())
            };
            match result {
                Ok(()) => {}
                Err(FeatureError::Unsupported(reason)) => {
                    self.invalidate_light(format!(
                        "native light update after feature writes at {pos:?}: {reason}"
                    ));
                    break;
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        if self.light_missing.is_none() {
            self.capture_light_outputs();
        }
        Ok(())
    }

    fn invalidate_light(&mut self, reason: String) {
        self.light_missing = Some(reason.clone());
        for (&(x, z), holder) in &mut self.holders {
            if self.region.owned_chunk(ChunkPos::new(x, z)).is_some() {
                self.region.owned_chunk_mut(ChunkPos::new(x, z)).light = None;
            }
            for status in [ChunkStatus::InitializeLight, ChunkStatus::Light] {
                let progress = &mut holder.progress.stages[status.index()];
                if progress.state.has_data() {
                    progress.state = StageState::Partial;
                    progress.missing.push(reason.clone());
                }
            }
        }
    }

    fn capture_light_outputs(&mut self) {
        for pos in self.light.take_changed_columns() {
            if self.region.owned_chunk(pos).is_none() {
                continue;
            }
            let snapshot = self.light.chunk_light(pos);
            if self.light.is_initialized(pos)
                || snapshot
                    .sections
                    .iter()
                    .any(|section| section.sky.is_some() || section.block.is_some())
            {
                self.region.owned_chunk_mut(pos).light = Some(snapshot);
            } else {
                // Last nonempty section removal can retire neighbouring padding.
                self.region.owned_chunk_mut(pos).light = None;
            }
        }
    }

    pub(super) fn initialize_light(&mut self, pos: ChunkPos) -> Result<Vec<String>, String> {
        self.refresh_light_inputs(None)?;
        if let Some(reason) = &self.light_missing {
            return Ok(vec![reason.clone()]);
        }
        {
            let chunk = self
                .region
                .owned_chunk(pos)
                .expect("INITIALIZE_LIGHT feature input");
            self.light
                .initialize_generated_chunk(&chunk)
                .map_err(|error| error.to_string())?;
        }
        self.capture_light_outputs();
        Ok(Vec::new())
    }

    pub(super) fn propagate_light(&mut self, pos: ChunkPos) -> Result<Vec<String>, String> {
        if let Some(reason) = &self.light_missing {
            return Ok(vec![reason.clone()]);
        }
        self.light
            .propagate_chunk(pos)
            .map_err(|error| error.to_string())?;
        self.capture_light_outputs();
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::{ChunkHolder, StageState};
    use crate::lighting::ChunkLight;
    use crate::{WorldGenerator, MIN_Y, WORLD_HEIGHT};

    fn setup() -> (GenerationState, serde_json::Value) {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../data/lighting_reference_26_1_v2.json"))
                .unwrap();
        let fixture = data["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "floor_cave")
            .unwrap()
            .clone();
        let mut state = GenerationState::new(WorldGenerator::new(0));
        for pos in fixture["chunks"].as_array().unwrap() {
            let pos = ChunkPos::new(
                pos[0].as_i64().unwrap() as i32,
                pos[1].as_i64().unwrap() as i32,
            );
            let mut holder = ChunkHolder::new(pos);
            for stage in &mut holder.progress.stages[..=ChunkStatus::Features.index()] {
                stage.state = StageState::Complete;
            }
            state.holders.insert((pos.x, pos.z), holder);
            state.region.owned_chunk_mut(pos).states = vec![0; WORLD_HEIGHT as usize * 256];
        }
        for row in fixture["boxes"].as_array().unwrap() {
            let b: Vec<i32> = serde_json::from_value(row.clone()).unwrap();
            for y in b[1]..=b[4] {
                for z in b[2]..=b[5] {
                    for x in b[0]..=b[3] {
                        state
                            .region
                            .owned_chunk_mut(ChunkPos::new(x >> 4, z >> 4))
                            .set((x & 15) as usize, y, (z & 15) as usize, b[6] as u32);
                    }
                }
            }
        }
        (state, fixture)
    }

    fn expected(data: &serde_json::Value) -> ChunkLight {
        let decode = |value: &serde_json::Value| {
            value.as_str().map(|s| {
                s.as_bytes()
                    .chunks_exact(2)
                    .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                    .collect()
            })
        };
        ChunkLight {
            min_section_y: data["min_section_y"].as_i64().unwrap() as i32,
            sections: data["sections"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| crate::lighting::LightSection {
                    sky: decode(&s[0]),
                    block: decode(&s[1]),
                    sky_empty: s[2].as_bool().unwrap(),
                    block_empty: s[3].as_bool().unwrap(),
                })
                .collect(),
        }
    }

    fn run_initialization(state: &mut GenerationState, fixture: &serde_json::Value) {
        for p in fixture["chunks"].as_array().unwrap() {
            let pos = ChunkPos::new(p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32);
            state
                .run_stage(WorldGenerator::new(0), pos, ChunkStatus::InitializeLight)
                .unwrap();
        }
    }

    #[test]
    fn actual_light_stages_publish_native_layers_without_claiming_spawn_or_full() {
        let (mut state, fixture) = setup();
        let pos = ChunkPos::new(0, 0);
        run_initialization(&mut state, &fixture);
        assert_eq!(
            state.region.owned_chunk(pos).unwrap().light(),
            Some(&expected(&fixture["initialized"]))
        );
        assert_eq!(
            state.holders[&(0, 0)]
                .progress
                .stage(ChunkStatus::Light)
                .state,
            StageState::Pending
        );
        state
            .run_stage(WorldGenerator::new(0), pos, ChunkStatus::Light)
            .unwrap();
        assert_eq!(
            state.region.owned_chunk(pos).unwrap().light(),
            Some(&expected(&fixture["center_only"]))
        );
        assert!(state.light.is_enabled(pos));
        assert!(!state.light.is_enabled(ChunkPos::new(1, 0)));
        state
            .run_stage(WorldGenerator::new(0), pos, ChunkStatus::Light)
            .unwrap();
        let progress = &state.holders[&(0, 0)].progress;
        assert_eq!(progress.stage(ChunkStatus::Light).attempts, 1);
        assert_eq!(
            progress.stage(ChunkStatus::Spawn).state,
            StageState::Pending
        );
        assert_eq!(progress.stage(ChunkStatus::Full).state, StageState::Pending);
    }

    #[test]
    fn unsupported_light_changing_edits_invalidate_snapshots_and_completed_claims() {
        let (mut state, fixture) = setup();
        let pos = ChunkPos::new(0, 0);
        run_initialization(&mut state, &fixture);
        state
            .run_stage(WorldGenerator::new(0), pos, ChunkStatus::Light)
            .unwrap();
        let before = state
            .region
            .owned_chunk(pos)
            .unwrap()
            .light()
            .unwrap()
            .clone();
        let ore = crate::block_predicate::catalog()
            .default_state("coal_ore")
            .unwrap();
        state.region.owned_chunk_mut(pos).set(0, MIN_Y, 0, ore);
        state.refresh_light_inputs(Some(pos)).unwrap();
        assert!(state.light_missing.is_none());
        assert_eq!(
            state.region.owned_chunk(pos).unwrap().light(),
            Some(&before)
        );
        let glow = crate::block_predicate::catalog()
            .default_state("glowstone")
            .unwrap();
        state.region.owned_chunk_mut(pos).set(0, MIN_Y, 0, glow);
        state.refresh_light_inputs(Some(pos)).unwrap();
        assert!(state
            .light_missing
            .as_ref()
            .unwrap()
            .contains("invalidation"));
        assert!(state.region.owned_chunk(pos).unwrap().light().is_none());
        assert_eq!(
            state.holders[&(0, 0)]
                .progress
                .stage(ChunkStatus::Light)
                .state,
            StageState::Partial
        );
        assert!(!state.propagate_light(pos).unwrap().is_empty());
    }

    #[test]
    fn ordered_neighbour_feature_writes_update_light_and_keep_completed_claims() {
        use crate::feature_world::FeatureWorld;
        let (mut state, fixture) = setup();
        let pos = ChunkPos::new(0, 0);
        run_initialization(&mut state, &fixture);
        state
            .run_stage(WorldGenerator::new(0), pos, ChunkStatus::Light)
            .unwrap();
        let available = state
            .holders
            .iter()
            .filter_map(|(&p, h)| h.progress.available_status().map(|s| (p, s)))
            .collect();
        state
            .region
            .begin_source(ChunkPos::new(1, 0), ChunkStatus::Features, available);
        let glow = crate::block_predicate::catalog()
            .default_state("glowstone")
            .unwrap();
        assert!(state.region.set_feature_block((0, MIN_Y, 0), glow, 2));
        assert!(state
            .region
            .set_feature_block((0, MIN_Y, 0), crate::block::STONE, 2));
        assert!(state.region.set_feature_block((0, MIN_Y, 0), glow, 2));
        assert_eq!(state.region.light_updates.len(), 3);
        state.region.end_source();
        state.refresh_light_inputs(None).unwrap();
        assert!(state.region.light_updates.is_empty());
        assert!(state.light_missing.is_none());
        assert_eq!(state.light.block_brightness((0, MIN_Y, 0)), 15);
        assert_eq!(
            state.region.owned_chunk(pos).unwrap().light(),
            Some(&state.light.chunk_light(pos))
        );
        assert_eq!(
            state.holders[&(0, 0)]
                .progress
                .stage(ChunkStatus::Light)
                .state,
            StageState::Complete
        );
        assert_eq!(
            state.holders[&(0, 0)]
                .progress
                .stage(ChunkStatus::Light)
                .attempts,
            1
        );
    }

    #[test]
    fn missing_feature_dependency_does_not_panic_during_light_cleanup() {
        let world = crate::GenerationWorld::new(0);
        let pos = ChunkPos::new(0, 0);
        let error = world
            .state
            .lock()
            .unwrap()
            .run_stage(WorldGenerator::new(0), pos, ChunkStatus::Features)
            .unwrap_err();
        assert!(error.to_string().contains("dependency"));
        assert_eq!(
            world
                .progress(pos)
                .unwrap()
                .unwrap()
                .stage(ChunkStatus::Features)
                .state,
            StageState::Failed
        );
    }

    #[test]
    fn light_publication_keeps_untouched_distant_columns_shared() {
        let (mut state, fixture) = setup();
        run_initialization(&mut state, &fixture);
        let far = ChunkPos::new(1000, 1000);
        let mut holder = ChunkHolder::new(far);
        for stage in &mut holder.progress.stages[..=ChunkStatus::Features.index()] {
            stage.state = StageState::Complete;
        }
        state.holders.insert((far.x, far.z), holder);
        state
            .region
            .owned_chunk_mut(far)
            .set(0, MIN_Y, 0, crate::block::STONE);
        state
            .run_stage(WorldGenerator::new(0), far, ChunkStatus::InitializeLight)
            .unwrap();
        let retained = state.region.owned_chunk(far).unwrap();
        assert!(retained.light().is_some());
        state
            .run_stage(
                WorldGenerator::new(0),
                ChunkPos::new(0, 0),
                ChunkStatus::Light,
            )
            .unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&retained, &state.region.owned_chunk(far).unwrap()),
            "unrelated light work must not clone/repack an already-retained column"
        );
    }
}
