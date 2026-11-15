use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::connect_info::MockConnectInfo;
use futures_util::{SinkExt, StreamExt};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use tokio::io::DuplexStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};
use uuid::Uuid;

use paintboard::admin;
use paintboard::admin::ban::BanStore;
use paintboard::board::memory::Board;
use paintboard::config::{self, AppConfig};
use paintboard::constants::{
    ACK_INVALID_TOKEN, ACK_OK, OP_C2S_PAINT, OP_S2C_COMPRESSED_TICK, OP_S2C_DICT,
    OP_S2C_PAINT_EVENT, OP_S2C_PAINT_RESULT,
};
use paintboard::http;
use paintboard::tick::coordinator::TickCoordinator;
use paintboard::tick::pipeline::PaintPipeline;
use paintboard::token::cooldown::CooldownCache;
use paintboard::token::manager::TokenManager;
use paintboard::types::{Rgb, Rgba};
use paintboard::ws::ip_limiter::IpConnectionLimiter;

static TEST_LOCK: Mutex<()> = Mutex::new(());

const READ_TIMEOUT: Duration = Duration::from_secs(2);

type WsStream = WebSocketStream<DuplexStream>;

struct TestServer {
    app: axum::Router,
    pipeline: std::sync::Arc<PaintPipeline>,
    tick: std::sync::Arc<TickCoordinator>,
    token_manager: std::sync::Arc<TokenManager>,
    root_dir: PathBuf,
}

impl TestServer {
    async fn start(cfg: AppConfig, root_dir: PathBuf) -> Self {
        config::apply_hot(&cfg).expect("apply test config");

        let config_path = root_dir.join("test-config.toml");
        let config_handle = admin::hot_reload::init(config_path, cfg.clone());

        let board = Board::new(true, Rgba(0));
        let tick = std::sync::Arc::new(TickCoordinator::new(cfg.tps));
        tick.set_epsilon_ns(0);

        let pipeline = PaintPipeline::new(
            board.clone(),
            tick.clone(),
            1,
            cfg.data_dir.clone(),
            cfg.persistence.wal_flush_every_ticks,
            None,
        );
        let token_manager = std::sync::Arc::new(TokenManager::new());
        let cooldown = std::sync::Arc::new(CooldownCache::new());
        let ban_store = std::sync::Arc::new(BanStore::new());
        let ip_limiter = std::sync::Arc::new(IpConnectionLimiter::new(
            cfg.network.max_connections_per_ip,
        ));

        let app = http::build_router(
            config_handle,
            board,
            token_manager.clone(),
            cooldown,
            ban_store,
            pipeline.clone(),
            ip_limiter,
        )
        .await
        .expect("build router");
        let app = app.layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));

        Self {
            app,
            pipeline,
            tick,
            token_manager,
            root_dir,
        }
    }

    fn issue_token_bytes(&self, uid: i64, cooldown_ms: i64) -> [u8; 16] {
        let (record, _) = self.token_manager.issue_token(uid, cooldown_ms, None);
        let uuid = Uuid::parse_str(&record.token).expect("token uuid");
        *uuid.as_bytes()
    }

    async fn connect_ws(&self, query: Option<&str>) -> WsStream {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let service = TowerToHyperService::new(self.app.clone().into_service::<Incoming>());
        tokio::spawn(async move {
            let io = TokioIo::new(server);
            let conn = http1::Builder::new().serve_connection(io, service);
            let _ = conn.with_upgrades().await;
        });

        let mut url = "ws://localhost/api/paintboard/ws".to_string();
        if let Some(query) = query {
            url.push('?');
            url.push_str(query);
        }
        let (ws, _resp) = client_async(url, client)
            .await
            .expect("connect websocket");
        ws
    }

    fn flush_ticks(&self, count: usize) {
        for _ in 0..count {
            let tick = self.tick.bump_manual_tick();
            self.pipeline.handle_tick(tick);
        }
    }

    async fn shutdown(self) {
        let _ = std::fs::remove_dir_all(&self.root_dir);
    }
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("ikapb-{label}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

fn base_config(root: &Path) -> AppConfig {
    let default_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/default.toml");
    let mut cfg = config::load_from(default_path.as_path()).expect("load default config");

    cfg.bind_address = "127.0.0.1:0".to_string();
    cfg.no_access_key_verify = true;
    cfg.tps = 1;
    cfg.persistence.persist_board = false;
    cfg.persistence.wal_flush_every_ticks = 0;
    cfg.data_dir = root.join("data");
    cfg.sqlite_path = root.join("data").join("tokens.db");
    cfg.network.max_connections_per_ip = None;

    cfg
}

fn paint_request_bytes(
    x: u16,
    y: u16,
    rgb: Rgb,
    uid: u32,
    token: [u8; 16],
    id: u32,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(31);
    buf.push(OP_C2S_PAINT);
    buf.extend_from_slice(&x.to_le_bytes());
    buf.extend_from_slice(&y.to_le_bytes());
    buf.push(rgb.0);
    buf.push(rgb.1);
    buf.push(rgb.2);
    let uid_bytes = uid.to_le_bytes();
    buf.extend_from_slice(&uid_bytes[..3]);
    buf.extend_from_slice(&token);
    buf.extend_from_slice(&id.to_le_bytes());
    buf
}

fn decode_acks(bytes: &[u8]) -> Vec<(u32, u8)> {
    assert_eq!(bytes.len() % 6, 0, "ack frame length must be multiple of 6");
    bytes
        .chunks_exact(6)
        .map(|chunk| {
            assert_eq!(
                chunk[0],
                OP_S2C_PAINT_RESULT,
                "ack opcode must be 0xFF"
            );
            let id = u32::from_le_bytes([chunk[1], chunk[2], chunk[3], chunk[4]]);
            let status = chunk[5];
            (id, status)
        })
        .collect()
}

fn decode_events(bytes: &[u8]) -> Vec<(u16, u16, Rgb)> {
    assert_eq!(
        bytes.len() % 8,
        0,
        "event frame length must be multiple of 8"
    );
    bytes
        .chunks_exact(8)
        .map(|chunk| {
            assert_eq!(
                chunk[0],
                OP_S2C_PAINT_EVENT,
                "event opcode must be 0xFA"
            );
            let x = u16::from_le_bytes([chunk[1], chunk[2]]);
            let y = u16::from_le_bytes([chunk[3], chunk[4]]);
            let rgb = Rgb(chunk[5], chunk[6], chunk[7]);
            (x, y, rgb)
        })
        .collect()
}

fn parse_compressed_tick_frame(bytes: &[u8]) -> (u64, u16, u16, u32, u8, &[u8]) {
    assert!(
        bytes.len() >= 18,
        "compressed tick frame must include 18-byte header"
    );
    assert_eq!(
        bytes[0],
        OP_S2C_COMPRESSED_TICK,
        "compressed tick opcode must be 0xF7"
    );
    let tick = u64::from_le_bytes(bytes[1..9].try_into().expect("tick bytes"));
    let part_idx = u16::from_le_bytes(bytes[9..11].try_into().expect("part_idx bytes"));
    let part_total = u16::from_le_bytes(bytes[11..13].try_into().expect("part_total bytes"));
    let raw_len = u32::from_le_bytes(bytes[13..17].try_into().expect("raw_len bytes"));
    let flags = bytes[17];
    let payload = &bytes[18..];
    (tick, part_idx, part_total, raw_len, flags, payload)
}

fn parse_dict_frame(bytes: &[u8]) -> (u32, Vec<u8>) {
    assert!(bytes.len() >= 9, "dictionary frame must include header");
    assert_eq!(bytes[0], OP_S2C_DICT, "dictionary opcode must be 0xF6");
    let dict_id = u32::from_le_bytes(bytes[1..5].try_into().expect("dict id"));
    let size = u32::from_le_bytes(bytes[5..9].try_into().expect("dict size")) as usize;
    assert_eq!(
        bytes.len(),
        9 + size,
        "dictionary frame length mismatch"
    );
    let payload = bytes[9..].to_vec();
    (dict_id, payload)
}

async fn recv_binary(ws: &mut WsStream) -> Vec<u8> {
    loop {
        let msg = tokio::time::timeout(READ_TIMEOUT, ws.next())
            .await
            .expect("timeout waiting for message")
            .expect("stream closed")
            .expect("websocket error");
        match msg {
            Message::Binary(bytes) => return bytes.to_vec(),
            Message::Close(frame) => panic!("unexpected close: {frame:?}"),
            _ => {}
        }
    }
}

async fn recv_binary_with_opcode(ws: &mut WsStream, opcode: u8) -> Vec<u8> {
    for _ in 0..4 {
        let bytes = recv_binary(ws).await;
        if bytes.first().copied() == Some(opcode) {
            return bytes;
        }
    }
    panic!("expected opcode {opcode:#x} not received");
}

async fn recv_close(ws: &mut WsStream) -> Option<CloseCode> {
    loop {
        let msg = tokio::time::timeout(READ_TIMEOUT, ws.next())
            .await
            .expect("timeout waiting for close")
            .expect("stream closed")
            .expect("websocket error");
        match msg {
            Message::Close(frame) => return frame.map(|f| f.code),
            Message::Binary(_) => continue,
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_rejects_invalid_token() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-auth-invalid");
    let cfg = base_config(&root);
    let server = TestServer::start(cfg, root).await;

    let _valid_token = server.issue_token_bytes(42, 0);
    let mut ws = server.connect_ws(None).await;

    let bad_token = [0u8; 16];
    let payload = paint_request_bytes(10, 20, Rgb(1, 2, 3), 42, bad_token, 1001);
    ws.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary(&mut ws).await;
    let acks = decode_acks(&ack_frame);
    assert_eq!(acks, vec![(1001, ACK_INVALID_TOKEN)]);

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_rejects_paint_outside_event_time() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-event-time");
    let mut cfg = base_config(&root);

    // Set event in the future
    let now = paintboard::token::cooldown::current_time_ms() as u64;
    cfg.event.start_timestamp = now + 100_000;
    cfg.event.end_timestamp = now + 200_000;

    let server = TestServer::start(cfg.clone(), root.clone()).await;

    let token = server.issue_token_bytes(300, 0);
    let mut ws = server.connect_ws(None).await;

    // 1. Before event starts
    let payload = paint_request_bytes(10, 10, Rgb(1, 1, 1), 300, token, 7001);
    ws.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary(&mut ws).await;
    let acks = decode_acks(&ack_frame);
    assert_eq!(acks, vec![(7001, paintboard::constants::ACK_FORBIDDEN)]);

    // 2. Hot-reload to include current time
    let mut new_cfg = cfg.clone();
    new_cfg.event.start_timestamp = now - 10_000;
    paintboard::admin::hot_reload::apply_config(&new_cfg);

    // Wait a bit for config to be applied
    tokio::time::sleep(Duration::from_millis(100)).await;

    let payload = paint_request_bytes(10, 10, Rgb(2, 2, 2), 300, token, 7002);
    ws.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary(&mut ws).await;
    let acks = decode_acks(&ack_frame);
    assert_eq!(acks, vec![(7002, ACK_OK)]);

    // 3. Hot-reload to end the event
    let mut final_cfg = cfg.clone();
    final_cfg.event.end_timestamp = now - 5_000;
    paintboard::admin::hot_reload::apply_config(&final_cfg);

    tokio::time::sleep(Duration::from_millis(100)).await;

    let payload = paint_request_bytes(10, 10, Rgb(3, 3, 3), 300, token, 7003);
    ws.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary(&mut ws).await;
    let acks = decode_acks(&ack_frame);
    assert_eq!(acks, vec![(7003, paintboard::constants::ACK_FORBIDDEN)]);

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_rejects_uid_mismatch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-auth-uid-mismatch");
    let cfg = base_config(&root);
    let server = TestServer::start(cfg, root).await;

    let token = server.issue_token_bytes(7, 0);
    let mut ws = server.connect_ws(None).await;

    let payload = paint_request_bytes(5, 6, Rgb(9, 9, 9), 8, token, 2001);
    ws.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary(&mut ws).await;
    let acks = decode_acks(&ack_frame);
    assert_eq!(acks, vec![(2001, ACK_INVALID_TOKEN)]);

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_paint_pipeline_broadcasts_events() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-pipeline");
    let cfg = base_config(&root);
    let server = TestServer::start(cfg, root).await;

    let token = server.issue_token_bytes(101, 0);
    let mut writer = server.connect_ws(None).await;
    let mut reader = server.connect_ws(Some("readonly=1")).await;

    let payload = paint_request_bytes(12, 34, Rgb(7, 8, 9), 101, token, 3001);
    writer.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary(&mut writer).await;
    let acks = decode_acks(&ack_frame);
    assert_eq!(acks, vec![(3001, ACK_OK)]);

    server.flush_ticks(1);

    let event_frame = recv_binary(&mut reader).await;
    let events = decode_events(&event_frame);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], (12, 34, Rgb(7, 8, 9)));

    drop(writer);
    drop(reader);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_accepts_batched_paint_requests() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-batched");
    let cfg = base_config(&root);
    let server = TestServer::start(cfg, root).await;

    let token = server.issue_token_bytes(55, 0);
    let mut ws = server.connect_ws(None).await;

    let first = paint_request_bytes(1, 1, Rgb(10, 11, 12), 55, token, 4001);
    let second = paint_request_bytes(2, 1, Rgb(20, 21, 22), 55, token, 4002);
    let mut payload = Vec::with_capacity(first.len() + second.len());
    payload.extend_from_slice(&first);
    payload.extend_from_slice(&second);

    ws.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary(&mut ws).await;
    let mut acks = decode_acks(&ack_frame);
    acks.sort_by_key(|(id, _)| *id);
    assert_eq!(acks, vec![(4001, ACK_OK), (4002, ACK_OK)]);

    server.flush_ticks(1);

    let event_frame = recv_binary(&mut ws).await;
    let mut events = decode_events(&event_frame);
    events.sort_by_key(|(x, y, _)| (*y as u32, *x as u32));
    assert_eq!(
        events,
        vec![
            (1, 1, Rgb(10, 11, 12)),
            (2, 1, Rgb(20, 21, 22))
        ]
    );

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_unknown_opcode_closes_connection() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-unknown-opcode");
    let cfg = base_config(&root);
    let server = TestServer::start(cfg, root).await;

    let mut ws = server.connect_ws(None).await;
    ws.send(Message::Binary(vec![0xAA].into()))
        .await
        .expect("send");

    let close = recv_close(&mut ws).await;
    assert!(
        matches!(close, Some(CloseCode::Protocol)),
        "expected protocol close, got {close:?}"
    );

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_incomplete_packet_closes_connection() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-incomplete");
    let cfg = base_config(&root);
    let server = TestServer::start(cfg, root).await;

    let mut ws = server.connect_ws(None).await;
    let mut payload = vec![OP_C2S_PAINT];
    payload.extend_from_slice(&[0u8; 10]);
    ws.send(Message::Binary(payload.into())).await.expect("send");

    let close = recv_close(&mut ws).await;
    assert!(
        matches!(close, Some(CloseCode::Protocol)),
        "expected protocol close, got {close:?}"
    );

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_oversized_ack_buffer_closes_connection() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-ack-overflow");
    let mut cfg = base_config(&root);
    cfg.websocket.max_ack_buffer_bytes = 6;
    cfg.websocket.ws_send_period_ms = 50;
    let server = TestServer::start(cfg, root).await;

    let token = server.issue_token_bytes(99, 0);
    let mut ws = server.connect_ws(None).await;

    let first = paint_request_bytes(3, 3, Rgb(1, 1, 1), 99, token, 5001);
    let second = paint_request_bytes(4, 3, Rgb(2, 2, 2), 99, token, 5002);
    let mut payload = Vec::with_capacity(first.len() + second.len());
    payload.extend_from_slice(&first);
    payload.extend_from_slice(&second);

    ws.send(Message::Binary(payload.into())).await.expect("send");

    let close = recv_close(&mut ws).await;
    assert!(close.is_some(), "expected connection close");

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_sends_compressed_tick_when_requested() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-compressed");
    let cfg = base_config(&root);
    let server = TestServer::start(cfg, root).await;

    let token = server.issue_token_bytes(200, 0);
    let mut ws = server.connect_ws(Some("enc=zstd")).await;

    let payload = paint_request_bytes(25, 26, Rgb(9, 8, 7), 200, token, 6001);
    ws.send(Message::Binary(payload.into())).await.expect("send");

    let ack_frame = recv_binary_with_opcode(&mut ws, OP_S2C_PAINT_RESULT).await;
    let acks = decode_acks(&ack_frame);
    assert_eq!(acks, vec![(6001, ACK_OK)]);

    server.flush_ticks(1);

    let tick_frame = recv_binary_with_opcode(&mut ws, OP_S2C_COMPRESSED_TICK).await;
    let (tick, part_idx, part_total, raw_len, flags, payload) =
        parse_compressed_tick_frame(&tick_frame);
    assert!(tick > 0, "tick index should advance");
    assert_eq!(part_idx, 0);
    assert_eq!(part_total, 1);
    assert_eq!(flags, 0);
    assert_eq!(raw_len, 8);

    let decompressed = zstd::bulk::decompress(payload, raw_len as usize)
        .expect("decompress tick payload");
    let events = decode_events(&decompressed);
    assert_eq!(events, vec![(25, 26, Rgb(9, 8, 7))]);

    drop(ws);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_sends_dictionary_frame_when_available() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = unique_temp_dir("ws-dict");
    let mut cfg = base_config(&root);
    let dict_path = root.join("data").join("dict.bin");
    std::fs::create_dir_all(dict_path.parent().expect("dict parent"))
        .expect("create dict dir");
    let dict_bytes = b"test-dictionary";
    std::fs::write(&dict_path, dict_bytes).expect("write dict bytes");
    cfg.zstd.zstd_dictionary_path = Some(dict_path);
    let server = TestServer::start(cfg, root).await;

    let mut ws = server.connect_ws(Some("enc=zstd&dict=7")).await;
    let dict_frame = recv_binary(&mut ws).await;
    let (dict_id, payload) = parse_dict_frame(&dict_frame);
    assert_eq!(dict_id, 7);
    assert_eq!(payload, dict_bytes.to_vec());

    drop(ws);
    server.shutdown().await;
}
