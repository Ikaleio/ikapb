use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message};
use bytes::{BufMut, BytesMut};
use futures_util::{SinkExt, stream::SplitSink};
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::compress::{dict::Dict, zstd};
use crate::constants::{OP_S2C_COMPRESSED_TICK, OP_S2C_PAINT_EVENT, OP_S2C_PING};
use crate::types::OutboundFrame;
use crate::ws::codec;
use crate::ws::connection::ConnState;

struct ConnectionCloseLogger {
    conn_id: u64,
    client_ip: IpAddr,
    remote_addr: IpAddr,
    reason: Option<String>,
}

impl ConnectionCloseLogger {
    fn new(conn: &ConnState) -> Self {
        Self {
            conn_id: conn.id,
            client_ip: conn.client_ip,
            remote_addr: conn.peer_addr,
            reason: None,
        }
    }

    fn set_reason(&mut self, reason: impl Into<String>) {
        self.reason = Some(reason.into());
    }
}

impl Drop for ConnectionCloseLogger {
    fn drop(&mut self) {
        let reason = self
            .reason
            .take()
            .unwrap_or_else(|| "dispatcher ended".to_string());
        tracing::info!(
            conn_id = self.conn_id,
            client_ip = %self.client_ip,
            remote_addr = %self.remote_addr,
            reason = reason.as_str(),
            "websocket connection closed"
        );
    }
}

fn filtered_payload(
    frame: &OutboundFrame,
    allowed_ips: &std::collections::HashSet<std::net::IpAddr>,
) -> Option<bytes::Bytes> {
    if let Some(events) = frame.events.as_ref() {
        let mut buf = BytesMut::with_capacity(events.len() * 8);
        for event in events.iter() {
            if allowed_ips.contains(&event.ip) {
                let (x, y) = event.idx.to_coords();
                let (r, g, b) = event.rgba.components();
                buf.put_u8(OP_S2C_PAINT_EVENT);
                buf.put_u16_le(x);
                buf.put_u16_le(y);
                buf.put_u8(r);
                buf.put_u8(g);
                buf.put_u8(b);
            }
        }
        if buf.is_empty() {
            None
        } else {
            Some(buf.freeze())
        }
    } else {
        Some(frame.bytes.clone())
    }
}

pub struct DispatchContext {
    pub period: Duration,
    pub use_zstd: bool,
    pub zstd_level: i32,
    pub dict: Option<Dict>,
    pub max_latency: Duration,
}

/// Periodically drains the per-connection queue, merges acks and
/// sends either raw 0xFA or compressed tick frames.
pub async fn run_dispatcher(
    mut sink: SplitSink<axum::extract::ws::WebSocket, Message>,
    mut queue: mpsc::Receiver<OutboundFrame>,
    ctx: DispatchContext,
    conn: std::sync::Arc<ConnState>,
) {
    let mut ticker = interval(ctx.period);
    let min_period = Duration::from_millis(5); // Minimum 5ms under backpressure.
    let normal_period = ctx.period;
    let mut close_log = ConnectionCloseLogger::new(&conn);

    loop {
        ticker.tick().await;

        if conn.heartbeat.timed_out().await {
            close_log.set_reason("ping timeout");
            let _ = sink
                .send(Message::Close(Some(CloseFrame {
                    code: 1001,
                    reason: Cow::Borrowed("Ping timeout"),
                })))
                .await;
            return;
        }

        if conn.close_flag.load(Ordering::Relaxed) {
            let (code, reason) = conn.close_reason().unwrap_or((1011, "Closing"));
            close_log.set_reason(format!("server scheduled close ({code}: {reason})"));
            let _ = sink
                .send(Message::Close(Some(CloseFrame {
                    code,
                    reason: Cow::Borrowed(reason),
                })))
                .await;
            return;
        }

        // Enforce max latency for the first pending ACK (slow-consumer protection).
        if ctx.max_latency.as_millis() > 0 {
            let first_pending_at = {
                let guard = conn.first_pending_at.lock();
                *guard
            };
            if let Some(first_pending_at) = first_pending_at {
                let latency = std::time::Instant::now().duration_since(first_pending_at);
                if latency > ctx.max_latency {
                    tracing::warn!(
                        conn_id = conn.id,
                        client_ip = %conn.client_ip,
                        latency_ms = latency.as_millis(),
                        threshold_ms = ctx.max_latency.as_millis(),
                        "client latency exceeded threshold, closing connection"
                    );
                    close_log.set_reason("client latency exceeded threshold");
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code: 1011,
                            reason: Cow::Borrowed("Latency exceeded"),
                        })))
                        .await;
                    return;
                }
            }
        }

        // Adjust flush period based on ACK buffer pressure.
        let ack_pressure = {
            let pending = conn.pending_acks.lock();
            let len = pending.len();
            drop(pending);
            len
        };

        let threshold = conn.max_ack_buffer_bytes / 4; // 25% capacity threshold.
        let new_period = if ack_pressure > threshold {
            // Under backpressure: flush faster.
            min_period
        } else {
            // Normal: use the configured period.
            normal_period
        };

        // If period changes, recreate ticker.
        if (new_period == min_period && normal_period != min_period)
            || (new_period == normal_period && ack_pressure <= threshold / 2)
        {
            ticker = interval(new_period);
        }

        // Drain queue non-blocking.
        let mut tick_frames: Vec<OutboundFrame> = Vec::new();
        let mut misc_frames: Vec<OutboundFrame> = Vec::new();
        while let Ok(frame) = queue.try_recv() {
            // Identify ack frames by opcode prefix 0xFF
            if !frame.bytes.is_empty() && frame.bytes[0] == OP_S2C_PAINT_EVENT {
                tick_frames.push(frame);
            } else if !frame.bytes.is_empty() && frame.bytes[0] == OP_S2C_PING {
                misc_frames.push(frame);
            } else if !frame.bytes.is_empty() && frame.bytes[0] == OP_S2C_COMPRESSED_TICK {
                tick_frames.push(frame);
            } else {
                // Unknown payload: treat as tick frame to preserve ordering.
                tick_frames.push(frame);
            }
        }

        let ack_bytes = {
            let mut pending = conn.pending_acks.lock();
            if pending.is_empty() {
                None
            } else {
                let bytes = pending.split().freeze();
                // ACKs drained; reset the timestamp.
                *conn.first_pending_at.lock() = None;
                Some(bytes)
            }
        };

        // Nothing to send this period.
        if ack_bytes.is_none() && tick_frames.is_empty() && misc_frames.is_empty() {
            continue;
        }

        // First, flush ACKs (coalesced) in a single frame.
        if let Some(pending) = ack_bytes {
            if sink.send(Message::Binary(pending.to_vec())).await.is_err() {
                close_log.set_reason("failed to send ack frame");
                return;
            }
        }

        // Send misc frames (e.g., ping) in FIFO order
        for frame in misc_frames {
            if sink
                .send(Message::Binary(frame.bytes.to_vec()))
                .await
                .is_err()
            {
                close_log.set_reason("failed to send misc frame");
                return;
            }
        }

        // Send tick frames: compress if enabled, otherwise send raw
        for frame in tick_frames {
            let Some(payload) = conn
                .watch_ips
                .as_ref()
                .map(|allowed| filtered_payload(&frame, allowed))
                .unwrap_or_else(|| Some(frame.bytes.clone()))
            else {
                continue;
            };

            if ctx.use_zstd {
                // Compress entire tick data
                let raw = payload.clone();
                match zstd::compress_frame_cached(
                    frame.tick.0,
                    raw.as_ref(),
                    ctx.zstd_level,
                    ctx.dict.as_ref(),
                ) {
                    Ok(compressed) => {
                        // Build 0xF7 header + compressed payload
                        let header = codec::encode_compressed_tick_header(
                            frame.tick.0,
                            0, // part_idx (always 0, no splitting)
                            1, // part_total (always 1, no splitting)
                            raw.len() as u32,
                            0, // flags
                        );
                        let mut out =
                            bytes::BytesMut::with_capacity(header.len() + compressed.len());
                        out.extend_from_slice(&header);
                        out.extend_from_slice(&compressed);
                        if sink
                            .send(Message::Binary(out.freeze().to_vec()))
                            .await
                            .is_err()
                        {
                            close_log.set_reason("failed to send compressed tick frame");
                            return;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            conn_id = conn.id,
                            client_ip = %conn.client_ip,
                            error = %err,
                            "failed to compress tick frame"
                        );
                        // Fallback: send uncompressed
                        if sink.send(Message::Binary(payload.to_vec())).await.is_err() {
                            close_log.set_reason("failed to send fallback tick frame");
                            return;
                        }
                    }
                }
            } else {
                // Send uncompressed 0xFA stream directly
                if sink.send(Message::Binary(payload.to_vec())).await.is_err() {
                    close_log.set_reason("failed to send tick frame");
                    return;
                }
            }
        }
    }
}
