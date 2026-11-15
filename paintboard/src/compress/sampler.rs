use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use anyhow::{Context, Result};
use bytes::Bytes;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::config::AppConfig;
use crate::types::TickIndex;

const CHANNEL_BOUND: usize = 64;

#[derive(Clone)]
pub struct ZstdSampler {
    sender: mpsc::Sender<Command>,
    state: Arc<SamplerState>,
    tick_interval: u64,
    dict_path: PathBuf,
    dict_size: usize,
    worker: Arc<Mutex<Option<JoinHandle<Result<()>>>>>,
}

struct SamplerState {
    sample_dir: PathBuf,
    max_samples: usize,
    stored: AtomicUsize,
    seq: AtomicU64,
}

#[derive(Debug)]
struct Sample {
    tick: TickIndex,
    bytes: Bytes,
}

#[derive(Debug)]
enum Command {
    Sample(Sample),
    Shutdown,
}

impl ZstdSampler {
    pub async fn maybe_from_config(cfg: &AppConfig) -> Result<Option<Self>> {
        if !cfg.zstd.zstd_sampling_enabled {
            return Ok(None);
        }

        let dict_path = match &cfg.zstd.zstd_dictionary_path {
            Some(path) => path.clone(),
            None => {
                warn!(
                    "zstd sampling is enabled but zstd_dictionary_path is not set; sampling disabled"
                );
                return Ok(None);
            }
        };

        let sample_dir = cfg
            .zstd
            .zstd_sampling_dir
            .clone()
            .and_then(|p| {
                if p.as_os_str().is_empty() {
                    None
                } else {
                    Some(p)
                }
            })
            .unwrap_or_else(|| cfg.data_dir.join("zstd-samples"));

        fs::create_dir_all(&sample_dir).await.with_context(|| {
            format!(
                "failed to create sampler directory {}",
                sample_dir.display()
            )
        })?;

        let (sender, receiver) = mpsc::channel(CHANNEL_BOUND);
        let state = Arc::new(SamplerState {
            sample_dir,
            max_samples: cfg.zstd.zstd_sampling_max_samples.max(1),
            stored: AtomicUsize::new(0),
            seq: AtomicU64::new(0),
        });

        let state_clone = Arc::clone(&state);
        let worker = tokio::spawn(async move { run_worker(receiver, state_clone).await });

        let sampler = Self {
            sender,
            state,
            tick_interval: cfg.zstd.zstd_sampling_tick_interval.max(1),
            dict_path: dict_path.clone(),
            dict_size: cfg.zstd.zstd_dictionary_max_bytes.max(1),
            worker: Arc::new(Mutex::new(Some(worker))),
        };

        info!(
            "zstd sampling enabled: dir={}, dict_path={}, tick_interval={}, max_samples={}",
            sampler.state.sample_dir.display(),
            sampler.dict_path.display(),
            sampler.tick_interval,
            sampler.state.max_samples
        );

        Ok(Some(sampler))
    }

    pub fn record(&self, tick: TickIndex, bytes: Bytes) {
        if bytes.is_empty() {
            return;
        }

        if tick.0 % self.tick_interval != 0 {
            return;
        }

        if self.state.stored.load(Ordering::Relaxed) >= self.state.max_samples {
            return;
        }

        match self
            .sender
            .try_send(Command::Sample(Sample { tick, bytes }))
        {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                debug!(
                    "zstd sampler channel is full; dropping sample for tick {}",
                    tick.0
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                debug!("zstd sampler channel closed; skipping sample");
            }
        }
    }

    pub async fn finalize(&self) -> Result<Option<PathBuf>> {
        if let Err(err) = self.sender.send(Command::Shutdown).await {
            debug!(
                "zstd sampler shutdown signal failed to send (already closed?): {}",
                err
            );
        }

        if let Some(handle) = self.worker.lock().await.take() {
            if let Err(join_err) = handle.await {
                return Err(anyhow::anyhow!(
                    "zstd sampler worker panicked: {}",
                    join_err
                ));
            }
        }

        let total = self.state.stored.load(Ordering::Relaxed);
        if total == 0 {
            info!("zstd sampler collected 0 samples; skipping dictionary generation");
            return Ok(None);
        }

        let sample_dir = self.state.sample_dir.clone();
        let dict_path = self.dict_path.clone();
        let dict_size = self.dict_size;

        let dict_path_clone = dict_path.clone();
        tokio::task::spawn_blocking(move || train_dictionary(&sample_dir, &dict_path, dict_size))
            .await
            .map_err(|join_err| {
                anyhow::anyhow!("dictionary training task panicked: {}", join_err)
            })??;

        info!(
            "zstd dictionary generated at {} ({} samples, {} bytes target)",
            dict_path_clone.display(),
            total,
            dict_size
        );

        Ok(Some(dict_path_clone))
    }
}

async fn run_worker(mut rx: mpsc::Receiver<Command>, state: Arc<SamplerState>) -> Result<()> {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            Command::Shutdown => break,
            Command::Sample(sample) => {
                if state.stored.load(Ordering::Relaxed) >= state.max_samples {
                    continue;
                }

                let ordinal = state.seq.fetch_add(1, Ordering::Relaxed);
                let file_name = format!("{:020}_{}.bin", sample.tick.0, ordinal);
                let target = state.sample_dir.join(file_name);
                let tmp = append_extension(&target, "tmp");

                if let Err(err) = write_sample(&tmp, &target, &sample.bytes).await {
                    warn!(
                        "failed to persist zstd sample for tick {}: {}",
                        sample.tick.0, err
                    );
                    continue;
                }

                state.stored.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    Ok(())
}

async fn write_sample(tmp: &Path, final_path: &Path, data: &[u8]) -> Result<()> {
    let mut file = fs::File::create(tmp)
        .await
        .with_context(|| format!("failed to create {}", tmp.display()))?;
    file.write_all(data)
        .await
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("failed to sync {}", tmp.display()))?;
    drop(file);
    fs::rename(tmp, final_path).await.with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

fn append_extension(path: &Path, ext: &str) -> PathBuf {
    let mut tmp = path.to_path_buf();
    tmp.set_extension(format!(
        "{}{}",
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| format!("{}.", s))
            .unwrap_or_else(|| String::new()),
        ext
    ));
    tmp
}

fn train_dictionary(sample_dir: &Path, dict_path: &Path, dict_size: usize) -> Result<()> {
    use std::fs;

    let mut entries: Vec<_> = fs::read_dir(sample_dir)
        .with_context(|| format!("failed to read {}", sample_dir.display()))?
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                let path = e.path();
                if path.extension().and_then(|s| s.to_str()) == Some("bin") {
                    Some(path)
                } else {
                    None
                }
            })
        })
        .collect();

    entries.sort();

    if entries.is_empty() {
        warn!(
            "zstd sampler has no .bin samples under {}; skipping dictionary generation",
            sample_dir.display()
        );
        return Ok(());
    }

    let mut samples = Vec::with_capacity(entries.len());
    let mut total_bytes = 0usize;
    for path in &entries {
        let data =
            fs::read(path).with_context(|| format!("failed to read sample {}", path.display()))?;
        if data.is_empty() {
            continue;
        }
        total_bytes += data.len();
        samples.push(data);
    }

    if samples.is_empty() {
        warn!(
            "zstd sampler samples are empty under {}; skipping dictionary",
            sample_dir.display()
        );
        return Ok(());
    }

    const MIN_DICT_SIZE: usize = 256;
    if total_bytes < MIN_DICT_SIZE {
        warn!(
            "zstd sampler collected only {} bytes (< {} required); skipping dictionary generation",
            total_bytes, MIN_DICT_SIZE
        );
        return Ok(());
    }

    let target_size = dict_size.min(total_bytes).max(MIN_DICT_SIZE);

    let sample_refs: Vec<_> = samples.iter().map(|s| s.as_slice()).collect();
    if target_size < dict_size {
        info!(
            "zstd sampler reducing dictionary size to {} bytes (total sample bytes: {})",
            target_size, total_bytes
        );
    }

    let dict = zstd::dict::from_samples(&sample_refs, target_size)
        .context("zstd dictionary training failed")?;

    if let Some(parent) = dict_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let tmp_path = dict_path.with_extension("tmp");
    fs::write(&tmp_path, &dict)
        .with_context(|| format!("failed to write temp dict {}", tmp_path.display()))?;
    fs::rename(&tmp_path, dict_path)
        .with_context(|| format!("failed to finalize dict {}", dict_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_extension_preserves_original() {
        let path = PathBuf::from("/tmp/sample.bin");
        let tmp = append_extension(&path, "tmp");
        assert_eq!(tmp, PathBuf::from("/tmp/sample.bin.tmp"));

        let path = PathBuf::from("/tmp/sample");
        let tmp = append_extension(&path, "tmp");
        assert_eq!(tmp, PathBuf::from("/tmp/sample.tmp"));
    }
}
