use std::collections::HashMap;
use std::net::IpAddr;

use crate::types::{PixelIndex, Rgba, WriteEntry};

#[derive(Default)]
pub struct ShardWriteSet {
    active: HashMap<PixelIndex, WriteEntry>,
    frozen: HashMap<PixelIndex, WriteEntry>,
    /// entries explicitly scheduled for the next tick (soft-window assignment)
    next: HashMap<PixelIndex, WriteEntry>,
}

impl ShardWriteSet {
    /// Insert into current active buffer (default)
    pub fn insert(&mut self, entry: WriteEntry) {
        match self.active.get(&entry.idx) {
            Some(existing) if existing.seq.0 >= entry.seq.0 => {}
            _ => {
                self.active.insert(entry.idx, entry);
            }
        }
    }

    /// Insert into the buffer scheduled for the next tick (soft-window target)
    pub fn insert_next(&mut self, entry: WriteEntry) {
        match self.next.get(&entry.idx) {
            Some(existing) if existing.seq.0 >= entry.seq.0 => {}
            _ => {
                self.next.insert(entry.idx, entry);
            }
        }
    }

    pub fn flip(&mut self) {
        // Move current active into frozen for draining
        std::mem::swap(&mut self.active, &mut self.frozen);
        // Promote the 'next' buffer to become the new active
        self.active = std::mem::take(&mut self.next);
        // ensure next is empty
        self.next = HashMap::new();
    }

    pub fn drain_sorted(&mut self) -> Vec<(PixelIndex, Rgba, u32, IpAddr)> {
        let mut entries: Vec<_> = self
            .frozen
            .drain()
            .map(|(_idx, entry)| (entry.idx, entry.rgba, entry.uid, entry.ip))
            .collect();
        entries.sort_by_key(|(idx, _, _, _)| idx.0);
        entries
    }
}
