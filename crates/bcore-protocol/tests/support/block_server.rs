use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bcore_core::ChunkPos;
use bcore_protocol::chunk::{block_state, ChunkColumn};
use bcore_protocol::chunk_store::ChunkStore;
use bcore_protocol::shared::{ServerState, SharedServer};
use bcore_protocol::world::chunks_in_view;
use bcore_protocol::world_state::World;
use bcore_worldgen::block_entity::PendingBlockEntity;
use bcore_worldgen::lighting::GenerationLight;

pub struct Fixture {
    pub root: PathBuf,
    pub store: ChunkStore,
    pub originals: Vec<((i32, i32), ChunkColumn)>,
}

impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir().join("opencode");
        std::fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "bcore-block-socket-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let store = ChunkStore::at(&root);
        store.prepare_world(-42).unwrap();
        let mut originals = Vec::new();
        let mut light = GenerationLight::default();
        for pos in chunks_in_view(0, -1, 1) {
            let mut column = ChunkColumn::flat();
            for x in 0..16 {
                for z in 0..16 {
                    column.set(x, 64, z, block_state::STONE);
                }
            }
            if pos == (0, -1) {
                column.set(15, 65, 7, block_state::STONE);
                column.set(2, 65, 7, block_state::CHEST);
                assert!(column.set_pending_block_entity(
                    ChunkPos::new(pos.0, pos.1),
                    2,
                    65,
                    7,
                    PendingBlockEntity::dummy((2, 65, -9))
                ));
            }
            light
                .initialize_chunk(ChunkPos::new(pos.0, pos.1), column.states())
                .unwrap();
            originals.push((pos, column));
        }
        for &(pos, _) in &originals {
            light.propagate_chunk(ChunkPos::new(pos.0, pos.1)).unwrap();
        }
        for (pos, column) in &mut originals {
            assert!(column.set_light(light.chunk_light(ChunkPos::new(pos.0, pos.1))));
            store.save(pos.0, pos.1, column).unwrap();
        }
        Self {
            root,
            store,
            originals,
        }
    }

    pub fn live(&self) -> TestServer {
        let state = Arc::new(
            ServerState::with_world(World::open(-42, self.store.clone()).unwrap())
                .with_view_distance(1),
        );
        for name in ["Builder", "Viewer", "Far"] {
            state.add_op(name);
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let running = Arc::clone(&state);
        let worker =
            std::thread::spawn(move || bcore_protocol::server::run_with_state(listener, running));
        TestServer {
            address,
            state,
            worker: Some(worker),
        }
    }

    pub fn read(&self, pos: bcore_core::BlockPos) -> u32 {
        let column = self
            .store
            .load(pos.chunk().x, pos.chunk().z)
            .unwrap()
            .unwrap();
        let (x, y, z) = pos.local();
        column.get(x as usize, y, z as usize).unwrap()
    }

    pub fn bytes(&self) -> Vec<Vec<u8>> {
        self.originals
            .iter()
            .map(|(pos, _)| std::fs::read(self.store.chunk_path(pos.0, pos.1)).unwrap())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

pub struct TestServer {
    pub address: SocketAddr,
    pub state: SharedServer,
    worker: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn stop(mut self) {
        self.state.request_shutdown();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.state.world().pending_payloads() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            self.state.world().pending_payloads(),
            0,
            "owned payload jobs finished"
        );
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.request_shutdown();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
