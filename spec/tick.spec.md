# Tick Subsystem Specification

## 1. Overview

The tick subsystem manages time-based coordination of paint operations using a tick-based consistency model.

## 2. Constants and Configuration

| Parameter          | Default               | Description                              |
| ------------------ | --------------------- | ---------------------------------------- |
| `TPS`              | 128                   | Ticks per second                         |
| `tick_duration_ns` | `1,000,000,000 / TPS` | Duration of one tick in nanoseconds      |
| `epsilon_ns`       | 500,000 (0.5ms)       | Soft window for tick boundary assignment |

## 3. State

### 3.1 Tick Coordinator State

```rust
TickCoordinator {
    tick_index: AtomicU64,      // Current global tick number
    tick_duration: Duration,    // Duration of one tick
    epsilon_ns: u64,           // Soft boundary window
    tps: u32                   // Current TPS setting
}
```

**Invariants:**

- `tick_index` is monotonically increasing
- `tick_duration = Duration::from_nanos(1_000_000_000 / tps)`

### 3.2 Tick Index

Type: `u64`

**Properties:**

- Monotonically increasing
- Wraps at `u64::MAX` (overflow after ~4.6 × 10^12 years at 128 TPS)
- Initial value: 0 or restored from snapshot

## 4. Tick Assignment

### 4.1 Paint Request Tick Assignment

**Input:** `arrival_time: Instant` (monotonic clock)

**Let:**

- `tick_deadline = tick_start + tick_duration`
- `soft_boundary = tick_deadline - epsilon_ns`

**Assignment Rules:**

1. If `arrival_time < soft_boundary`:

   - Assign to current tick (`tick_index`)

2. If `arrival_time >= soft_boundary` AND `arrival_time < tick_deadline`:
   - Assign to next tick (`tick_index + 1`)

**Rationale:** The soft boundary reduces latency jitter for requests arriving near tick boundaries.

## 5. Tick Lifecycle

### 5.1 Tick Flip Operation

**Trigger:** Every `tick_duration` nanoseconds

**Procedure:**

1. Increment `tick_index` atomically
2. Signal all write-set shards to flip `active ↔ frozen`
3. New tick begins accepting paint requests immediately

**Time Complexity:** O(1)

### 5.2 Tick Commit Operation

**Trigger:** After tick flip

**Input:** `frozen` write set from previous tick

**Procedure:**

1. For each `(pixel_index, rgb, ingest_seq)` in `frozen` (parallel):
   - Write RGB to board at `pixel_index`
   - Append 8-byte paint event to broadcast buffer
2. Sort paint events by `pixel_index` (ascending) for compression efficiency
3. Compress broadcast buffer (if zstd enabled)
4. Dispatch to all connections

## 6. Write Set

### 6.1 Data Structure

```rust
ShardWriteSet {
    active: HashMap<u32, (Rgb, IngestSeq)>,
    frozen: HashMap<u32, (Rgb, IngestSeq)>
}
```

**Key:** `pixel_index` (u32)
**Value:** `(rgb: Rgb, seq: IngestSeq)`

### 6.2 Insert Operation

**Function:** `insert(pixel_index: u32, rgb: Rgb, seq: IngestSeq)`

**Precondition:** `pixel_index < BOARD_SIZE`

**Behavior:**

- If `active[pixel_index]` does not exist OR `active[pixel_index].seq < seq`:
  - Set `active[pixel_index] = (rgb, seq)`
- Otherwise: no-op (earlier write wins)

**Semantics:** Last-Writer-Wins within the same tick (by `ingest_seq`)

### 6.3 Flip Operation

**Function:** `flip()`

**Effects:**

1. Swap `active` and `frozen` references (O(1))
2. Clear the new `active` (previously `frozen`)

### 6.4 Drain Operation

**Function:** `drain_sorted() -> Vec<(u32, Rgb)>`

**Returns:** Contents of `frozen`, sorted by `pixel_index` ascending

**Postcondition:** `frozen` is empty after call

## 7. Ingest Sequence

### 7.1 Definition

Type: `u64`

**Properties:**

- Globally unique within a server instance
- Monotonically increasing
- Assigned to each paint request on arrival

### 7.2 Usage

Determines write ordering within a tick:

- Higher `ingest_seq` takes precedence for same `pixel_index`
- Guarantees deterministic Last-Writer-Wins semantics

## 8. Hot Reload

### 8.1 TPS Change

**Procedure:**

1. New TPS value is stored
2. The tick loop observes the new TPS immediately (there is no requirement that the change waits for a tick boundary)
3. `tick_duration` is recalculated as `Duration::from_nanos(1_000_000_000 / new_tps)`

**Note:** The implementation does not guarantee that TPS changes are atomic at tick boundaries.
