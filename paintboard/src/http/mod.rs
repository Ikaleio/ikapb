use std::sync::Arc;

use crate::{
    admin::ban::BanStore,
    admin::hot_reload::ConfigHandle,
    board::memory::BoardHandle,
    tick::pipeline::PaintPipeline,
    token::{cooldown::CooldownCache, manager::TokenManager},
    ws::ip_limiter::IpConnectionLimiter,
};
use anyhow::Result;
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_MAX_AGE, ACCESS_CONTROL_REQUEST_HEADERS,
};
use axum::http::uri::PathAndQuery;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::{
    Router,
    body::Body,
    http::{Request, Uri},
    middleware::{Next, from_fn},
    response::Response,
    routing::{delete, get, post},
};
use std::str::FromStr;

pub mod api;
pub mod getboard;
pub mod gettoken;
pub mod static_files;

#[derive(Clone)]
pub struct HttpState {
    pub config: ConfigHandle,
    pub board: BoardHandle,
    pub token_manager: Arc<TokenManager>,
    pub cooldown: Arc<CooldownCache>,
    pub ban_store: Arc<BanStore>,
    pub pipeline: Arc<PaintPipeline>,
    pub ip_limiter: Arc<IpConnectionLimiter>,
}

pub async fn build_router(
    cfg: ConfigHandle,
    board: BoardHandle,
    token_manager: Arc<TokenManager>,
    cooldown: Arc<CooldownCache>,
    ban_store: Arc<BanStore>,
    pipeline: Arc<PaintPipeline>,
    ip_limiter: Arc<IpConnectionLimiter>,
) -> Result<Router> {
    let state = HttpState {
        config: cfg,
        board,
        token_manager,
        cooldown,
        ban_store,
        pipeline,
        ip_limiter,
    };

    let router = Router::new()
        .route("/", get(static_files::serve_index))
        .route("/favicon.ico", get(static_files::serve_favicon))
        .route("/api", get(api::handle_root))
        .route("/api/", get(api::handle_root))
        .route("/api/paintboard/getboard", get(getboard::handle_getboard))
        .route("/api/paintboard/getboard/", get(getboard::handle_getboard))
        .route("/api/paintboard/gettoken", post(gettoken::handle_gettoken))
        .route("/api/paintboard/gettoken/", post(gettoken::handle_gettoken))
        .route("/api/auth/gettoken", post(gettoken::handle_gettoken))
        .route("/api/auth/gettoken/", post(gettoken::handle_gettoken))
        .route("/api/paintboard/ws", get(crate::ws::handle_ws_upgrade))
        .route("/api/paintboard/ws/", get(crate::ws::handle_ws_upgrade))
        .route(
            "/admin/fills",
            post(crate::admin::routes::fill::handle_fill),
        )
        .route(
            "/admin/tokens",
            get(crate::admin::routes::tokens::list_tokens)
                .post(crate::admin::routes::tokens::create_token),
        )
        .route(
            "/admin/tokens/:uid",
            get(crate::admin::routes::tokens::get_token)
                .put(crate::admin::routes::tokens::put_token)
                .patch(crate::admin::routes::tokens::patch_token)
                .delete(crate::admin::routes::tokens::delete_token),
        )
        .route(
            "/admin/bans",
            get(crate::admin::routes::bans::list_bans).post(crate::admin::routes::bans::create_ban),
        )
        .route(
            "/admin/bans/:uid",
            delete(crate::admin::routes::bans::delete_ban),
        )
        .route(
            "/admin/ws/connections",
            get(crate::admin::routes::ws::list_ws_connections),
        )
        .route(
            "/vendor/zstd/zstd-codec-binding-wasm.js",
            get(static_files::serve_zstd_wasm_binding),
        )
        .route(
            "/vendor/zstd/zstd-codec-browser.js",
            get(static_files::serve_zstd_browser_glue),
        )
        .layer(from_fn(cors_allow_all))
        .layer(from_fn(trim_trailing_slash));

    let router = router.with_state(state);
    Ok(router)
}

async fn cors_allow_all(req: Request<Body>, next: Next) -> Response {
    // Capture the request path before moving the request into the next handler
    let path = req.uri().path().to_string();

    // Handle preflight requests quickly
    if req.method() == Method::OPTIONS {
        // Mirror the requested header list when present so custom headers (e.g. X-Admin-Key)
        // always pass the browser preflight check.
        let allow_headers = req
            .headers()
            .get(ACCESS_CONTROL_REQUEST_HEADERS)
            .cloned()
            .unwrap_or_else(|| HeaderValue::from_static("*"));

        let builder = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"))
            .header(
                ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET,POST,PUT,PATCH,DELETE,OPTIONS"),
            )
            .header(ACCESS_CONTROL_ALLOW_HEADERS, allow_headers)
            .header(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));

        return builder.body(Body::from(())).unwrap();
    }

    let mut res = next.run(req).await;
    // Ensure simple CORS header on all responses
    res.headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));

    // For admin routes, also advertise that the X-Admin-Key header is allowed.
    if path.starts_with("/admin") {
        res.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("X-Admin-Key, Content-Type, Authorization"),
        );
    }

    res
}

async fn trim_trailing_slash(mut req: Request<Body>, next: Next) -> Response {
    if let Some(pq) = req.uri().path_and_query() {
        let path = pq.path();
        if path.len() > 1 && path.ends_with('/') {
            let trimmed_path = path.trim_end_matches('/');
            let new_pq = if let Some(query) = pq.query() {
                format!("{}?{}", trimmed_path, query)
            } else {
                trimmed_path.to_string()
            };
            let mut parts = req.uri().clone().into_parts();
            if let Ok(parsed) = PathAndQuery::from_str(&new_pq) {
                parts.path_and_query = Some(parsed);
                if let Ok(new_uri) = Uri::from_parts(parts) {
                    *req.uri_mut() = new_uri;
                }
            }
        }
    }
    next.run(req).await
}
