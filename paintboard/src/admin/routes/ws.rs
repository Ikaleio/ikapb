use axum::{extract::State, http::HeaderMap, response::Response};
use serde::Serialize;

use crate::{
    admin::routes::{admin_ok, ensure_admin},
    http::HttpState,
};

#[derive(Serialize)]
pub struct WsConnectionsResponse {
    pub items: Vec<WsConnectionCount>,
}

#[derive(Serialize)]
pub struct WsConnectionCount {
    pub ip: String,
    pub connections: u64,
}

pub async fn list_ws_connections(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    let items = state
        .ip_limiter
        .snapshot()
        .into_iter()
        .map(|(ip, count)| WsConnectionCount {
            ip: ip.to_string(),
            connections: count as u64,
        })
        .collect();

    admin_ok(WsConnectionsResponse { items })
}
