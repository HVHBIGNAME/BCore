//! Real SPAWN execution over retained biome, block and light storage.
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use crate::feature_world::{FeatureWorld, Pos};
use crate::generated_entity::{GeneratedEntity, GeneratedMob};
use crate::spawn::{
    self, MobKind, SpawnBorder, SpawnDifficulty, SpawnEnvironment, SpawnError, SpawnOptions,
    SpawnRandom, SpawnReport, SpawnResult, SpawnedMob,
};
use crate::{ChunkPos, WorldGenerator};

use super::GenerationState;

/// Server-owned inputs which cannot be derived from a world seed. Tests may
/// supply explicitly controlled entropy; production identities use host entropy.
pub trait SpawnInputs: Send + Sync {
    fn spawn_mobs(&self) -> bool;
    fn entity_seed(&self, kind: MobKind) -> SpawnResult<i64>;
    fn difficulty_at(&self, pos: Pos) -> SpawnResult<SpawnDifficulty>;
    fn game_time(&self) -> SpawnResult<i64>;
    fn border(&self) -> SpawnBorder {
        SpawnBorder::default()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SpawnSettings {
    pub spawn_mobs: bool,
    pub game_time: i64,
    pub overworld_time: i64,
    pub difficulty: u8,
    pub moon_brightness: f32,
    pub border: SpawnBorder,
}

impl Default for SpawnSettings {
    fn default() -> Self {
        Self {
            spawn_mobs: true,
            game_time: 0,
            overworld_time: 0,
            difficulty: 2,
            moon_brightness: 1.0,
            border: SpawnBorder::default(),
        }
    }
}

/// Independent inputs for a fresh world. Settings can change while generation
/// runs: the small settings lock is independent from the long-lived world lock.
#[derive(Debug)]
pub struct WorldSpawnInputs {
    settings: RwLock<SpawnSettings>,
    entropy: RandomState,
    counter: AtomicU64,
}

impl Default for WorldSpawnInputs {
    fn default() -> Self {
        Self {
            settings: RwLock::new(SpawnSettings::default()),
            entropy: RandomState::new(),
            counter: AtomicU64::new(0),
        }
    }
}

impl WorldSpawnInputs {
    pub fn settings(&self) -> SpawnSettings {
        *self.settings.read().expect("world spawn-input lock")
    }

    pub fn set_settings(&self, settings: SpawnSettings) -> SpawnResult<()> {
        let b = settings.border;
        if settings.difficulty > 3
            || !(0.0..=1.0).contains(&settings.moon_brightness)
            || ![b.min_x, b.min_z, b.max_x, b.max_z]
                .into_iter()
                .all(f64::is_finite)
            || b.min_x > b.max_x
            || b.min_z > b.max_z
        {
            return Err(SpawnError::InvalidSettings("world spawn inputs".into()));
        }
        *self.settings.write().expect("world spawn-input lock") = settings;
        Ok(())
    }
}

impl SpawnInputs for WorldSpawnInputs {
    fn spawn_mobs(&self) -> bool {
        self.settings().spawn_mobs
    }
    fn entity_seed(&self, _kind: MobKind) -> SpawnResult<i64> {
        // RandomState is keyed from host entropy. Neither the key nor this
        // counter depends on world seed, placement RNG or region RNG.
        let nonce = self.counter.fetch_add(1, Ordering::Relaxed);
        Ok(self.entropy.hash_one(nonce) as i64)
    }
    fn difficulty_at(&self, _pos: Pos) -> SpawnResult<SpawnDifficulty> {
        let settings = self.settings();
        Ok(SpawnDifficulty {
            difficulty: settings.difficulty,
            overworld_time: settings.overworld_time,
            inhabited_time: 0, // WorldGenRegion, not a populated LevelChunk.
            moon_brightness: settings.moon_brightness,
        })
    }
    fn game_time(&self) -> SpawnResult<i64> {
        Ok(self.settings().game_time)
    }
    fn border(&self) -> SpawnBorder {
        self.settings().border
    }
}

struct RegionSpawnEnvironment<'a> {
    region: &'a crate::region::FeatureRegion,
    light: &'a crate::lighting::GenerationLight,
    light_missing: Option<&'a str>,
    inputs: &'a dyn SpawnInputs,
    random: SpawnRandom,
}

impl SpawnEnvironment for RegionSpawnEnvironment<'_> {
    fn spawn_mobs(&self) -> bool {
        self.inputs.spawn_mobs()
    }
    fn random(&mut self) -> &mut SpawnRandom {
        &mut self.random
    }
    fn entity_seed(&mut self, kind: MobKind) -> SpawnResult<i64> {
        self.inputs.entity_seed(kind)
    }
    fn current_difficulty_at(&self, pos: Pos) -> SpawnResult<SpawnDifficulty> {
        self.inputs.difficulty_at(pos)
    }
    fn game_time(&self) -> SpawnResult<i64> {
        self.inputs.game_time()
    }
    fn border(&self) -> SpawnBorder {
        self.inputs.border()
    }
    fn raw_brightness(
        &self,
        _world: &dyn FeatureWorld,
        pos: Pos,
        sky_darken: i32,
    ) -> SpawnResult<i32> {
        if let Some(reason) = self.light_missing {
            return Err(SpawnError::MissingData(format!("SPAWN light: {reason}")));
        }
        Ok(self.light.raw_brightness(pos, sky_darken))
    }
    // WorldGenRegion.getSkyDarken() returns zero even when ServerLevel is dark.
    // Its default obstruction query does not index stored proto-entity NBT.
    fn add_fresh_mob(&mut self, mob: &SpawnedMob) -> SpawnResult<()> {
        let saved = GeneratedEntity::Mob(Box::new(GeneratedMob::from_spawn(mob)?));
        self.region
            .store_generated_entity(saved)
            .map_err(|error| SpawnError::MissingData(error.to_string()))
    }
}

impl GenerationState {
    pub(super) fn spawn_source(
        &mut self,
        generator: WorldGenerator,
        pos: ChunkPos,
        disable_mob_generation: bool,
    ) -> Result<Vec<String>, String> {
        let mut environment = RegionSpawnEnvironment {
            region: &self.region,
            light: &self.light,
            light_missing: self.light_missing.as_deref(),
            inputs: self.spawn_inputs.as_ref(),
            random: SpawnRandom::for_region(generator.seed(), [pos.x, pos.z]),
        };
        let result = spawn::spawn_original_mobs(
            &self.region,
            &mut environment,
            generator.seed(),
            [pos.x, pos.z],
            SpawnOptions {
                disable_mob_generation,
                ..SpawnOptions::default()
            },
        );
        let (report, missing) = match result {
            Ok(report) => (report, Vec::new()),
            Err(failure) => (failure.partial, vec![failure.error.to_string()]),
        };
        // Added mobs already belong to the region, including a successful prefix
        // before an error. A Partial claim is retained and is never rerun.
        self.holders.get_mut(&(pos.x, pos.z)).unwrap().spawn = Some(report);
        Ok(missing)
    }
}

impl super::GenerationWorld {
    /// Actual finalized results of a SPAWN attempt, including its partial prefix.
    pub fn spawn_report(
        &self,
        pos: ChunkPos,
    ) -> Result<Option<SpawnReport>, super::GenerationError> {
        let state = self
            .state
            .lock()
            .map_err(|_| super::GenerationError::Poisoned)?;
        Ok(state
            .holders
            .get(&(pos.x, pos.z))
            .and_then(|h| h.spawn.clone()))
    }
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
