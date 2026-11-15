# WebSocket Protocol Specification

## 1. Overview

The WebSocket subsystem handles real-time bidirectional communication for paint operations and board updates.

## 2. Connection Establishment

### 2.1 Endpoint

```
ws[s]://{host}/ws[?params]
```

### 2.2 URL Parameters

| Parameter   | Values                        | Default | Description                                                           |
| ----------- | ----------------------------- | ------- | --------------------------------------------------------------------- |
| `enc`       | `none`, `zstd`                | `none`  | Server-to-client compression mode                                     |
| `dict`      | `<id>`                        | -       | Zstd dictionary identifier                                            |
| `readonly`  | `1`                           | -       | Read-only mode (no paint allowed)                                     |
| `writeonly` | `1`                           | -       | Write-only mode (no broadcast received)                               |
| `watch_ips` | comma-separated `IpAddr` list | -       | Admin-only filter restricting which source IPs are forwarded          |
| `admin_key` | `<string>`                    | -       | Admin credential for watch filter when `X-Admin-Key` header is absent |
| `tps`       | `<number>`                    | -       | Informational only; server ignores                                    |

**Mode Rules:**

- If neither `readonly` nor `writeonly` is set: read-write mode
- `readonly=1` and `writeonly=1` are mutually exclusive
- If both are present, the server returns HTTP 400 and does not upgrade the connection.

### 2.3 Admin Watch Filter

- Admin authorization requirement: The HTTP upgrade request must supply a credential matching `admin_manage_key`.
  - If `X-Admin-Key` header is present and non-empty, only that value is checked.
  - Otherwise, the `admin_key` query parameter is checked.
- When admin authorization is valid and `watch_ips` query parameter is present:
  - The parameter value is split by commas; each segment is parsed as `IpAddr` (IPv4 or IPv6). Invalid segments are ignored.
  - The connection's own resolved client IP is automatically added to the allowlist.
  - Only paint events whose source IP is in the allowlist are forwarded to this WebSocket.
  - Compression is disabled for the connection even if `enc=zstd` and `dict` are specified; the server always sends uncompressed `0xFA` streams.
  - The watch filter is activated whenever `watch_ips` is present and admin authorization is valid.

### 2.4 Dictionary Negotiation

- `dict=<id>` is only considered when `enc=zstd`.
- If `enc=zstd` AND `dict=<id>` is present AND the server dictionary is available:
  1. Server sends a `0xF6` dictionary frame immediately after upgrade.
  2. All subsequent `0xF7` frames for this connection use this dictionary.
- If `enc=zstd` but `dict` is absent or the server dictionary is unavailable, the server compresses without a dictionary and does not send `0xF6`.

### 2.5 Origin Policy

- The server does not restrict `Origin` values; all WebSocket upgrade requests are accepted regardless of origin.
- The upgrade response inherits the HTTP CORS policy and includes `Access-Control-Allow-Origin: *`.

## 3. Message Opcodes

### 3.1 Opcode Table

| Opcode | Direction | Name                  | Length       |
| ------ | --------- | --------------------- | ------------ |
| `0xFE` | C→S       | Paint Request         | 31 bytes     |
| `0xFF` | S→C       | Paint Result (ACK)    | 6 bytes      |
| `0xFA` | S→C       | Paint Event           | 8 bytes      |
| `0xF7` | S→C       | Compressed Tick Frame | 18 + payload |
| `0xF6` | S→C       | Dictionary Frame      | variable     |
| `0xFC` | S→C       | Ping                  | 1 byte       |
| `0xFB` | C→S       | Pong                  | 1 byte       |

## 4. Message Formats

### 4.1 Paint Request (0xFE) - Client to Server

**Total Length:** 31 bytes (fixed)

| Offset | Size | Type   | Field                      |
| ------ | ---- | ------ | -------------------------- |
| 0      | 1    | u8     | Opcode (`0xFE`)            |
| 1      | 2    | u16_le | x coordinate               |
| 3      | 2    | u16_le | y coordinate               |
| 5      | 1    | u8     | Red value                  |
| 6      | 1    | u8     | Green value                |
| 7      | 1    | u8     | Blue value                 |
| 8      | 3    | u24_le | UID (lower 24 bits of u32) |
| 11     | 16   | bytes  | Token (UUID binary)        |
| 27     | 4    | u32_le | Client request ID          |

**UID Encoding:**

- The UID is a 32-bit unsigned integer, but only the lower 24 bits are transmitted
- Bytes 8-10 contain the UID in little-endian order: `[uid & 0xFF, (uid >> 8) & 0xFF, (uid >> 16) & 0xFF]`
- Maximum representable UID: 16,777,215 (2^24 - 1)

**Validation:**

1. Packet length MUST be exactly 31 bytes
2. `x` MUST be in range `[0, 999]`
3. `y` MUST be in range `[0, 599]`
4. Token MUST exist and not be revoked
5. Token's UID (lower 24 bits) MUST match the provided UID
6. Connection mode MUST NOT be `readonly`
7. Current server time MUST be within the configured event time range (if `start_timestamp` or `end_timestamp` are non-zero)

### 4.2 Paint Result (0xFF) - Server to Client

**Total Length:** 6 bytes (fixed)

| Offset | Size | Type   | Field             |
| ------ | ---- | ------ | ----------------- |
| 0      | 1    | u8     | Opcode (`0xFF`)   |
| 1      | 4    | u32_le | Client request ID |
| 5      | 1    | u8     | Status code       |

**Status Codes:**

| Code   | Name          | Description              |
| ------ | ------------- | ------------------------ |
| `0xEF` | OK            | Successfully enqueued    |
| `0xEE` | COOLING       | Cooldown period active   |
| `0xED` | INVALID_TOKEN | Token invalid or expired |
| `0xEC` | BAD_REQUEST   | Malformed request        |
| `0xEB` | FORBIDDEN     | Operation not permitted  |
| `0xEA` | SERVER_ERROR  | Internal server error    |

### 4.3 Paint Event (0xFA) - Server to Client

**Total Length:** 8 bytes (fixed)

| Offset | Size | Type   | Field           |
| ------ | ---- | ------ | --------------- |
| 0      | 1    | u8     | Opcode (`0xFA`) |
| 1      | 2    | u16_le | x coordinate    |
| 3      | 2    | u16_le | y coordinate    |
| 5      | 1    | u8     | Red value       |
| 6      | 1    | u8     | Green value     |
| 7      | 1    | u8     | Blue value      |

**Semantics:**

- Represents a committed pixel change
- Sent to all connections (except `writeonly`)
- Multiple events may be concatenated in one WebSocket frame

### 4.4 Compressed Tick Frame (0xF7) - Server to Client

**Header Length:** 18 bytes
**Payload:** Variable (zstd compressed `0xFA` events)

| Offset | Size | Type   | Field                         |
| ------ | ---- | ------ | ----------------------------- |
| 0      | 1    | u8     | Opcode (`0xF7`)               |
| 1      | 8    | u64_le | Tick index                    |
| 9      | 2    | u16_le | Part index (always 0)         |
| 11     | 2    | u16_le | Part total (always 1)         |
| 13     | 4    | u32_le | Uncompressed payload length   |
| 17     | 1    | u8     | Flags (reserved, currently 0) |
| 18     | N    | bytes  | Zstd compressed payload       |

**Payload Content:** Concatenated `0xFA` paint events (after decompression)

**Invariants:**

- Only sent when `enc=zstd`
- `part_index = 0` (no fragmentation)
- `part_total = 1` (single part)

### 4.5 Dictionary Frame (0xF6) - Server to Client

| Offset | Size | Type   | Field                               |
| ------ | ---- | ------ | ----------------------------------- |
| 0      | 1    | u8     | Opcode (`0xF6`)                     |
| 1      | 4    | u32_le | Dictionary ID (echo of `dict=<id>`) |
| 5      | 4    | u32_le | Dictionary byte length `N`          |
| 9      | N    | bytes  | Raw dictionary content              |

**Timing:** Sent once immediately after WebSocket upgrade (if dictionary negotiated)

### 4.6 Ping (0xFC) - Server to Client

**Total Length:** 1 byte

| Offset | Size | Type | Field           |
| ------ | ---- | ---- | --------------- |
| 0      | 1    | u8   | Opcode (`0xFC`) |

### 4.7 Pong (0xFB) - Client to Server

**Total Length:** 1 byte

| Offset | Size | Type | Field           |
| ------ | ---- | ---- | --------------- |
| 0      | 1    | u8   | Opcode (`0xFB`) |

**Requirement:** Client MUST respond with `0xFB` immediately upon receiving `0xFC`

## 5. Connection Lifecycle

### 5.1 Outbound Queue

**Limit:** 32 frames per connection

**Behavior:** If queue exceeds limit, connection is terminated with close code 1011

### 5.2 ACK Buffer

**Limit:** `max_ack_buffer_bytes` (configurable)

**Behavior:** If buffer exceeds limit, connection is terminated

### 5.3 WebSocket Close Codes

| Code | Name             | Cause                                                                               |
| ---- | ---------------- | ----------------------------------------------------------------------------------- |
| 1001 | Going Away       | Ping timeout (no pong received)                                                     |
| 1002 | Protocol Error   | Unknown opcode, unexpected pong, incomplete packet                                  |
| 1008 | Policy Violation | Reserved (IP connection limits are enforced by rejecting the upgrade with HTTP 429) |
| 1009 | Message Too Big  | Reserved                                                                            |
| 1011 | Internal Error   | Slow consumer / outbound queue overflow / server error                              |

## 6. Unknown Opcode Handling

### 6.1 Server Behavior

When server receives unknown C→S opcode:

1. Close connection immediately
2. Send close code `1002` (Protocol Error)

### 6.2 Client Compatibility

Server MUST NOT send `0xF7` or `0xF6` to connections where `enc=none`

## 7. Message Batching

### 7.1 Server to Client

- Paint events (`0xFA`) for a single tick are concatenated into one WebSocket frame
- ACKs (`0xFF`) are batched and sent together
- Compressed frames (`0xF7`) contain all tick events

### 7.2 Client to Server

- Multiple paint requests MAY be sent in a single WebSocket frame
- Each request MUST be exactly 31 bytes
