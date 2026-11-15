use thiserror::Error;

#[derive(Debug, Error)]
pub enum PaintboardError {
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("authentication error: {0}")]
    Auth(String),
    #[error("cooldown violation: {0}")]
    Cooldown(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("configuration error: {0}")]
    Config(String),
}

impl PaintboardError {
    pub fn to_ack_code(&self) -> u8 {
        use crate::constants::*;
        match self {
            PaintboardError::Protocol(_) => ACK_BAD_REQ,
            PaintboardError::Auth(_) => ACK_INVALID_TOKEN,
            PaintboardError::Cooldown(_) => ACK_COOLING,
            PaintboardError::Io(_) | PaintboardError::Db(_) | PaintboardError::Config(_) => {
                ACK_SERVER_ERR
            }
        }
    }
}
