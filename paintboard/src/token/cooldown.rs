use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;

#[derive(Default)]
pub struct CooldownCache {
    entries: DashMap<String, i64>,
}

impl CooldownCache {
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    pub fn can_paint(&self, token: &str, now_ms: i64, cooldown_ms: i64) -> bool {
        match self.entries.get(token) {
            Some(entry) => now_ms - *entry >= cooldown_ms,
            None => true,
        }
    }

    pub fn mark_painted(&self, token: &str, now_ms: i64) {
        self.entries.insert(token.to_string(), now_ms);
    }

    pub fn snapshot(&self) -> Vec<(String, i64)> {
        self.entries
            .iter()
            .map(|it| (it.key().clone(), *it.value()))
            .collect()
    }

    pub fn evict(&self, token: &str) {
        self.entries.remove(token);
    }

    pub fn load_state(&self, token: String, last_ms: i64) {
        self.entries.insert(token, last_ms);
    }
}

pub fn current_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
