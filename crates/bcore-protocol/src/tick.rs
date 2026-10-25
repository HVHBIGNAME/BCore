//! Fixed-rate Minecraft world tick loop and TPS accounting.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::shared::SharedServer;

const TICK_INTERVAL: Duration = Duration::from_millis(50);

/// Live tick counters, shared with diagnostics and future status responses.
#[derive(Debug, Default)]
pub struct TickState {
    total_ticks: AtomicU64,
    last_second_ticks: AtomicU64,
}

impl TickState {
    pub fn total_ticks(&self) -> u64 {
        self.total_ticks.load(Ordering::Relaxed)
    }
    pub fn tps(&self) -> u64 {
        self.last_second_ticks.load(Ordering::Relaxed)
    }
}

/// Start the server's 20 TPS loop. The returned counters remain valid thereafter.
pub fn start(server: SharedServer) -> Arc<TickState> {
    let (state, _worker) = spawn(server);
    state
}

/// Owned server worker: shutdown joins final dirty saves instead of detaching them.
pub struct TickWorker {
    server: SharedServer,
    state: Arc<TickState>,
    worker: Option<thread::JoinHandle<()>>,
}

impl TickWorker {
    pub fn state(&self) -> &Arc<TickState> {
        &self.state
    }
}

impl Drop for TickWorker {
    fn drop(&mut self) {
        self.server.request_shutdown();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                eprintln!("[bcore] tick worker panicked during shutdown");
            }
        }
    }
}

pub(crate) fn start_owned(server: SharedServer) -> TickWorker {
    let (state, worker) = spawn(Arc::clone(&server));
    TickWorker {
        server,
        state,
        worker: Some(worker),
    }
}

fn spawn(server: SharedServer) -> (Arc<TickState>, thread::JoinHandle<()>) {
    let state = Arc::new(TickState::default());
    let counters = Arc::clone(&state);
    let worker = thread::Builder::new()
        .name("bcore-tick".into())
        .spawn(move || run(server, counters))
        .expect("spawn tick thread");
    (state, worker)
}

struct TickExit<'a>(&'a crate::shared::ServerState);

impl Drop for TickExit<'_> {
    fn drop(&mut self) {
        self.0.request_shutdown();
        self.0.blocks.cancel_pending();
    }
}

fn run(server: SharedServer, state: Arc<TickState>) {
    let _exit = TickExit(&server);
    let mut blocks = match crate::blocks::BlockOwner::new(&server) {
        Ok(owner) => owner,
        Err(error) => {
            eprintln!("[bcore] gameplay owner admission failed: {error}");
            return;
        }
    };
    let started = Instant::now();
    let mut next_tick = started + TICK_INTERVAL;
    let mut last_report = started;
    let mut ticks_since_report = 0u64;
    while !server.is_shutting_down() {
        let now = Instant::now();
        if now < next_tick {
            thread::sleep(next_tick - now);
        }
        if server.is_shutting_down() {
            break;
        }
        let tick = state.total_ticks.fetch_add(1, Ordering::Relaxed) + 1;
        ticks_since_report += 1;
        state.last_second_ticks.store(
            (tick as f64 / started.elapsed().as_secs_f64().max(1.0)).round() as u64,
            Ordering::Relaxed,
        );
        server.tick_clock();
        if let Err(error) = blocks.tick(&server) {
            eprintln!("[bcore] gameplay dirty save failed: {error}");
            break;
        }
        next_tick += TICK_INTERVAL;
        if next_tick < Instant::now() {
            next_tick = Instant::now() + TICK_INTERVAL;
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            let seconds = last_report.elapsed().as_secs_f64();
            let tps = ticks_since_report as f64 / seconds;
            println!("[bcore] TPS: {:.1} ({} ticks)", tps, ticks_since_report);
            ticks_since_report = 0;
            last_report = Instant::now();
        }
    }
    if let Err(error) = blocks.finish(&server) {
        eprintln!("[bcore] final gameplay dirty save failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counters_start_at_zero() {
        let state = TickState::default();
        assert_eq!(state.total_ticks(), 0);
        assert_eq!(state.tps(), 0);
    }
}
