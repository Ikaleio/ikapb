use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::{
    extract::{ConnectInfo, State, ws::WebSocketUpgrade},
    http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
    http::{HeaderMap, HeaderValue, StatusCode, Uri},
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tracing::{info, warn};
use url::form_urlencoded;

use crate::{
    http::HttpState,
    types::{ConnCaps, Enc, WsMode},
    ws::{connection::ConnState, dispatcher},
};
use crate::util::ip::{parse_ip, resolve_client_ip};

static CONN_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn parse_query(uri: &Uri) -> Result<ConnCaps, &'static str> {
    let mut enc_requested = false;
    let mut dict_id: Option<u32> = None;
    let mut readonly = false;
    let mut writeonly = false;

    if let Some(query) = uri.query() {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "enc" if value.eq_ignore_ascii_case("zstd") => {
                    enc_requested = true;
                }
                "dict" => {
                    if let Ok(id) = value.parse::<u32>() {
                        dict_id = Some(id);
                    }
                }
                "readonly" if value == "1" => readonly = true,
                "writeonly" if value == "1" => writeonly = true,
                // maxframe negotiation removed: server no longer slices tick frames.
                _ => {}
            }
        }
    }

    if readonly && writeonly {
        return Err("readonly and writeonly are mutually exclusive");
    }

    let mode = if readonly {
        WsMode::Readonly
    } else if writeonly {
        WsMode::Writeonly
    } else {
        WsMode::ReadWrite
    };

    // Tightened rule: `dict` does not implicitly enable zstd.
    let enc = if enc_requested {
        Enc::Zstd { dict_id }
    } else {
        Enc::None
    };

    Ok(ConnCaps { enc, mode })
}

fn parse_watch_ips(uri: &Uri) -> Option<HashSet<IpAddr>> {
    let mut ips = HashSet::new();
    let mut seen = false;
    if let Some(query) = uri.query() {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            if key.as_ref() == "watch_ips" {
                seen = true;
                for part in value.split(',') {
                    if let Some(ip) = parse_ip(part) {
                        ips.insert(ip);
                    }
                }
            }
        }
    }
    if seen { Some(ips) } else { None }
}

fn parse_admin_key(uri: &Uri) -> Option<String> {
    if let Some(query) = uri.query() {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            if key.as_ref() == "admin_key" {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

pub async fn handle_ws_upgrade(
    ws: WebSocketUpgrade,
    uri: Uri,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<HttpState>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    let caps = match parse_query(&uri) {
        Ok(caps) => caps,
        Err(msg) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };
    let admin_key = headers
        .get("x-admin-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .or_else(|| parse_admin_key(&uri));
    let admin_authorized = !cfg.admin_manage_key.is_empty()
        && admin_key.as_deref() == Some(cfg.admin_manage_key.as_str());
    let trusted_proxy_ips: Vec<IpAddr> = cfg
        .network
        .trusted_proxies
        .iter()
        .filter_map(|proxy| proxy.parse::<IpAddr>().ok())
        .collect();

    let remote_ip = peer_addr.ip();
    let client_ip = resolve_client_ip(&headers, remote_ip, &trusted_proxy_ips);
    let watch_ips = if admin_authorized {
        parse_watch_ips(&uri).and_then(|mut ips| {
            ips.insert(client_ip);
            if ips.is_empty() { None } else { Some(ips) }
        })
    } else {
        None
    };
    let forwarded_for = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned());
    let x_real_ip = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned());

    // SPEC: IP connection limit check
    if !state.ip_limiter.try_acquire(client_ip) {
        warn!(
            client_ip = %client_ip,
            remote_addr = %remote_ip,
            current_count = state.ip_limiter.count(client_ip),
            "IP connection limit exceeded, rejecting WebSocket upgrade"
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "IP connection limit exceeded",
        )
            .into_response();
    }

    let ip_limiter = state.ip_limiter.clone();

    let mut response = ws
        .on_upgrade(move |socket| async move {
            // Ensure IP slot is released when connection closes
            struct IpGuard {
                limiter: Arc<crate::ws::ip_limiter::IpConnectionLimiter>,
                ip: IpAddr,
            }
            impl Drop for IpGuard {
                fn drop(&mut self) {
                    self.limiter.release(self.ip);
                }
            }
            let _ip_guard = IpGuard {
                limiter: ip_limiter,
                ip: client_ip,
            };

            let cfg = cfg.clone();
            let conn_id = CONN_COUNTER.fetch_add(1, Ordering::Relaxed);
            info!(
                conn_id,
                client_ip = %client_ip,
                remote_addr = %remote_ip,
                proxied = client_ip != remote_ip,
                forwarded_for = forwarded_for.as_deref(),
                x_real_ip = x_real_ip.as_deref(),
                ?caps,
                "websocket connection established"
            );

            // SPEC: Slow-consumer protection - outbound queue = 32 frames.
            let (tx, rx) = mpsc::channel(32);

            // Optional dictionary loading: only when (enc=zstd AND dict=<id>) and watch filter is not active.
            let dict = if watch_ips.is_none() {
                match caps.enc {
                    crate::types::Enc::Zstd {
                        dict_id: Some(dict_id),
                    } => cfg
                        .zstd
                        .zstd_dictionary_path
                        .as_ref()
                        .and_then(|path| crate::compress::dict::load_dict(path, dict_id).ok())
                        .flatten(),
                    _ => None,
                }
            } else {
                None
            };

            let conn_state = Arc::new(ConnState::new(
                conn_id,
                caps.clone(),
                tx,
                state.token_manager.clone(),
                state.cooldown.clone(),
                state.ban_store.clone(),
                state.pipeline.clone(),
                state.config.clone(),
                cfg.token.global_cooldown_ms,
                cfg.websocket.max_ack_buffer_bytes,
                remote_ip,
                client_ip,
                watch_ips.clone(),
            ));
            // Attach compression parameters to dispatcher context.
            let zstd_level = cfg.zstd.zstd_level;

            // If negotiated, send a single 0xF6 dictionary frame before the dispatcher starts.
            let dict_frame: Option<Bytes> =
                dict.as_ref().map(crate::compress::dict::encode_dict_frame);

            let period = Duration::from_millis(cfg.websocket.ws_send_period_ms);

            // Spawn recv loop and dispatcher.
            let (mut ws_sink, ws_stream) = socket.split();

            // Subscribe to global tick broadcast and forward to the per-connection queue.
            let mut forward_task = None;
            if !matches!(caps.mode, crate::types::WsMode::Writeonly) {
                let subscriber = state.pipeline.subscribe();
                let forward_conn = conn_state.clone();
                forward_task = Some(tokio::spawn(async move {
                    forward_broadcast_to_conn(subscriber, forward_conn).await;
                }));
            }

            // Start randomized heartbeat: inject 0xFC Ping frames via the connection queue.
            let heartbeat_task = {
                let hb_conn = conn_state.clone();
                tokio::spawn(async move {
                    let sender_conn = hb_conn.clone();
                    hb_conn
                        .heartbeat
                        .schedule_ping(move |bytes| {
                            let conn = sender_conn.clone();
                            let frame = crate::types::OutboundFrame {
                                bytes,
                                is_compressed: false,
                                tick: conn.pipeline.current_tick(),
                                part_idx: 0,
                                part_total: 1,
                                events: None,
                            };
                            let _ = conn.send_queue.try_send(frame);
                        })
                        .await;
                })
            };

            // If negotiated, send 0xF6 once (before starting dispatcher).
            if let Some(dict_bytes) = dict_frame {
                use futures_util::SinkExt;
                let _ = ws_sink
                    .send(axum::extract::ws::Message::Binary(dict_bytes.to_vec()))
                    .await;
            }

            let max_latency_ms = cfg.websocket.max_client_latency_ms;
            let use_zstd =
                watch_ips.is_none() && matches!(caps.enc, crate::types::Enc::Zstd { .. });
            let dict_clone = dict.clone();

            let ctx = dispatcher::DispatchContext {
                period,
                use_zstd,
                zstd_level,
                dict: dict_clone,
                max_latency: Duration::from_millis(max_latency_ms),
            };

            let dispatch_conn = conn_state.clone();
            let recv_state = conn_state.clone();
            let recv_fut = crate::ws::connection::recv_loop(ws_stream, recv_state);
            let dispatch_fut = dispatcher::run_dispatcher(ws_sink, rx, ctx, dispatch_conn);
            tokio::pin!(recv_fut);
            tokio::pin!(dispatch_fut);

            tokio::select! {
                _ = &mut dispatch_fut => {}
                _ = &mut recv_fut => {
                    conn_state.schedule_close(1001, "Client disconnected");
                    let _ = (&mut dispatch_fut).await;
                }
            }

            if let Some(handle) = forward_task {
                handle.abort();
                let _ = handle.await;
            }
            heartbeat_task.abort();
            let _ = heartbeat_task.await;
        })
        .into_response();

    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));

    response
}

// effective_frame_cap removed: frame slicing and client-declared maxframe are deprecated.

async fn forward_broadcast_to_conn(
    mut rx: tokio::sync::broadcast::Receiver<crate::types::OutboundFrame>,
    conn: Arc<ConnState>,
) {
    loop {
        match rx.recv().await {
            Ok(frame) => match conn.send_queue.try_send(frame) {
                Ok(_) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    conn.mark_over_capacity();
                    return;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return,
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                // Lagged: drop and continue
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}
