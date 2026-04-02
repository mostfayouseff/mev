//! Token-bucket rate limiter for trade submissions.
//!
//! DESIGN: Simple sliding window over a ring buffer of timestamps.
//! Thread-safe via `Mutex` (low contention — only acquired on trade submission).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::warn;

pub struct RateLimiter {
    max_per_minute: u32,
    window: Duration,
    timestamps: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            max_per_minute,
            window: Duration::from_secs(60),
            timestamps: Mutex::new(VecDeque::new()),
        }
    }

    /// Returns `true` if a trade can proceed; `false` if rate-limited.
    pub fn allow(&self) -> bool {
        let mut ts = self.timestamps.lock();
        let now = Instant::now();
        let cutoff = now - self.window;

        // Purge old timestamps
        while ts.front().map(|&t| t < cutoff).unwrap_or(false) {
            ts.pop_front();
        }

        if ts.len() >= self.max_per_minute as usize {
            warn!(
                count = ts.len(),
                max = self.max_per_minute,
                "Rate limit reached"
            );
            return false;
        }

        ts.push_back(now);
        true
    }

    pub fn current_rate(&self) -> u32 {
        let ts = self.timestamps.lock();
        ts.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit() {
        let rl = RateLimiter::new(3);
        assert!(rl.allow());
        assert!(rl.allow());
        assert!(rl.allow());
        assert!(!rl.allow()); // 4th denied
    }
}
