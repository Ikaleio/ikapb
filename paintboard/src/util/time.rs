use once_cell::sync::Lazy;
use std::time::Instant;

static START: Lazy<Instant> = Lazy::new(Instant::now);

pub fn now_ns_monotonic() -> u64 {
    START.elapsed().as_nanos() as u64
}
