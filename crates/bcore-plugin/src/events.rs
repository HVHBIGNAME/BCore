use bcore_core::{BlockPos, Identifier, Position};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ServerEvent {
    PlayerJoin {
        player: String,
        position: Position,
    },
    PlayerChat {
        player: String,
        message: String,
    },
    BlockBreak {
        player: String,
        position: BlockPos,
        block: Identifier,
    },
}

pub type EventHandler = Arc<dyn Fn(&ServerEvent) + Send + Sync + 'static>;

#[derive(Default)]
pub(crate) struct EventDispatcher {
    handlers: Vec<EventHandler>,
}

impl EventDispatcher {
    pub(crate) fn subscribe(&mut self, handler: EventHandler) {
        self.handlers.push(handler);
    }
    pub(crate) fn fire(&self, event: &ServerEvent) {
        for handler in &self.handlers {
            handler(event);
        }
    }
}
