//! Fixed-rate Minecraft world tick loop and TPS accounting.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::gameplay::{
    encode_update_time, ClockUpdate, CLOCK_DAY_TIME, CLOCK_WORLD_AGE, TICKS_PER_DAY,
};
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
    let state = Arc::new(TickState::default());
    let counters = Arc::clone(&state);
    thread::Builder::new()
        .name("bcore-tick".into())
        .spawn(move || run(server, counters))
        .expect("spawn tick thread");
    state
}

fn run(server: SharedServer, state: Arc<TickState>) {
    let started = Instant::now();
    let mut next_tick = started + TICK_INTERVAL;
    let mut last_report = started;
    let mut ticks_since_report = 0u64;
    while !server.is_shutting_down() {
        let now = Instant::now();
        if now < next_tick {
            thread::sleep(next_tick - now);
        }
        let tick = state.total_ticks.fetch_add(1, Ordering::Relaxed) + 1;
        ticks_since_report += 1;
        state.last_second_ticks.store(
            (tick as f64 / started.elapsed().as_secs_f64().max(1.0)).round() as u64,
            Ordering::Relaxed,
        );
        let day_time = (1000 + tick as i64).rem_euclid(TICKS_PER_DAY);
        let packet = encode_update_time(
            tick as i64,
            &[
                ClockUpdate::running(CLOCK_WORLD_AGE, tick as i64),
                ClockUpdate::running(CLOCK_DAY_TIME, day_time),
            ],
        );
        server.broadcast(&packet);
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
