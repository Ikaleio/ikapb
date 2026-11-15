use rand::Rng;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::ws::codec;

pub struct Heartbeat {
    inflight: Mutex<bool>,
    timeout: Duration,
    last_ping_at: Mutex<Option<Instant>>,
    last_pong_at: Mutex<Instant>,
}

impl Heartbeat {
    pub fn new(timeout: Duration) -> Self {
        Self {
            inflight: Mutex::new(false),
            timeout,
            last_ping_at: Mutex::new(None),
            last_pong_at: Mutex::new(Instant::now()),
        }
    }

    pub async fn schedule_ping<F>(&self, mut sender: F)
    where
        F: FnMut(bytes::Bytes) + Send,
    {
        // Jittered ping: randomly sleep within [0.5*timeout, 1.5*timeout]
        let base = self.timeout;
        loop {
            let base_ms = base.as_millis() as u64;
            let jitter_range_ms = (base_ms / 2).max(100);
            let min_delay = base_ms.saturating_sub(jitter_range_ms);
            let max_delay = base_ms + jitter_range_ms;
            let delay_ms = {
                let mut rng = rand::thread_rng();
                rng.gen_range(min_delay..=max_delay)
            };
            sleep(Duration::from_millis(delay_ms)).await;
            let mut inflight = self.inflight.lock().await;
            if !*inflight {
                sender(codec::encode_ping());
                *inflight = true;
                *self.last_ping_at.lock().await = Some(Instant::now());
            }
        }
    }

    pub async fn handle_pong(&self) {
        let mut inflight = self.inflight.lock().await;
        *inflight = false;
        *self.last_pong_at.lock().await = Instant::now();
    }

    pub async fn timed_out(&self) -> bool {
        let inflight = *self.inflight.lock().await;
        if !inflight {
            return false;
        }
        let now = Instant::now();
        let last_ping = *self.last_ping_at.lock().await;
        match last_ping {
            Some(lp) => now.duration_since(lp) > self.timeout * 2,
            None => false,
        }
    }
}
