# Compression Subsystem Specification

## 1. Overview

The compression subsystem provides zstd compression for server-to-client broadcast messages.

## 2. Configuration

| Parameter                     | Default              | Description                                      |
| ----------------------------- | -------------------- | ------------------------------------------------ |
| `zstd_level`                  | 5                    | Compression level (0 = adaptive, 1-22 = fixed)   |
| `zstd_dictionary_path`        | `data/zstd-dict.bin` | Path to dictionary file                          |
| `zstd_sampling_enabled`       | false                | Enable sample collection for dictionary training |
| `zstd_sampling_dir`           | `data/zstd-samples`  | Directory for collected samples                  |
| `zstd_sampling_tick_interval` | 1                    | Collect sample every N ticks                     |
| `zstd_sampling_max_samples`   | 1024                 | Maximum samples to retain                        |
| `zstd_dictionary_max_bytes`   | 65536                | Target dictionary size                           |

## 3. Compression Workflow

### 3.1 Per-Tick Compression

**Input:** Concatenated `0xFA` paint events for one tick (raw bytes)

**Procedure:**

1. If input is empty: skip compression
2. Sort events by pixel index (already done in tick commit)
3. Compress with zstd at configured level
4. If dictionary is loaded: use dictionary for compression
5. Return compressed bytes

**Output:** Zstd frame (with or without dictionary)

### 3.2 Compression Ratio

**Definition:**

```
compression_ratio = uncompressed_size / compressed_size
```

**Metric:** `compression_ratio` gauge

## 4. Dictionary Management

### 4.1 Dictionary Loading

**Trigger:** Server startup

**Procedure:**

1. Check if `zstd_dictionary_path` exists
2. If exists: load dictionary content
3. Validate dictionary format
4. Store in memory for compression/decompression

### 4.2 Dictionary Frame (0xF6)

**Format:**
| Offset | Size | Type | Field |
|--------|------|------|-------|
| 0 | 1 | u8 | Opcode (`0xF6`) |
| 1 | 4 | u32_le | Dictionary ID (echo of `dict=<id>`) |
| 5 | 4 | u32_le | Dictionary byte length `N` |
| 9 | N | bytes | Raw dictionary content |

**Timing:** Sent once per connection immediately after WebSocket upgrade

**Condition:** Only sent when `enc=zstd` AND `dict=<id>` parameter matches loaded dictionary

### 4.3 Dictionary Training

**Trigger:** Server shutdown (if `zstd_sampling_enabled = true`)

**Input:** Collected samples from `zstd_sampling_dir`

**Procedure:**

1. Load all sample files
2. Train dictionary using zstd training API
3. Limit output to `zstd_dictionary_max_bytes`
4. Write to `zstd_dictionary_path`

## 5. Sample Collection

### 5.1 Sample Format

Each sample is the uncompressed tick data (concatenated `0xFA` events)

### 5.2 Collection Procedure

**Trigger:** Every `zstd_sampling_tick_interval` ticks

**Procedure:**

1. Check if `zstd_sampling_enabled`
2. Check if sample count < `zstd_sampling_max_samples`
3. If conditions met: write tick data to `{zstd_sampling_dir}/sample-{tick_index}.bin`

## 6. Broadcast Modes

### 6.1 Uncompressed Mode (enc=none)

**Behavior:**

- Send raw `0xFA` events concatenated
- Single WebSocket binary frame per tick
- No `0xF7` header

### 6.2 Compressed Mode (enc=zstd)

**Behavior:**

- Send `0xF7` frame with compressed payload
- Dictionary may be used if negotiated
- One frame per tick (no fragmentation)
- Exception: When a WebSocket connection activates the admin watch filter (valid admin credential via `X-Admin-Key` header or `admin_key` query parameter and non-empty `watch_ips` allowlist), compression is skipped and raw `0xFA` streams are sent even if the client requested `enc=zstd`.

## 7. Frame Reuse

### 7.1 Compression Caching

**Optimization:** Compress tick data once per tick per (compression parameters), broadcast to multiple connections

**Procedure:**

1. Collect all paint events for a tick in uncompressed `0xFA` stream form
2. For a given `(zstd_level, dict_id)` pair, compress at most once for that `tick_index`
3. For each `enc=zstd` connection whose negotiated parameters match `(zstd_level, dict_id)`:
   - Reuse the cached compressed payload for that tick
4. For each `enc=none` connection:
   - Reuse the same uncompressed `0xFA` stream

**Invariant:** Compression is performed at most once per tick per `(zstd_level, dict_id)` pair

## 8. Decompression (Client Reference)

### 8.1 0xF7 Frame Processing

**Procedure:**

1. Read 18-byte header
2. Extract `raw_len` (uncompressed size)
3. Read remaining bytes as compressed payload
4. Decompress using zstd (with dictionary if `0xF6` was received)
5. Parse decompressed data as concatenated `0xFA` events

### 8.2 Dictionary Usage

If `0xF6` frame was received:

- Store dictionary content
- Use for all subsequent `0xF7` decompression

If no `0xF6` frame:

- Decompress `0xF7` without dictionary
