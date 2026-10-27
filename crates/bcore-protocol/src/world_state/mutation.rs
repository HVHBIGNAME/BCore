//! World-scoped block ownership, payload invalidation and inert BCC dirty saves.
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, RwLockReadGuard, RwLockWriteGuard, TryLockError};
use std::thread::{self, ThreadId};

use bcore_core::{BlockPos, ChunkPos};
#[cfg(test)]
use bcore_worldgen::generation::{ChunkAccessKind, ChunkStatus};
use bcore_worldgen::generation::{LevelAccess, LevelBlockEdit};

use super::{payload_shard, Ordering, World, WorldError};

#[derive(Debug)]
pub(crate) struct WorldMutationOwner {
    world: World,
    thread: ThreadId,
    _local: PhantomData<Rc<()>>,
}

pub(crate) struct WorldUpdates<'a> {
    pub(super) world: &'a World,
    pub(super) access: LevelAccess<'a>,
    _publication: RwLockWriteGuard<'a, ()>,
}

impl WorldUpdates<'_> {
    pub(crate) fn loaded_block(&self, pos: BlockPos) -> Option<u32> {
        self.access.block(pos)
    }

    pub(crate) fn edit_block(
        &mut self,
        pos: BlockPos,
        expected: u32,
        after: u32,
    ) -> Result<Option<LevelBlockEdit>, WorldError> {
        let edited = self.access.edit_block(pos, expected, after)?;
        if let Some(edit) = &edited {
            self.world.record_edit(edit);
        }
        Ok(edited)
    }

    pub(crate) fn light_payload(&self, pos: ChunkPos) -> Option<Vec<u8>> {
        self.access
            .read_light(pos, crate::chunk::encode_native_light_payload)
    }
}

impl Drop for WorldMutationOwner {
    fn drop(&mut self) {
        let mut owner = self
            .world
            .inner
            .mutation_owner
            .lock()
            .expect("world mutation owner lock");
        if *owner == Some(self.thread) {
            *owner = None;
        }
    }
}

impl World {
    pub(crate) fn bind_mutations(&self) -> Result<WorldMutationOwner, WorldError> {
        let mut owner = self
            .inner
            .mutation_owner
            .lock()
            .expect("world mutation owner lock");
        if owner.is_some() {
            return Err(WorldError::Mutation("world already has a mutation owner"));
        }
        let thread = thread::current().id();
        *owner = Some(thread);
        Ok(WorldMutationOwner {
            world: self.clone(),
            thread,
            _local: PhantomData,
        })
    }

    pub(super) fn check_mutation_owner(
        &self,
        owner: &WorldMutationOwner,
    ) -> Result<(), WorldError> {
        if !Arc::ptr_eq(&self.inner, &owner.world.inner) || thread::current().id() != owner.thread {
            return Err(WorldError::Mutation(
                "foreign world or wrong mutation thread",
            ));
        }
        Ok(())
    }

    pub(crate) fn read_publication(&self) -> RwLockReadGuard<'_, ()> {
        self.inner
            .publication
            .read()
            .expect("world publication lock")
    }

    pub(crate) fn try_updates(
        &self,
        owner: &WorldMutationOwner,
    ) -> Result<Option<WorldUpdates<'_>>, WorldError> {
        self.check_mutation_owner(owner)?;
        let publication = match self.inner.publication.try_write() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => {
                return Err(WorldError::Mutation("world publication lock poisoned"))
            }
        };
        let Some(access) = self.inner.generation.try_level_access()? else {
            return Ok(None);
        };
        Ok(Some(WorldUpdates {
            world: self,
            access,
            _publication: publication,
        }))
    }

    #[cfg(test)]
    pub(crate) fn loaded_block(&self, pos: BlockPos) -> Result<Option<u32>, WorldError> {
        if !(crate::chunk::MIN_Y..=crate::chunk::MAX_Y).contains(&pos.y) {
            return Ok(None);
        }
        let Some(access) = self
            .inner
            .generation
            .chunk_access(pos.chunk(), ChunkStatus::Full)?
        else {
            return Ok(None);
        };
        Ok(access.read(|view| {
            if view.kind != ChunkAccessKind::Level {
                return None;
            }
            let (x, y, z) = pos.local();
            view.chunk.get(x as usize, y, z as usize)
        })?)
    }

    #[cfg(test)]
    pub(crate) fn edit_loaded_block(
        &self,
        owner: &WorldMutationOwner,
        pos: BlockPos,
        expected: u32,
        state: u32,
    ) -> Result<Option<LevelBlockEdit>, WorldError> {
        self.check_mutation_owner(owner)?;
        let _publication = self
            .inner
            .publication
            .write()
            .expect("world publication lock");
        let edited = self
            .inner
            .generation
            .edit_level_block(pos, expected, state)?;
        if let Some(edit) = &edited {
            self.record_edit(edit);
        }
        Ok(edited)
    }

    fn record_edit(&self, edit: &LevelBlockEdit) {
        let revision = self.inner.mutation_revision.fetch_add(1, Ordering::Relaxed);
        assert_ne!(revision, u64::MAX, "world mutation revision exhausted");
        let mut dirty = self.inner.dirty_chunks.lock().expect("dirty chunk lock");
        for pos in &edit.changed_chunks {
            self.inner.payloads[payload_shard(pos.x, pos.z)]
                .write()
                .expect("payload shard lock")
                .remove(&(pos.x, pos.z));
            dirty.insert(*pos, revision);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Write};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;

    use super::*;
    use crate::chunk::{block_state, ChunkColumn};
    use crate::chunk_store::{encode_chunk, ChunkStore};
    use crate::shared::ServerState;
    use crate::world::PlayerView;

    fn column() -> ChunkColumn {
        let mut column = ChunkColumn::flat();
        assert!(column.set(0, 64, 0, block_state::STONE));
        assert!(column.set(3, 70, 3, block_state::CHEST));
        assert!(column.set_pending_block_entity(
            ChunkPos::new(0, 0),
            3,
            70,
            3,
            bcore_worldgen::block_entity::PendingBlockEntity::dummy((3, 70, 3))
        ));
        column
    }

    fn world() -> (World, ChunkColumn) {
        let world = World::in_memory(0);
        let column = column();
        world
            .inner
            .generation
            .admit_saved_chunks(vec![column.clone().into_saved_data(ChunkPos::new(0, 0))])
            .unwrap();
        (world, column)
    }

    #[test]
    fn clean_tick_never_waits_for_a_chunk_publication_reader() {
        let world = World::in_memory(0);
        let publication = world.read_publication();
        let running = world.clone();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let owner = running.bind_mutations().unwrap();
            tx.send(running.flush_dirty_chunks(&owner).unwrap())
                .unwrap();
        });
        let completed = rx.recv_timeout(std::time::Duration::from_secs(1));
        drop(publication);
        worker.join().unwrap();
        assert_eq!(
            completed.unwrap(),
            0,
            "clean tick must not acquire publication lock"
        );
    }

    #[test]
    fn busy_generation_defers_updates_instead_of_waiting_on_the_tick_thread() {
        let world = World::in_memory(0);
        let busy = world.inner.generation.level_access().unwrap();
        let running = world.clone();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let owner = running.bind_mutations().unwrap();
            tx.send(running.try_updates(&owner).unwrap().is_none())
                .unwrap();
        });
        let deferred = rx.recv_timeout(std::time::Duration::from_secs(1));
        drop(busy);
        worker.join().unwrap();
        assert!(deferred.unwrap());
        let owner = world.bind_mutations().unwrap();
        assert!(world.try_updates(&owner).unwrap().is_some());
    }

    #[test]
    fn completing_an_older_save_preserves_newer_dirty_data_and_inert_pending_tags() {
        let directory = Directory::new();
        let store = ChunkStore::at(&directory.0);
        store.prepare_world(0).unwrap();
        let original = column();
        store.save(0, 0, &original).unwrap();
        let world = World::open(0, store.clone()).unwrap();
        world.try_chunk(0, 0).unwrap();
        let owner = world.bind_mutations().unwrap();
        let pos = BlockPos::new(0, 64, 0);
        world
            .edit_loaded_block(&owner, pos, block_state::STONE, block_state::DIRT)
            .unwrap();
        let old = world
            .try_updates(&owner)
            .unwrap()
            .unwrap()
            .dirty_snapshot()
            .unwrap();
        world
            .edit_loaded_block(&owner, pos, block_state::DIRT, block_state::DIAMOND_BLOCK)
            .unwrap();
        old.write(Some(&store)).unwrap();
        world.complete_dirty_save(&owner, &old).unwrap();
        assert_eq!(
            store.load(0, 0).unwrap().unwrap().get(0, 64, 0),
            Some(block_state::DIRT)
        );
        assert_eq!(
            world.loaded_block(pos).unwrap(),
            Some(block_state::DIAMOND_BLOCK)
        );
        assert_eq!(world.inner.dirty_chunks.lock().unwrap().len(), 1);
        assert_eq!(world.flush_dirty_chunks(&owner).unwrap(), 1);
        let saved = store.load(0, 0).unwrap().unwrap();
        assert_eq!(saved.get(0, 64, 0), Some(block_state::DIAMOND_BLOCK));
        assert_eq!(
            saved.pending_block_entities(),
            original.pending_block_entities()
        );
        assert!(world.inner.dirty_chunks.lock().unwrap().is_empty());
    }

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = std::env::temp_dir().join("opencode");
            std::fs::create_dir_all(&parent).unwrap();
            let path = parent.join(format!(
                "bcore-block-mutation-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn mutation_ownership_rejects_a_second_thread_and_foreign_world_token() {
        let (world, _) = world();
        let owner = world.bind_mutations().unwrap();
        assert!(world.bind_mutations().is_err());
        let other = world.clone();
        assert!(thread::spawn(move || other.bind_mutations().is_err())
            .join()
            .unwrap());
        let foreign = World::in_memory(0);
        let foreign_owner = foreign.bind_mutations().unwrap();
        assert!(world
            .edit_loaded_block(
                &foreign_owner,
                BlockPos::new(0, 64, 0),
                block_state::STONE,
                block_state::AIR
            )
            .is_err());
        assert_eq!(
            world.loaded_block(BlockPos::new(0, 64, 0)).unwrap(),
            Some(block_state::STONE)
        );
        drop(owner);
        assert!(world.bind_mutations().is_ok());
    }

    #[test]
    fn canonical_level_edit_invalidates_payloads_preserves_pending_tags_and_updates_heightmaps() {
        let (world, column) = world();
        world.cache_column(0, 0, &column);
        let before = world.cached_payload(0, 0).unwrap();
        let progress = world.generation_progress(0, 0).unwrap();
        let level = world
            .request_native_chunk(0, 0, ChunkStatus::Full, false)
            .unwrap()
            .unwrap();
        let owner = world.bind_mutations().unwrap();
        let pos = BlockPos::new(1, 75, 1);
        let edited = world
            .edit_loaded_block(&owner, pos, block_state::AIR, block_state::DIAMOND_BLOCK)
            .unwrap()
            .unwrap();
        assert_eq!(edited.changed_chunks, [ChunkPos::new(0, 0)]);
        assert!(world.cached_payload(0, 0).is_none());
        assert_eq!(world.generation_progress(0, 0).unwrap(), progress);
        let view = level.view().unwrap();
        assert_eq!(view.chunk.get(1, 75, 1), Some(block_state::DIAMOND_BLOCK));
        assert!(view.chunk.worldgen_heightmaps().is_none());
        assert!(view.chunk.light().is_none());
        assert_eq!(
            view.chunk.pending_block_entities(),
            column.pending_block_entities()
        );
        let mut expected = column.clone();
        expected.set(1, 75, 1, block_state::DIAMOND_BLOCK);
        assert_eq!(ChunkColumn::from_generated(&view.chunk), expected);
        assert_ne!(world.try_chunk_payload(0, 0).unwrap(), before);
        assert_eq!(world.flush_dirty_chunks(&owner).unwrap(), 1);
        assert!(world.inner.dirty_chunks.lock().unwrap().is_empty());
        world.native_requests().unwrap().shutdown();
    }

    #[test]
    fn rejected_compare_state_be_registry_position_and_unloaded_edits_preserve_all_data() {
        let (world, column) = world();
        let owner = world.bind_mutations().unwrap();
        assert!(world
            .edit_loaded_block(
                &owner,
                BlockPos::new(0, 64, 0),
                block_state::DIRT,
                block_state::AIR
            )
            .unwrap()
            .is_none());
        assert!(world
            .edit_loaded_block(
                &owner,
                BlockPos::new(3, 70, 3),
                block_state::CHEST,
                block_state::AIR
            )
            .is_err());
        assert!(world
            .edit_loaded_block(
                &owner,
                BlockPos::new(0, 64, 0),
                block_state::STONE,
                block_state::CHEST
            )
            .is_err());
        assert!(world
            .edit_loaded_block(
                &owner,
                BlockPos::new(0, 320, 0),
                block_state::AIR,
                block_state::STONE
            )
            .is_err());
        assert!(world
            .edit_loaded_block(
                &owner,
                BlockPos::new(0, 64, 0),
                block_state::STONE,
                bcore_registry::BLOCK_STATE_COUNT as u32
            )
            .is_err());
        assert!(world
            .edit_loaded_block(
                &owner,
                BlockPos::new(1600, 64, 0),
                block_state::AIR,
                block_state::STONE
            )
            .is_err());
        assert!(world.generation_progress(100, 0).unwrap().is_none());
        let current = world
            .inner
            .generation
            .chunk_snapshot(ChunkPos::new(0, 0))
            .unwrap()
            .unwrap();
        assert_eq!(
            encode_chunk(0, 0, &ChunkColumn::from_generated(&current)),
            encode_chunk(0, 0, &column)
        );
        assert!(world.inner.dirty_chunks.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_dirty_save_keeps_original_bytes_and_retries_without_losing_pending_membership() {
        let directory = Directory::new();
        let store = ChunkStore::at(&directory.0);
        store.prepare_world(0).unwrap();
        let column = column();
        store.save(0, 0, &column).unwrap();
        let world = World::open(0, store.clone()).unwrap();
        world.try_chunk(0, 0).unwrap();
        let original = std::fs::read(store.chunk_path(0, 0)).unwrap();
        let owner = world.bind_mutations().unwrap();
        world
            .edit_loaded_block(
                &owner,
                BlockPos::new(0, 64, 0),
                block_state::STONE,
                block_state::DIAMOND_BLOCK,
            )
            .unwrap()
            .unwrap();
        let retained = directory.0.join("retained-chunks");
        std::fs::rename(directory.0.join("chunks"), &retained).unwrap();
        std::fs::write(directory.0.join("chunks"), b"blocked directory").unwrap();
        assert!(world.flush_dirty_chunks(&owner).is_err());
        assert_eq!(world.inner.dirty_chunks.lock().unwrap().len(), 1);
        assert_eq!(
            std::fs::read(retained.join(store.chunk_path(0, 0).file_name().unwrap())).unwrap(),
            original
        );
        std::fs::remove_file(directory.0.join("chunks")).unwrap();
        std::fs::rename(retained, directory.0.join("chunks")).unwrap();
        assert_eq!(world.flush_dirty_chunks(&owner).unwrap(), 1);
        let mut expected = column;
        expected.set(0, 64, 0, block_state::DIAMOND_BLOCK);
        assert_eq!(store.load(0, 0).unwrap().unwrap(), expected);
        assert_eq!(
            World::open(0, store).unwrap().try_chunk(0, 0).unwrap().0,
            expected
        );
    }

    #[test]
    fn first_chunk_delivery_subscribes_before_an_edit_can_queue_its_correction() {
        struct PausedWriter {
            started: Option<mpsc::Sender<()>>,
            edited: mpsc::Receiver<()>,
            bytes: Vec<u8>,
        }
        impl Write for PausedWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.started.take().unwrap().send(()).unwrap();
                self.edited.recv().unwrap();
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (world, column) = world();
        let original = world.cache_column(0, 0, &column).payload;
        let server = Arc::new(ServerState::with_world(world.clone()));
        let player = server.join("Viewer", [1; 16]);
        let (start_tx, start_rx) = mpsc::channel();
        let (edited_tx, edited_rx) = mpsc::channel();
        let running = Arc::clone(&server);
        let worker = thread::spawn(move || {
            let owner = running.world().bind_mutations().unwrap();
            start_rx.recv().unwrap();
            assert!(running.players()[0].context().loaded.contains(&(0, 0)));
            let pos = BlockPos::new(0, 64, 0);
            running
                .world()
                .edit_loaded_block(&owner, pos, block_state::STONE, block_state::DIAMOND_BLOCK)
                .unwrap()
                .unwrap();
            running.send_to_chunk(
                pos.chunk(),
                None,
                &crate::blocks::packets::encode_block_update(pos, block_state::DIAMOND_BLOCK),
            );
            edited_tx.send(()).unwrap();
        });
        let mut view = PlayerView::new(0.5, 65.0, 0.5).with_view_distance(0);
        let mut writer = PausedWriter {
            started: Some(start_tx),
            edited: edited_rx,
            bytes: Vec::new(),
        };
        assert_eq!(
            view.stream_chunks_from_with(&mut writer, &world, |view, loaded| player
                .update_view(view, Some(loaded)))
                .unwrap(),
            1
        );
        worker.join().unwrap();
        let mut frames = Cursor::new(writer.bytes);
        let mut terrain = None;
        while frames.position() < frames.get_ref().len() as u64 {
            let (id, body) = crate::packet::read_frame(&mut frames).unwrap();
            if id == crate::world::CB_MAP_CHUNK {
                terrain = Some(body);
            }
        }
        assert_eq!(terrain.unwrap(), *original);
        assert_eq!(
            player.outbox.drain(),
            crate::blocks::packets::encode_block_update(
                BlockPos::new(0, 64, 0),
                block_state::DIAMOND_BLOCK
            )
        );
        assert_ne!(world.try_chunk_payload(0, 0).unwrap(), *original);
    }
}
