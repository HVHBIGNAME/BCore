//! Minecraft Java Edition version identity for this server.

/// Protocol version for Minecraft Java Edition 26.2.
pub const PROTOCOL_VERSION: i32 = 776;

/// Human-readable Minecraft version string reported to clients.
pub const MC_VERSION: &str = "26.2";

/// Data version (`version.json` `world_version`).
/// Verified against the official 26.2 server's `version.json`.
pub const DATA_VERSION: i32 = 4903;

/// Server implementation name reported in status.
pub const IMPLEMENTATION_NAME: &str = "BCore";
