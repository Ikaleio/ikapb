use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::types::{PixelIndex, Rgba, TickIndex};

#[derive(Debug, Clone, Copy)]
pub struct WalEntry {
    pub tick: TickIndex,
    pub idx: PixelIndex,
    pub rgba: Rgba,
    pub uid: u32,
}

fn wal_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wal.bin")
}

pub async fn append_batch(
    data_dir: &Path,
    tick: TickIndex,
    entries: &[(PixelIndex, Rgba, u32)],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(data_dir).await.ok();
    let path = wal_path(data_dir);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .with_context(|| format!("open wal for append: {}", path.display()))?;

    // Format: [b"WAL1"][count:U32][tick:U64][repeat count * (idx:U32, rgba:U32, uid:U32)]
    let mut buf = Vec::with_capacity(8 + entries.len() * 12);
    buf.extend_from_slice(b"WAL1");
    buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    buf.extend_from_slice(&tick.0.to_le_bytes());
    for (idx, rgba, uid) in entries {
        buf.extend_from_slice(&idx.0.to_le_bytes());
        buf.extend_from_slice(&rgba.0.to_le_bytes());
        buf.extend_from_slice(&uid.to_le_bytes());
    }
    file.write_all(&buf).await?;
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}

pub async fn replay_since(data_dir: &Path, since: TickIndex) -> Result<Vec<WalEntry>> {
    let path = wal_path(data_dir);
    let mut file = match File::open(&path).await {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).await?;
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= buf.len() {
        if &buf[i..i + 4] != b"WAL1" {
            break;
        }
        i += 4;
        if i + 4 + 8 > buf.len() {
            break;
        }
        let count = u32::from_le_bytes(buf[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        let tick = TickIndex(u64::from_le_bytes(buf[i..i + 8].try_into().unwrap()));
        i += 8;
        // read entries
        for _ in 0..count {
            if i + 12 > buf.len() {
                break;
            }
            let idx = PixelIndex(u32::from_le_bytes(buf[i..i + 4].try_into().unwrap()));
            i += 4;
            let rgba = Rgba(u32::from_le_bytes(buf[i..i + 4].try_into().unwrap()));
            i += 4;
            let uid = u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
            i += 4;
            if tick.0 > since.0 {
                out.push(WalEntry {
                    tick,
                    idx,
                    rgba,
                    uid,
                });
            }
        }
    }
    Ok(out)
}

pub async fn rotate(data_dir: &Path) -> Result<()> {
    let path = wal_path(data_dir);
    let ts = Utc::now().timestamp_millis();
    let rotated = data_dir.join(format!("wal-{}.bin", ts));
    if fs::rename(&path, &rotated).await.is_err() {
        // no wal yet, ignore
    }
    Ok(())
}
