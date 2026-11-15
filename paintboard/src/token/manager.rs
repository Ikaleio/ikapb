use dashmap::DashMap;
use uuid::Uuid;

use super::cooldown::current_time_ms;
use super::store::TokenRecord;

#[derive(Debug, thiserror::Error)]
pub enum TokenManagerError {
    #[error("invalid token uuid")]
    InvalidUuid,
}

/// `TokenManager` maintains the set of issued tokens in memory, keyed by `uid`.
#[derive(Debug)]
pub struct TokenManager {
    // uid -> TokenRecord
    tokens_by_uid: DashMap<i64, TokenRecord>,
    // token UUID -> uid
    token_to_uid: DashMap<Uuid, i64>,
}

impl TokenManager {
    pub fn new() -> Self {
        Self {
            tokens_by_uid: DashMap::new(),
            token_to_uid: DashMap::new(),
        }
    }

    /// Load tokens from storage records
    pub fn load_tokens(&self, records: &[TokenRecord]) {
        for record in records {
            if let Ok(uuid) = Uuid::parse_str(&record.token) {
                self.token_to_uid.insert(uuid, record.uid);
                self.tokens_by_uid.insert(record.uid, record.clone());
            }
        }
    }

    /// Issues a brand-new token and tracks it as active, returning the record.
    /// If the uid already has a token, the old token will be removed.
    /// Returns (new_record, old_token_if_replaced)
    pub fn issue_token(
        &self,
        uid: i64,
        cooldown_ms: i64,
        ip: Option<String>,
    ) -> (TokenRecord, Option<String>) {
        let token = Uuid::new_v4();
        let now_ms = current_time_ms();

        let previous = self.tokens_by_uid.remove(&uid).map(|(_, r)| r);
        if let Some(old) = &previous {
            if let Ok(old_uuid) = Uuid::parse_str(&old.token) {
                self.token_to_uid.remove(&old_uuid);
            }
        }

        let record = TokenRecord {
            uid,
            token: token.to_string(),
            issued_at_ms: now_ms,
            cooldown_ms,
            ip,
            last_paint_at_ms: None,
        };

        self.token_to_uid.insert(token, uid);
        self.tokens_by_uid.insert(uid, record.clone());

        (record, previous.map(|r| r.token))
    }

    pub fn upsert_record(
        &self,
        record: TokenRecord,
    ) -> Result<Option<TokenRecord>, TokenManagerError> {
        let uuid = Uuid::parse_str(&record.token).map_err(|_| TokenManagerError::InvalidUuid)?;

        let previous = self.tokens_by_uid.insert(record.uid, record.clone());
        if let Some(old) = &previous {
            if let Ok(old_uuid) = Uuid::parse_str(&old.token) {
                self.token_to_uid.remove(&old_uuid);
            }
        }

        if let Some(existing_uid) = self.token_to_uid.insert(uuid, record.uid) {
            if existing_uid != record.uid {
                self.tokens_by_uid.remove(&existing_uid);
            }
        }

        Ok(previous)
    }

    pub fn remove(&self, token: &Uuid) -> Option<TokenRecord> {
        let removed_uid = self.token_to_uid.remove(token).map(|(_, uid)| uid)?;
        self.tokens_by_uid.remove(&removed_uid).map(|(_, v)| v)
    }

    pub fn active_record(&self, token: &Uuid) -> Option<TokenRecord> {
        self.token_to_uid
            .get(token)
            .and_then(|uid| self.tokens_by_uid.get(uid.value()))
            .map(|record| record.clone())
    }

    pub fn token_for_uid(&self, uid: i64) -> Option<Uuid> {
        self.tokens_by_uid
            .get(&uid)
            .and_then(|record| Uuid::parse_str(&record.token).ok())
    }

    /// Returns `true` when the provided UUID is known.
    pub fn validate_uuid(&self, token: &Uuid) -> bool {
        self.active_record(token).is_some()
    }

    /// Returns the cooldown_ms for a token, or None if not found
    pub fn get_cooldown_ms(&self, token: &Uuid) -> Option<i64> {
        self.active_record(token).map(|r| r.cooldown_ms)
    }

    /// Returns the effective cooldown_ms for a token, resolving -1 to global config value.
    /// Returns None if token not found.
    pub fn get_effective_cooldown_ms(&self, token: &Uuid, global_cooldown_ms: i64) -> Option<i64> {
        self.active_record(token).map(|r| {
            if r.cooldown_ms == -1 {
                global_cooldown_ms
            } else {
                r.cooldown_ms
            }
        })
    }

    /// Returns `true` when the provided token bytes are recognized.
    pub fn validate_token_bytes(&self, token: &[u8]) -> bool {
        if token.len() != 16 {
            return false;
        }
        match Uuid::from_slice(token) {
            Ok(uuid) => self.validate_uuid(&uuid),
            Err(_) => false,
        }
    }
}

impl Default for TokenManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::TokenManager;

    #[test]
    fn issued_tokens_are_tracked() {
        let manager = TokenManager::new();
        let (record, old) = manager.issue_token(12345, -1, None);
        assert!(old.is_none()); // First token, no old token
        let token = uuid::Uuid::parse_str(&record.token).unwrap();
        assert!(manager.validate_uuid(&token));
        assert!(manager.validate_token_bytes(token.as_bytes()));
        assert_eq!(record.uid, 12345);
        assert_eq!(record.cooldown_ms, -1);

        assert_eq!(
            manager.get_effective_cooldown_ms(&token, 30000),
            Some(30000)
        );
        assert_eq!(
            manager.get_effective_cooldown_ms(&token, 60000),
            Some(60000)
        );
    }

    #[test]
    fn same_uid_replaces_old_token() {
        let manager = TokenManager::new();
        let (record1, old1) = manager.issue_token(12345, -1, None);
        assert!(old1.is_none());
        let token1 = uuid::Uuid::parse_str(&record1.token).unwrap();
        assert!(manager.validate_uuid(&token1));

        // Issue new token for same uid
        let (record2, old2) = manager.issue_token(12345, 5000, None);
        assert_eq!(old2, Some(record1.token.clone())); // Old token returned
        let token2 = uuid::Uuid::parse_str(&record2.token).unwrap();

        // Old token should be invalid now
        assert!(!manager.validate_uuid(&token1));
        // New token should be valid
        assert!(manager.validate_uuid(&token2));
        assert_eq!(record2.uid, 12345);
        assert_eq!(record2.cooldown_ms, 5000);
    }

    #[test]
    fn specific_cooldown_overrides_global() {
        let manager = TokenManager::new();
        let (record, _old) = manager.issue_token(12345, 5000, None);
        let token = uuid::Uuid::parse_str(&record.token).unwrap();

        assert_eq!(manager.get_effective_cooldown_ms(&token, 30000), Some(5000));
        assert_eq!(record.cooldown_ms, 5000);
    }

    #[test]
    fn unknown_tokens_are_rejected() {
        let manager = TokenManager::new();
        let random_token = uuid::Uuid::new_v4();
        assert!(!manager.validate_uuid(&random_token));
        assert!(!manager.validate_token_bytes(random_token.as_bytes()));
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        let manager = TokenManager::new();
        // Too short, should be rejected immediately.
        let result = manager.validate_token_bytes(&[0xFF]);
        assert!(!result);
    }
}
