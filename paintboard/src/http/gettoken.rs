use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use reqwest::header::USER_AGENT;
use serde::{Deserialize, Serialize};
use tracing::error;
use std::net::{IpAddr, SocketAddr};

use crate::http::HttpState;
use crate::util::ip::resolve_client_ip;

#[derive(Deserialize)]
pub struct GetTokenRequest {
    pub uid: i64,
    #[serde(default)]
    pub access_key: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub token: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub code: u16,
    pub data: T,
}

#[derive(Deserialize)]
struct VerifyResponse {
    success: bool,
    message: Option<String>,
    uid: Option<i64>,
}

pub async fn handle_gettoken(
    State(state): State<HttpState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<GetTokenRequest>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if !cfg.no_access_key_verify {
        if let Err(code) = validate_access_key(payload.uid, &payload.access_key).await {
            return json_response(StatusCode::FORBIDDEN, 403, ErrorResponse { error: code });
        }
    }

    if state
        .ban_store
        .active_record(payload.uid, crate::token::cooldown::current_time_ms())
        .is_some()
    {
        return json_response(
            StatusCode::FORBIDDEN,
            403,
            ErrorResponse {
                error: "BANNED_UID".to_string(),
            },
        );
    }

    // Use -1 to indicate “follow global cooldown”.
    let trusted_proxy_ips: Vec<IpAddr> = cfg
        .network
        .trusted_proxies
        .iter()
        .filter_map(|proxy| proxy.parse::<IpAddr>().ok())
        .collect();
    let remote_ip = peer_addr.ip();
    let client_ip = resolve_client_ip(&headers, remote_ip, &trusted_proxy_ips);

    let (record, old_token) = state
        .token_manager
        .issue_token(payload.uid, -1, Some(client_ip.to_string()));
    let token_str = record.token.clone();

    if let Some(old) = &old_token {
        state.cooldown.evict(old);
    }

    match tokio::task::spawn_blocking({
        let path = cfg.sqlite_path.clone();
        let record = record.clone();
        move || -> anyhow::Result<()> {
            let conn = rusqlite::Connection::open(&path)?;
            // Delete old token from database if exists
            if let Some(old) = old_token {
                crate::token::store::delete_token(&conn, &old)?;
            }
            crate::token::store::upsert_token(&conn, &record)?;
            Ok(())
        }
    })
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(err)) => error!("failed to persist token: {}", err),
        Err(err) => error!("failed to join token persistence task: {}", err),
    }

    json_response(StatusCode::OK, 200, TokenResponse { token: token_str })
}

async fn validate_access_key(uid: i64, access_key: &str) -> Result<(), String> {
    let access_key = access_key.trim();
    if access_key.is_empty() {
        return Err("MISSING_ACCESS_KEY".to_string());
    }

    let url = format!("https://www.luogu.me/paintboard/verify/{}", access_key);

    let resp = reqwest::Client::new()
        .get(&url)
        .header(USER_AGENT, "Uptime-Kuma")
        .send()
        .await
        .map_err(|err| {
            error!("failed to fetch access key verification: {}", err);
            "UNKNOWN_ERROR".to_string()
        })?;

    let status = resp.status();
    let verify = resp.json::<VerifyResponse>().await.map_err(|err| {
        error!("failed to parse verification response: {}", err);
        "UNKNOWN_ERROR".to_string()
    })?;

    if !status.is_success() {
        return Err("UNKNOWN_ERROR".to_string());
    }

    if !verify.success {
        let msg = verify.message.unwrap_or_default();
        return if msg == "凭据无效" {
            Err("INVALID_ACCESS_KEY".to_string())
        } else {
            Err("UNKNOWN_ERROR".to_string())
        };
    }

    if let Some(returned_uid) = verify.uid {
        if returned_uid != uid {
            return Err("UID_MISMATCH".to_string());
        }
    }

    Ok(())
}

fn json_response<T: Serialize>(status: StatusCode, code: u16, data: T) -> Response {
    (status, Json(ApiResponse { code, data })).into_response()
}
