//! World access shared by placed features and their configured implementations.
use crate::ore::OreWorld;
use crate::structure::template::TemplateEffect;
use crate::tick_request::{TickRequest, TickTarget};

pub type Pos = (i32, i32, i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureHeightmap {
    WorldSurfaceWg,
    WorldSurface,
    OceanFloorWg,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

/// Heights are first-free Y values; all block positions are absolute.
/// The source stage controls read availability and the write radius.
pub trait FeatureWorld: OreWorld {
    fn feature_biome(&self, pos: Pos) -> u32;
    fn feature_height(&self, kind: FeatureHeightmap, x: i32, z: i32) -> i32;
    fn can_write_feature(&self, pos: Pos) -> bool;
    fn set_feature_block(&mut self, pos: Pos, state: u32, flags: i32) -> bool;
    fn mark_feature_postprocessing(&mut self, pos: Pos);
    fn schedule_feature_tick(&mut self, request: TickRequest) -> bool;

    /// Resolve a hive at the lookup boundary, including supported pending data.
    /// None means no hive was found; unsupported worlds must report an error.
    fn lookup_feature_beehive(&mut self, _pos: Pos) -> Result<Option<Vec<i32>>, FeatureError> {
        Err(FeatureError::Unsupported("feature beehive lookup".into()))
    }

    /// Native typed brushable lookup followed by setLootTable if present. The
    /// lookup may materialize pending DUMMY data even after a rejected write.
    /// False means the lookup found no brushable, not an unsupported world.
    fn set_feature_brushable_loot(
        &mut self,
        _pos: Pos,
        _table: &str,
        _seed: i64,
    ) -> Result<bool, FeatureError> {
        Err(FeatureError::Unsupported(
            "brushable feature loot lookup".into(),
        ))
    }

    /// Immediate template load/clear/entity callbacks in the current source task.
    fn apply_feature_template_effect(
        &mut self,
        _effect: TemplateEffect,
    ) -> Result<(), FeatureError> {
        Err(FeatureError::Unsupported("feature template effects".into()))
    }

    /// StructureTemplate's updateNeighborsAt callback, after each final shape.
    fn update_feature_neighbours(&mut self, _pos: Pos, _state: u32) -> Result<(), FeatureError> {
        Err(FeatureError::Unsupported(
            "feature neighbour notification".into(),
        ))
    }

    /// Membership in the pending native tick queue, including earlier requests.
    fn has_feature_tick(&self, _pos: Pos, _target: TickTarget) -> Result<bool, FeatureError> {
        Err(FeatureError::Unsupported(
            "feature scheduled-tick lookup".into(),
        ))
    }

    fn reset_feature_potent_sulfur(&mut self, _pos: Pos) -> Result<bool, FeatureError> {
        Err(FeatureError::Unsupported(
            "potent sulfur block-entity lookup".into(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureError {
    Unsupported(String),
    InvalidConfig(String),
    MissingData(String),
}

impl std::fmt::Display for FeatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(detail) => write!(f, "unsupported feature operation: {detail}"),
            Self::InvalidConfig(detail) => write!(f, "invalid feature configuration: {detail}"),
            Self::MissingData(detail) => write!(f, "missing feature data: {detail}"),
        }
    }
}

impl std::error::Error for FeatureError {}
