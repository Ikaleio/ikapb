use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::constants::{
    OP_C2S_PAINT, OP_C2S_PONG, OP_S2C_COMPRESSED_TICK, OP_S2C_PAINT_EVENT, OP_S2C_PAINT_RESULT,
    OP_S2C_PING,
};
use crate::types::{PaintAck, Rgb};

#[derive(Debug, Clone)]
pub enum Packet {
    Paint {
        x: u16,
        y: u16,
        rgb: Rgb,
        uid: u32,
        token: [u8; 16],
        id: u32,
    },
    Pong,
}

#[derive(Debug)]
pub enum DecodeError {
    UnknownOpcode(u8),
    Incomplete,
}

pub fn decode_packets(mut buf: Bytes) -> Result<Vec<Packet>, DecodeError> {
    let mut packets = Vec::new();
    while buf.remaining() > 0 {
        let opcode = buf.get_u8();
        match opcode {
            OP_C2S_PONG => packets.push(Packet::Pong),
            OP_C2S_PAINT => {
                if buf.remaining() < 30 {
                    return Err(DecodeError::Incomplete);
                }
                let x = buf.get_u16_le();
                let y = buf.get_u16_le();
                let r = buf.get_u8();
                let g = buf.get_u8();
                let b = buf.get_u8();
                let uid = {
                    let mut bytes = [0u8; 4];
                    bytes[0] = buf.get_u8();
                    bytes[1] = buf.get_u8();
                    bytes[2] = buf.get_u8();
                    bytes
                };
                let mut token = [0u8; 16];
                buf.copy_to_slice(&mut token);
                let id = buf.get_u32_le();
                packets.push(Packet::Paint {
                    x,
                    y,
                    rgb: Rgb(r, g, b),
                    uid: u32::from_le_bytes(uid),
                    token,
                    id,
                });
            }
            _ => return Err(DecodeError::UnknownOpcode(opcode)),
        }
    }
    Ok(packets)
}

pub fn encode_paint_ack(ack: PaintAck) -> Bytes {
    let mut buf = BytesMut::with_capacity(1 + 4 + 1);
    buf.put_u8(OP_S2C_PAINT_RESULT);
    buf.put_u32_le(ack.id);
    buf.put_u8(ack.status);
    buf.freeze()
}

pub fn encode_paint_event(x: u16, y: u16, r: u8, g: u8, b: u8) -> Bytes {
    let mut buf = BytesMut::with_capacity(1 + 2 + 2 + 3);
    buf.put_u8(OP_S2C_PAINT_EVENT);
    buf.put_u16_le(x);
    buf.put_u16_le(y);
    buf.put_u8(r);
    buf.put_u8(g);
    buf.put_u8(b);
    buf.freeze()
}

pub fn encode_compressed_tick_header(
    tick: u64,
    part_idx: u16,
    part_total: u16,
    raw_len: u32,
    flags: u8,
) -> Bytes {
    let mut buf = BytesMut::with_capacity(1 + 8 + 2 + 2 + 4 + 1);
    buf.put_u8(OP_S2C_COMPRESSED_TICK);
    buf.put_u64_le(tick);
    buf.put_u16_le(part_idx);
    buf.put_u16_le(part_total);
    buf.put_u32_le(raw_len);
    buf.put_u8(flags);
    buf.freeze()
}

pub fn encode_ping() -> Bytes {
    let mut buf = BytesMut::with_capacity(1);
    buf.put_u8(OP_S2C_PING);
    buf.freeze()
}
