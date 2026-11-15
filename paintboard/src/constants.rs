pub const BOARD_WIDTH: u32 = 1000;
pub const BOARD_HEIGHT: u32 = 600;

pub const OP_S2C_PING: u8 = 0xFC;
pub const OP_C2S_PONG: u8 = 0xFB;
pub const OP_S2C_PAINT_EVENT: u8 = 0xFA;
pub const OP_C2S_PAINT: u8 = 0xFE;
pub const OP_S2C_PAINT_RESULT: u8 = 0xFF;
pub const OP_S2C_DICT: u8 = 0xF6;
pub const OP_S2C_COMPRESSED_TICK: u8 = 0xF7;

pub const ACK_OK: u8 = 0xEF;
pub const ACK_COOLING: u8 = 0xEE;
pub const ACK_INVALID_TOKEN: u8 = 0xED;
pub const ACK_BAD_REQ: u8 = 0xEC;
pub const ACK_FORBIDDEN: u8 = 0xEB;
pub const ACK_SERVER_ERR: u8 = 0xEA;
pub const ACK_RATE_LIMITED: u8 = 0xE9;
pub const CLOSE_PING_TIMEOUT: &str = "1001 Ping timeout";
pub const CLOSE_PROTOCOL_VIOLATION: &str = "1002 Protocol violation";
pub const CLOSE_IP_LIMIT: &str = "1008 IP connection limit exceeded";
pub const CLOSE_MESSAGE_TOO_BIG: &str = "1009 Message too big";
pub const CLOSE_SERVER_ERROR: &str = "1011 Server error processing message";
