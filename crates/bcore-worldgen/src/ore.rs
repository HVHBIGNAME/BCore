//! OreFeature target replacement and air-exposure predicates.
use crate::features::{ore_positions, ore_state, OreKind};
use crate::simplex::WorldgenRandom;
use crate::{block, MAX_Y, MIN_Y};

/// All block states in the corresponding vanilla 26.1 tags.
pub const STONE_ORE_REPLACEABLES: &[u32] = &[1, 2, 4, 6];
pub const DEEPSLATE_ORE_REPLACEABLES: &[u32] = &[23452, 27923, 27924, 27925];
pub const BASE_STONE_OVERWORLD: &[u32] = &[1, 2, 4, 6, 23452, 27923, 27924, 27925];

/// Reads include neighbours of writable positions. Unknown chunks must not be
/// substituted with air. Out-of-build-height reads are handled by the feature.
pub trait OreWorld {
    fn ocean_floor_wg(&self, x: i32, z: i32) -> i32;
    fn get_block(&self, pos: (i32, i32, i32)) -> Option<u32>;
    fn set_block(&mut self, pos: (i32, i32, i32), state: u32) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub struct OreConfig {
    pub kind: OreKind,
    pub size: usize,
    pub discard_on_air_exposure: f32,
}

impl OreConfig {
    pub fn replacement(self, current: u32) -> Option<u32> {
        let state = ore_state(self.kind);
        if matches!(
            self.kind,
            OreKind::Granite
                | OreKind::Diorite
                | OreKind::Andesite
                | OreKind::Tuff
                | OreKind::Dirt
                | OreKind::Gravel
                | OreKind::Clay
        ) {
            return BASE_STONE_OVERWORLD.contains(&current).then_some(state);
        }
        if STONE_ORE_REPLACEABLES.contains(&current) {
            return Some(state);
        }
        if !DEEPSLATE_ORE_REPLACEABLES.contains(&current) {
            return None;
        }
        Some(match self.kind {
            OreKind::Coal => block::DEEPSLATE_COAL_ORE,
            OreKind::Iron => block::DEEPSLATE_IRON_ORE,
            OreKind::Copper => block::DEEPSLATE_COPPER_ORE,
            OreKind::Gold => block::DEEPSLATE_GOLD_ORE,
            OreKind::Diamond => block::DEEPSLATE_DIAMOND_ORE,
            OreKind::Emerald => block::DEEPSLATE_EMERALD_ORE,
            OreKind::Lapis => block::DEEPSLATE_LAPIS_ORE,
            OreKind::Redstone => block::DEEPSLATE_REDSTONE_ORE,
            OreKind::Infested => block::INFESTED_DEEPSLATE,
            _ => unreachable!("base-stone targets handled above"),
        })
    }
}

/// Calls replacement and exposure checks in the native fill traversal order.
pub fn place(
    world: &mut impl OreWorld,
    random: &mut WorldgenRandom,
    origin: (i32, i32, i32),
    config: OreConfig,
) -> bool {
    assert!(config.size <= 64, "ore size exceeds native codec range");
    assert!((0.0..=1.0).contains(&config.discard_on_air_exposure));
    let positions = ore_positions(
        random,
        &|x, z| world.ocean_floor_wg(x, z),
        origin,
        config.size,
    );
    let mut placed = false;
    for pos in positions {
        let Some(current) = world.get_block(pos) else {
            continue;
        };
        let Some(replacement) = config.replacement(current) else {
            continue;
        };
        let chance = config.discard_on_air_exposure;
        let skip_air_check = chance <= 0.0 || (chance < 1.0 && random.next_float() >= chance);
        if !skip_air_check && adjacent_to_air(world, pos) {
            continue;
        }
        placed |= world.set_block(pos, replacement);
    }
    placed
}

fn adjacent_to_air(world: &impl OreWorld, (x, y, z): (i32, i32, i32)) -> bool {
    for pos in [
        (x, y - 1, z),
        (x, y + 1, z),
        (x, y, z - 1),
        (x, y, z + 1),
        (x - 1, y, z),
        (x + 1, y, z),
    ] {
        if !(MIN_Y..=MAX_Y).contains(&pos.1) {
            return true;
        }
        let state = world
            .get_block(pos)
            .expect("ore region missing an exposure-check neighbour");
        if crate::heightmap::is_air(state) {
            return true;
        }
    }
    false
}
