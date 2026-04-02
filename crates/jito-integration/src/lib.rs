//! jito-integration — Jito bundle submission and tip strategy.
//!
//! DESIGN:
//! - Jito bundles are submitted via the Jito block engine REST API.
//! - The tip strategy is dynamic: we start at `jito_tip_lamports` and increase
//!   proportionally to the opportunity profit (so we win auction without overpaying).
//! - We track bundle landing rate and adjust tips accordingly.
//! - No Jito SDK dependency to avoid version lock-in; we talk directly to the REST API.

pub mod bundle;
pub mod tip;

pub use bundle::JitoClient;
pub use tip::TipStrategy;

/// Jito block engine endpoint (mainnet).
pub const JITO_BLOCK_ENGINE_URL: &str = "https://mainnet.block-engine.jito.wtf/api/v1/bundles";
/// Jito block engine endpoint (devnet).
pub const JITO_BLOCK_ENGINE_DEVNET: &str = "https://devnet.block-engine.jito.wtf/api/v1/bundles";
