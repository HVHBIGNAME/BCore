//! Client-paced chunk credit, independent of inbound movement/action packet rate.
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_millis(50);
const MAX_BATCH: f64 = 64.0;

pub(super) struct ChunkFlow {
    desired: f64,
    credit: f64,
    last: Instant,
    next: Instant,
    in_flight: bool,
}

impl ChunkFlow {
    pub(super) fn new(now: Instant, initially_sent: usize) -> Self {
        Self {
            desired: 4.0,
            credit: 0.0,
            last: now,
            next: now + INTERVAL,
            in_flight: initially_sent != 0,
        }
    }

    pub(super) fn acknowledge(&mut self, desired: f32) {
        if self.in_flight && desired.is_finite() && desired >= 0.0 {
            self.in_flight = false;
            self.desired = f64::from(desired).clamp(0.01, MAX_BATCH);
            self.credit = self.credit.min(self.desired.max(1.0));
        }
    }

    pub(super) fn allowance(&mut self, now: Instant) -> Option<usize> {
        if now < self.next {
            return None;
        }
        self.next = now + INTERVAL;
        self.credit = (self.credit
            + now.duration_since(self.last).as_secs_f64() * 20.0 * self.desired)
            .min(self.desired.max(1.0));
        self.last = now;
        (!self.in_flight && self.credit >= 1.0).then_some(self.credit.floor() as usize)
    }

    pub(super) fn poll(&mut self, now: Instant) -> Option<usize> {
        (now >= self.next).then(|| self.allowance(now).unwrap_or(0))
    }

    pub(super) fn time_until_poll(&self, now: Instant) -> Duration {
        self.next
            .saturating_duration_since(now)
            .clamp(Duration::from_millis(1), INTERVAL)
    }

    pub(super) fn sent(&mut self, count: usize) {
        if count != 0 {
            self.credit = (self.credit - count as f64).max(0.0);
            self.in_flight = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_flood_and_missing_ack_do_not_multiply_chunk_batches() {
        let now = Instant::now();
        let mut flow = ChunkFlow::new(now, 0);
        for n in 0..50 {
            assert_eq!(flow.allowance(now + Duration::from_millis(n)), None);
        }
        assert_eq!(flow.allowance(now + INTERVAL), Some(4));
        flow.sent(4);
        assert_eq!(flow.allowance(now + Duration::from_secs(5)), None);
        flow.acknowledge(4.0);
        assert_eq!(flow.allowance(now + Duration::from_millis(5050)), Some(4));
    }

    #[test]
    fn fractional_credit_is_respected_and_unsolicited_ack_cannot_inflate_it() {
        let now = Instant::now();
        let mut flow = ChunkFlow::new(now, 4);
        flow.acknowledge(0.5);
        flow.acknowledge(10000.0);
        assert_eq!(flow.allowance(now + INTERVAL), None);
        assert_eq!(flow.allowance(now + INTERVAL * 2), Some(1));
        flow.sent(1);
        assert_eq!(flow.allowance(now + INTERVAL * 3), None);
        flow.acknowledge(f32::NAN);
        assert_eq!(flow.allowance(now + INTERVAL * 4), None);
        flow.acknowledge(10000.0);
        assert_eq!(flow.allowance(now + INTERVAL * 5), Some(64));
    }

    #[test]
    fn outstanding_batches_do_not_stop_demand_refresh_or_add_a_second_read_delay() {
        let now = Instant::now();
        let mut flow = ChunkFlow::new(now, 4);
        assert_eq!(
            flow.time_until_poll(now + Duration::from_millis(40)),
            Duration::from_millis(10)
        );
        assert_eq!(flow.poll(now + INTERVAL), Some(0));
        assert_eq!(flow.poll(now + INTERVAL), None);
        assert_eq!(
            flow.time_until_poll(now + INTERVAL * 3),
            Duration::from_millis(1)
        );
        flow.acknowledge(16.0);
        assert_eq!(flow.poll(now + INTERVAL * 3), Some(16));
    }
}
