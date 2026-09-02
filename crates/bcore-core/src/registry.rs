//! Static block/item registry — **development bootstrap only**.
//!
//! In modern Minecraft the block/item IDs are data-driven: they are assigned
//! during the "registry sync" phase of the configuration/login state, and they
//! are **not** fixed constants. The numeric IDs below exist purely to bootstrap
//! early development (flat/simple worldgen, tests) and are NOT authoritative
//! for Minecraft 26.2.
//!
//! Do not use these IDs for network serialization of chunks or items until
//! `bcore-registry` implements registry sync against the real vanilla data.

/// A block id. See the module docs: these are bootstrap values, not the real
/// 26.2 registry ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u16);

pub const AIR: BlockId = BlockId(0);
pub const STONE: BlockId = BlockId(1);
pub const GRANITE: BlockId = BlockId(2);
pub const POLISHED_GRANITE: BlockId = BlockId(3);
pub const DIORITE: BlockId = BlockId(4);
pub const POLISHED_DIORITE: BlockId = BlockId(5);
pub const ANDESITE: BlockId = BlockId(6);
pub const POLISHED_ANDESITE: BlockId = BlockId(7);
pub const GRASS_BLOCK: BlockId = BlockId(8);
pub const DIRT: BlockId = BlockId(9);
pub const COARSE_DIRT: BlockId = BlockId(10);
pub const PODZOL: BlockId = BlockId(11);
pub const COBBLESTONE: BlockId = BlockId(12);
pub const BEDROCK: BlockId = BlockId(13);
pub const SAND: BlockId = BlockId(14);
pub const GRAVEL: BlockId = BlockId(15);
pub const WATER: BlockId = BlockId(16);
pub const LAVA: BlockId = BlockId(17);
