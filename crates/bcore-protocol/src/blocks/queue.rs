//! Bounded connection-to-tick request queue with shutdown cancellation.
use std::sync::mpsc::{
    self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError, TrySendError,
};
use std::sync::Mutex;
use std::time::Duration;

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
    ) -> Result<(), PacketError> {
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
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(outcome) => return outcome.map_err(|error| std::io::Error::other(error).into()),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(PacketError::Malformed("gameplay owner stopped"))
                }
                Err(RecvTimeoutError::Timeout)
                    if server.is_shutting_down() || player.is_kicked() =>
                {
                    return Err(PacketError::Malformed("gameplay connection stopped"));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
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
