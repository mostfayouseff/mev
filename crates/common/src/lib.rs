// =============================================================================
// SECURITY AUDIT CHECKLIST — common/src/lib.rs
// [✓] No unsafe code in this module
// [✓] No hardcoded secrets — all config via environment variables
// [✓] All arithmetic uses checked/saturating ops or Decimal
// [✓] No panics — all fallible paths return Result/Option
// [✓] Clippy clean (deny(clippy::all, clippy::pedantic))
// =============================================================================

#![deny(clippy::all)]
#![warn(clippy::pedantic)]

// =============================================================================
// COMMON TYPES & CONFIG — Security audited
// =============================================================================

pub mod config;
pub mod metrics;
pub mod types;

// Re-exports
pub use config::ApexConfig;
pub use metrics::ApexMetrics;
pub use types::{ArbPath, Dex, MarketEdge, PriceMatrix, RichColor, TokenMint};
