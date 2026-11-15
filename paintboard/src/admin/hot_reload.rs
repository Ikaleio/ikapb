use std::path::PathBuf;
use std::sync::{Arc, Weak};

use anyhow::Result;
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use tracing::{error, info, warn};

use crate::config::{self, AppConfig};
use crate::tick::coordinator::TickCoordinator;

static HOT_RELOAD: OnceCell<Arc<HotReloadState>> = OnceCell::new();

#[derive(Clone)]
pub struct ConfigHandle {
    inner: Arc<RwLock<AppConfig>>,
}

impl ConfigHandle {
    pub fn current(&self) -> AppConfig {
        self.with(|cfg| cfg.clone())
    }

    pub fn with<R>(&self, f: impl FnOnce(&AppConfig) -> R) -> R {
        let guard = self.inner.read();
        f(&*guard)
    }
}

struct HotReloadState {
    config_path: RwLock<PathBuf>,
    config: Arc<RwLock<AppConfig>>,
    tick: RwLock<Option<Weak<TickCoordinator>>>,
}

impl HotReloadState {
    fn new(path: PathBuf, cfg: AppConfig) -> Self {
        Self {
            config_path: RwLock::new(path),
            config: Arc::new(RwLock::new(cfg)),
            tick: RwLock::new(None),
        }
    }

    fn config_handle(&self) -> ConfigHandle {
        ConfigHandle {
            inner: self.config.clone(),
        }
    }

    fn set_path(&self, path: PathBuf) {
        *self.config_path.write() = path;
    }

    fn path(&self) -> PathBuf {
        self.config_path.read().clone()
    }

    fn replace_config(&self, cfg: AppConfig) {
        *self.config.write() = cfg;
    }

    fn register_tick(&self, tick: Arc<TickCoordinator>) {
        *self.tick.write() = Some(Arc::downgrade(&tick));
    }

    fn apply_new_config(&self, new_cfg: AppConfig) -> Result<()> {
        config::apply_hot(&new_cfg)?;

        let mut guard = self.config.write();
        let old_cfg = guard.clone();
        *guard = new_cfg.clone();
        drop(guard);

        self.update_tick(&old_cfg, &new_cfg);
        Ok(())
    }

    fn update_tick(&self, old_cfg: &AppConfig, new_cfg: &AppConfig) {
        if old_cfg.tps == new_cfg.tps {
            return;
        }

        let maybe_tick = self.tick.read().clone();
        match maybe_tick.and_then(|weak| weak.upgrade()) {
            Some(tick) => {
                tick.set_tps(new_cfg.tps);
                info!(
                    old_tps = old_cfg.tps,
                    new_tps = new_cfg.tps,
                    "tick TPS updated via hot reload"
                );
            }
            None => warn!("tick coordinator not registered; cannot update TPS via hot reload"),
        }
    }
}

pub fn init(config_path: impl Into<PathBuf>, initial: AppConfig) -> ConfigHandle {
    let path = config_path.into();
    if let Some(state) = HOT_RELOAD.get() {
        state.set_path(path);
        state.replace_config(initial);
        return state.config_handle();
    }

    let state = Arc::new(HotReloadState::new(path, initial));
    let handle = state.config_handle();
    if HOT_RELOAD.set(state).is_err() {
        warn!("hot reload state was initialized multiple times");
    }
    handle
}

pub fn register_tick_coordinator(coordinator: Arc<TickCoordinator>) {
    if let Some(state) = HOT_RELOAD.get() {
        state.register_tick(coordinator);
    } else {
        warn!("hot reload state not initialized; tick coordinator registration ignored");
    }
}

pub fn install_handlers() {
    let Some(state) = HOT_RELOAD.get().cloned() else {
        warn!("hot reload state not initialized; skipping signal handlers");
        return;
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        tokio::spawn(async move {
            let mut hup = match signal(SignalKind::hangup()) {
                Ok(signal) => signal,
                Err(err) => {
                    error!(?err, "failed to install SIGHUP handler");
                    return;
                }
            };

            while hup.recv().await.is_some() {
                if let Err(err) = reload_once(&state) {
                    error!(?err, "failed to reload configuration");
                }
            }
        });
    }

    #[cfg(not(unix))]
    {
        let _ = state;
        info!("hot reload signals are not supported on this platform");
    }
}

fn reload_once(state: &HotReloadState) -> Result<()> {
    let path = state.path();
    let cfg = config::load_from(path.as_path())?;
    state.apply_new_config(cfg)?;
    info!(path = %path.display(), "configuration hot reloaded");
    Ok(())
}

pub fn apply_config(cfg: &AppConfig) {
    if let Some(state) = HOT_RELOAD.get() {
        if let Err(err) = state.apply_new_config(cfg.clone()) {
            error!(?err, "failed to apply hot configuration");
        }
    } else {
        warn!("hot reload state not initialized; apply_config ignored");
    }
}

pub fn config_handle() -> Option<ConfigHandle> {
    HOT_RELOAD.get().map(|state| state.config_handle())
}
