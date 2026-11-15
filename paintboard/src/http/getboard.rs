use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::http::HttpState;

#[derive(Deserialize, Default)]
pub struct GetBoardQuery {
    pub enc: Option<String>,
}

pub async fn handle_getboard(
    Query(q): Query<GetBoardQuery>,
    State(state): State<HttpState>,
    req_headers: HeaderMap,
) -> Response {
    let cfg = state.config.current();
    let tick = state.pipeline.current_tick().0;
    let etag_value = format!("\"rev-{}\"", tick);
    let etag_header = header::HeaderValue::from_str(&etag_value).unwrap();

    let not_modified = req_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map_or(false, |raw| {
            raw.split(',')
                .map(|tag| tag.trim())
                .any(|tag| tag == "*" || tag == etag_value)
        });

    if not_modified {
        let mut headers = HeaderMap::new();
        headers.insert(header::ETAG, etag_header.clone());
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }

    let mut bytes = state.board.snapshot_rgb24();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert("X-Width", header::HeaderValue::from_static("1000"));
    headers.insert("X-Height", header::HeaderValue::from_static("600"));
    headers.insert("X-Pixel-Format", header::HeaderValue::from_static("RGB24"));
    headers.insert(header::ETAG, etag_header);

    if matches!(q.enc.as_deref(), Some("zstd")) {
        if let Ok(compressed) =
            crate::compress::zstd::compress_frame(&bytes, cfg.zstd.zstd_level, None)
        {
            bytes = compressed;
            headers.insert("X-Encoded", header::HeaderValue::from_static("zstd"));
        }
    }

    (StatusCode::OK, headers, bytes).into_response()
}
