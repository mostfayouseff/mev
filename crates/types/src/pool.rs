//! Liquidity pool state — the fundamental on-chain resource for arbitrage.
//!
//! DESIGN: `PoolState` is intentionally lightweight (two u64 reserves + fee).
//! The full account data is parsed once in the ingress layer and stored here.
//! The core engine operates exclusively on `PoolState` — no RPC in the hot path.

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use crate::dex::DexKind;
use crate::token::TokenMint;

/// Snapshot of a liquidity pool at a specific slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolState {
    /// Pool account address on-chain.
    pub address: Pubkey,
    /// Which DEX protocol this pool belongs to.
    pub dex: DexKind,
    /// Input-side token.
    pub token_a: TokenMint,
    /// Output-side token.
    pub token_b: TokenMint,
    /// Reserve of token A (raw, unscaled).
    pub reserve_a: u64,
    /// Reserve of token B (raw, unscaled).
    pub reserve_b: u64,
    /// Pool trading fee in basis points (e.g. 25 = 0.25%).
    pub fee_bps: u32,
    /// Last observed slot. Used to detect stale state.
    pub last_slot: u64,
}

impl PoolState {
    /// Constant-product AMM quote: how many token_b do we get for `amount_in` of token_a?
    /// Returns `None` if reserves are zero (invalid state) or result would overflow.
    ///
    /// Formula: out = (reserve_b * amount_in * (10_000 - fee_bps)) /
    ///                (reserve_a * 10_000 + amount_in * (10_000 - fee_bps))
    ///
    /// Uses u128 throughout to prevent overflow for large reserves.
    #[inline]
    pub fn quote_a_to_b(&self, amount_in: u64) -> Option<u64> {
        if self.reserve_a == 0 || self.reserve_b == 0 {
            return None;
        }
        let fee_factor = (10_000u128).checked_sub(self.fee_bps as u128)?;
        let numerator = (self.reserve_b as u128)
            .checked_mul(amount_in as u128)?
            .checked_mul(fee_factor)?;
        let denominator = (self.reserve_a as u128)
            .checked_mul(10_000)?
            .checked_add((amount_in as u128).checked_mul(fee_factor)?)?;
        if denominator == 0 {
            return None;
        }
        Some((numerator / denominator) as u64)
    }

    /// Reverse quote: token_b -> token_a.
    #[inline]
    pub fn quote_b_to_a(&self, amount_in: u64) -> Option<u64> {
        // Swap reserves and delegate to quote_a_to_b
        let swapped = PoolState {
            reserve_a: self.reserve_b,
            reserve_b: self.reserve_a,
            ..self.clone()
        };
        swapped.quote_a_to_b(amount_in)
    }

    /// Returns true if this pool's slot is older than `max_age_slots`.
    #[inline]
    pub fn is_stale(&self, current_slot: u64, max_age_slots: u64) -> bool {
        current_slot.saturating_sub(self.last_slot) > max_age_slots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pool(reserve_a: u64, reserve_b: u64, fee_bps: u32) -> PoolState {
        PoolState {
            address: Pubkey::new_unique(),
            dex: DexKind::Raydium,
            token_a: TokenMint::new(Pubkey::new_unique(), 9),
            token_b: TokenMint::new(Pubkey::new_unique(), 6),
            reserve_a,
            reserve_b,
            fee_bps,
            last_slot: 100,
        }
    }

    #[test]
    fn quote_zero_reserves_returns_none() {
        let pool = make_pool(0, 1_000_000, 30);
        assert!(pool.quote_a_to_b(1_000).is_none());
    }

    #[test]
    fn quote_constant_product_no_fee() {
        // With zero fee: out = reserve_b * in / (reserve_a + in)
        // 1000 * 1000 / (1000 + 1000) = 500
        let pool = make_pool(1_000, 1_000, 0);
        let out = pool.quote_a_to_b(1_000).unwrap();
        assert_eq!(out, 500);
    }

    #[test]
    fn quote_with_fee_reduces_output() {
        let pool_no_fee = make_pool(1_000, 1_000, 0);
        let pool_with_fee = make_pool(1_000, 1_000, 30);
        let out_no_fee = pool_no_fee.quote_a_to_b(100).unwrap();
        let out_with_fee = pool_with_fee.quote_a_to_b(100).unwrap();
        assert!(out_with_fee < out_no_fee);
    }
}
