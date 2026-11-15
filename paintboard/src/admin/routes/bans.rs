use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;

use crate::admin::ban::BanRecord;
use crate::admin::routes::{admin_error, admin_ok, ensure_admin};
use crate::http::HttpState;
use crate::token::cooldown::current_time_ms;

#[derive(Deserialize)]
pub struct BanQuery {
    pub uid: Option<i64>,
    #[serde(default)]
    pub active_only: Option<bool>,
}

#[derive(Deserialize)]
pub struct BanRequest {
    pub uid: i64,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub expire_at_ms: Option<i64>,
}

#[derive(Serialize)]
pub struct BanListResponse {
    pub items: Vec<BanRecord>,
}

#[derive(Serialize)]
pub struct BanDeleteResponse {
    pub removed: Option<BanRecord>,
}

pub async fn create_ban(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(body): Json<BanRequest>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    let now_ms = current_time_ms();
    if let Some(expire) = body.expire_at_ms {
        if expire <= now_ms {
            return admin_error(StatusCode::BAD_REQUEST, 400, "EXPIRE_IN_PAST");
        }
    }

    let record = BanRecord {
        uid: body.uid,
        reason: body.reason,
        created_at_ms: now_ms,
        expire_at_ms: body.expire_at_ms,
    };

    let persist_record = record.clone();
    let path = cfg.sqlite_path.clone();
    let persist = spawn_blocking(move || -> anyhow::Result<()> {
        let conn = rusqlite::Connection::open(&path)?;
        crate::admin::store::upsert_ban(&conn, &persist_record)?;
        Ok(())
    })
    .await;

    match persist {
        Ok(Ok(())) => {
            state.ban_store.upsert(record.clone());
            admin_ok(record)
        }
        _ => admin_error(StatusCode::INTERNAL_SERVER_ERROR, 500, "PERSIST_FAILED"),
    }
}

pub async fn list_bans(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<BanQuery>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    let now_ms = current_time_ms();
    let mut bans = state.ban_store.all();
    if let Some(uid) = query.uid {
        bans.retain(|b| b.uid == uid);
    }
    if query.active_only.unwrap_or(false) {
        bans.retain(|b| b.is_active(now_ms));
    }
    admin_ok(BanListResponse { items: bans })
}

pub async fn delete_ban(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(uid): Path<i64>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    let path = cfg.sqlite_path.clone();
    let persist = spawn_blocking(move || -> anyhow::Result<()> {
        let conn = rusqlite::Connection::open(&path)?;
        crate::admin::store::delete_ban(&conn, uid)?;
        Ok(())
    })
    .await;

    let removed = state.ban_store.remove(uid);

    match persist {
        Ok(Ok(())) => admin_ok(BanDeleteResponse { removed }),
        _ => admin_error(StatusCode::INTERNAL_SERVER_ERROR, 500, "PERSIST_FAILED"),
    }
}
