use anyhow::Result;
use rusqlite::{Connection, params};

use super::ban::BanRecord;

pub fn load_all_bans(conn: &Connection) -> Result<Vec<BanRecord>> {
    let mut stmt = conn.prepare("SELECT uid, reason, created_at_ms, expire_at_ms FROM bans")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(BanRecord {
                uid: row.get(0)?,
                reason: row.get(1)?,
                created_at_ms: row.get(2)?,
                expire_at_ms: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn upsert_ban(conn: &Connection, record: &BanRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO bans(uid, reason, created_at_ms, expire_at_ms) VALUES(?,?,?,?)\
         ON CONFLICT(uid) DO UPDATE SET reason=excluded.reason, created_at_ms=excluded.created_at_ms, expire_at_ms=excluded.expire_at_ms",
        params![
            record.uid,
            record.reason,
            record.created_at_ms,
            record.expire_at_ms
        ],
    )?;
    Ok(())
}

pub fn delete_ban(conn: &Connection, uid: i64) -> Result<()> {
    conn.execute("DELETE FROM bans WHERE uid = ?", params![uid])?;
    Ok(())
}
