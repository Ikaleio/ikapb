use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::types::TickIndex;
use crate::util::time::now_ns_monotonic;

pub struct TickCoordinator {
    tick_index: AtomicU64,
    tick_duration_ns: AtomicU64,
    epsilon_ns: AtomicU64,
    /// monotonic ns when the current tick started
    current_tick_start_ns: AtomicU64,
}

impl TickCoordinator {
    pub fn new(tps: u32) -> Self {
        let duration_ns = if tps == 0 {
            0
        } else {
            1_000_000_000u64 / tps as u64
        };
        Self {
            tick_index: AtomicU64::new(0),
            tick_duration_ns: AtomicU64::new(duration_ns.max(1)),
            epsilon_ns: AtomicU64::new(500_000),
            current_tick_start_ns: AtomicU64::new(now_ns_monotonic()),
        }
    }

    pub fn tick_index(&self) -> TickIndex {
        TickIndex(self.tick_index.load(Ordering::Relaxed))
    }

    /// Returns the monotonic timestamp (ns) when the current tick started.
    pub fn current_tick_start_ns(&self) -> u64 {
        self.current_tick_start_ns.load(Ordering::Relaxed)
    }

    pub fn set_tick_index(&self, tick: TickIndex) {
        self.tick_index.store(tick.0, Ordering::Relaxed);
    }

    pub fn set_tps(&self, tps: u32) {
        let duration_ns = if tps == 0 {
            0
        } else {
            1_000_000_000u64 / tps as u64
        };
        self.tick_duration_ns
            .store(duration_ns.max(1), Ordering::Relaxed);
    }

    pub fn bump_manual_tick(&self) -> TickIndex {
        let tick = self.tick_index.fetch_add(1, Ordering::Relaxed) + 1;
        self.current_tick_start_ns
            .store(now_ns_monotonic(), Ordering::Relaxed);
        TickIndex(tick)
    }

    pub fn set_epsilon_ns(&self, epsilon_ns: u64) {
        self.epsilon_ns.store(epsilon_ns, Ordering::Relaxed);
    }

    /// Decide which tick a timestamp belongs to (accounting for the epsilon soft window).
    /// If now is in [deadline - epsilon, deadline), it may be attributed to the next tick.
    pub fn tick_for_timestamp(&self, now_ns: u64, last_tick_start_ns: u64) -> TickIndex {
        let duration = self.tick_duration_ns.load(Ordering::Relaxed);
        let epsilon = self.epsilon_ns.load(Ordering::Relaxed);
        let current_tick = self.tick_index.load(Ordering::Relaxed);

        let deadline = last_tick_start_ns + duration;

        if now_ns >= deadline.saturating_sub(epsilon) {
            TickIndex(current_tick + 1)
        } else {
            TickIndex(current_tick)
        }
    }

    pub async fn run<F>(&self, mut on_tick: F)
    where
        F: FnMut(TickIndex) + Send,
    {
        let mut last_tick = now_ns_monotonic();
        self.current_tick_start_ns
            .store(last_tick, Ordering::Relaxed);
        loop {
            let duration = self.tick_duration_ns.load(Ordering::Relaxed);
            let deadline = last_tick + duration;
            let now = now_ns_monotonic();
            if now < deadline {
                let sleep_ns = deadline - now;
                tokio::time::sleep(Duration::from_nanos(sleep_ns)).await;
            }
            let tick = self.tick_index.fetch_add(1, Ordering::Relaxed) + 1;
            self.current_tick_start_ns
                .store(deadline, Ordering::Relaxed);
            on_tick(TickIndex(tick));
            last_tick = deadline;
        }
    }
}
