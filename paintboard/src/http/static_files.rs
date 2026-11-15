use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};

/// Serve the main HTML page.
pub async fn serve_index() -> Response {
    let html = include_str!("../../static/index.html");
    Html(html).into_response()
}

/// Serve favicon (avoid 404 noise).
pub async fn serve_favicon() -> Response {
    // Return 404.
    (StatusCode::NOT_FOUND, "").into_response()
}

/// Serve embedded zstd wasm binding script.
pub async fn serve_zstd_wasm_binding() -> Response {
    let js = include_str!("../../static/vendor/zstd/zstd-codec-binding-wasm.js");
    let mut resp = Response::new(js.into());
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript"),
    );
    resp
}

/// Serve browser-side zstd helper wrapper.
pub async fn serve_zstd_browser_glue() -> Response {
    let js = include_str!("../../static/vendor/zstd/zstd-codec-browser.js");
    let mut resp = Response::new(js.into());
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript"),
    );
    resp
}
