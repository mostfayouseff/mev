//! ingress — zero-copy transaction parsing + account-change subscription.
//!
//! DESIGN:
//! - WebSocket connection to the private RPC endpoint using `tokio-tungstenite`.
//! - Zero-copy deserialisation: raw bytes are parsed directly without intermediate
//!   `serde_json::Value` allocation where possible.
//! - An eBPF-style fast filter runs before any heap allocation to discard irrelevant txs.
//! - Parsed pool updates are pushed into a bounded `crossbeam_channel::Sender` so the
//!   core engine can pull at its own pace — no back-pressure on the ingress thread.

pub mod filter;
pub mod parser;
pub mod websocket;

use crossbeam_channel::{Receiver, Sender};
use types::pool::PoolState;

/// Event emitted by the ingress layer whenever a pool state changes.
#[derive(Debug, Clone)]
pub struct PoolUpdateEvent {
    pub pool: PoolState,
    /// Slot at which this update was observed.
    pub slot: u64,
    /// Monotonic nanoseconds when parsed (for latency tracking).
    pub parsed_at_ns: u64,
}

/// Create an ingress pipeline: returns a channel receiver that delivers `PoolUpdateEvent`s.
/// The pipeline runs in background Tokio tasks.
///
/// `capacity` is the bounded channel depth; backpressure will cause ingress to drop updates
/// (logged as a metric) rather than unboundedly buffer. This is intentional — stale data
/// is worse than missing data in an MEV context.
pub fn create_pipeline(capacity: usize) -> (Sender<PoolUpdateEvent>, Receiver<PoolUpdateEvent>) {
    crossbeam_channel::bounded(capacity)
}

pub use filter::CandidateFilter;
pub use parser::AccountParser;
pub use websocket::IngressWs;
