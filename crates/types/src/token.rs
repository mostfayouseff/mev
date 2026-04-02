//! Token-related types.

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

/// Canonical token mint descriptor.
/// `decimals` is stored alongside the mint to avoid repeated RPC lookups.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenMint {
    pub address: Pubkey,
    /// Token decimals (0–18). Used for human-readable amount conversion.
    pub decimals: u8,
    /// Optional human-readable ticker for logging (not used in hot path).
    pub symbol: Option<String>,
}

impl TokenMint {
    #[inline]
    pub fn new(address: Pubkey, decimals: u8) -> Self {
        Self {
            address,
            decimals,
            symbol: None,
        }
    }

    /// Convert raw lamport-like units to a floating representation (logging only).
    pub fn to_ui_amount(&self, raw: u64) -> f64 {
        raw as f64 / 10f64.powi(self.decimals as i32)
    }
}
