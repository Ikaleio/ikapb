use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use bytes::{BufMut, Bytes, BytesMut};

#[derive(Debug, Clone)]
pub struct Dict {
    pub id: u32,
    bytes: Bytes,
}

impl Dict {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub fn load_dict(path: &Path, id: u32) -> Result<Option<Dict>> {
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read(path).with_context(|| format!("failed to read dict {}", path.display()))?;
    Ok(Some(Dict {
        id,
        bytes: Bytes::from(data),
    }))
}

pub fn encode_dict_frame(dict: &Dict) -> Bytes {
    let mut buf = BytesMut::with_capacity(1 + 4 + 4 + dict.bytes.len());
    buf.put_u8(crate::constants::OP_S2C_DICT);
    buf.put_u32_le(dict.id);
    buf.put_u32_le(dict.bytes.len() as u32);
    buf.extend_from_slice(&dict.bytes);
    buf.freeze()
}
