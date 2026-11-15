# Board Subsystem Specification

## 1. Overview

The board subsystem manages an in-memory pixel canvas with persistence capabilities.

## 2. Constants

| Constant       | Value     | Description                                                     |
| -------------- | --------- | --------------------------------------------------------------- |
| `BOARD_WIDTH`  | 1000      | Canvas width in pixels                                          |
| `BOARD_HEIGHT` | 600       | Canvas height in pixels                                         |
| `BOARD_SIZE`   | 600,000   | Total pixel count (`BOARD_WIDTH × BOARD_HEIGHT`)                |
| `RGB24_SIZE`   | 1,800,000 | Board size in bytes when serialized as RGB24 (`BOARD_SIZE × 3`) |

## 3. Data Structures

### 3.1 Pixel Index

```
pixel_index = y × BOARD_WIDTH + x
```

**Preconditions:**

- `x ∈ [0, BOARD_WIDTH - 1]` (i.e., `x ∈ [0, 999]`)
- `y ∈ [0, BOARD_HEIGHT - 1]` (i.e., `y ∈ [0, 599]`)

**Postcondition:**

- `pixel_index ∈ [0, BOARD_SIZE - 1]` (i.e., `pixel_index ∈ [0, 599999]`)

### 3.2 RGBA32 Format

Each pixel is stored as a 32-bit unsigned integer:

```
RGBA32 = 0x00RRGGBB
```

- Bits 23-16: Red component (0-255)
- Bits 15-8: Green component (0-255)
- Bits 7-0: Blue component (0-255)
- Bits 31-24: Reserved (always 0)

### 3.3 RGB24 Format (Serialization)

When serialized for HTTP response or snapshot:

```
offset = pixel_index × 3
byte[offset + 0] = R
byte[offset + 1] = G
byte[offset + 2] = B
```

Pixels are stored in row-major order (left-to-right, top-to-bottom).

## 4. In-Memory State

### 4.1 Board Structure

```rust
Board {
    rgba: Vec<AtomicU32>,      // length = BOARD_SIZE
    shadow_rgb: Option<Vec<u8>> // length = RGB24_SIZE when enabled
}
```

**Invariants:**

- `rgba.len() == BOARD_SIZE`
- If `shadow_rgb.is_some()`, then `shadow_rgb.unwrap().len() == RGB24_SIZE`

### 4.2 Pixel Write Operation

**Function:** `store(x: u16, y: u16, r: u8, g: u8, b: u8)`

**Preconditions:**

- `x < BOARD_WIDTH`
- `y < BOARD_HEIGHT`

**Effects:**

1. Compute `idx = y × BOARD_WIDTH + x`
2. Compute `rgba_value = (r as u32) << 16 | (g as u32) << 8 | (b as u32)`
3. Store `rgba_value` to `rgba[idx]` using `AtomicU32::store(Relaxed)`
4. If `shadow_rgb` is enabled:
   - `shadow_rgb[idx × 3 + 0] = r`
   - `shadow_rgb[idx × 3 + 1] = g`
   - `shadow_rgb[idx × 3 + 2] = b`

### 4.3 Board Read Operation

**Function:** `copy_rgb24() -> Vec<u8>`

**Postconditions:**

- Returns a `Vec<u8>` of length `RGB24_SIZE`
- For each `pixel_index ∈ [0, BOARD_SIZE - 1]`:
  - `result[pixel_index × 3 + 0] = (rgba[pixel_index] >> 16) & 0xFF`
  - `result[pixel_index × 3 + 1] = (rgba[pixel_index] >> 8) & 0xFF`
  - `result[pixel_index × 3 + 2] = rgba[pixel_index] & 0xFF`

## 5. Persistence

### 5.1 Snapshot Format

**Filename:** `snapshot-{tick_index}.bin`

**Content:**

- Raw RGB24 data, exactly `RGB24_SIZE` bytes
- No header or metadata
- Row-major order

### 5.2 Snapshot Write Operation

**Trigger:** Every `snapshot_interval_sec` seconds (configurable)

**Procedure:**

1. Copy board to RGB24 format
2. Write to temporary file `snapshot-{tick_index}.bin.tmp`
3. Call `fsync()` on the file descriptor
4. Rename to `snapshot-{tick_index}.bin` (atomic operation)

**Postcondition:**

- File exists at `{data_dir}/snapshot-{tick_index}.bin`
- File size equals `RGB24_SIZE` bytes

### 5.3 Snapshot Load Operation

**Trigger:** Server startup

**Procedure:**

1. Scan `data_dir` for files matching `snapshot-*.bin`
2. Parse `tick_index` from each filename
3. Select file with highest `tick_index`
4. Read `RGB24_SIZE` bytes from file
5. For each `pixel_index`:
   - `r = file_data[pixel_index × 3 + 0]`
   - `g = file_data[pixel_index × 3 + 1]`
   - `b = file_data[pixel_index × 3 + 2]`
   - Store to `rgba[pixel_index]`

**Postcondition:**

- `tick_index` is set to the value from the loaded snapshot filename

### 5.4 WAL (Optional)

**Entry Format:**

```
(tick_index: u64, x: u16, y: u16, r: u8, g: u8, b: u8, uid: u32)
```

**Write Trigger:** Every `wal_flush_every_ticks` ticks

**Replay:** After snapshot load, replay all WAL entries where `entry.tick_index > snapshot.tick_index`

## 6. HTTP API

### 6.1 Get Board

**Endpoint:** `GET /api/paintboard/getboard[?enc=zstd]`

**Query Parameters:**

- `enc` (optional):
  - absent or any value other than `zstd`: response body is raw RGB24
  - `zstd`: response body is zstd-compressed RGB24

**Response Headers:**

- `Content-Type: application/octet-stream`
- `X-Width: 1000`
- `X-Height: 600`
- `X-Pixel-Format: RGB24`
- `ETag: "rev-{tick_index}"`
- When `enc=zstd`, the server also includes `X-Encoded: zstd`.

**Response Body:**

- Exactly `RGB24_SIZE` bytes of RGB24 pixel data
- Row-major order

When `enc=zstd`:

- The server compresses the raw RGB24 byte stream using zstd.
- The response body is the zstd frame bytes.
