//! Tick-owned hotbar, ordered edits, viewer publication and coalesced acknowledgements.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use bcore_core::BlockPos;
use bcore_worldgen::generation::LevelBlockEdit;

use super::saving::{SaveCompletion, SaveJob, SaveWorker};
use super::{bindings, packets, PlayerContext};
use crate::gameplay::GameMode;
use crate::shared::{PlayerHandle, PlayerId, ServerState};
use crate::world_state::{WorldError, WorldMutationOwner, WorldUpdates};

#[derive(Default)]
pub(super) struct Hotbar {
    pub selected: u8,
    pub slots: [Option<packets::SimpleStack>; 9],
}

pub(crate) struct BlockOwner {
    pub(super) owner: WorldMutationOwner,
    pub(super) hotbars: BTreeMap<PlayerId, Hotbar>,
    acks: BTreeMap<PlayerId, i32>,
    light_updates: BTreeSet<bcore_core::ChunkPos>,
    deferred: Option<super::queue::QueuedBlock>,
    saver: SaveWorker,
}

impl BlockOwner {
    pub(crate) fn new(server: &ServerState) -> Result<Self, WorldError> {
        Ok(Self {
            owner: server.world().bind_mutations()?,
            hotbars: BTreeMap::new(),
            acks: BTreeMap::new(),
            light_updates: BTreeSet::new(),
            deferred: None,
            saver: SaveWorker::new(server.world().store().cloned())?,
        })
    }

    pub(crate) fn tick(&mut self, server: &ServerState) -> Result<(), WorldError> {
        if let Some(done) = self.saver.poll()? {
            self.saved(server, done)?;
        }
        self.hotbars.retain(|id, _| server.player(*id).is_some());
        if self.deferred.is_none() {
            self.deferred = server.blocks.pop();
        }
        if self.deferred.is_none() && self.acks.is_empty() {
            return Ok(());
        }
        let Some(mut updates) = server.world().try_updates(&self.owner)? else {
            return Ok(());
        };
        let started = Instant::now();
        for index in 0..super::queue::MAX_PENDING_ACTIONS {
            if index != 0 && started.elapsed() >= Duration::from_millis(10) {
                break;
            }
            let Some(work) = self.deferred.take().or_else(|| server.blocks.pop()) else {
                break;
            };
            let outcome = if server.is_shutting_down()
                || work.player.is_kicked()
                || !server.has_player(&work.player)
            {
                Err(WorldError::Mutation("inactive gameplay connection"))
            } else {
                self.apply(
                    server,
                    &mut updates,
                    &work.player,
                    &work.context,
                    work.packet,
                )
            };
            let _ = work.reply.send(outcome);
        }
        self.flush_light(server, &updates);
        if !self.saver.active() {
            let batch = updates.dirty_snapshot()?;
            if batch.is_empty() {
                self.flush_acks(server);
            } else {
                self.saver.start(SaveJob {
                    batch,
                    acks: std::mem::take(&mut self.acks),
                })?;
            }
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self, server: &ServerState) -> Result<(), WorldError> {
        if let Some(done) = self.saver.finish()? {
            self.saved(server, done)?;
        }
        server.world().flush_dirty_chunks(&self.owner)?;
        self.flush_acks(server);
        Ok(())
    }

    fn saved(&mut self, server: &ServerState, done: SaveCompletion) -> Result<(), WorldError> {
        done.result?;
        server
            .world()
            .complete_dirty_save(&self.owner, &done.job.batch)?;
        for (player, sequence) in done.job.acks {
            server.send_to(player, &packets::encode_ack(sequence));
        }
        Ok(())
    }

    fn flush_acks(&mut self, server: &ServerState) {
        for (player, sequence) in std::mem::take(&mut self.acks) {
            server.send_to(player, &packets::encode_ack(sequence));
        }
    }

    fn apply(
        &mut self,
        server: &ServerState,
        updates: &mut WorldUpdates<'_>,
        player: &PlayerHandle,
        context: &PlayerContext,
        packet: packets::BlockPacket,
    ) -> Result<(), WorldError> {
        match packet {
            packets::BlockPacket::Carried { slot } => {
                if context.ready {
                    self.hotbars.entry(player.id).or_default().selected = slot;
                }
            }
            packets::BlockPacket::Creative { slot, stack } => {
                let hotbar = self.hotbars.entry(player.id).or_default();
                if context.ready
                    && context.game_mode == GameMode::Creative
                    && stack
                        .is_none_or(|stack| bindings::block_item(stack.item, stack.count).is_some())
                {
                    hotbar.slots[slot as usize] = stack;
                }
                server.send_to(
                    player.id,
                    &packets::encode_inventory(slot, hotbar.slots[slot as usize]),
                );
            }
            packets::BlockPacket::Action {
                action,
                pos,
                sequence,
                ..
            } => {
                if action <= 2 {
                    self.ack(player.id, sequence);
                    self.break_block(server, updates, player, context, action, pos)?;
                    self.correct(server, updates, player, context, pos);
                }
            }
            packets::BlockPacket::Place {
                hand,
                hit,
                sequence,
            } => {
                self.ack(player.id, sequence);
                self.place_block(server, updates, player, context, hand, hit)?;
                self.correct(server, updates, player, context, hit.pos);
                self.correct(
                    server,
                    updates,
                    player,
                    context,
                    packets::adjacent(hit.pos, hit.face),
                );
            }
        }
        Ok(())
    }

    fn ack(&mut self, player: PlayerId, sequence: i32) {
        self.acks
            .entry(player)
            .and_modify(|old| *old = (*old).max(sequence))
            .or_insert(sequence);
    }

    pub(super) fn correct(
        &self,
        server: &ServerState,
        updates: &WorldUpdates<'_>,
        player: &PlayerHandle,
        context: &PlayerContext,
        pos: BlockPos,
    ) {
        if super::in_world(pos) && context.has_chunk(pos) {
            if let Some(state) = updates.loaded_block(pos) {
                server.send_to(player.id, &packets::encode_block_update(pos, state));
            }
        }
    }

    pub(super) fn publish(
        &mut self,
        server: &ServerState,
        player: PlayerId,
        edit: &LevelBlockEdit,
    ) -> Result<(), WorldError> {
        server.send_to_chunk(
            edit.pos.chunk(),
            Some(player),
            &packets::encode_block_update(edit.pos, edit.after),
        );
        self.light_updates.extend(&edit.light_changed_chunks);
        Ok(())
    }

    fn flush_light(&mut self, server: &ServerState, updates: &WorldUpdates<'_>) {
        for pos in std::mem::take(&mut self.light_updates) {
            if let Some(data) = updates.light_payload(pos) {
                server.send_to_chunk(pos, None, &packets::encode_light(pos, &data));
            }
        }
    }
}
