//! Arbitrage path types.
//!
//! DESIGN: `ArbPath` uses `SmallVec<[Hop; 6]>` to keep up-to-6-hop paths
//! entirely on the stack (each Hop is ~80 bytes; 6 × 80 = 480 bytes, well
//! within a typical cache line cluster). This avoids heap allocation in the
//! hot loop where thousands of paths are evaluated per second.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use solana_sdk::pubkey::Pubkey;

use crate::dex::DexKind;
use crate::pool::PoolState;

/// Maximum hops per path. Increasing this widens the search space O(pools^N).
pub const MAX_HOPS: usize = 6;

/// A single hop: swap through one pool in a specific direction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hop {
    /// Pool account being traded against.
    pub pool: Pubkey,
    /// DEX protocol for this hop.
    pub dex: DexKind,
    /// True = trade token_a → token_b; False = token_b → token_a.
    pub a_to_b: bool,
    /// Cached snapshot of pool state at path-discovery time.
    pub pool_state: PoolState,
}

/// A complete circular arbitrage path starting and ending at `start_token`.
/// `hops` must form a valid token chain: each hop's output = next hop's input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbPath {
    /// Token we start with (and must end with for profit).
    pub start_token: Pubkey,
    /// Ordered sequence of swaps.
    pub hops: SmallVec<[Hop; MAX_HOPS]>,
    /// Deterministic path ID: SHA-256 of all pool pubkeys in order (truncated to u64).
    pub path_id: u64,
    /// Cached expected output for `optimal_input` after pre-simulation.
    pub expected_out: u64,
    /// Input amount that maximises profit for this path.
    pub optimal_input: u64,
    /// Estimated net profit in lamports (negative = loss).
    pub net_profit_lamports: i64,
}

impl ArbPath {
    /// Simulate the full path forward-pass: apply each pool's quote in sequence.
    /// Returns the final output amount or `None` if any hop returns `None`.
    ///
    /// Pure function — no side effects. Safe to call concurrently.
    pub fn simulate_forward(&self, input_amount: u64) -> Option<u64> {
        let mut amount = input_amount;
        for hop in &self.hops {
            amount = if hop.a_to_b {
                hop.pool_state.quote_a_to_b(amount)?
            } else {
                hop.pool_state.quote_b_to_a(amount)?
            };
        }
        Some(amount)
    }

    /// Net profit = output − input.
    /// Positive = profitable (before fees); negative = loss.
    #[inline]
    pub fn gross_profit(&self, input_amount: u64) -> Option<i64> {
        let out = self.simulate_forward(input_amount)?;
        Some(out as i64 - input_amount as i64)
    }

    /// Number of hops.
    #[inline]
    pub fn len(&self) -> usize {
        self.hops.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.hops.is_empty()
    }
}

/// Result of evaluating a candidate path — emitted by the scoring engine.
#[derive(Debug, Clone)]
pub struct ScoredPath {
    pub path: ArbPath,
    /// Score ∈ [0, 1) computed by the scoring function; higher = better.
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenMint;

    fn make_hop(reserve_a: u64, reserve_b: u64, a_to_b: bool) -> Hop {
        let pool_state = PoolState {
            address: Pubkey::new_unique(),
            dex: DexKind::Raydium,
            token_a: TokenMint::new(Pubkey::new_unique(), 9),
            token_b: TokenMint::new(Pubkey::new_unique(), 9),
            reserve_a,
            reserve_b,
            fee_bps: 0,
            last_slot: 1,
        };
        Hop {
            pool: pool_state.address,
            dex: DexKind::Raydium,
            a_to_b,
            pool_state,
        }
    }

    #[test]
    fn simulate_two_hop_roundtrip_with_zero_fee() {
        // Hop1: 1000 A -> 500 B (50% slippage, zero fee)
        // Hop2: 500 B -> 250 A (50% slippage, zero fee)
        let hop1 = make_hop(1000, 1000, true);
        let hop2 = make_hop(1000, 1000, false); // b_to_a mode

        let start = Pubkey::new_unique();
        let path = ArbPath {
            start_token: start,
            hops: SmallVec::from_vec(vec![hop1, hop2]),
            path_id: 42,
            expected_out: 0,
            optimal_input: 0,
            net_profit_lamports: 0,
        };

        let out = path.simulate_forward(1000);
        assert!(out.is_some());
        // Should be less than input (no arb here)
        assert!(out.unwrap() < 1000);
    }
}
