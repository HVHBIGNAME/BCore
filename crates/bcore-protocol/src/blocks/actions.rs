//! Supported instant creative destroy and full-cube placement validation.
use bcore_core::BlockPos;

use super::{bindings, packets, runtime::BlockOwner, PlayerContext};
use crate::shared::{PlayerHandle, ServerState};
use crate::world_state::WorldError;

impl BlockOwner {
    pub(super) fn break_block(
        &self,
        server: &ServerState,
        player: &PlayerHandle,
        context: &PlayerContext,
        action: u8,
        pos: BlockPos,
    ) -> Result<(), WorldError> {
        if action != 0 || !context.can_edit(pos) {
            return Ok(());
        }
        let Some(before) = server.world().loaded_block(pos)? else {
            return Ok(());
        };
        if !bindings::basic_state(before) {
            return Ok(());
        }
        if let Some(edit) = server.world().edit_loaded_block(
            &self.owner,
            pos,
            before,
            bcore_registry::block_state::AIR,
        )? {
            self.publish(server, player.id, &edit)?;
        }
        Ok(())
    }

    pub(super) fn place_block(
        &self,
        server: &ServerState,
        player: &PlayerHandle,
        context: &PlayerContext,
        hand: u8,
        hit: packets::BlockHit,
    ) -> Result<(), WorldError> {
        if hand != 0 || !packets::hit_is_valid(hit) || !context.can_edit(hit.pos) {
            return Ok(());
        }
        let Some(clicked) = server.world().loaded_block(hit.pos)? else {
            return Ok(());
        };
        if !bindings::basic_state(clicked) {
            return Ok(());
        }
        let pos = packets::adjacent(hit.pos, hit.face);
        if !context.has_chunk(pos)
            || !super::in_world(pos)
            || context.overlaps(pos)
            || server
                .players()
                .iter()
                .any(|other| other.id != player.id && other.context().overlaps(pos))
        {
            return Ok(());
        }
        let Some(stack) = self
            .hotbars
            .get(&player.id)
            .and_then(|bar| bar.slots[bar.selected as usize])
        else {
            return Ok(());
        };
        let Some(after) = bindings::block_item(stack.item, stack.count) else {
            return Ok(());
        };
        let Some(before) = server.world().loaded_block(pos)? else {
            return Ok(());
        };
        if !bcore_worldgen::is_air(before) {
            return Ok(());
        }
        if let Some(edit) = server
            .world()
            .edit_loaded_block(&self.owner, pos, before, after)?
        {
            self.publish(server, player.id, &edit)?;
        }
        Ok(())
    }
}
