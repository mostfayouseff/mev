//! Circuit breaker — stops all trading after too many consecutive losses.
//!
//! DESIGN: Uses an atomic `u32` loss counter (SeqCst ordering for correctness).
//! No mutex needed — the counter increment and trip check are separate atomic ops,
//! which is safe because: (a) we only need eventual consistency for the threshold
//! check, and (b) false negatives (allowing one extra trade under contention) are
//! acceptable while false positives (blocking when we shouldn't) are not harmful.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use tracing::{error, info, warn};

pub struct CircuitBreaker {
    /// Number of consecutive losses since last reset.
    loss_count: AtomicU32,
    /// When `true`, all execution is blocked.
    tripped: AtomicBool,
    /// Threshold after which the breaker trips.
    threshold: u32,
}

impl CircuitBreaker {
    pub fn new(threshold: u32) -> Self {
        Self {
            loss_count: AtomicU32::new(0),
            tripped: AtomicBool::new(false),
            threshold,
        }
    }

    /// Returns `true` if the circuit breaker is open (execution blocked).
    #[inline]
    pub fn is_tripped(&self) -> bool {
        self.tripped.load(Ordering::SeqCst)
    }

    /// Record a successful trade — resets the loss counter.
    pub fn record_success(&self) {
        self.loss_count.store(0, Ordering::SeqCst);
        info!("Circuit breaker: success recorded, counter reset");
    }

    /// Record a failed/losing trade.
    /// Trips the breaker if the loss count exceeds the threshold.
    pub fn record_loss(&self) {
        let prev = self.loss_count.fetch_add(1, Ordering::SeqCst);
        let current = prev + 1;
        warn!(current, threshold = self.threshold, "Circuit breaker: loss recorded");

        if current >= self.threshold {
            self.tripped.store(true, Ordering::SeqCst);
            error!(
                threshold = self.threshold,
                "CIRCUIT BREAKER TRIPPED — all execution paused. Manual reset required."
            );
            metrics::metrics().circuit_breaker_trips.inc();
        }
    }

    /// Manually reset the circuit breaker (operator action).
    pub fn reset(&self) {
        self.loss_count.store(0, Ordering::SeqCst);
        self.tripped.store(false, Ordering::SeqCst);
        info!("Circuit breaker: manually reset");
    }

    pub fn loss_count(&self) -> u32 {
        self.loss_count.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_after_threshold() {
        let cb = CircuitBreaker::new(3);
        assert!(!cb.is_tripped());
        cb.record_loss();
        cb.record_loss();
        assert!(!cb.is_tripped());
        cb.record_loss();
        assert!(cb.is_tripped());
    }

    #[test]
    fn reset_clears_trip() {
        let cb = CircuitBreaker::new(1);
        cb.record_loss();
        assert!(cb.is_tripped());
        cb.reset();
        assert!(!cb.is_tripped());
    }

    #[test]
    fn success_resets_counter() {
        let cb = CircuitBreaker::new(3);
        cb.record_loss();
        cb.record_loss();
        cb.record_success();
        cb.record_loss();
        assert!(!cb.is_tripped()); // counter was reset; only 1 loss now
    }
}
