//! Development protocol 26.1; migration to 26.2 follows worldgen parity work.

/// Protocol version for Minecraft Java Edition 26.1.
pub const PROTOCOL_VERSION: i32 = 775;

/// Human-readable Minecraft version string reported to clients.
pub const MC_VERSION: &str = "26.1";

/// Data version (`version.json` `world_version`).
/// Approximate: verify against the 26.1 server jar before registry-sync/login-gameplay.
pub const DATA_VERSION: i32 = 4903;

/// Server implementation name reported in status.
pub const IMPLEMENTATION_NAME: &str = "BCore";
