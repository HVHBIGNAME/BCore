//! JVM virtualization bridge for Bukkit/Spigot/Paper plugins.
//!
//! The bridge deliberately keeps the original plugin bytecode intact. Java API
//! classes are supplied by the plugin class path and their native methods are
//! the seam where calls are forwarded to BCore. See `docs/adr/0001-plugin-translation-strategy.md`.

mod bridge;

pub use bridge::{JavaBridge, JavaBridgeError, JavaPluginHandle};
