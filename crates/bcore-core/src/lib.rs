//! Core shared types for BCore: version constants, protocol primitives,
//! positions, identifiers, and a bootstrap registry.

pub mod identifier;
pub mod position;
pub mod registry;
pub mod varint;
pub mod version;

pub use identifier::Identifier;
pub use position::{BlockPos, ChunkPos, Position};
pub use version::{DATA_VERSION, IMPLEMENTATION_NAME, MC_VERSION, PROTOCOL_VERSION};
