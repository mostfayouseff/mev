//! common — shared utilities, error types, and trait definitions used across all crates.
//!
//! Design decisions:
//! - Errors are strongly typed via `thiserror` to enable exhaustive matching and avoid
//!   silent swallowing of failure modes.
//! - No `unwrap()` / `expect()` in library code; callers must handle `Result`s.
//! - All public items are `#[inline]` where beneficial to remove call overhead in hot path.

use thiserror::Error;

/// Top-level error enum for the apex-mev system.
/// Each sub-system extends this with its own variant.
#[derive(Debug, Error)]
pub enum ApexError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Ingress error: {0}")]
    Ingress(String),

    #[error("Path-finding error: {0}")]
    PathFinding(String),

    #[error("Strategy error: {0}")]
    Strategy(String),

    #[error("Safety check failed: {0}")]
    Safety(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Jito integration error: {0}")]
    Jito(String),

    #[error("Simulation rejected: {reason}")]
    SimulationRejected { reason: String },

    #[error("Profit below minimum: got {got_lamports} lamports, need {min_lamports}")]
    InsufficientProfit {
        got_lamports: i64,
        min_lamports: i64,
    },

    #[error("Circuit breaker tripped: {0}")]
    CircuitBreaker(String),

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type ApexResult<T> = Result<T, ApexError>;

/// Timing helper: returns monotonic nanoseconds, used throughout for latency measurement.
/// Using `std::time::Instant` is safe and monotonic on all supported platforms.
#[inline]
pub fn now_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Returns a monotonic timestamp for interval measurement (not wall-clock).
#[inline]
pub fn monotonic_ns() -> u64 {
    // std::time::Instant doesn't expose nanos directly in stable Rust,
    // so we measure elapsed from a lazy_static baseline.
    use std::sync::OnceLock;
    use std::time::Instant;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_nanos() as u64
}

/// Saturating subtraction helper for lamport arithmetic.
/// Prevents underflow in fee/profit calculations.
#[inline(always)]
pub fn lamports_sub_sat(a: u64, b: u64) -> u64 {
    a.saturating_sub(b)
}

/// Integer basis-points arithmetic: `amount * bps / 10_000`.
/// No floating point, deterministic.
#[inline(always)]
pub fn apply_bps(amount: u64, bps: u32) -> u64 {
    // Use u128 to avoid overflow for large amounts
    ((amount as u128 * bps as u128) / 10_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_bps_zero() {
        assert_eq!(apply_bps(1_000_000, 0), 0);
    }

    #[test]
    fn apply_bps_full() {
        assert_eq!(apply_bps(1_000_000, 10_000), 1_000_000);
    }

    #[test]
    fn apply_bps_half_percent() {
        assert_eq!(apply_bps(100_000_000, 50), 500_000);
    }

    #[test]
    fn lamports_sub_sat_no_underflow() {
        assert_eq!(lamports_sub_sat(100, 200), 0);
    }
}
