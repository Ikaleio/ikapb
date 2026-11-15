use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BanRecord {
    pub uid: i64,
    #[serde(default)]
    pub reason: Option<String>,
    pub created_at_ms: i64,
    #[serde(default)]
    pub expire_at_ms: Option<i64>,
}

impl BanRecord {
    pub fn is_active(&self, now_ms: i64) -> bool {
        match self.expire_at_ms {
            Some(expire) => expire > now_ms,
            None => true,
        }
    }
}

#[derive(Debug, Default)]
pub struct BanStore {
    entries: DashMap<i64, BanRecord>,
}

impl BanStore {
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    pub fn load(&self, records: Vec<BanRecord>) {
        for record in records {
            self.entries.insert(record.uid, record);
        }
    }

    pub fn upsert(&self, record: BanRecord) {
        self.entries.insert(record.uid, record);
    }

    pub fn remove(&self, uid: i64) -> Option<BanRecord> {
        self.entries.remove(&uid).map(|(_, v)| v)
    }

    pub fn get(&self, uid: i64) -> Option<BanRecord> {
        self.entries.get(&uid).map(|r| r.clone())
    }

    pub fn active_record(&self, uid: i64, now_ms: i64) -> Option<BanRecord> {
        self.entries
            .get(&uid)
            .map(|r| r.clone())
            .filter(|r| r.is_active(now_ms))
    }

    pub fn all(&self) -> Vec<BanRecord> {
        self.entries.iter().map(|r| r.clone()).collect()
    }
}
