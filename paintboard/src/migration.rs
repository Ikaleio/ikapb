use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use tracing::{info, warn};

use crate::constants::{BOARD_HEIGHT, BOARD_WIDTH};
use crate::token::store::{self, TokenRecord};

pub struct MigrationResult {
    pub board_pixels: Option<Vec<u8>>,
    pub imported_token_count: usize,
}

pub fn import_if_present(sqlite_conn: &Connection) -> Result<Option<MigrationResult>> {
    let cwd = std::env::current_dir().context("failed to get current working directory")?;
    let migration_path = cwd.join("migration.db");

    if !migration_path.exists() {
        return Ok(None);
    }

    let mut old_conn = Connection::open(&migration_path).with_context(|| {
        format!(
            "failed to open migration database at {}",
            migration_path.display()
        )
    })?;

    let board_pixels = load_board_pixels(&mut old_conn)?;
    let tokens = load_tokens(&mut old_conn)?;

    drop(old_conn);

    for record in &tokens {
        store::upsert_token(sqlite_conn, record)?;
    }

    info!(count = tokens.len(), path = %migration_path.display(), "imported tokens from migration database");

    let backup_path = migration_path.with_file_name("migration.db.bak");
    fs::rename(&migration_path, &backup_path).with_context(|| {
        format!(
            "failed to rename migration database from {} to {}",
            migration_path.display(),
            backup_path.display()
        )
    })?;

    info!(path = %backup_path.display(), "renamed migration database after import");

    Ok(Some(MigrationResult {
        board_pixels,
        imported_token_count: tokens.len(),
    }))
}

fn load_board_pixels(conn: &mut Connection) -> Result<Option<Vec<u8>>> {
    let mut stmt = conn.prepare("SELECT width, height, pixels FROM board_data WHERE id = 1")?;
    let row = stmt
        .query_row([], |row| {
            let width: i64 = row.get(0)?;
            let height: i64 = row.get(1)?;
            let pixels: Vec<u8> = row.get(2)?;
            Ok((width, height, pixels))
        })
        .optional()?;

    let Some((width, height, pixels)) = row else {
        return Ok(None);
    };

    if width as u32 != BOARD_WIDTH || height as u32 != BOARD_HEIGHT {
        warn!(
            width,
            height,
            expected_width = BOARD_WIDTH,
            expected_height = BOARD_HEIGHT,
            "migration board size mismatch; skipping board import"
        );
        return Ok(None);
    }

    let expected_len = (BOARD_WIDTH * BOARD_HEIGHT * 3) as usize;
    if pixels.len() != expected_len {
        warn!(
            got = pixels.len(),
            expected = expected_len,
            "migration board pixel blob length mismatch; skipping board import"
        );
        return Ok(None);
    }

    info!(
        width,
        height, "imported board pixels from migration database"
    );

    Ok(Some(pixels))
}

fn load_tokens(conn: &mut Connection) -> Result<Vec<TokenRecord>> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut stmt = conn.prepare("SELECT token, uid FROM tokens ORDER BY rowid")?;
    let mut records = Vec::new();
    let mut index_by_uid: HashMap<i64, usize> = HashMap::new();

    let rows = stmt.query_map([], |row| {
        let token: String = row.get(0)?;
        let uid: i64 = row.get(1)?;
        Ok((token, uid))
    })?;

    for row in rows {
        let (token, uid) = row?;
        if let Ok(uuid) = uuid::Uuid::parse_str(&token) {
            let record = TokenRecord {
                uid,
                token: uuid.to_string(),
                issued_at_ms: now_ms,
                cooldown_ms: -1,
                ip: None,
                last_paint_at_ms: None,
            };
            if let Some(idx) = index_by_uid.get(&uid) {
                records[*idx] = record;
            } else {
                let idx = records.len();
                records.push(record);
                index_by_uid.insert(uid, idx);
            }
        } else {
            warn!(
                ?uid,
                token, "skipping migration token that is not a valid UUID"
            );
        }
    }

    Ok(records)
}
