//! Minecraft 26.2 version constants.

/// Protocol version for Minecraft Java Edition 26.2.
pub const PROTOCOL_VERSION: i32 = 776;

/// Human-readable Minecraft version string reported to clients.
pub const MC_VERSION: &str = "26.2";

/// Data version for Minecraft 26.2, used by NBT `DataVersion` and registry sync.
///
/// TODO: pin the exact release data version before implementing registry sync
/// or login gameplay. The 26.2 snapshots ended at data version 4893
/// (Snapshot 8); the release value is slightly higher. Do not treat this value
/// as authoritative until verified against the vanilla `version.json`.
pub const DATA_VERSION: i32 = 4894;

/// Server implementation name reported in status.
pub const IMPLEMENTATION_NAME: &str = "BCore";
