//! Input amount optimizer — finds the `optimal_input` that maximises net profit.
//!
//! ALGORITHM: Ternary search over the input range [MIN_INPUT, MAX_INPUT].
//! Ternary search works because the profit function is unimodal for constant-
//! product AMMs (convex in the region of interest).
//!
//! DETERMINISM: Fixed step sizes, fixed iteration count → same path + same pool
//! states → same optimal input every time.
//!
//! The optimizer runs off-chain (CPU) against cached pool states — no RPC calls.

use types::path::ArbPath;

/// Range and precision configuration for the ternary search.
pub struct OptimizerConfig {
    /// Smallest amount to consider (dust threshold).
    pub min_input: u64,
    /// Maximum input (capped at position size limit).
    pub max_input: u64,
    /// Number of ternary search iterations (45 gives ~2^-45 precision).
    pub iterations: u32,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            min_input: 1_000,          // 0.000001 SOL
            max_input: 200_000_000_000, // 200 SOL
            iterations: 45,
        }
    }
}

pub struct InputOptimizer {
    config: OptimizerConfig,
}

impl InputOptimizer {
    pub fn new(config: OptimizerConfig) -> Self {
        Self { config }
    }

    /// Find the input amount that maximises gross profit for `path`.
    /// Returns `(optimal_input, expected_output)` or `None` if no profitable input exists.
    pub fn optimize(&self, path: &ArbPath) -> Option<(u64, u64)> {
        let lo = self.config.min_input;
        let hi = self.config.max_input;

        // Ternary search on the profit function.
        // f(x) = simulate_forward(x) - x
        let profit = |x: u64| -> i64 {
            path.simulate_forward(x)
                .map(|out| out as i64 - x as i64)
                .unwrap_or(i64::MIN)
        };

        let mut lo = lo;
        let mut hi = hi;

        for _ in 0..self.config.iterations {
            if hi <= lo + 2 {
                break;
            }
            let m1 = lo + (hi - lo) / 3;
            let m2 = hi - (hi - lo) / 3;
            if profit(m1) < profit(m2) {
                lo = m1;
            } else {
                hi = m2;
            }
        }

        let opt_input = (lo + hi) / 2;
        let out = path.simulate_forward(opt_input)?;
        let p = out as i64 - opt_input as i64;

        if p <= 0 {
            None
        } else {
            Some((opt_input, out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;
    use solana_sdk::pubkey::Pubkey;
    use types::{
        dex::DexKind,
        path::{ArbPath, Hop},
        pool::PoolState,
        token::TokenMint,
    };

    fn arb_path_with_imbalance(reserve_a: u64, reserve_b: u64) -> ArbPath {
        // A→B→A cycle. Pool 1: imbalanced (more B than A). Pool 2: reverse.
        // When reserve_b >> reserve_a, A→B is cheap; B→A back should yield profit.
        let tok_a = Pubkey::new_unique();
        let tok_b = Pubkey::new_unique();
        let pool1 = PoolState {
            address: Pubkey::new_unique(),
            dex: DexKind::Raydium,
            token_a: TokenMint::new(tok_a, 9),
            token_b: TokenMint::new(tok_b, 9),
            reserve_a,
            reserve_b,
            fee_bps: 0,
            last_slot: 1,
        };
        let pool2 = PoolState {
            address: Pubkey::new_unique(),
            dex: DexKind::Raydium,
            token_a: TokenMint::new(tok_b, 9),
            token_b: TokenMint::new(tok_a, 9),
            reserve_a: reserve_b, // symmetric
            reserve_b: reserve_a,
            fee_bps: 0,
            last_slot: 1,
        };

        let hop1 = Hop { pool: pool1.address, dex: DexKind::Raydium, a_to_b: true, pool_state: pool1 };
        let hop2 = Hop { pool: pool2.address, dex: DexKind::Raydium, a_to_b: true, pool_state: pool2 };

        ArbPath {
            start_token: tok_a,
            hops: SmallVec::from_vec(vec![hop1, hop2]),
            path_id: 1,
            expected_out: 0,
            optimal_input: 0,
            net_profit_lamports: 0,
        }
    }

    #[test]
    fn optimizer_returns_none_for_balanced_pool() {
        // Perfectly balanced round-trip yields nothing (even zero-fee).
        let path = arb_path_with_imbalance(1_000_000, 1_000_000);
        let opt = InputOptimizer::new(OptimizerConfig::default());
        // Balanced: output < input always (constant product).
        assert!(opt.optimize(&path).is_none());
    }
}
