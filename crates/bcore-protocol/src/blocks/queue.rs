//! Bounded connection-to-tick request queue with shutdown cancellation.
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::Mutex;

use super::{packets::BlockPacket, PlayerContext};
use crate::packet::PacketError;
use crate::shared::{PlayerHandle, ServerState};
use crate::world_state::WorldError;

pub(super) const MAX_PENDING_ACTIONS: usize = 128;

#[derive(Debug)]
pub(super) struct QueuedBlock {
    pub player: PlayerHandle,
    pub context: PlayerContext,
    pub packet: BlockPacket,
    pub reply: Sender<Result<(), WorldError>>,
}

#[derive(Debug)]
pub(crate) struct BlockQueue {
    tx: SyncSender<QueuedBlock>,
    rx: Mutex<Receiver<QueuedBlock>>,
}

/// Connection-local completions; receiving another frame never waits for a tick.
#[derive(Default)]
pub(crate) struct PendingActions {
    replies: VecDeque<Receiver<Result<(), WorldError>>>,
}

impl PendingActions {
    pub(crate) fn submit(
        &mut self,
        server: &ServerState,
        player: &PlayerHandle,
        packet: BlockPacket,
    ) -> Result<(), PacketError> {
        self.poll()?;
        if self.replies.len() >= MAX_PENDING_ACTIONS {
            return Err(PacketError::Malformed("gameplay action queue full"));
        }
        self.replies
            .push_back(server.blocks.submit(server, player, packet)?);
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> Result<(), PacketError> {
        while let Some(reply) = self.replies.front() {
            match reply.try_recv() {
                Ok(outcome) => {
                    self.replies.pop_front();
                    outcome.map_err(std::io::Error::other)?;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(PacketError::Malformed("gameplay owner stopped"));
                }
            }
        }
        Ok(())
    }
}

impl Default for BlockQueue {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(MAX_PENDING_ACTIONS);
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }
}

impl BlockQueue {
    pub(crate) fn submit(
        &self,
        server: &ServerState,
        player: &PlayerHandle,
        packet: BlockPacket,
    ) -> Result<Receiver<Result<(), WorldError>>, PacketError> {
        if server.is_shutting_down() || player.is_kicked() || !server.has_player(player) {
            return Err(PacketError::Malformed("inactive gameplay connection"));
        }
        let context = player.context();
        let (tx, rx) = mpsc::channel();
        let work = QueuedBlock {
            player: player.clone(),
            context,
            packet,
            reply: tx,
        };
        match self.tx.try_send(work) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(PacketError::Malformed("gameplay action queue full"))
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(PacketError::Malformed("gameplay owner stopped"))
            }
        }
        Ok(rx)
    }

    pub(super) fn pop(&self) -> Option<QueuedBlock> {
        match self.rx.lock().expect("gameplay queue lock").try_recv() {
            Ok(work) => Some(work),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    pub(crate) fn cancel_pending(&self) {
        while let Some(work) = self.pop() {
            // The socket may have closed while it was waiting for the owner.
            let _ = work
                .reply
                .send(Err(WorldError::Mutation("gameplay owner stopped")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_actions_are_bounded_and_owner_cancellation_resolves_every_reply() {
        let server = ServerState::default();
        let player = server.join("Builder", [1; 16]);
        let mut replies = Vec::new();
        for _ in 0..MAX_PENDING_ACTIONS {
            replies.push(
                server
                    .blocks
                    .submit(&server, &player, BlockPacket::Carried { slot: 0 })
                    .unwrap(),
            );
        }
        assert!(matches!(
            server
                .blocks
                .submit(&server, &player, BlockPacket::Carried { slot: 0 }),
            Err(PacketError::Malformed("gameplay action queue full"))
        ));
        server.blocks.cancel_pending();
        for reply in replies {
            assert!(matches!(
                reply.try_recv(),
                Ok(Err(WorldError::Mutation("gameplay owner stopped")))
            ));
        }
        server.leave(player.id);
        assert!(server
            .blocks
            .submit(&server, &player, BlockPacket::Carried { slot: 0 })
            .is_err());
    }
}
