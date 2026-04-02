//! execution — transaction building, signing, and submission.
//!
//! DESIGN:
//! - Only called with a `ValidatedOpportunity` (passed all safety checks).
//! - Builds a `VersionedTransaction` (v0) with Address Lookup Tables to compress accounts.
//! - Atomically includes all hop instructions in a single transaction — if any hop fails,
//!   the entire tx reverts (Solana's native atomicity).
//! - If `use_jito = true`: wraps in a Jito bundle with a tip transaction.
//! - If `dry_run = true`: logs the transaction but does not submit.
//! - Rate-limit gate (max_trades_per_minute) enforced here before any submission.

pub mod builder;
pub mod rate_limiter;
pub mod submitter;

pub use builder::TransactionBuilder;
pub use rate_limiter::RateLimiter;
pub use submitter::Submitter;

use common::ApexError;
use safety::ValidatedOpportunity;

/// Outcome of an execution attempt.
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub success: bool,
    pub signature: Option<String>,
    pub profit_lamports: i64,
    pub latency_ms: u64,
    pub error: Option<String>,
}
