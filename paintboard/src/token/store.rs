use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub uid: i64,
    pub token: String,
    pub issued_at_ms: i64,
    pub cooldown_ms: i64,
    pub ip: Option<String>,
    pub last_paint_at_ms: Option<i64>,
}

fn checkpoint_wal(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
}

#[derive(Debug, Default, Clone)]
pub struct LegacyMigrationOutcome {
    pub migrated: usize,
    pub expired_or_revoked: usize,
    pub invalid_tokens: usize,
    pub stale_state_rows: usize,
    pub deduplicated: usize,
}

pub fn init_schema(conn: &Connection, schema: &str) -> Result<Option<LegacyMigrationOutcome>> {
    let migrated_tokens = migrate_legacy_tables(conn)?;

    for statement in schema.split(';') {
        let trimmed = statement.trim();
        if trimmed.is_empty() {
            continue;
        }

        conn.execute_batch(&format!("{};", trimmed))?;
    }

    // Drop legacy token_state table if it still exists.
    conn.execute_batch("DROP TABLE IF EXISTS token_state;")?;

    if let Some((records, stats)) = &migrated_tokens {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO tokens(uid, token, issued_at_ms, cooldown_ms, ip, last_paint_at_ms)
                 VALUES(?,?,?,?,?,?)",
            )?;

            for record in records {
                stmt.execute(params![
                    record.uid,
                    record.token,
                    record.issued_at_ms,
                    record.cooldown_ms,
                    record.ip,
                    record.last_paint_at_ms,
                ])?;
            }
        }
        tx.commit()?;

        checkpoint_wal(conn)?;
        return Ok(Some(stats.clone()));
    }

    checkpoint_wal(conn)?;
    Ok(None)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Option<Vec<String>>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({});", table))
        .with_context(|| format!("failed to inspect table info for {}", table))?;
    let rows = stmt
        .query_map([], |row| row.get(1))?
        .collect::<rusqlite::Result<Vec<String>>>()?;

    if rows.is_empty() {
        Ok(None)
    } else {
        Ok(Some(rows))
    }
}

fn migrate_legacy_tables(
    conn: &Connection,
) -> Result<Option<(Vec<TokenRecord>, LegacyMigrationOutcome)>> {
    let Some(columns) = table_columns(conn, "tokens")? else {
        return Ok(None);
    };

    let has_expire = columns.iter().any(|c| c == "expire_at_ms");
    let has_revoked = columns.iter().any(|c| c == "revoked");
    let has_last_paint = columns.iter().any(|c| c == "last_paint_at_ms");
    let has_ban_reason = columns.iter().any(|c| c == "ban_reason");

    // Already on the new schema; nothing to do.
    if !has_expire && !has_revoked && has_last_paint && !has_ban_reason {
        return Ok(None);
    }

    if !has_expire && !has_revoked && has_last_paint && has_ban_reason {
        let mut stmt = conn.prepare(
            "SELECT uid, token, issued_at_ms, cooldown_ms, ip, last_paint_at_ms FROM tokens",
        )?;
        let records = stmt
            .query_map([], |row| {
                Ok(TokenRecord {
                    uid: row.get(0)?,
                    token: row.get(1)?,
                    issued_at_ms: row.get(2)?,
                    cooldown_ms: row.get(3)?,
                    ip: row.get(4)?,
                    last_paint_at_ms: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut outcome = LegacyMigrationOutcome::default();
        outcome.migrated = records.len();

        // Drop old tables and indexes before recreating the new schema.
        drop_indexes(conn, "tokens")?;
        conn.execute_batch("DROP TABLE IF EXISTS tokens;")?;

        return Ok(Some((records, outcome)));
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut state_by_token: HashMap<String, i64> = HashMap::new();
    if table_columns(conn, "token_state")?.is_some() {
        let mut stmt = conn.prepare("SELECT token, last_paint_at_ms FROM token_state")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (token, last) = row?;
            state_by_token.insert(token, last);
        }
    }

    let mut migrated_records: HashMap<i64, TokenRecord> = HashMap::new();
    let mut outcome = LegacyMigrationOutcome::default();

    let mut stmt = conn.prepare(
        "SELECT token, uid, issued_at_ms, cooldown_ms, COALESCE(expire_at_ms, 0), COALESCE(revoked, 0) FROM tokens",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;

    for row in rows {
        let (token, uid, issued_at_ms, cooldown_ms, expire_at_ms, revoked) = row?;
        if expire_at_ms > 0 && expire_at_ms <= now_ms {
            outcome.expired_or_revoked += 1;
            continue;
        }

        if revoked != 0 {
            outcome.expired_or_revoked += 1;
            continue;
        }

        let Ok(uuid) = uuid::Uuid::parse_str(&token) else {
            outcome.invalid_tokens += 1;
            continue;
        };

        let last_paint_at_ms = state_by_token.remove(&token);
        let record = TokenRecord {
            uid,
            token: uuid.to_string(),
            issued_at_ms,
            cooldown_ms,
            ip: None,
            last_paint_at_ms,
        };

        match migrated_records.get(&uid) {
            Some(existing) if existing.issued_at_ms > issued_at_ms => {
                outcome.deduplicated += 1;
            }
            _ => {
                if migrated_records.insert(uid, record).is_some() {
                    outcome.deduplicated += 1;
                }
            }
        }
    }

    outcome.stale_state_rows = state_by_token.len();

    // Drop old tables and indexes before recreating the new schema.
    drop_indexes(conn, "tokens")?;
    drop_indexes(conn, "token_state")?;
    conn.execute_batch("DROP TABLE IF EXISTS token_state;")?;
    conn.execute_batch("DROP TABLE IF EXISTS tokens;")?;

    let records: Vec<TokenRecord> = migrated_records.into_values().collect();
    outcome.migrated = records.len();

    Ok(Some((records, outcome)))
}

fn drop_indexes(conn: &Connection, table: &str) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = ?1 AND name NOT LIKE 'sqlite_autoindex%';",
    )?;

    let names = stmt
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;

    for name in names {
        conn.execute(&format!("DROP INDEX IF EXISTS {}", name), [])?;
    }

    Ok(())
}

pub fn load_all_tokens(conn: &Connection) -> Result<Vec<TokenRecord>> {
    let mut stmt = conn.prepare(
        "SELECT uid, token, issued_at_ms, cooldown_ms, ip, last_paint_at_ms FROM tokens",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(TokenRecord {
                uid: row.get(0)?,
                token: row.get(1)?,
                issued_at_ms: row.get(2)?,
                cooldown_ms: row.get(3)?,
                ip: row.get(4)?,
                last_paint_at_ms: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_token(conn: &Connection, token: &str) -> Result<Option<TokenRecord>> {
    let mut stmt = conn.prepare(
        "SELECT uid, token, issued_at_ms, cooldown_ms, ip, last_paint_at_ms FROM tokens WHERE token = ?",
    )?;
    let mut rows = stmt.query([token])?;
    if let Some(row) = rows.next()? {
        Ok(Some(TokenRecord {
            uid: row.get(0)?,
            token: row.get(1)?,
            issued_at_ms: row.get(2)?,
            cooldown_ms: row.get(3)?,
            ip: row.get(4)?,
            last_paint_at_ms: row.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn get_token_by_uid(conn: &Connection, uid: i64) -> Result<Option<TokenRecord>> {
    let mut stmt = conn.prepare(
        "SELECT uid, token, issued_at_ms, cooldown_ms, ip, last_paint_at_ms FROM tokens WHERE uid = ?",
    )?;
    let mut rows = stmt.query([uid])?;
    if let Some(row) = rows.next()? {
        Ok(Some(TokenRecord {
            uid: row.get(0)?,
            token: row.get(1)?,
            issued_at_ms: row.get(2)?,
            cooldown_ms: row.get(3)?,
            ip: row.get(4)?,
            last_paint_at_ms: row.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn upsert_token(conn: &Connection, record: &TokenRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO tokens(uid, token, issued_at_ms, cooldown_ms, ip, last_paint_at_ms)
         VALUES(?,?,?,?,?,?)
         ON CONFLICT(uid) DO UPDATE SET
            token=excluded.token,
            issued_at_ms=excluded.issued_at_ms,
            cooldown_ms=excluded.cooldown_ms,
            ip=excluded.ip,
            last_paint_at_ms=excluded.last_paint_at_ms",
        params![
            record.uid,
            record.token,
            record.issued_at_ms,
            record.cooldown_ms,
            record.ip,
            record.last_paint_at_ms,
        ],
    )?;
    checkpoint_wal(conn)?;
    Ok(())
}

pub fn persist_last_paint_batch(conn: &Connection, states: &[(String, i64)]) -> Result<()> {
    if states.is_empty() {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE tokens SET last_paint_at_ms = ? WHERE token = ?")?;
        for (token, last) in states {
            stmt.execute(params![last, token])?;
        }
    }
    tx.commit()?;
    checkpoint_wal(conn)?;
    Ok(())
}

pub fn clear_last_paint(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE tokens SET last_paint_at_ms = NULL", [])?;
    checkpoint_wal(conn)?;
    Ok(())
}

pub fn delete_token(conn: &Connection, token: &str) -> Result<()> {
    conn.execute("DELETE FROM tokens WHERE token = ?", params![token])?;
    checkpoint_wal(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upserts_and_clears_last_paint() {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            "CREATE TABLE tokens(
                uid INTEGER PRIMARY KEY,
                token TEXT NOT NULL,
                issued_at_ms INTEGER NOT NULL,
                cooldown_ms INTEGER NOT NULL,
                ip TEXT,
                last_paint_at_ms INTEGER
            );",
        )
        .unwrap();

        let record = TokenRecord {
            uid: 7,
            token: "token-a".into(),
            issued_at_ms: 10,
            cooldown_ms: 1000,
            ip: None,
            last_paint_at_ms: Some(50),
        };

        upsert_token(&conn, &record).unwrap();

        let loaded = load_all_tokens(&conn).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].last_paint_at_ms, Some(50));

        persist_last_paint_batch(&conn, &[("token-a".to_string(), 99)]).unwrap();
        let updated = load_all_tokens(&conn).unwrap();
        assert_eq!(updated[0].last_paint_at_ms, Some(99));

        clear_last_paint(&conn).unwrap();
        let cleared = load_all_tokens(&conn).unwrap();
        assert!(cleared[0].last_paint_at_ms.is_none());
    }

    #[test]
    fn migrates_legacy_tables() {
        let schema_sql = r#"
CREATE TABLE IF NOT EXISTS tokens(
  uid INTEGER PRIMARY KEY,
  token TEXT NOT NULL,
  issued_at_ms INTEGER NOT NULL,
  cooldown_ms INTEGER NOT NULL,
  ip TEXT,
  last_paint_at_ms INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_tokens_token_unique ON tokens(token);
"#;

        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            "CREATE TABLE tokens(
                token TEXT PRIMARY KEY,
                uid INTEGER NOT NULL,
                issued_at_ms INTEGER NOT NULL,
                cooldown_ms INTEGER NOT NULL,
                expire_at_ms INTEGER,
                revoked INTEGER DEFAULT 0
            );
            CREATE INDEX idx_tokens_uid ON tokens(uid);
            CREATE TABLE token_state(token TEXT PRIMARY KEY, last_paint_at_ms INTEGER);
            CREATE INDEX idx_token_state_token ON token_state(token);
            CREATE TABLE bans(uid INTEGER PRIMARY KEY, reason TEXT, created_at_ms INTEGER NOT NULL, expire_at_ms INTEGER);
            ",
        )
        .unwrap();

        let t1 = uuid::Uuid::from_u128(1);
        let t2 = uuid::Uuid::from_u128(2);
        let t3 = uuid::Uuid::from_u128(3);
        let t4 = uuid::Uuid::from_u128(4);
        let t5 = uuid::Uuid::from_u128(5);

        conn.execute(
            "INSERT INTO tokens(token, uid, issued_at_ms, cooldown_ms, expire_at_ms, revoked) VALUES(?,?,?,?,?,?)",
            params![t1.to_string(), 1, 100i64, 5000i64, Option::<i64>::None, 0i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tokens(token, uid, issued_at_ms, cooldown_ms, expire_at_ms, revoked) VALUES(?,?,?,?,?,?)",
            params![t2.to_string(), 1, 200i64, 3000i64, Option::<i64>::None, 0i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tokens(token, uid, issued_at_ms, cooldown_ms, expire_at_ms, revoked) VALUES(?,?,?,?,?,?)",
            params![t3.to_string(), 1, 150i64, 3000i64, 1i64, 0i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tokens(token, uid, issued_at_ms, cooldown_ms, expire_at_ms, revoked) VALUES(?,?,?,?,?,?)",
            params![t4.to_string(), 2, 250i64, 2000i64, Option::<i64>::None, 0i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tokens(token, uid, issued_at_ms, cooldown_ms, expire_at_ms, revoked) VALUES(?,?,?,?,?,?)",
            params!["not-a-uuid", 3i64, 50i64, 1000i64, Option::<i64>::None, 0i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tokens(token, uid, issued_at_ms, cooldown_ms, expire_at_ms, revoked) VALUES(?,?,?,?,?,?)",
            params![t5.to_string(), 4, 175i64, 1500i64, Option::<i64>::None, 1i64],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO token_state(token, last_paint_at_ms) VALUES(?, ?)",
            params![t1.to_string(), 42i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO token_state(token, last_paint_at_ms) VALUES(?, ?)",
            params!["stale-token", 99i64],
        )
        .unwrap();

        let outcome = init_schema(&conn, schema_sql).unwrap().unwrap();

        let tokens = load_all_tokens(&conn).unwrap();
        assert_eq!(tokens.len(), 2);

        let by_uid: HashMap<i64, TokenRecord> = tokens.into_iter().map(|r| (r.uid, r)).collect();

        let uid1 = by_uid.get(&1).unwrap();
        assert_eq!(uid1.token, t2.to_string());
        assert_eq!(uid1.cooldown_ms, 3000);
        assert!(uid1.last_paint_at_ms.is_none());

        let uid2 = by_uid.get(&2).unwrap();
        assert_eq!(uid2.token, t4.to_string());
        assert_eq!(uid2.cooldown_ms, 2000);
        assert_eq!(uid2.last_paint_at_ms, None);

        let mut check_stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'token_state';",
            )
            .unwrap();
        let has_token_state = check_stmt.exists([]).unwrap();
        assert!(!has_token_state);

        assert_eq!(outcome.migrated, 2);
        assert_eq!(outcome.expired_or_revoked, 2);
        assert_eq!(outcome.invalid_tokens, 1);
        assert_eq!(outcome.stale_state_rows, 1);
        assert_eq!(outcome.deduplicated, 1);
    }

    #[test]
    fn migrates_removes_ban_reason_column() {
        let schema_sql = r#"
CREATE TABLE IF NOT EXISTS tokens(
  uid INTEGER PRIMARY KEY,
  token TEXT NOT NULL,
  issued_at_ms INTEGER NOT NULL,
  cooldown_ms INTEGER NOT NULL,
  ip TEXT,
  last_paint_at_ms INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_tokens_token_unique ON tokens(token);
"#;

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tokens(
                uid INTEGER PRIMARY KEY,
                token TEXT NOT NULL,
                issued_at_ms INTEGER NOT NULL,
                cooldown_ms INTEGER NOT NULL,
                ip TEXT,
                ban_reason TEXT,
                last_paint_at_ms INTEGER
            );
            CREATE UNIQUE INDEX idx_tokens_token_unique ON tokens(token);",
        )
        .unwrap();

        let t1 = uuid::Uuid::from_u128(1).to_string();
        let t2 = uuid::Uuid::from_u128(2).to_string();

        conn.execute(
            "INSERT INTO tokens(uid, token, issued_at_ms, cooldown_ms, ip, ban_reason, last_paint_at_ms) VALUES(?,?,?,?,?,?,?)",
            params![1i64, t1, 10i64, 1000i64, "203.0.113.1", "old-ban-reason", 50i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tokens(uid, token, issued_at_ms, cooldown_ms, ip, ban_reason, last_paint_at_ms) VALUES(?,?,?,?,?,?,?)",
            params![2i64, t2, 20i64, -1i64, Option::<String>::None, Option::<String>::None, Option::<i64>::None],
        )
        .unwrap();

        let outcome = init_schema(&conn, schema_sql).unwrap().unwrap();
        assert_eq!(outcome.migrated, 2);
        assert_eq!(outcome.expired_or_revoked, 0);
        assert_eq!(outcome.invalid_tokens, 0);
        assert_eq!(outcome.stale_state_rows, 0);
        assert_eq!(outcome.deduplicated, 0);

        let columns = table_columns(&conn, "tokens").unwrap().unwrap();
        assert!(!columns.iter().any(|c| c == "ban_reason"));

        let mut records = load_all_tokens(&conn).unwrap();
        records.sort_by_key(|r| r.uid);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].uid, 1);
        assert_eq!(records[0].ip.as_deref(), Some("203.0.113.1"));
        assert_eq!(records[0].last_paint_at_ms, Some(50));
        assert_eq!(records[1].uid, 2);
        assert!(records[1].ip.is_none());
        assert!(records[1].last_paint_at_ms.is_none());
    }
}
