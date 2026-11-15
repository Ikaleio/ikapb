//! Integration tests for spec compliance.

use bytes::Bytes;
use paintboard::tick::coordinator::TickCoordinator;
use paintboard::ws::codec;

#[test]
fn test_compressed_tick_header_u64() {
    let tick = 0x123456789ABCDEFu64;
    let header = codec::encode_compressed_tick_header(tick, 0, 1, 100, 0);

    // opcode(1) + tick(8) + part_idx(2) + part_total(2) + raw_len(4) + flags(1)
    assert_eq!(header.len(), 18);

    let tick_bytes = &header[1..9];
    let decoded_tick = u64::from_le_bytes(tick_bytes.try_into().unwrap());
    assert_eq!(decoded_tick, tick);
}

#[test]
fn test_unknown_opcode_returns_error() {
    let data = vec![0xFF];
    let result = codec::decode_packets(Bytes::from(data));

    assert!(result.is_err());
    match result {
        Err(codec::DecodeError::UnknownOpcode(0xFF)) => {}
        _ => panic!("Expected UnknownOpcode error"),
    }
}

#[test]
fn test_incomplete_packet_returns_error() {
    let mut data = vec![0xFE];
    // Paint opcode without enough bytes.
    data.extend_from_slice(&[0u8; 10]);
    let result = codec::decode_packets(Bytes::from(data));

    assert!(result.is_err());
    match result {
        Err(codec::DecodeError::Incomplete) => {}
        _ => panic!("Expected Incomplete error"),
    }
}

#[test]
fn test_epsilon_soft_window() {
    let coord = TickCoordinator::new(128); // 128 TPS
    coord.set_epsilon_ns(500_000); // 0.5ms

    let current_tick = coord.tick_index();

    let duration_ns = 1_000_000_000u64 / 128;
    let last_tick_start = 0u64;
    let deadline = last_tick_start + duration_ns;

    // Before the soft window: should stay in the current tick.
    let before_window = deadline - 1_000_000; // 1ms before deadline
    let tick1 = coord.tick_for_timestamp(before_window, last_tick_start);
    assert_eq!(tick1, current_tick);

    // Inside the soft window: may be attributed to the next tick.
    let in_window = deadline - 300_000; // 0.3ms before deadline (within epsilon)
    let tick2 = coord.tick_for_timestamp(in_window, last_tick_start);
    assert_eq!(tick2.0, current_tick.0 + 1);

    // After the deadline: should be attributed to the next tick.
    let after_deadline = deadline + 100_000; // 0.1ms after deadline
    let tick3 = coord.tick_for_timestamp(after_deadline, last_tick_start);
    assert_eq!(tick3.0, current_tick.0 + 1);
}

#[test]
fn test_gettoken_response_has_code() {
    use paintboard::http::gettoken::ApiResponse;
    use serde_json;

    let response = ApiResponse {
        code: 200,
        data: paintboard::http::gettoken::TokenResponse {
            token: "test-token".to_string(),
        },
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"code\":200"));
    assert!(json.contains("\"data\""));
}
