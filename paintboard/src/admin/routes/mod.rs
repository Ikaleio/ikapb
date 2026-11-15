pub mod bans;
pub mod fill;
pub mod tokens;
pub mod ws;

use axum::{
    Json,
    http::HeaderMap,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::config::AppConfig;

pub fn ensure_admin(headers: &HeaderMap, cfg: &AppConfig) -> Result<(), Response> {
    if cfg.admin_manage_key.is_empty() {
        return Err(admin_error(StatusCode::UNAUTHORIZED, 401, "ADMIN_DISABLED"));
    }
    match headers
        .get("X-Admin-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
    {
        Some(key) if !key.is_empty() && key == cfg.admin_manage_key => Ok(()),
        _ => Err(admin_error(StatusCode::UNAUTHORIZED, 401, "UNAUTHORIZED")),
    }
}

pub fn admin_error(status: StatusCode, code: u16, error: impl Into<String>) -> Response {
    (
        status,
        Json(ApiResponse {
            code,
            data: ApiError {
                error: error.into(),
            },
        }),
    )
        .into_response()
}

pub fn admin_ok<T: Serialize>(data: T) -> Response {
    Json(ApiResponse { code: 200, data }).into_response()
}

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub code: u16,
    pub data: T,
}

#[derive(Serialize)]
pub struct ApiError {
    pub error: String,
}
