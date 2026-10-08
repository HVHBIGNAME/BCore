//! World access shared by placed features and their configured implementations.
use crate::ore::OreWorld;
use crate::tick_request::TickRequest;

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
