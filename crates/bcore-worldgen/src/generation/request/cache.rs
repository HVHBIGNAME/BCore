use bcore_core::ChunkPos;

use crate::generation::{ChunkStatus, ReturnedChunk};

#[derive(Clone)]
struct Entry {
    pos: ChunkPos,
    status: ChunkStatus,
    object: Option<ReturnedChunk>,
}

#[derive(Default)]
pub(super) struct RequestCache {
    slots: [Option<Entry>; 4],
}

impl RequestCache {
    pub fn lookup(
        &self,
        pos: ChunkPos,
        status: ChunkStatus,
        create: bool,
    ) -> Option<Option<ReturnedChunk>> {
        self.slots
            .iter()
            .flatten()
            .find(|entry| {
                entry.pos == pos && entry.status == status && (!create || entry.object.is_some())
            })
            .map(|entry| entry.object.clone())
    }

    pub fn store(&mut self, pos: ChunkPos, status: ChunkStatus, object: Option<ReturnedChunk>) {
        // Native hits do not promote entries. Only stores shift these four slots.
        self.slots.rotate_right(1);
        self.slots[0] = Some(Entry {
            pos,
            status,
            object,
        });
    }

    pub fn clear(&mut self) {
        self.slots = Default::default();
    }

    pub fn keys(&self) -> Vec<(ChunkPos, ChunkStatus)> {
        self.slots
            .iter()
            .flatten()
            .map(|entry| (entry.pos, entry.status))
            .collect()
    }
}
