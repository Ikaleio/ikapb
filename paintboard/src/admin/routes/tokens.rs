use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;
use uuid::Uuid;

use crate::admin::routes::{admin_error, admin_ok, ensure_admin};
use crate::http::HttpState;
use crate::token::cooldown::current_time_ms;
use crate::token::manager::TokenManagerError;
use crate::token::store::TokenRecord;

#[derive(Deserialize)]
pub struct TokenQuery {
    pub uid: Option<i64>,
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub page_size: Option<usize>,
}

#[derive(Deserialize)]
pub struct TokenCreateRequest {
    pub uid: i64,
    pub cooldown_ms: i64,
    pub issued_at_ms: Option<i64>,
    #[serde(default)]
    pub ip: Option<Option<String>>,
    #[serde(default)]
    pub last_paint_at_ms: Option<Option<i64>>,
}

#[derive(Deserialize)]
pub struct TokenPatchRequest {
    pub cooldown_ms: Option<i64>,
    pub issued_at_ms: Option<i64>,
    #[serde(default)]
    pub ip: Option<Option<String>>,
    #[serde(default)]
    pub last_paint_at_ms: Option<Option<i64>>,
}

#[derive(Deserialize)]
pub struct TokenPutRequest {
    pub cooldown_ms: i64,
    pub issued_at_ms: Option<i64>,
    #[serde(default)]
    pub ip: Option<Option<String>>,
    #[serde(default)]
    pub last_paint_at_ms: Option<Option<i64>>,
}

#[derive(Serialize)]
pub struct TokenListResponse {
    pub items: Vec<TokenDto>,
    pub total: usize,
}

#[derive(Serialize)]
pub struct TokenDto {
    pub token: String,
    pub uid: i64,
    pub cooldown_ms: i64,
    pub issued_at_ms: i64,
    pub ip: Option<String>,
    pub last_paint_at_ms: Option<i64>,
}

pub async fn list_tokens(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    let path = cfg.sqlite_path.clone();
    let load = spawn_blocking(move || -> anyhow::Result<Vec<TokenRecord>> {
        let conn = rusqlite::Connection::open(&path)?;
        crate::token::store::load_all_tokens(&conn)
    })
    .await;

    let mut records = match load {
        Ok(Ok(records)) => records,
        _ => return admin_error(StatusCode::INTERNAL_SERVER_ERROR, 500, "LOAD_FAILED"),
    };

    records.retain(|r| query.uid.map(|u| u == r.uid).unwrap_or(true));

    let page_size = query.page_size.unwrap_or(50).max(1);
    let page = query.page.unwrap_or(0);
    let start = page.saturating_mul(page_size);
    let total = records.len();
    if start < records.len() {
        let end = (start + page_size).min(records.len());
        records = records[start..end].to_vec();
    } else {
        records.clear();
    }

    let items = records.into_iter().map(TokenDto::from).collect();
    admin_ok(TokenListResponse { items, total })
}

pub async fn get_token(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(uid): Path<i64>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    match fetch_token_by_uid(&cfg.sqlite_path, uid).await {
        Ok(Some(record)) => admin_ok(TokenDto::from(record)),
        Ok(None) => admin_error(StatusCode::NOT_FOUND, 404, "NOT_FOUND"),
        Err(_) => admin_error(StatusCode::INTERNAL_SERVER_ERROR, 500, "LOAD_FAILED"),
    }
}

pub async fn create_token(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(body): Json<TokenCreateRequest>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    let now_ms = current_time_ms();
    if let Some(resp) = validate_fields(body.cooldown_ms) {
        return resp;
    }
    if state.ban_store.active_record(body.uid, now_ms).is_some() {
        return admin_error(StatusCode::FORBIDDEN, 403, "BANNED_UID");
    }

    let token_uuid = Uuid::new_v4();
    let existing_token = state
        .token_manager
        .token_for_uid(body.uid)
        .map(|uuid| uuid.to_string());

    let record = TokenRecord {
        uid: body.uid,
        token: token_uuid.to_string(),
        issued_at_ms: body.issued_at_ms.unwrap_or(now_ms),
        cooldown_ms: body.cooldown_ms,
        ip: body.ip.unwrap_or(None),
        last_paint_at_ms: body.last_paint_at_ms.unwrap_or(None),
    };

    if let Err(resp) = persist_token(&cfg.sqlite_path, &record).await {
        return resp;
    }

    if let Some(last_ms) = record.last_paint_at_ms {
        state.cooldown.load_state(record.token.clone(), last_ms);
    }

    match state.token_manager.upsert_record(record.clone()) {
        Ok(previous) => {
            if let Some(prev) = previous {
                if prev.token != record.token {
                    state.cooldown.evict(&prev.token);
                }
            }
            if let Some(prev_uid_token) = existing_token {
                if prev_uid_token != record.token {
                    state.cooldown.evict(&prev_uid_token);
                }
            }
            if record.last_paint_at_ms.is_none() {
                state.cooldown.evict(&record.token);
            }
            admin_ok(TokenDto::from(record))
        }
        Err(TokenManagerError::InvalidUuid) => {
            admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_TOKEN")
        }
    }
}

pub async fn put_token(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(uid): Path<i64>,
    Json(body): Json<TokenPutRequest>,
) -> impl IntoResponse {
    handle_update(state, headers, uid, body.into()).await
}

pub async fn patch_token(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(uid): Path<i64>,
    Json(body): Json<TokenPatchRequest>,
) -> impl IntoResponse {
    handle_update(state, headers, uid, body).await
}

pub async fn delete_token(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(uid): Path<i64>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    match fetch_token_by_uid(&cfg.sqlite_path, uid).await {
        Ok(Some(record)) => {
            let path = cfg.sqlite_path.clone();
            let token_for_delete = record.token.clone();
            let persist = spawn_blocking(move || -> anyhow::Result<()> {
                let conn = rusqlite::Connection::open(&path)?;
                crate::token::store::delete_token(&conn, &token_for_delete)?;
                Ok(())
            })
            .await;

            state
                .token_manager
                .remove(&Uuid::parse_str(&record.token).unwrap());
            state.cooldown.evict(&record.token);

            match persist {
                Ok(Ok(())) => admin_ok(Some(TokenDto::from(record))),
                _ => admin_error(StatusCode::INTERNAL_SERVER_ERROR, 500, "PERSIST_FAILED"),
            }
        }
        Ok(None) => admin_ok::<Option<TokenDto>>(None),
        Err(_) => admin_error(StatusCode::INTERNAL_SERVER_ERROR, 500, "LOAD_FAILED"),
    }
}

async fn handle_update(
    state: HttpState,
    headers: HeaderMap,
    uid: i64,
    patch: TokenPatchRequest,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    let Some(existing) = (match fetch_token_by_uid(&cfg.sqlite_path, uid).await {
        Ok(record) => record,
        Err(_) => return admin_error(StatusCode::INTERNAL_SERVER_ERROR, 500, "LOAD_FAILED"),
    }) else {
        return admin_error(StatusCode::NOT_FOUND, 404, "NOT_FOUND");
    };

    let now_ms = current_time_ms();
    let mut updated = existing.clone();
    if let Some(cooldown) = patch.cooldown_ms {
        updated.cooldown_ms = cooldown;
    }
    if let Some(issued) = patch.issued_at_ms {
        updated.issued_at_ms = issued;
    }
    if let Some(ip) = patch.ip {
        updated.ip = ip;
    }
    let reset_last_paint = apply_last_paint(&mut updated, patch.last_paint_at_ms.as_ref());

    if let Some(resp) = validate_fields(updated.cooldown_ms) {
        return resp;
    }

    let token_uuid = match Uuid::parse_str(&updated.token) {
        Ok(uuid) => uuid,
        Err(_) => return admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_TOKEN"),
    };

    if state.ban_store.active_record(updated.uid, now_ms).is_some() {
        return admin_error(StatusCode::FORBIDDEN, 403, "BANNED_UID");
    }

    let uid_existing_token = state
        .token_manager
        .token_for_uid(updated.uid)
        .filter(|uuid| *uuid != token_uuid)
        .map(|uuid| uuid.to_string());

    if let Err(resp) = persist_token(&cfg.sqlite_path, &updated).await {
        return resp;
    }

    if let Some(last_ms) = updated.last_paint_at_ms {
        state.cooldown.load_state(updated.token.clone(), last_ms);
    } else if reset_last_paint {
        state.cooldown.evict(&updated.token);
    }

    match state.token_manager.upsert_record(updated.clone()) {
        Ok(previous) => {
            if let Some(prev) = previous {
                if prev.token != updated.token {
                    state.cooldown.evict(&prev.token);
                }
            }
            if let Some(token) = uid_existing_token {
                if token != updated.token {
                    state.cooldown.evict(&token);
                }
            }
            admin_ok(TokenDto::from(updated))
        }
        Err(TokenManagerError::InvalidUuid) => {
            admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_TOKEN")
        }
    }
}

async fn persist_token(path: &std::path::Path, record: &TokenRecord) -> Result<(), Response> {
    let path = path.to_path_buf();
    let record = record.clone();
    let persist = spawn_blocking(move || -> anyhow::Result<()> {
        let conn = rusqlite::Connection::open(&path)?;
        crate::token::store::upsert_token(&conn, &record)?;
        Ok(())
    })
    .await;

    match persist {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(admin_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            500,
            "PERSIST_FAILED",
        )),
        Err(_) => Err(admin_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            500,
            "PERSIST_FAILED",
        )),
    }
}

async fn fetch_token_by_uid(
    path: &std::path::Path,
    uid: i64,
) -> Result<Option<TokenRecord>, Response> {
    let path = path.to_path_buf();
    let fetch = spawn_blocking(move || -> anyhow::Result<Option<TokenRecord>> {
        let conn = rusqlite::Connection::open(&path)?;
        crate::token::store::get_token_by_uid(&conn, uid)
    })
    .await;

    match fetch {
        Ok(Ok(record)) => Ok(record),
        _ => Err(admin_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            500,
            "LOAD_FAILED",
        )),
    }
}

impl From<TokenRecord> for TokenDto {
    fn from(value: TokenRecord) -> Self {
        Self {
            token: value.token,
            uid: value.uid,
            cooldown_ms: value.cooldown_ms,
            issued_at_ms: value.issued_at_ms,
            ip: value.ip,
            last_paint_at_ms: value.last_paint_at_ms,
        }
    }
}

impl From<TokenPutRequest> for TokenPatchRequest {
    fn from(value: TokenPutRequest) -> Self {
        Self {
            cooldown_ms: Some(value.cooldown_ms),
            issued_at_ms: value.issued_at_ms,
            ip: Some(value.ip.unwrap_or(None)),
            last_paint_at_ms: Some(value.last_paint_at_ms.unwrap_or(None)),
        }
    }
}

fn validate_fields(cooldown_ms: i64) -> Option<Response> {
    if cooldown_ms < -1 {
        return Some(admin_error(
            StatusCode::BAD_REQUEST,
            400,
            "INVALID_COOLDOWN",
        ));
    }
    None
}

fn apply_last_paint(record: &mut TokenRecord, patch: Option<&Option<i64>>) -> bool {
    match patch {
        Some(Some(value)) => {
            record.last_paint_at_ms = Some(*value);
            true
        }
        Some(None) => {
            record.last_paint_at_ms = None;
            true
        }
        None => false,
    }
}
