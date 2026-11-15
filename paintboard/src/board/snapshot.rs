use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::time::interval;

use crate::types::TickIndex;

use super::{memory::BoardHandle, wal};

pub async fn periodic_snapshot(
    board: BoardHandle,
    interval_sec: u64,
    data_dir: &Path,
    retain: usize,
) -> Result<()> {
    let mut ticker = interval(Duration::from_secs(interval_sec.max(1)));
    let tick_rx = board.subscribe_tick();
    loop {
        ticker.tick().await;
        let tick = TickIndex(*tick_rx.borrow());
        write_snapshot(board.clone(), data_dir, tick, retain).await?;
    }
}

pub async fn write_snapshot(
    board: BoardHandle,
    data_dir: &Path,
    tick: TickIndex,
    retain: usize,
) -> Result<()> {
    let filename = data_dir.join(format!("snapshot-{}.bin", tick.0));
    let tmp = data_dir.join(format!("snapshot-{}.tmp", tick.0));
    let bytes = board.snapshot_rgb24();
    fs::create_dir_all(data_dir).await?;
    let mut file = File::create(&tmp).await?;
    file.write_all(&bytes).await?;
    file.flush().await?;
    file.sync_all().await?;
    fs::rename(tmp, filename).await?;
    if let Err(err) = wal::rotate(data_dir).await {
        tracing::warn!("failed to rotate wal: {}", err);
    }
    prune_old_snapshots(data_dir, retain).await?;
    Ok(())
}

pub async fn load_latest_snapshot(data_dir: &Path) -> Result<Option<(TickIndex, Vec<u8>)>> {
    // SPEC 5.1: Scan data_dir/snapshot-*.bin and load the latest snapshot.
    let mut entries = tokio::fs::read_dir(data_dir).await?;
    let mut latest_tick = 0u64;
    let mut latest_path: Option<std::path::PathBuf> = None;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            if let Some(tick_str) = filename
                .strip_prefix("snapshot-")
                .and_then(|s| s.strip_suffix(".bin"))
            {
                if let Ok(tick) = tick_str.parse::<u64>() {
                    if tick > latest_tick {
                        latest_tick = tick;
                        latest_path = Some(path);
                    }
                }
            }
        }
    }

    if let Some(path) = latest_path {
        let data = tokio::fs::read(&path).await?;
        tracing::info!(
            "loaded snapshot from {} (tick {})",
            path.display(),
            latest_tick
        );
        Ok(Some((TickIndex(latest_tick), data)))
    } else {
        Ok(None)
    }
}

async fn prune_old_snapshots(data_dir: &Path, retain: usize) -> Result<()> {
    if retain == 0 {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(data_dir).await?;
    let mut snapshots: Vec<(u64, std::path::PathBuf)> = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|f| f.to_str()) {
            if let Some(tick_str) = name
                .strip_prefix("snapshot-")
                .and_then(|s| s.strip_suffix(".bin"))
            {
                if let Ok(tick) = tick_str.parse::<u64>() {
                    snapshots.push((tick, path));
                }
            }
        }
    }

    if snapshots.len() <= retain {
        return Ok(());
    }

    snapshots.sort_by_key(|(tick, _)| *tick);
    let to_remove = snapshots.len().saturating_sub(retain);
    for (_, path) in snapshots.into_iter().take(to_remove) {
        if let Err(err) = tokio::fs::remove_file(&path).await {
            tracing::warn!("failed to remove old snapshot {}: {}", path.display(), err);
        }
    }
    Ok(())
}
