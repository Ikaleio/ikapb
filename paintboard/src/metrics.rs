use once_cell::sync::Lazy;
use prometheus::{Counter, Gauge, Histogram, register_counter, register_gauge, register_histogram};

static PAINT_QPS: Lazy<Gauge> =
    Lazy::new(|| register_gauge!("paint_qps", "Paint commands per second").unwrap());
static TICK_COMMIT_LATENCY_MS_P99: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "tick_commit_latency_ms",
        "Tick commit latency distribution in milliseconds"
    )
    .unwrap()
});
static EGRESS_MBPS: Lazy<Gauge> =
    Lazy::new(|| register_gauge!("egress_mbps", "Outbound bandwidth").unwrap());
static COMPRESSION_RATIO: Lazy<Gauge> = Lazy::new(|| {
    register_gauge!("compression_ratio", "Compression ratio for outbound frames").unwrap()
});
static ZSTD_FRAMES_TOTAL: Lazy<Counter> =
    Lazy::new(|| register_counter!("zstd_frames_total", "Total zstd-compressed frames").unwrap());
static BOARD_GET_QPS: Lazy<Gauge> =
    Lazy::new(|| register_gauge!("board_get_qps", "HTTP getboard QPS").unwrap());
static COOLDOWN_FLUSH_OPS: Lazy<Counter> =
    Lazy::new(|| register_counter!("cooldown_flush_ops", "Cooldown flush operations").unwrap());
static SQLITE_WRITE_LATENCY_MS: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "sqlite_write_latency_ms",
        "SQLite write latency in milliseconds"
    )
    .unwrap()
});

pub fn init() {
    Lazy::force(&PAINT_QPS);
    Lazy::force(&TICK_COMMIT_LATENCY_MS_P99);
    Lazy::force(&EGRESS_MBPS);
    Lazy::force(&COMPRESSION_RATIO);
    Lazy::force(&ZSTD_FRAMES_TOTAL);
    Lazy::force(&BOARD_GET_QPS);
    Lazy::force(&COOLDOWN_FLUSH_OPS);
    Lazy::force(&SQLITE_WRITE_LATENCY_MS);
}

pub fn record_paint_qps(value: f64) {
    PAINT_QPS.set(value);
}

pub fn observe_tick_commit_latency_ms(value: f64) {
    TICK_COMMIT_LATENCY_MS_P99.observe(value);
}

pub fn record_egress_mbps(value: f64) {
    EGRESS_MBPS.set(value);
}

pub fn record_compression_ratio(value: f64) {
    COMPRESSION_RATIO.set(value);
}

pub fn inc_zstd_frames() {
    ZSTD_FRAMES_TOTAL.inc();
}

pub fn record_board_get_qps(value: f64) {
    BOARD_GET_QPS.set(value);
}

pub fn inc_cooldown_flush_ops() {
    COOLDOWN_FLUSH_OPS.inc();
}

pub fn observe_sqlite_write_latency_ms(value: f64) {
    SQLITE_WRITE_LATENCY_MS.observe(value);
}
