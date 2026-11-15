use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub bind_address: String,
    pub tps: u32,
    pub no_access_key_verify: bool,
    pub background_color: String,
    pub data_dir: PathBuf,
    pub sqlite_path: PathBuf,
    #[serde(default = "default_admin_manage_key")]
    pub admin_manage_key: String,
    #[serde(default = "default_admin_max_blob_bytes")]
    pub admin_max_blob_bytes: u64,

    #[serde(default)]
    pub network: NetworkConfig,

    #[serde(default)]
    pub websocket: WebSocketConfig,

    #[serde(default)]
    pub persistence: PersistenceConfig,

    #[serde(default)]
    pub token: TokenConfig,

    #[serde(default)]
    pub zstd: ZstdConfig,

    #[serde(default)]
    pub event: EventConfig,

    // Backward compatibility: legacy flat fields (deprecated).
    #[serde(default, skip_serializing)]
    pub trusted_proxies: Option<Vec<String>>,
    #[serde(default, skip_serializing)]
    pub max_connections_per_ip: Option<usize>,
    #[serde(default, skip_serializing)]
    pub ws_send_period_ms: Option<u64>,
    #[serde(default, skip_serializing)]
    pub max_ack_buffer_bytes: Option<usize>,
    #[serde(default, skip_serializing)]
    pub max_client_latency_ms: Option<u64>,
    #[serde(default, skip_serializing)]
    pub persist_board: Option<bool>,
    #[serde(default, skip_serializing)]
    pub snapshot_interval_sec: Option<u64>,
    #[serde(default, skip_serializing)]
    pub snapshot_retain: Option<usize>,
    #[serde(default, skip_serializing)]
    pub wal_flush_every_ticks: Option<u32>,
    #[serde(default, skip_serializing)]
    pub global_cooldown_ms: Option<i64>,
    #[serde(default, skip_serializing)]
    pub cooldown_flush_interval_ms: Option<u64>,
    #[serde(default, skip_serializing)]
    pub zstd_level: Option<i32>,
    #[serde(default, skip_serializing)]
    pub zstd_dictionary_path: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    pub zstd_dictionary_max_bytes: Option<usize>,
    #[serde(default, skip_serializing)]
    pub zstd_sampling_enabled: Option<bool>,
    #[serde(default, skip_serializing)]
    pub zstd_sampling_dir: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    pub zstd_sampling_tick_interval: Option<u64>,
    #[serde(default, skip_serializing)]
    pub zstd_sampling_max_samples: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    #[serde(default)]
    pub max_connections_per_ip: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    pub ws_send_period_ms: u64,
    pub max_ack_buffer_bytes: usize,
    pub max_client_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceConfig {
    pub persist_board: bool,
    pub snapshot_interval_sec: u64,
    pub snapshot_retain: usize,
    pub wal_flush_every_ticks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenConfig {
    pub global_cooldown_ms: i64,
    pub cooldown_flush_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZstdConfig {
    pub zstd_level: i32,
    pub zstd_dictionary_path: Option<PathBuf>,
    pub zstd_dictionary_max_bytes: usize,
    #[serde(default)]
    pub zstd_sampling_enabled: bool,
    #[serde(default)]
    pub zstd_sampling_dir: Option<PathBuf>,
    #[serde(default = "default_zstd_sampling_tick_interval")]
    pub zstd_sampling_tick_interval: u64,
    #[serde(default = "default_zstd_sampling_max_samples")]
    pub zstd_sampling_max_samples: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventConfig {
    #[serde(default)]
    pub start_timestamp: u64,
    #[serde(default)]
    pub end_timestamp: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            trusted_proxies: Vec::new(),
            max_connections_per_ip: None,
        }
    }
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            ws_send_period_ms: 20,
            max_ack_buffer_bytes: 209_715_200,
            max_client_latency_ms: 3000,
        }
    }
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            persist_board: false,
            snapshot_interval_sec: 10,
            snapshot_retain: 8,
            wal_flush_every_ticks: 8,
        }
    }
}

impl Default for TokenConfig {
    fn default() -> Self {
        Self {
            global_cooldown_ms: 0,
            cooldown_flush_interval_ms: 0,
        }
    }
}

impl Default for ZstdConfig {
    fn default() -> Self {
        Self {
            zstd_level: 5,
            zstd_dictionary_path: Some(PathBuf::from("data/zstd-dict.bin")),
            zstd_dictionary_max_bytes: default_zstd_dictionary_max_bytes(),
            zstd_sampling_enabled: false,
            zstd_sampling_dir: Some(PathBuf::from("data/zstd-samples")),
            zstd_sampling_tick_interval: default_zstd_sampling_tick_interval(),
            zstd_sampling_max_samples: default_zstd_sampling_max_samples(),
        }
    }
}

pub fn load_from(path: &Path) -> Result<AppConfig> {
    let mut settings = config::Config::builder();
    settings = settings.add_source(config::File::from(path.to_path_buf()));
    let cfg = settings
        .build()
        .with_context(|| format!("failed to load config from {}", path.display()))?;
    let mut cfg: AppConfig = cfg.try_deserialize().context("invalid config structure")?;

    // Backward compatibility: migrate legacy flat fields into nested structs.
    migrate_legacy_fields(&mut cfg);

    Ok(cfg)
}

fn migrate_legacy_fields(cfg: &mut AppConfig) {
    // Migrate network config.
    if let Some(proxies) = cfg.trusted_proxies.take() {
        if cfg.network.trusted_proxies.is_empty() {
            cfg.network.trusted_proxies = proxies;
        }
    }
    if cfg.max_connections_per_ip.is_some() && cfg.network.max_connections_per_ip.is_none() {
        cfg.network.max_connections_per_ip = cfg.max_connections_per_ip.take();
    }

    // Migrate websocket config.
    if let Some(val) = cfg.ws_send_period_ms {
        cfg.websocket.ws_send_period_ms = val;
    }
    if let Some(val) = cfg.max_ack_buffer_bytes {
        cfg.websocket.max_ack_buffer_bytes = val;
    }
    if let Some(val) = cfg.max_client_latency_ms {
        cfg.websocket.max_client_latency_ms = val;
    }

    // Migrate persistence config.
    if let Some(val) = cfg.persist_board {
        cfg.persistence.persist_board = val;
    }
    if let Some(val) = cfg.snapshot_interval_sec {
        cfg.persistence.snapshot_interval_sec = val;
    }
    if let Some(val) = cfg.snapshot_retain {
        cfg.persistence.snapshot_retain = val;
    }
    if let Some(val) = cfg.wal_flush_every_ticks {
        cfg.persistence.wal_flush_every_ticks = val;
    }

    // Migrate token config.
    if let Some(val) = cfg.global_cooldown_ms {
        cfg.token.global_cooldown_ms = val;
    }
    if let Some(val) = cfg.cooldown_flush_interval_ms {
        cfg.token.cooldown_flush_interval_ms = val;
    }

    // Migrate zstd config.
    if let Some(val) = cfg.zstd_level {
        cfg.zstd.zstd_level = val;
    }
    if cfg.zstd_dictionary_path.is_some() {
        cfg.zstd.zstd_dictionary_path = cfg.zstd_dictionary_path.take();
    }
    if let Some(val) = cfg.zstd_dictionary_max_bytes {
        cfg.zstd.zstd_dictionary_max_bytes = val;
    }
    if let Some(val) = cfg.zstd_sampling_enabled {
        cfg.zstd.zstd_sampling_enabled = val;
    }
    if cfg.zstd_sampling_dir.is_some() {
        cfg.zstd.zstd_sampling_dir = cfg.zstd_sampling_dir.take();
    }
    if let Some(val) = cfg.zstd_sampling_tick_interval {
        cfg.zstd.zstd_sampling_tick_interval = val;
    }
    if let Some(val) = cfg.zstd_sampling_max_samples {
        cfg.zstd.zstd_sampling_max_samples = val;
    }
}

pub fn apply_hot(cfg: &AppConfig) -> Result<()> {
    if cfg.tps == 0 {
        anyhow::bail!("tps must be greater than zero");
    }
    if cfg.websocket.ws_send_period_ms == 0 {
        anyhow::bail!("ws_send_period_ms must be greater than zero");
    }
    if cfg.persistence.persist_board && cfg.persistence.snapshot_interval_sec == 0 {
        anyhow::bail!(
            "snapshot_interval_sec must be greater than zero when persistence is enabled"
        );
    }

    fs::create_dir_all(&cfg.data_dir)
        .with_context(|| format!("failed to ensure data directory {}", cfg.data_dir.display()))?;
    if let Some(parent) = cfg.sqlite_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to ensure sqlite directory {}", parent.display()))?;
    }
    if cfg.zstd.zstd_sampling_enabled {
        if let Some(dir) = &cfg.zstd.zstd_sampling_dir {
            fs::create_dir_all(dir).with_context(|| {
                format!("failed to ensure zstd sampling directory {}", dir.display())
            })?;
        } else {
            let mut default_dir = cfg.data_dir.clone();
            default_dir.push("zstd-samples");
            fs::create_dir_all(&default_dir).with_context(|| {
                format!(
                    "failed to ensure default zstd sampling directory {}",
                    default_dir.display()
                )
            })?;
        }
    }

    for proxy in &cfg.network.trusted_proxies {
        proxy
            .parse::<std::net::IpAddr>()
            .with_context(|| format!("invalid trusted proxy address: {}", proxy))?;
    }

    if cfg.admin_max_blob_bytes == 0 {
        anyhow::bail!("admin_max_blob_bytes must be greater than zero");
    }

    Ok(())
}

fn default_zstd_sampling_tick_interval() -> u64 {
    1
}

fn default_zstd_sampling_max_samples() -> usize {
    256
}

fn default_zstd_dictionary_max_bytes() -> usize {
    65_536
}

fn default_admin_manage_key() -> String {
    String::new()
}

fn default_admin_max_blob_bytes() -> u64 {
    5_242_880
}
