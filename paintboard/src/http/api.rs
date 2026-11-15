use axum::{http::StatusCode, response::IntoResponse};

pub async fn handle_root() -> impl IntoResponse {
    let hash = option_env!("GIT_HASH").unwrap_or("UNKNOWN");
    let body = format!("ikapb BUILD {} (distantworld)\nMade by Ikaleio.", hash);
    (StatusCode::OK, body)
}
