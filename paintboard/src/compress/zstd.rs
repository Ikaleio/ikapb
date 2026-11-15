use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use bytes::Bytes;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

use super::dict::Dict;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    tick: u64,
    level: i32,
    dict_id: Option<u32>,
}

static COMPRESS_CACHE: Lazy<Mutex<HashMap<CacheKey, Bytes>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static LAST_PRUNE_TICK: AtomicU64 = AtomicU64::new(0);

// Keep a small rolling window. This bounds memory and is sufficient for multi-connection fanout.
const CACHE_TICK_WINDOW: u64 = 8;
const MAX_CACHE_ENTRIES: usize = 256;

pub fn compress_frame(raw: &[u8], level: i32, dict: Option<&Dict>) -> Result<Bytes> {
    let compressed = match dict {
        Some(d) => {
            // Use dictionary compression.
            let mut encoder = zstd::stream::Encoder::with_dictionary(Vec::new(), level, d.bytes())?;
            std::io::Write::write_all(&mut encoder, raw)?;
            encoder.finish()?
        }
        None => zstd::bulk::compress(raw, level)?,
    };
    Ok(Bytes::from(compressed))
}

pub fn compress_frame_cached(
    tick: u64,
    raw: &[u8],
    level: i32,
    dict: Option<&Dict>,
) -> Result<Bytes> {
    let key = CacheKey {
        tick,
        level,
        dict_id: dict.map(|d| d.id),
    };

    {
        let cache = COMPRESS_CACHE.lock();
        if let Some(hit) = cache.get(&key) {
            return Ok(hit.clone());
        }
    }

    let compressed = compress_frame(raw, level, dict)?;

    let mut cache = COMPRESS_CACHE.lock();
    cache.insert(key, compressed.clone());

    // Opportunistic pruning (only on tick advancement).
    let last = LAST_PRUNE_TICK.load(Ordering::Relaxed);
    if tick > last
        && LAST_PRUNE_TICK
            .compare_exchange(last, tick, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let min_tick = tick.saturating_sub(CACHE_TICK_WINDOW);
        cache.retain(|k, _| k.tick >= min_tick);
        if cache.len() > MAX_CACHE_ENTRIES {
            // Last-resort bound.
            cache.clear();
        }
    }

    Ok(compressed)
}

pub fn ratio(raw_len: usize, compressed_len: usize) -> f64 {
    if raw_len == 0 {
        return 1.0;
    }
    raw_len as f64 / compressed_len.max(1) as f64
}
