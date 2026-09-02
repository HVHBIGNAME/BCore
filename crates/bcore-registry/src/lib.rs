//! BCore data-driven block/item registry (future work).
//!
//! Modern Minecraft assigns block/item IDs during the login "registry sync"
//! phase from data files; they are NOT fixed constants. This crate will host
//! the registry-sync implementation and the generated vanilla data. Until then,
//! `bcore_core::registry` holds a small static bootstrap table.
