//! One queued continuation per view; admission selects from its latest demand.
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex};

use super::{GenerationWork, World};

pub(crate) const VIEW_LOOKAHEAD: usize = 64;

pub(crate) struct PayloadStream {
    requests: Arc<StreamRequests>,
}

#[derive(Default)]
struct Demand {
    wanted: Vec<(i32, i32)>,
    version: u64,
    active: bool,
    closed: bool,
}

pub(super) struct StreamRequests {
    world: World,
    demand: Mutex<Demand>,
    queue: mpsc::Sender<GenerationWork>,
}

impl PayloadStream {
    pub(super) fn new(world: World, queue: mpsc::Sender<GenerationWork>) -> Self {
        Self {
            requests: Arc::new(StreamRequests {
                world,
                demand: Mutex::new(Demand::default()),
                queue,
            }),
        }
    }

    pub(crate) fn update(&self, positions: impl IntoIterator<Item = (i32, i32)>) {
        let mut wanted = Vec::new();
        for (x, z) in positions.into_iter().take(VIEW_LOOKAHEAD) {
            if !wanted.contains(&(x, z))
                && self.requests.world.cached_chunk(x, z).is_none()
                && self.requests.world.payload_error(x, z).is_none()
            {
                wanted.push((x, z));
            }
        }
        {
            let mut demand = self.requests.demand.lock().expect("view payload demand");
            if wanted != demand.wanted {
                demand.wanted = wanted;
                demand.version = demand
                    .version
                    .checked_add(1)
                    .expect("view version exhausted");
            }
        }
        self.requests.enqueue();
    }
}

impl Drop for PayloadStream {
    fn drop(&mut self) {
        let mut demand = self
            .requests
            .demand
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        demand.closed = true;
        demand.wanted.clear();
    }
}

impl StreamRequests {
    pub(super) fn seed(&self) -> i64 {
        self.world.seed()
    }

    fn enqueue(self: &Arc<Self>) {
        {
            let mut demand = self.demand.lock().expect("view payload demand");
            if demand.active || demand.closed || demand.wanted.is_empty() {
                return;
            }
            demand.active = true;
        }
        if self.queue.send(GenerationWork::View(self.clone())).is_err() {
            self.demand.lock().expect("view payload demand").active = false;
            eprintln!(
                "[bcore] view payload dispatcher stopped for seed {}",
                self.seed()
            );
        }
    }

    pub(super) fn run(self: Arc<Self>) {
        let mut version = None;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let job = {
                let demand = self.demand.lock().expect("view payload demand");
                version = Some(demand.version);
                if demand.closed {
                    None
                } else {
                    demand
                        .wanted
                        .iter()
                        .find_map(|&(x, z)| self.world.reserve_generation(x, z))
                }
            };
            // Selection and reservation share the demand lock. Generation runs
            // after releasing it, so movement can replace the next target.
            let worked = job.is_some();
            if let Some(job) = job {
                job.run();
            }
            worked
        }));
        let changed = {
            let mut demand = self
                .demand
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            demand.active = false;
            Some(demand.version) != version
        };
        match result {
            Ok(worked) if worked || changed => self.enqueue(),
            Ok(_) => {}
            Err(_) => eprintln!(
                "[bcore] view payload request panicked for seed {}",
                self.seed()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkColumn;
    use crate::chunk_store::ChunkStore;
    use crate::world::PlayerView;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> (Self, World) {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = std::env::temp_dir().join("opencode");
            std::fs::create_dir_all(&parent).unwrap();
            let path = parent.join(format!(
                "bcore-view-demand-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            let store = ChunkStore::at(&path);
            store.prepare_world(0).unwrap();
            for (x, z) in [(0, 0), (6, -3), (7, -3)] {
                store.save(x, z, &ChunkColumn::flat()).unwrap();
            }
            (Self(path), World::with_store(0, store))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn queued_view_admits_the_latest_position_without_generating_the_retired_one() {
        let (_fixture, world) = Fixture::new();
        let (tx, rx) = mpsc::channel();
        let stream = PayloadStream::new(world.clone(), tx);
        stream.update([(0, 0)]);
        stream.update([(6, -3), (7, -3)]);
        let work = rx.try_recv().unwrap();
        assert!(rx.try_recv().is_err(), "one queued continuation per view");
        work.run();
        assert!(world.cached_chunk(6, -3).is_some());
        assert!(world.generation_progress(0, 0).unwrap().is_none());
        rx.try_recv().unwrap().run();
        assert!(world.cached_chunk(7, -3).is_some());
        rx.try_recv().unwrap().run();
        assert!(
            rx.try_recv().is_err(),
            "fully cached demand must become idle"
        );
        assert_eq!(world.pending_payloads(), 0);
    }

    #[test]
    fn disconnected_views_cancel_unstarted_work_and_keep_other_views_live() {
        let (_fixture, world) = Fixture::new();
        let (tx, rx) = mpsc::channel();
        let old = PayloadStream::new(world.clone(), tx.clone());
        let live = PayloadStream::new(world.clone(), tx);
        old.update([(0, 0)]);
        live.update([(6, -3)]);
        drop(old);
        rx.try_recv().unwrap().run();
        assert!(world.generation_progress(0, 0).unwrap().is_none());
        rx.try_recv().unwrap().run();
        assert!(world.cached_chunk(6, -3).is_some());
        drop(live);
        rx.try_recv().unwrap().run();
        assert!(rx.try_recv().is_err());
        assert_eq!(world.pending_payloads(), 0);
    }

    #[test]
    fn a_missing_nearest_chunk_does_not_hide_ready_terrain_within_the_view() {
        let world = World::flat_fixture([(1, 0)]);
        let mut view = PlayerView::new(0.5, 64.0, 0.5).with_view_distance(1);
        let mut bytes = Vec::new();
        assert_eq!(
            view.stream_ready_chunks_from_with(&mut bytes, &world, 1, |_, _| {})
                .unwrap(),
            1
        );
        assert_eq!(view.loaded_chunks().copied().collect::<Vec<_>>(), [(1, 0)]);
        assert!(world.cached_chunk(0, 0).is_none());
        assert_eq!(world.pending_payloads(), 0);
        let mut cursor = std::io::Cursor::new(bytes);
        let mut terrain = Vec::new();
        while cursor.position() < cursor.get_ref().len() as u64 {
            let (id, payload) = crate::packet::read_frame(&mut cursor).unwrap();
            if id == crate::world::CB_MAP_CHUNK {
                terrain.push(payload);
            }
        }
        assert_eq!(terrain, [world.cached_payload(1, 0).unwrap()]);
    }
}
