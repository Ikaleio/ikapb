use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::extract::ws::Message;
use bytes::{Bytes, BytesMut};
use futures_util::stream::{SplitStream, StreamExt};
use tokio::sync::mpsc;
use tracing::warn;
use uuid::Uuid;

use crate::admin::ban::BanStore;
use crate::admin::hot_reload::ConfigHandle;
use crate::constants::{ACK_COOLING, ACK_FORBIDDEN, ACK_INVALID_TOKEN, ACK_OK};
use crate::errors::PaintboardError;
use crate::tick::pipeline::PaintPipeline;
use crate::token::cooldown::current_time_ms;
use crate::token::store::TokenRecord;
use crate::token::{cooldown::CooldownCache, manager::TokenManager};
use crate::types::{ConnCaps, OutboundFrame, PaintAck, WsMode};
use crate::ws::codec::{self, Packet};
use crate::ws::heartbeat::Heartbeat;

pub struct ConnState {
    pub id: u64,
    pub mode: WsMode,
    pub caps: ConnCaps,
    pub send_queue: mpsc::Sender<OutboundFrame>,
    pub last_pong_at: std::time::Instant,
    pub using_dict: Option<u32>,
    pub token_manager: Arc<TokenManager>,
    pub cooldown: Arc<CooldownCache>,
    pub ban_store: Arc<BanStore>,
    pub pipeline: Arc<PaintPipeline>,
    pub config_handle: ConfigHandle,
    pub heartbeat: Arc<Heartbeat>,
    pub pending_acks: parking_lot::Mutex<BytesMut>,
    pub close_flag: AtomicBool,
    close_intent: parking_lot::Mutex<Option<(u16, &'static str)>>,
    pub global_cooldown_ms: i64,
    pub max_ack_buffer_bytes: usize,
    /// Enqueue time of the first pending ACK (for latency detection).
    pub first_pending_at: parking_lot::Mutex<Option<std::time::Instant>>,
    pub peer_addr: IpAddr,
    pub client_ip: IpAddr,
    pub watch_ips: Option<HashSet<IpAddr>>,
}

impl ConnState {
    pub fn new(
        id: u64,
        caps: ConnCaps,
        queue: mpsc::Sender<OutboundFrame>,
        token_manager: Arc<TokenManager>,
        cooldown: Arc<CooldownCache>,
        ban_store: Arc<BanStore>,
        pipeline: Arc<PaintPipeline>,
        config_handle: ConfigHandle,
        global_cooldown_ms: i64,
        max_ack_buffer_bytes: usize,
        peer_addr: IpAddr,
        client_ip: IpAddr,
        watch_ips: Option<HashSet<IpAddr>>,
    ) -> Self {
        let using_dict = match caps.enc {
            crate::types::Enc::Zstd { dict_id } => dict_id,
            crate::types::Enc::None => None,
        };
        Self {
            id,
            mode: caps.mode,
            caps,
            send_queue: queue,
            last_pong_at: std::time::Instant::now(),
            using_dict,
            token_manager,
            cooldown,
            ban_store,
            pipeline,
            config_handle,
            heartbeat: Arc::new(Heartbeat::new(std::time::Duration::from_secs(10))),
            pending_acks: parking_lot::Mutex::new(BytesMut::new()),
            close_flag: AtomicBool::new(false),
            close_intent: parking_lot::Mutex::new(None),
            global_cooldown_ms,
            max_ack_buffer_bytes,
            first_pending_at: parking_lot::Mutex::new(None),
            peer_addr,
            client_ip,
            watch_ips,
        }
    }
}

fn enqueue_ack(state: &Arc<ConnState>, id: u32, status: u8) {
    let mut pending = state.pending_acks.lock();

    // Enforce a hard cap to protect the server; do not drop ACKs.
    if pending.len() >= state.max_ack_buffer_bytes {
        warn!(
            conn_id = state.id,
            client_ip = %state.client_ip,
            buffer_size = pending.len(),
            "ACK buffer exceeded absolute limit, closing connection"
        );
        state.mark_over_capacity();
        return;
    }

    // Record enqueue time for latency measurement when the first ACK is queued.
    if pending.is_empty() {
        *state.first_pending_at.lock() = Some(std::time::Instant::now());
    }

    pending.extend_from_slice(&codec::encode_paint_ack(PaintAck { id, status }));
}

fn effective_cooldown_ms(record: &TokenRecord, global_cooldown_ms: i64) -> i64 {
    if record.cooldown_ms == -1 {
        global_cooldown_ms
    } else {
        record.cooldown_ms
    }
}

impl ConnState {
    pub fn schedule_close(&self, code: u16, reason: &'static str) {
        if !self.close_flag.swap(true, Ordering::Relaxed) {
            *self.close_intent.lock() = Some((code, reason));
            warn!(
                conn_id = self.id,
                client_ip = %self.client_ip,
                close_code = code,
                reason,
                "scheduling connection close"
            );
        }
    }

    pub fn mark_over_capacity(&self) {
        if !self.close_flag.load(Ordering::Relaxed) {
            self.schedule_close(1011, "Slow consumer");
        }
    }

    pub fn close_reason(&self) -> Option<(u16, &'static str)> {
        *self.close_intent.lock()
    }
}

pub async fn recv_loop(
    mut stream: SplitStream<axum::extract::ws::WebSocket>,
    state: Arc<ConnState>,
) {
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Binary(bytes)) => {
                match codec::decode_packets(Bytes::from(bytes)) {
                    Ok(packets) => {
                        let mut should_close = false;
                        for packet in packets {
                            match packet {
                                Packet::Paint {
                                    x,
                                    y,
                                    rgb,
                                    uid,
                                    token,
                                    id,
                                    ..
                                } => {
                                    // SPEC 3.1: Validate token.
                                    let token_uuid = match Uuid::from_slice(&token) {
                                        Ok(uuid) => uuid,
                                        Err(_) => {
                                            enqueue_ack(&state, id, ACK_INVALID_TOKEN);
                                            continue;
                                        }
                                    };

                                    let Some(record) =
                                        state.token_manager.active_record(&token_uuid)
                                    else {
                                        enqueue_ack(&state, id, ACK_INVALID_TOKEN);
                                        continue;
                                    };

                                    if record.uid as u32 != uid {
                                        enqueue_ack(&state, id, ACK_INVALID_TOKEN);
                                        continue;
                                    }

                                    if matches!(state.mode, WsMode::Readonly) {
                                        enqueue_ack(&state, id, ACK_FORBIDDEN);
                                        continue;
                                    }

                                    let now_ms = current_time_ms();

                                    // Event time range check
                                    let event = state.config_handle.current().event;
                                    let allowed = match (event.start_timestamp, event.end_timestamp) {
                                        (0, 0) => true,
                                        (start, 0) => now_ms as u64 >= start,
                                        (0, end) => now_ms as u64 <= end,
                                        (start, end) => now_ms as u64 >= start && now_ms as u64 <= end,
                                    };
                                    if !allowed {
                                        enqueue_ack(&state, id, ACK_FORBIDDEN);
                                        continue;
                                    }
                                    if state.ban_store.active_record(record.uid, now_ms).is_some() {
                                        enqueue_ack(&state, id, ACK_FORBIDDEN);
                                        continue;
                                    }

                                    let token_str = record.token.clone();

                                    let cooldown_ms =
                                        effective_cooldown_ms(&record, state.global_cooldown_ms);
                                    if cooldown_ms > 0 {
                                        if !state.cooldown.can_paint(
                                            &token_str,
                                            now_ms,
                                            cooldown_ms,
                                        ) {
                                            enqueue_ack(&state, id, ACK_COOLING);
                                            continue;
                                        }
                                        state.cooldown.mark_painted(&token_str, now_ms);
                                    }

                                    // SPEC 3.1: Enqueue paint (accounting for epsilon soft window).
                                    let now_ns = crate::util::time::now_ns_monotonic();
                                    let target_tick = state.pipeline.tick_for_timestamp(now_ns);
                                    let current_tick = state.pipeline.current_tick();

                                    let ingest_result = if target_tick.0 > current_tick.0 {
                                        // Belongs to the next tick; use the "next" buffer.
                                        state.pipeline.ingest_next(x, y, rgb, uid, state.client_ip)
                                    } else {
                                        state.pipeline.ingest(x, y, rgb, uid, state.client_ip)
                                    };

                                    match ingest_result {
                                        Ok(_) => enqueue_ack(&state, id, ACK_OK),
                                        Err(err) => {
                                            let code = err.to_ack_code();
                                            warn!(
                                                conn_id = state.id,
                                                error = %err,
                                                "failed to ingest paint"
                                            );
                                            enqueue_ack(&state, id, code);
                                            if matches!(err, PaintboardError::Protocol(_)) {
                                                state.schedule_close(1002, "Protocol violation");
                                                should_close = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                                Packet::Pong => {
                                    // Heartbeat: client Pong received.
                                    state.heartbeat.handle_pong().await;
                                }
                            }
                        }
                        if should_close {
                            break;
                        }
                    }
                    Err(codec::DecodeError::UnknownOpcode(opcode)) => {
                        warn!(
                            conn_id = state.id,
                            opcode, "unknown opcode received, closing connection"
                        );
                        state.schedule_close(1002, "Protocol violation");
                        break;
                    }
                    Err(codec::DecodeError::Incomplete) => {
                        warn!(conn_id = state.id, "incomplete packet received");
                        // Close to avoid desynchronization.
                        state.schedule_close(1002, "Protocol violation");
                        break;
                    }
                }
            }
            Ok(_) => {}
            Err(_e) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::effective_cooldown_ms;
    use crate::token::store::TokenRecord;

    fn record_with_cooldown(cooldown_ms: i64) -> TokenRecord {
        TokenRecord {
            uid: 1,
            token: "00000000-0000-0000-0000-000000000000".to_string(),
            issued_at_ms: 0,
            cooldown_ms,
            ip: None,
            last_paint_at_ms: None,
        }
    }

    #[test]
    fn uses_global_when_token_follows_global() {
        let record = record_with_cooldown(-1);
        assert_eq!(effective_cooldown_ms(&record, 30_000), 30_000);
    }

    #[test]
    fn uses_token_specific_cooldown_when_set() {
        let record = record_with_cooldown(5_000);
        assert_eq!(effective_cooldown_ms(&record, 30_000), 5_000);
    }
}
