use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::{BufMut, BytesMut};
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::board::memory::BoardHandle;
use crate::board::wal;
use crate::compress::sampler::ZstdSampler;
use crate::constants::OP_S2C_PAINT_EVENT;
use crate::errors::PaintboardError;
use crate::tick::coordinator::TickCoordinator;
use crate::tick::write_set::ShardWriteSet;
use crate::types::{
    IngestSeq, OutboundFrame, PaintEvent, PixelIndex, Rgb, Rgba, TickIndex, WriteEntry,
};

pub struct PaintPipeline {
    board: BoardHandle,
    shards: Vec<Mutex<ShardWriteSet>>,
    broadcaster: broadcast::Sender<OutboundFrame>,
    coordinator: Arc<TickCoordinator>,
    next_seq: AtomicU64,
    wal_dir: PathBuf,
    wal_flush_every_ticks: u32,
    sampler: Option<ZstdSampler>,
}

impl PaintPipeline {
    pub fn new(
        board: BoardHandle,
        coordinator: Arc<TickCoordinator>,
        shard_count: usize,
        wal_dir: PathBuf,
        wal_flush_every_ticks: u32,
        sampler: Option<ZstdSampler>,
    ) -> Arc<Self> {
        let shard_count = shard_count.max(1);
        let shards = (0..shard_count)
            .map(|_| Mutex::new(ShardWriteSet::default()))
            .collect();
        let (tx, _rx) = broadcast::channel(128);
        Arc::new(Self {
            board,
            shards,
            broadcaster: tx,
            coordinator,
            next_seq: AtomicU64::new(0),
            wal_dir,
            wal_flush_every_ticks,
            sampler,
        })
    }

    pub fn spawn(self: Arc<Self>) {
        let coordinator = self.coordinator.clone();
        let pipeline = self.clone();
        tokio::spawn(async move {
            coordinator
                .run(move |tick| {
                    pipeline.handle_tick(tick);
                })
                .await;
        });
    }

    pub fn ingest(
        &self,
        x: u16,
        y: u16,
        rgb: Rgb,
        uid: u32,
        ip: IpAddr,
    ) -> Result<IngestSeq, PaintboardError> {
        let idx = PixelIndex::from_coords(x, y).ok_or_else(|| {
            PaintboardError::Protocol(format!("pixel out of bounds: ({}, {})", x, y))
        })?;
        let rgba = Rgba::from(rgb);
        let seq = IngestSeq(self.next_seq.fetch_add(1, Ordering::Relaxed) + 1);
        let shard_idx = (idx.0 as usize) % self.shards.len();
        let mut shard = self.shards[shard_idx].lock();
        shard.insert(WriteEntry {
            idx,
            rgba,
            seq,
            uid,
            ip,
        });
        Ok(seq)
    }

    /// Ingest but target the next tick (soft-window assignment)
    pub fn ingest_next(
        &self,
        x: u16,
        y: u16,
        rgb: Rgb,
        uid: u32,
        ip: IpAddr,
    ) -> Result<IngestSeq, PaintboardError> {
        let idx = PixelIndex::from_coords(x, y).ok_or_else(|| {
            PaintboardError::Protocol(format!("pixel out of bounds: ({}, {})", x, y))
        })?;
        let rgba = Rgba::from(rgb);
        let seq = IngestSeq(self.next_seq.fetch_add(1, Ordering::Relaxed) + 1);
        let shard_idx = (idx.0 as usize) % self.shards.len();
        let mut shard = self.shards[shard_idx].lock();
        shard.insert_next(WriteEntry {
            idx,
            rgba,
            seq,
            uid,
            ip,
        });
        Ok(seq)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OutboundFrame> {
        self.broadcaster.subscribe()
    }

    pub fn current_tick(&self) -> TickIndex {
        self.coordinator.tick_index()
    }

    /// Return coordinator's current tick start ns
    pub fn current_tick_start_ns(&self) -> u64 {
        self.coordinator.current_tick_start_ns()
    }

    /// Ask coordinator which tick the provided timestamp should map to (considers epsilon)
    pub fn tick_for_timestamp(&self, now_ns: u64) -> TickIndex {
        let last_start = self.coordinator.current_tick_start_ns();
        self.coordinator.tick_for_timestamp(now_ns, last_start)
    }

    pub fn handle_tick(&self, tick: TickIndex) {
        let mut entries = Vec::new();
        for shard in &self.shards {
            let mut guard = shard.lock();
            guard.flip();
            entries.extend(guard.drain_sorted());
        }

        if entries.is_empty() {
            self.board.publish_tick(tick.0);
            return;
        }

        let wal_entries: Vec<_> = entries
            .iter()
            .map(|(idx, rgba, uid, _)| (*idx, *rgba, *uid))
            .collect();

        let mut raw = BytesMut::with_capacity(entries.len() * 8);
        let mut events = Vec::with_capacity(entries.len());
        for (idx, rgba, uid, ip) in &entries {
            self.board.store(*idx, *rgba);
            let (x, y) = idx.to_coords();
            let (r, g, b) = rgba.components();
            raw.put_u8(OP_S2C_PAINT_EVENT);
            raw.put_u16_le(x);
            raw.put_u16_le(y);
            raw.put_u8(r);
            raw.put_u8(g);
            raw.put_u8(b);
            events.push(PaintEvent {
                idx: *idx,
                rgba: *rgba,
                uid: *uid,
                ip: *ip,
            });
        }

        self.board.publish_tick(tick.0);

        if self.wal_flush_every_ticks > 0
            && (tick.0 % self.wal_flush_every_ticks as u64) == 0
            && !wal_entries.is_empty()
        {
            let dir = self.wal_dir.clone();
            let wal_entries = wal_entries;
            tokio::spawn(async move {
                if let Err(err) = wal::append_batch(&dir, tick, &wal_entries).await {
                    tracing::error!("wal append failed: {}", err);
                }
            });
        }

        let bytes = raw.freeze();

        if let Some(ref sampler) = self.sampler {
            sampler.record(tick, bytes.clone());
        }

        let frame = OutboundFrame {
            bytes,
            is_compressed: false,
            tick,
            part_idx: 0,
            part_total: 1,
            events: Some(Arc::new(events)),
        };
        let _ = self.broadcaster.send(frame);
    }

    pub fn apply_admin_fill(&self, updates: Vec<(PixelIndex, Rgba)>) {
        if updates.is_empty() {
            return;
        }

        let tick = self.coordinator.bump_manual_tick();
        let mut raw = BytesMut::with_capacity(updates.len() * 8);
        let mut wal_entries = Vec::with_capacity(updates.len());

        for (idx, rgba) in &updates {
            self.board.store(*idx, *rgba);
            let (x, y) = idx.to_coords();
            let (r, g, b) = rgba.components();
            raw.put_u8(OP_S2C_PAINT_EVENT);
            raw.put_u16_le(x);
            raw.put_u16_le(y);
            raw.put_u8(r);
            raw.put_u8(g);
            raw.put_u8(b);
            wal_entries.push((*idx, *rgba, 0u32));
        }

        self.board.publish_tick(tick.0);

        if self.wal_flush_every_ticks > 0 {
            let dir = self.wal_dir.clone();
            tokio::spawn(async move {
                if let Err(err) = wal::append_batch(&dir, tick, &wal_entries).await {
                    tracing::error!("wal append failed: {}", err);
                }
            });
        }

        let bytes = raw.freeze();

        if let Some(ref sampler) = self.sampler {
            sampler.record(tick, bytes.clone());
        }

        let frame = OutboundFrame {
            bytes,
            is_compressed: false,
            tick,
            part_idx: 0,
            part_total: 1,
            events: None,
        };
        let _ = self.broadcaster.send(frame);
    }
}
