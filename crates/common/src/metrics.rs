// =============================================================================
// PROMETHEUS METRICS — Security audited
// =============================================================================

use prometheus::{
    register_counter, register_gauge, register_histogram, Counter, Gauge, Histogram, HistogramOpts, Opts,
};
use tracing::warn;

pub struct ApexMetrics {
    pub paths_evaluated: Counter,
    pub paths_profitable: Counter,
    pub bundles_submitted: Counter,
    pub bundles_landed: Counter,
    pub total_profit_lamports: Gauge,
    pub circuit_breaker_state: Gauge,
    pub hot_path_latency_us: Histogram,
    pub gnn_inference_latency_us: Histogram,
}

impl ApexMetrics {
    pub fn register() -> Result<Self, prometheus::Error> {
        Ok(Self {
            paths_evaluated: register_counter!(Opts::new(
                "apex_paths_evaluated_total",
                "Total arbitrage paths evaluated"
            ))?,

            paths_profitable: register_counter!(Opts::new(
                "apex_paths_profitable_total",
                "Profitable paths found"
            ))?,

            bundles_submitted: register_counter!(Opts::new(
                "apex_bundles_submitted_total",
                "Jito bundles submitted"
            ))?,

            bundles_landed: register_counter!(Opts::new(
                "apex_bundles_landed_total",
                "Jito bundles landed on-chain"
            ))?,

            total_profit_lamports: register_gauge!(Opts::new(
                "apex_total_profit_lamports",
                "Cumulative profit in lamports"
            ))?,

            circuit_breaker_state: register_gauge!(Opts::new(
                "apex_circuit_breaker_state",
                "1 = healthy, 0 = tripped"
            ))?,

            hot_path_latency_us: register_histogram!(HistogramOpts::new(
                "apex_hot_path_latency_us",
                "Full hot path latency in microseconds"
            )
            .buckets(vec![50.0, 100.0, 200.0, 500.0, 1000.0, 5000.0]))?,

            gnn_inference_latency_us: register_histogram!(HistogramOpts::new(
                "apex_gnn_inference_latency_us",
                "GNN inference latency in microseconds"
            )
            .buckets(vec![10.0, 20.0, 50.0, 100.0, 500.0]))?,
        })
    }

    pub fn observe_hot_path(&self, start: std::time::Instant) {
        let elapsed = start.elapsed().as_micros() as f64;
        let _ = self.hot_path_latency_us.observe(elapsed); // never panics in practice
    }
}
