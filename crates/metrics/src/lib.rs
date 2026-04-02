//! metrics — Prometheus-compatible metrics registry for the entire bot.
//!
//! DESIGN:
//! - All metrics are registered once at startup and exposed on a `/metrics` HTTP endpoint.
//! - Using `lazy_static`-style `OnceLock` to avoid global mutable state.
//! - Counter/Gauge/Histogram are all that's needed; avoid Summaries (they're expensive).
//! - Labels are kept minimal — high cardinality (e.g., per-pool labels) would OOM Prometheus.

use std::net::SocketAddr;
use std::sync::OnceLock;

use prometheus::{
    Encoder, Gauge, GaugeVec, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    Opts, Registry, TextEncoder,
};
use thiserror::Error;
use tracing::{error, info};

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("Prometheus error: {0}")]
    Prometheus(#[from] prometheus::Error),
    #[error("Bind error: {0}")]
    Bind(String),
}

/// All bot metrics in one struct — avoids scattered global state.
pub struct BotMetrics {
    pub registry: Registry,

    // ─── Ingress ──────────────────────────────────────────────────────────────
    /// Transactions received from the WebSocket feed.
    pub ingress_tx_received: IntCounter,
    /// Transactions passing initial candidate filters.
    pub ingress_candidates: IntCounter,
    /// Pool state update events.
    pub pool_updates: IntCounter,

    // ─── Path finding ─────────────────────────────────────────────────────────
    pub paths_evaluated: IntCounter,
    pub paths_profitable: IntCounter,
    /// Histogram of per-path evaluation latency in microseconds.
    pub path_eval_latency_us: HistogramVec,

    // ─── Strategy ─────────────────────────────────────────────────────────────
    pub strategy_ticks: IntCounter,
    pub opportunities_found: IntCounter,

    // ─── Safety ───────────────────────────────────────────────────────────────
    pub sim_attempts: IntCounter,
    pub sim_failures: IntCounter,
    pub circuit_breaker_trips: IntCounter,
    pub fail_fast_cancels: IntCounter,

    // ─── Execution ────────────────────────────────────────────────────────────
    pub trades_submitted: IntCounter,
    pub trades_confirmed: IntCounter,
    pub trades_failed: IntCounter,
    pub profit_lamports_total: IntCounter,
    pub trade_latency_ms: HistogramVec,

    // ─── Jito ─────────────────────────────────────────────────────────────────
    pub jito_bundles_sent: IntCounter,
    pub jito_bundles_landed: IntCounter,

    // ─── System ───────────────────────────────────────────────────────────────
    pub wallet_balance_lamports: IntGauge,
    pub current_slot: IntGauge,
    pub candidate_pool_count: IntGauge,
}

static METRICS: OnceLock<BotMetrics> = OnceLock::new();

/// Initialise the global metrics registry. Call once at startup.
/// Panics if called twice (programming error).
pub fn init_metrics() -> Result<&'static BotMetrics, MetricsError> {
    if METRICS.get().is_some() {
        panic!("init_metrics() called more than once");
    }

    let registry = Registry::new();

    macro_rules! counter {
        ($name:expr, $help:expr) => {{
            let c = IntCounter::new($name, $help)?;
            registry.register(Box::new(c.clone()))?;
            c
        }};
    }

    macro_rules! gauge {
        ($name:expr, $help:expr) => {{
            let g = IntGauge::new($name, $help)?;
            registry.register(Box::new(g.clone()))?;
            g
        }};
    }

    macro_rules! histogram_vec {
        ($name:expr, $help:expr, $labels:expr, $buckets:expr) => {{
            let opts = HistogramOpts::new($name, $help).buckets($buckets);
            let h = HistogramVec::new(opts, $labels)?;
            registry.register(Box::new(h.clone()))?;
            h
        }};
    }

    // Latency buckets in microseconds: 1, 5, 10, 50, 100, 500, 1000, 5000
    let us_buckets = vec![1.0, 5.0, 10.0, 50.0, 100.0, 500.0, 1_000.0, 5_000.0];
    // Latency buckets in milliseconds
    let ms_buckets = vec![1.0, 5.0, 10.0, 50.0, 100.0, 500.0, 1_000.0];

    let metrics = BotMetrics {
        ingress_tx_received: counter!("apex_ingress_tx_received_total", "Transactions received"),
        ingress_candidates: counter!("apex_ingress_candidates_total", "Candidate transactions"),
        pool_updates: counter!("apex_pool_updates_total", "Pool state updates"),
        paths_evaluated: counter!("apex_paths_evaluated_total", "Paths evaluated"),
        paths_profitable: counter!("apex_paths_profitable_total", "Paths with positive gross profit"),
        path_eval_latency_us: histogram_vec!(
            "apex_path_eval_latency_us",
            "Per-path evaluation latency in microseconds",
            &["dex"],
            us_buckets.clone()
        ),
        strategy_ticks: counter!("apex_strategy_ticks_total", "Strategy loop ticks"),
        opportunities_found: counter!("apex_opportunities_found_total", "Arb opportunities found"),
        sim_attempts: counter!("apex_sim_attempts_total", "Pre-simulation attempts"),
        sim_failures: counter!("apex_sim_failures_total", "Pre-simulation failures"),
        circuit_breaker_trips: counter!("apex_circuit_breaker_trips_total", "Circuit breaker trips"),
        fail_fast_cancels: counter!("apex_fail_fast_cancels_total", "Fail-fast cancellations"),
        trades_submitted: counter!("apex_trades_submitted_total", "Trades submitted"),
        trades_confirmed: counter!("apex_trades_confirmed_total", "Trades confirmed on-chain"),
        trades_failed: counter!("apex_trades_failed_total", "Trades failed or timed out"),
        profit_lamports_total: counter!("apex_profit_lamports_total", "Cumulative profit in lamports"),
        trade_latency_ms: histogram_vec!(
            "apex_trade_latency_ms",
            "End-to-end trade latency in milliseconds",
            &["outcome"],
            ms_buckets
        ),
        jito_bundles_sent: counter!("apex_jito_bundles_sent_total", "Jito bundles sent"),
        jito_bundles_landed: counter!("apex_jito_bundles_landed_total", "Jito bundles landed"),
        wallet_balance_lamports: gauge!("apex_wallet_balance_lamports", "Current wallet balance"),
        current_slot: gauge!("apex_current_slot", "Most recent observed slot"),
        candidate_pool_count: gauge!("apex_candidate_pool_count", "Active candidate pools"),
        registry,
    };

    Ok(METRICS.get_or_init(|| metrics))
}

/// Returns the global metrics handle. Panics if `init_metrics()` was not called first.
#[inline]
pub fn metrics() -> &'static BotMetrics {
    METRICS.get().expect("metrics not initialised — call init_metrics() at startup")
}

/// Render the current metrics as a Prometheus text exposition.
pub fn render_metrics() -> Result<String, MetricsError> {
    let m = metrics();
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    let metric_families = m.registry.gather();
    encoder.encode(&metric_families, &mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

/// Start a tiny HTTP server exposing `/metrics` for Prometheus scraping.
/// Runs in a background Tokio task; does not block.
pub async fn start_metrics_server(addr: SocketAddr) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(?e, "Failed to bind metrics server");
            return;
        }
    };
    info!(%addr, "Metrics server listening");

    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                tokio::spawn(async move {
                    let mut buf = [0u8; 512];
                    // Read enough to identify the request; we only serve /metrics GET.
                    let _ = stream.read(&mut buf).await;

                    let body = render_metrics().unwrap_or_else(|e| format!("# error: {e}"));
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
            Err(e) => {
                error!(?e, "Metrics accept error");
            }
        }
    }
}
