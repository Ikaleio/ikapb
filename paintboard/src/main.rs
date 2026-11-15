use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::info;

use paintboard::{
    admin,
    admin::ban::BanStore,
    board::{memory::Board, snapshot, wal},
    compress::sampler::ZstdSampler,
    config, http, logger, metrics, migration,
    tick::{coordinator::TickCoordinator, pipeline::PaintPipeline},
    token::{self, cooldown::CooldownCache, manager::TokenManager},
    types::{Rgba, TickIndex},
    ws::ip_limiter::IpConnectionLimiter,
};

#[tokio::main]
async fn main() -> Result<()> {
    let config_path = std::env::var("PAINTBOARD_CONFIG")
        .unwrap_or_else(|_| "paintboard/config/default.toml".into());
    let config_path_buf = std::path::PathBuf::from(&config_path);
    let cfg = config::load_from(config_path_buf.as_path())?;

    logger::init();
    config::apply_hot(&cfg)?;
    info!(?cfg, "configuration loaded");

    metrics::init();

    let config_handle = admin::hot_reload::init(config_path_buf, cfg.clone());
    admin::hot_reload::install_handlers();

    // SPEC (Token): initialize SQLite and load tokens.
    // Ensure data directory exists.
    tokio::fs::create_dir_all(&cfg.data_dir).await?;

    let sqlite_path = cfg.sqlite_path.clone();
    let schema_sql = include_str!("../schema/tokens.sql");

    // Execute SQLite operations in a blocking thread.
    let (tokens_records, migration_board_pixels, ban_records) =
        tokio::task::spawn_blocking(move || -> Result<_> {
            let conn = Connection::open(&sqlite_path)?;
            let migration = token::store::init_schema(&conn, schema_sql)?;

            let migration_result = migration::import_if_present(&conn)?;
            let board_pixels = migration_result.and_then(|res| res.board_pixels);

            if let Some(stats) = migration {
                info!(
                    migrated = stats.migrated,
                    expired_or_revoked = stats.expired_or_revoked,
                    invalid_tokens = stats.invalid_tokens,
                    stale_state_rows = stats.stale_state_rows,
                    deduplicated = stats.deduplicated,
                    "migrated legacy token schema"
                );
            }

            let tokens = token::store::load_all_tokens(&conn)?;
            token::store::clear_last_paint(&conn)?;
            let bans = admin::store::load_all_bans(&conn)?;

            info!(
                "loaded {} tokens and {} bans from SQLite",
                tokens.len(),
                bans.len()
            );
            Ok((tokens, board_pixels, bans))
        })
        .await??;

    let token_manager = Arc::new(TokenManager::new());
    token_manager.load_tokens(&tokens_records);

    let ban_store = Arc::new(BanStore::new());
    ban_store.load(ban_records);

    // SPEC (Token): initialize cooldown cache.
    let cooldown = Arc::new(CooldownCache::new());
    for record in &tokens_records {
        if let Some(last) = record.last_paint_at_ms {
            cooldown.load_state(record.token.clone(), last);
        }
    }

    // SPEC (Board): initialize the board and restore snapshot (if enabled).
    let background = parse_hex_color(&cfg.background_color)
        .with_context(|| format!("invalid background_color: {}", cfg.background_color))?;

    let board = Board::new(true, background);
    let data_dir = std::path::Path::new(&cfg.data_dir);
    let tick_coord = Arc::new(TickCoordinator::new(cfg.tps));
    admin::hot_reload::register_tick_coordinator(tick_coord.clone());

    if let Some(rgb24) = migration_board_pixels {
        info!("restoring board from migration database");
        board.restore_from_rgb24(&rgb24);
    } else if cfg.persistence.persist_board {
        let mut base_tick = TickIndex(0);
        match snapshot::load_latest_snapshot(data_dir).await? {
            Some((tick, rgb24)) => {
                info!("restoring board from snapshot at tick {}", tick.0);
                board.restore_from_rgb24(&rgb24);
                tick_coord.set_tick_index(tick);
                base_tick = tick;
            }
            None => {
                info!("persistence enabled but no snapshot found; starting fresh");
            }
        }

        if let Ok(wal_entries) = wal::replay_since(data_dir, base_tick).await {
            let mut latest = base_tick.0;
            for entry in wal_entries {
                board.store(entry.idx, entry.rgba);
                if entry.tick.0 > latest {
                    latest = entry.tick.0;
                }
            }
            if latest > base_tick.0 {
                tick_coord.set_tick_index(TickIndex(latest));
            }
        }
    } else {
        info!(
            "board persistence disabled; starting fresh with background {}",
            cfg.background_color
        );
    }

    board.publish_tick(tick_coord.tick_index().0);

    let wal_flush_every_ticks = if cfg.persistence.persist_board {
        cfg.persistence.wal_flush_every_ticks
    } else {
        0
    };

    let sampler = ZstdSampler::maybe_from_config(&cfg).await?;

    let pipeline = PaintPipeline::new(
        board.clone(),
        tick_coord.clone(),
        16,
        data_dir.to_path_buf(),
        wal_flush_every_ticks,
        sampler.clone(),
    );

    if cfg.persistence.persist_board {
        let board_snap = board.clone();
        let data_dir_snap = cfg.data_dir.clone();
        let interval_sec = cfg.persistence.snapshot_interval_sec;
        let retain = cfg.persistence.snapshot_retain;
        tokio::spawn(async move {
            if let Err(e) = snapshot::periodic_snapshot(
                board_snap,
                interval_sec,
                std::path::Path::new(&data_dir_snap),
                retain,
            )
            .await
            {
                tracing::error!("snapshot task failed: {}", e);
            }
        });
    }

    let tick_clone = tick_coord.clone();
    let pipeline_clone = pipeline.clone();
    tokio::spawn(async move {
        tick_clone
            .run(move |tick_idx| {
                pipeline_clone.handle_tick(tick_idx);
            })
            .await;
    });

    let ip_limiter = Arc::new(IpConnectionLimiter::new(cfg.network.max_connections_per_ip));

    let app = http::build_router(
        config_handle.clone(),
        board.clone(),
        token_manager.clone(),
        cooldown.clone(),
        ban_store.clone(),
        pipeline.clone(),
        ip_limiter.clone(),
    )
    .await?;

    let addr = cfg.bind_address.parse::<SocketAddr>()?;
    info!("starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let make_service = app.into_make_service_with_connect_info::<SocketAddr>();

    let server_result = axum::serve(listener, make_service)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            info!("shutdown signal received");

            if let Err(err) =
                persist_cooldown_snapshot(cooldown.clone(), config_handle.clone()).await
            {
                tracing::error!("failed to persist cooldown state: {}", err);
            } else {
                info!("cooldown state persisted");
            }
        })
        .await;

    if let Some(sampler) = sampler {
        sampler.finalize().await?;
    }

    server_result?;

    Ok(())
}

fn parse_hex_color(input: &str) -> Result<Rgba> {
    let trimmed = input.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if hex.len() != 6 {
        anyhow::bail!("expected 6-digit hex color, got '{}'", input);
    }
    let r = u8::from_str_radix(&hex[0..2], 16)?;
    let g = u8::from_str_radix(&hex[2..4], 16)?;
    let b = u8::from_str_radix(&hex[4..6], 16)?;
    Ok(Rgba(((r as u32) << 16) | ((g as u32) << 8) | b as u32))
}

async fn persist_cooldown_snapshot(
    cache: Arc<CooldownCache>,
    config: admin::hot_reload::ConfigHandle,
) -> Result<()> {
    let sqlite_path = config.with(|cfg| cfg.sqlite_path.clone());
    let snapshot = cache.snapshot();
    if snapshot.is_empty() {
        return Ok(());
    }

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = Connection::open(&sqlite_path)?;
        paintboard::token::store::persist_last_paint_batch(&conn, &snapshot)?;
        Ok(())
    })
    .await??;

    Ok(())
}
