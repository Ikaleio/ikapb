use bytes::{BufMut, Bytes, BytesMut};

pub fn write_u16_le(buf: &mut BytesMut, value: u16) {
    buf.put_u16_le(value);
}

pub fn write_u32_le(buf: &mut BytesMut, value: u32) {
    buf.put_u32_le(value);
}

pub fn join_bytes(chunks: &[Bytes]) -> Bytes {
    let total: usize = chunks.iter().map(|b| b.len()).sum();
    let mut buf = BytesMut::with_capacity(total);
    for chunk in chunks {
        buf.extend_from_slice(chunk);
    }
    buf.freeze()
}
