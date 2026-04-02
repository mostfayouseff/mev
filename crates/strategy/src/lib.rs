//! strategy — multi-DEX strategy loop: discovery, scheduling, and opportunity emission.
//!
//! DESIGN:
//! - The strategy runs a tick loop driven by incoming pool updates (not wall-clock).
//! - On each tick: refresh stale paths, re-score, emit the top-N opportunities.
//! - Flash-swap / launch-sniping hooks are registered as pluggable `StrategyHook`s.
//! - No state mutation outside of the strategy loop; all output goes through channels.

pub mod dex_adapters;
pub mod hooks;
pub mod loop_runner;

pub use loop_runner::StrategyRunner;

use types::path::ScoredPath;

/// An opportunity that has passed scoring and is ready for simulation.
#[derive(Debug, Clone)]
pub struct Opportunity {
    pub path: ScoredPath,
    /// Resolved optimal input after the optimizer ran.
    pub optimal_input: u64,
    /// Expected output lamports.
    pub expected_output: u64,
    /// Monotonic ns at opportunity creation.
    pub created_at_ns: u64,
}
