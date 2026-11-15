use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use bytes::Bytes;
use parking_lot::RwLock;
use tokio::sync::watch;

use crate::constants::{BOARD_HEIGHT, BOARD_WIDTH};
use crate::types::{PixelIndex, Rgba};

#[derive(Debug)]
pub struct Board {
    rgba: Vec<AtomicU32>,
    shadow_rgb: Option<RwLock<Vec<u8>>>,
    tick_notifier: watch::Sender<u64>,
}

pub type BoardHandle = Arc<Board>;

impl Board {
    pub fn new(with_shadow: bool, initial_color: Rgba) -> BoardHandle {
        let rgba = (0..(BOARD_WIDTH * BOARD_HEIGHT))
            .map(|_| AtomicU32::new(initial_color.0))
            .collect();
        let shadow_rgb = if with_shadow {
            let (r, g, b) = initial_color.components();
            let mut buf = vec![0u8; (BOARD_WIDTH * BOARD_HEIGHT * 3) as usize];
            for chunk in buf.chunks_exact_mut(3) {
                chunk[0] = r;
                chunk[1] = g;
                chunk[2] = b;
            }
            Some(RwLock::new(buf))
        } else {
            None
        };
        let (tx, _rx) = watch::channel(0u64);
        Arc::new(Self {
            rgba,
            shadow_rgb,
            tick_notifier: tx,
        })
    }

    pub fn store(&self, idx: PixelIndex, rgba: Rgba) {
        if let Some(shadow) = &self.shadow_rgb {
            let mut guard = shadow.write();
            let base = (idx.0 * 3) as usize;
            guard[base] = ((rgba.0 >> 16) & 0xFF) as u8;
            guard[base + 1] = ((rgba.0 >> 8) & 0xFF) as u8;
            guard[base + 2] = (rgba.0 & 0xFF) as u8;
        }
        if let Some(cell) = self.rgba.get(idx.0 as usize) {
            cell.store(rgba.0, Ordering::Relaxed);
        }
    }

    pub fn snapshot_rgb24(&self) -> Bytes {
        if let Some(shadow) = &self.shadow_rgb {
            let guard = shadow.read();
            return Bytes::copy_from_slice(&guard);
        }

        let mut buffer = vec![0u8; (BOARD_WIDTH * BOARD_HEIGHT * 3) as usize];
        for (i, cell) in self.rgba.iter().enumerate() {
            let value = cell.load(Ordering::Relaxed);
            let base = i * 3;
            buffer[base] = ((value >> 16) & 0xFF) as u8;
            buffer[base + 1] = ((value >> 8) & 0xFF) as u8;
            buffer[base + 2] = (value & 0xFF) as u8;
        }
        Bytes::from(buffer)
    }

    pub fn subscribe_tick(&self) -> watch::Receiver<u64> {
        self.tick_notifier.subscribe()
    }

    pub fn publish_tick(&self, tick: u64) {
        let _ = self.tick_notifier.send(tick);
    }

    /// SPEC 5.1: Restore the board from an RGB24 snapshot.
    pub fn restore_from_rgb24(&self, data: &[u8]) {
        let expected_len = (BOARD_WIDTH * BOARD_HEIGHT * 3) as usize;
        if data.len() != expected_len {
            tracing::warn!(
                "snapshot data length mismatch: expected {}, got {}",
                expected_len,
                data.len()
            );
            return;
        }

        for (i, chunk) in data.chunks_exact(3).enumerate() {
            if i >= self.rgba.len() {
                break;
            }
            let r = chunk[0];
            let g = chunk[1];
            let b = chunk[2];
            let rgba_val = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            self.rgba[i].store(rgba_val, Ordering::Relaxed);
        }

        if let Some(shadow) = &self.shadow_rgb {
            let mut guard = shadow.write();
            guard.copy_from_slice(data);
        }
    }
}
