use std::net::IpAddr;
use std::sync::Arc;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::{BOARD_HEIGHT, BOARD_WIDTH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rgba(pub u32);

impl Rgba {
    pub fn components(self) -> (u8, u8, u8) {
        let r = ((self.0 >> 16) & 0xFF) as u8;
        let g = ((self.0 >> 8) & 0xFF) as u8;
        let b = (self.0 & 0xFF) as u8;
        (r, g, b)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl From<Rgb> for Rgba {
    fn from(rgb: Rgb) -> Self {
        let Rgb(r, g, b) = rgb;
        Rgba(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PixelIndex(pub u32);

impl PixelIndex {
    pub fn from_coords(x: u16, y: u16) -> Option<Self> {
        if x as u32 >= BOARD_WIDTH || y as u32 >= BOARD_HEIGHT {
            return None;
        }
        Some(Self(y as u32 * BOARD_WIDTH + x as u32))
    }

    pub fn to_coords(self) -> (u16, u16) {
        let x = (self.0 % BOARD_WIDTH) as u16;
        let y = (self.0 / BOARD_WIDTH) as u16;
        (x, y)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct TickIndex(pub u64);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct IngestSeq(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaintCommand {
    pub x: u16,
    pub y: u16,
    pub rgb: Rgb,
    pub uid: u32,
    pub token: Uuid,
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaintAck {
    pub id: u32,
    pub status: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WsMode {
    Readonly,
    Writeonly,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Enc {
    None,
    Zstd { dict_id: Option<u32> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnCaps {
    pub enc: Enc,
    pub mode: WsMode,
}

#[derive(Debug, Clone)]
pub struct WriteEntry {
    pub idx: PixelIndex,
    pub rgba: Rgba,
    pub seq: IngestSeq,
    pub uid: u32,
    pub ip: IpAddr,
}

#[derive(Debug, Clone, Copy)]
pub struct PaintEvent {
    pub idx: PixelIndex,
    pub rgba: Rgba,
    pub uid: u32,
    pub ip: IpAddr,
}

#[derive(Debug, Clone)]
pub struct OutboundFrame {
    pub bytes: Bytes,
    pub is_compressed: bool,
    pub tick: TickIndex,
    pub part_idx: u16,
    pub part_total: u16,
    pub events: Option<Arc<Vec<PaintEvent>>>,
}
