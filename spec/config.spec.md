# Configuration Specification

## 1. Overview

The configuration subsystem manages runtime parameters with hot-reload support.

## 2. Configuration File

### 2.1 Location

- Default: `paintboard/config/default.toml`
- Override: Set `PAINTBOARD_CONFIG` environment variable

### 2.2 Format

TOML format with the following sections:

- Root level: Core parameters
- `[network]`: Network settings
- `[websocket]`: WebSocket settings
- `[persistence]`: Persistence settings
- `[token]`: Token settings
- `[zstd]`: Compression settings
- `[event]`: Event settings

## 3. Parameter Reference

### 3.1 Core Parameters

| Parameter              | Type   | Default            | Description                                                                   |
| ---------------------- | ------ | ------------------ | ----------------------------------------------------------------------------- |
| `bind_address`         | string | `"0.0.0.0:8775"`   | HTTP/WebSocket listen address                                                 |
| `tps`                  | u32    | 128                | Ticks per second                                                              |
| `no_access_key_verify` | bool   | false              | Skip access key verification                                                  |
| `background_color`     | string | `"#AAAAAA"`        | Default board background (hex RGB)                                            |
| `data_dir`             | string | `"data"`           | Data storage directory                                                        |
| `sqlite_path`          | string | `"data/tokens.db"` | SQLite database path                                                          |
| `admin_manage_key`     | string | `""`               | Shared secret required by `/admin/*` APIs; empty string disables admin access |
| `admin_max_blob_bytes` | u64    | 5242880            | Maximum accepted payload for admin image blobs (bytes)                        |

### 3.2 Network Parameters (`[network]`)

| Parameter                | Type     | Default | Description                                      |
| ------------------------ | -------- | ------- | ------------------------------------------------ |
| `trusted_proxies`        | string[] | `[]`    | Trusted reverse proxy IPs                        |
| `max_connections_per_ip` | u32      | -       | Max WebSocket connections per IP (unset/0 = unlimited; tracking still enabled for `/admin/ws/connections`) |

### 3.2.1 Cross-Origin HTTP Policy

- Every HTTP response includes `Access-Control-Allow-Origin: *`.
- Preflight (`OPTIONS`) requests return status `204` with headers:
  - `Access-Control-Allow-Origin: *`
  - `Access-Control-Allow-Methods: GET,POST,PUT,PATCH,DELETE,OPTIONS`
  - `Access-Control-Allow-Headers: <req.header(\"Access-Control-Request-Headers\")>`, if that request header is present; otherwise `*`.
  - `Access-Control-Max-Age: 86400`

### 3.2.2 Client IP Resolution

Some endpoints need a canonical `client_ip` (e.g. WebSocket connection limiting
and token issuance metadata).

Inputs:

- `remote_ip`: the peer socket IP address (the direct TCP peer).
- `trusted_proxies`: `network.trusted_proxies` parsed as `IpAddr`.
- Request headers:
  - `x-forwarded-for` (optional): comma-separated list of IPs.
  - `x-real-ip` (optional): single IP.

Parsing rule for an IP token (used for both headers):

- Trim whitespace.
- Strip leading/trailing single or double quotes.
- Accept formats:
  - `IpAddr` (e.g. `203.0.113.1`, `2001:db8::1`)
  - `SocketAddr` (e.g. `203.0.113.1:12345`, `[2001:db8::1]:12345`) and keep only the IP part
  - Bracketed IPv6 literal `[2001:db8::1]`

Procedure:

1. If `trusted_proxies` is empty OR `remote_ip ∉ trusted_proxies`: return `remote_ip`.
2. If `x-forwarded-for` is present:
   1. Parse each comma-separated token into an `IpAddr` using the parsing rule; discard tokens that fail to parse.
   2. Append `remote_ip` to the parsed list (the proxy chain).
   3. While the last element of the list is in `trusted_proxies`, remove it.
   4. If the list is non-empty: return the first element of the list.
3. If `x-real-ip` is present and parses to `real_ip`:
   - If `real_ip ∉ trusted_proxies`: return `real_ip`.
4. Otherwise return `remote_ip`.

### 3.3 WebSocket Parameters (`[websocket]`)

| Parameter               | Type | Default   | Description                       |
| ----------------------- | ---- | --------- | --------------------------------- |
| `ws_send_period_ms`     | u32  | 20        | Outbound batching period (ms)     |
| `max_ack_buffer_bytes`  | u64  | 209715200 | Max ACK buffer size (bytes)       |
| `max_client_latency_ms` | u32  | 3000      | Max tolerated client latency (ms) |

### 3.4 Persistence Parameters (`[persistence]`)

| Parameter               | Type | Default | Description                              |
| ----------------------- | ---- | ------- | ---------------------------------------- |
| `persist_board`         | bool | false   | Enable board persistence                 |
| `snapshot_interval_sec` | u32  | 10      | Snapshot interval (seconds)              |
| `snapshot_retain`       | u32  | 8       | Max snapshots to retain                  |
| `wal_flush_every_ticks` | u32  | 8       | WAL flush interval (ticks, 0 = disabled) |

### 3.5 Token Parameters (`[token]`)

| Parameter                    | Type | Default | Description                                                     |
| ---------------------------- | ---- | ------- | --------------------------------------------------------------- |
| `global_cooldown_ms`         | i64  | 0       | Default cooldown (ms, 0 = no cooldown)                          |
| `cooldown_flush_interval_ms` | u32  | 0       | Deprecated; cooldown state is only flushed on graceful shutdown |

### 3.6 Zstd Parameters (`[zstd]`)

| Parameter                     | Type   | Default                | Description                            |
| ----------------------------- | ------ | ---------------------- | -------------------------------------- |
| `zstd_level`                  | i32    | 5                      | Compression level (0 = adaptive, 1-22) |
| `zstd_dictionary_path`        | string | `"data/zstd-dict.bin"` | Dictionary file path                   |
| `zstd_dictionary_max_bytes`   | u32    | 65536                  | Max dictionary size (bytes)            |
| `zstd_sampling_enabled`       | bool   | false                  | Enable sample collection               |
| `zstd_sampling_dir`           | string | `"data/zstd-samples"`  | Sample directory                       |
| `zstd_sampling_tick_interval` | u32    | 1                      | Sample every N ticks                   |
| `zstd_sampling_max_samples`   | u32    | 1024                   | Max samples to retain                  |

### 3.7 Event Parameters (`[event]`)

| Parameter         | Type | Default | Description                                              |
| ----------------- | ---- | ------- | -------------------------------------------------------- |
| `start_timestamp` | u64  | 0       | Start time of the event (ms, 0 = no limit)               |
| `end_timestamp`   | u64  | 0       | End time of the event (ms, 0 = no limit)                 |

### 3.8 Admin Parameters

| Parameter              | Type   | Default | Description                                                                                        |
| ---------------------- | ------ | ------- | -------------------------------------------------------------------------------------------------- |
| `admin_manage_key`     | string | `""`    | Shared secret that must match the `X-Admin-Key` header. Empty string disables all admin endpoints. |
| `admin_max_blob_bytes` | u64    | 5242880 | Maximum size (bytes) accepted for admin-provided image blobs or fetched remote images.             |

## 4. Hot Reload

### 4.1 Supported Parameters

After a hot reload, the in-memory configuration snapshot is replaced immediately.

Runtime effects are parameter-specific:

- `tps`:

  - The server updates the tick coordinator via `set_tps(new_tps)` as part of the reload.
  - The change takes effect immediately in the tick loop (there is no “apply at next tick boundary” guarantee).

- WebSocket connection parameters (`ws_send_period_ms`, `max_ack_buffer_bytes`, `max_client_latency_ms`, and compression parameters such as `zstd_level`):

  - These values are captured when a WebSocket connection is upgraded.
  - Changing them via hot reload affects new WebSocket connections only; existing connections keep their original values.

- HTTP handlers that read config per request (e.g. `GET /api/paintboard/getboard?enc=zstd`):
  - Observe the new configuration immediately after reload.

### 4.2 Reload Trigger

**Procedure:**

1. Receive signal (e.g., SIGHUP) or API call
2. Re-parse configuration file
3. Validate the new configuration and replace the in-memory snapshot

### 4.3 Non-Reloadable Parameters

The following require server restart:

- `bind_address`
- `sqlite_path`
- `data_dir`

## 5. Validation

### 5.1 Constraints

| Parameter                    | Constraint                                                |
| ---------------------------- | --------------------------------------------------------- |
| `tps`                        | `tps >= 1`                                                |
| `bind_address`               | Valid `host:port` format                                  |
| `background_color`           | Valid hex RGB (`#RRGGBB`)                                 |
| `ws_send_period_ms`          | `ws_send_period_ms >= 1`                                  |
| `snapshot_interval_sec`      | When `persist_board = true`, `snapshot_interval_sec >= 1` |
| `admin_max_blob_bytes`       | `admin_max_blob_bytes >= 1`                               |
| `network.trusted_proxies[*]` | Each entry must parse as `IpAddr`                         |

Note: `zstd_level` is not range-validated by the configuration subsystem; invalid values may cause compression to fail at runtime.

### 5.2 Validation Timing

- Startup: All parameters validated
- Hot reload: Only changed parameters validated

## 6. Default Configuration

```toml
bind_address = "0.0.0.0:8775"
tps = 128
no_access_key_verify = false
background_color = "#AAAAAA"
data_dir = "data"
sqlite_path = "data/tokens.db"
admin_manage_key = ""
admin_max_blob_bytes = 5242880

[network]
trusted_proxies = []

[websocket]
ws_send_period_ms = 20
max_ack_buffer_bytes = 209715200
max_client_latency_ms = 3000

[persistence]
persist_board = false
snapshot_interval_sec = 10
snapshot_retain = 8
wal_flush_every_ticks = 8

[token]
global_cooldown_ms = 0
cooldown_flush_interval_ms = 0

[zstd]
zstd_level = 5
zstd_dictionary_path = "data/zstd-dict.bin"
zstd_dictionary_max_bytes = 65536
zstd_sampling_enabled = false
zstd_sampling_dir = "data/zstd-samples"
zstd_sampling_tick_interval = 1
zstd_sampling_max_samples = 1024

[event]
start_timestamp = 0
end_timestamp = 0
```
