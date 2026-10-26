//! Native full-cube item/state bindings and standing-player dimensions.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct BlockItem {
    item_id: u32,
    item: String,
    block: String,
    state_id: u32,
    max_stack_size: u8,
    stateless: bool,
    full_collision_cube: bool,
    has_block_entity: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(super) struct PlayerBounds {
    pub block_range_default: f64,
    pub creative_block_range_addition: f64,
    pub width: f64,
    pub height: f64,
    pub eye_height: f64,
}

struct Bindings {
    items: BTreeMap<u32, PlacementItem>,
    states: BTreeSet<u32>,
    player: PlayerBounds,
}

struct PlacementItem {
    states: [u32; 3],
    max_stack_size: u8,
}

fn add_pillars(items: &mut BTreeMap<u32, PlacementItem>, states: &mut BTreeSet<u32>) {
    #[derive(Deserialize)]
    struct Axes {
        x: u32,
        y: u32,
        z: u32,
    }
    #[derive(Deserialize)]
    struct Pillar {
        item_id: u32,
        item: String,
        block: String,
        block_class: String,
        states: Axes,
        default_state_id: u32,
        max_stack_size: u8,
        full_collision_cube: bool,
        has_block_entity: bool,
        empty_fluid: bool,
    }
    #[derive(Deserialize)]
    struct Capture {
        minecraft: String,
        protocol: i32,
        jar_sha256: String,
        axis_full_cube_block_items: Vec<Pillar>,
    }
    let capture: Capture =
        serde_json::from_str(include_str!("../../data/pillar_placement_26_2.json"))
            .expect("authenticated 26.2 pillar bindings");
    assert_eq!(capture.minecraft, bcore_registry::MC_VERSION);
    assert_eq!(capture.protocol, bcore_registry::PROTOCOL_VERSION);
    assert_eq!(capture.jar_sha256, bcore_registry::SERVER_JAR_SHA256);
    assert_eq!(capture.axis_full_cube_block_items.len(), 56);
    let registry = bcore_registry::vanilla();
    for row in capture.axis_full_cube_block_items {
        assert_eq!(
            row.block_class,
            "net.minecraft.world.level.block.RotatedPillarBlock"
        );
        assert!(row.full_collision_cube && !row.has_block_entity && row.empty_fluid);
        assert!((1..=64).contains(&row.max_stack_size));
        assert_eq!(row.default_state_id, row.states.y);
        assert_eq!(
            registry.registries["ITEM"][row.item_id as usize].key,
            row.item
        );
        let axes = [row.states.x, row.states.y, row.states.z];
        for (axis, id) in ["x", "y", "z"].into_iter().zip(axes) {
            let state = &registry.states[id as usize];
            assert_eq!(state.name, row.block);
            assert_eq!(state.properties.len(), 1);
            assert_eq!(state.properties["axis"], axis);
            assert_eq!(state.default, axis == "y");
            assert!(states.insert(id));
        }
        assert!(items
            .insert(
                row.item_id,
                PlacementItem {
                    states: axes,
                    max_stack_size: row.max_stack_size,
                },
            )
            .is_none());
    }
}

fn bindings() -> &'static Bindings {
    static DATA: OnceLock<Bindings> = OnceLock::new();
    DATA.get_or_init(|| {
        #[derive(Deserialize)]
        struct Capture {
            minecraft: String,
            protocol: i32,
            jar_sha256: String,
            standing_player: PlayerBounds,
            stateless_full_cube_block_items: Vec<BlockItem>,
        }
        let capture: Capture =
            serde_json::from_str(include_str!("../../data/block_actions_26_2.json"))
                .expect("authenticated 26.2 block-item bindings");
        assert_eq!(capture.minecraft, bcore_registry::MC_VERSION);
        assert_eq!(capture.protocol, bcore_registry::PROTOCOL_VERSION);
        assert_eq!(capture.jar_sha256, bcore_registry::SERVER_JAR_SHA256);
        assert_eq!(capture.stateless_full_cube_block_items.len(), 197);
        let registry = bcore_registry::vanilla();
        let mut items = BTreeMap::new();
        let mut states = BTreeSet::new();
        for row in capture.stateless_full_cube_block_items {
            let state = &registry.states[row.state_id as usize];
            assert_eq!(state.name, row.block);
            assert!(state.default && state.properties.is_empty());
            assert_eq!(
                registry.registries["ITEM"][row.item_id as usize].key,
                row.item
            );
            assert!(row.stateless && row.full_collision_cube && !row.has_block_entity);
            assert!((1..=64).contains(&row.max_stack_size));
            states.insert(row.state_id);
            assert!(items
                .insert(
                    row.item_id,
                    PlacementItem {
                        states: [row.state_id; 3],
                        max_stack_size: row.max_stack_size,
                    },
                )
                .is_none());
        }
        add_pillars(&mut items, &mut states);
        Bindings {
            items,
            states,
            player: capture.standing_player,
        }
    })
}

pub(super) fn block_item(item: u32, count: u8) -> Option<u32> {
    placement_state(item, count, 1)
}

pub(super) fn placement_state(item: u32, count: u8, face: u8) -> Option<u32> {
    let axis = match face {
        0 | 1 => 1,
        2 | 3 => 2,
        4 | 5 => 0,
        _ => return None,
    };
    let row = bindings().items.get(&item)?;
    (count > 0 && count <= row.max_stack_size).then_some(row.states[axis])
}

pub(super) fn basic_state(state: u32) -> bool {
    bindings().states.contains(&state)
}

pub(super) fn player_bounds() -> PlayerBounds {
    bindings().player
}
