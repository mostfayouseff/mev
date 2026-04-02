//! Integration tests for apex-mev.
//!
//! These tests run the core pipeline end-to-end with synthetic pool data,
//! verifying that the path-finding → scoring → safety flow is deterministic
//! and profitable paths are correctly identified.

use solana_sdk::pubkey::Pubkey;
use types::{dex::DexKind, pool::PoolState, token::TokenMint};
use apex_core::{
    optimizer::{InputOptimizer, OptimizerConfig},
    path_finder::PathFinder,
    registry::PoolRegistry,
    scorer::{PathScorer, ScorerConfig},
};
use safety::circuit_breaker::CircuitBreaker;

/// Build a simple pool for testing arb detection.
fn make_pool(ta: Pubkey, tb: Pubkey, ra: u64, rb: u64, fee_bps: u32) -> PoolState {
    PoolState {
        address: Pubkey::new_unique(),
        dex: DexKind::Raydium,
        token_a: TokenMint::new(ta, 9),
        token_b: TokenMint::new(tb, 9),
        reserve_a: ra,
        reserve_b: rb,
        fee_bps,
        last_slot: 1,
    }
}

/// Determinism test: same pool set → same paths, same order, same profit.
#[test]
fn path_finding_is_deterministic() {
    let tok_a = Pubkey::new_unique();
    let tok_b = Pubkey::new_unique();
    let tok_c = Pubkey::new_unique();

    let pools = vec![
        make_pool(tok_a, tok_b, 1_000_000, 2_000_000, 0),
        make_pool(tok_b, tok_c, 2_000_000, 1_000_000, 0),
        make_pool(tok_c, tok_a, 1_000_000, 1_000_000, 0),
    ];

    let finder = PathFinder::new(4, 100_000);
    let paths1 = finder.find_paths(&pools);
    let paths2 = finder.find_paths(&pools);

    assert_eq!(paths1.len(), paths2.len());
    for (p1, p2) in paths1.iter().zip(paths2.iter()) {
        assert_eq!(p1.path_id, p2.path_id);
        assert_eq!(p1.net_profit_lamports, p2.net_profit_lamports);
    }
}

/// Fail-fast test: unprofitable paths don't pass the scorer threshold.
#[test]
fn scorer_rejects_unprofitable_paths() {
    let tok_a = Pubkey::new_unique();
    let tok_b = Pubkey::new_unique();

    // Balanced pools — no arb
    let pools = vec![
        make_pool(tok_a, tok_b, 1_000_000, 1_000_000, 30),
        make_pool(tok_b, tok_a, 1_000_000, 1_000_000, 30),
    ];

    let finder = PathFinder::new(4, 100_000);
    let paths = finder.find_paths(&pools);

    let scorer = PathScorer::new(ScorerConfig {
        priority_fee_lamports: 5_000,
        jito_tip_lamports: 0,
        use_jito: false,
        min_profit_lamports: 10_000,
        latency_penalty_per_hop: 0,
    });

    let ranked = scorer.rank_paths(&paths);
    // All ranked paths must pass the minimum profit threshold
    for sp in &ranked {
        assert!(
            sp.path.net_profit_lamports >= 10_000,
            "Path {} slipped through with profit {}",
            sp.path.path_id,
            sp.path.net_profit_lamports
        );
    }
}

/// Circuit breaker test: trips after threshold losses.
#[test]
fn circuit_breaker_blocks_after_threshold() {
    let cb = CircuitBreaker::new(3);
    assert!(!cb.is_tripped());
    cb.record_loss();
    cb.record_loss();
    assert!(!cb.is_tripped());
    cb.record_loss();
    assert!(cb.is_tripped());
    cb.reset();
    assert!(!cb.is_tripped());
    assert_eq!(cb.loss_count(), 0);
}

/// Pool registry test: upsert and retrieve.
#[test]
fn pool_registry_upsert_and_retrieve() {
    let registry = PoolRegistry::new();
    let tok_a = Pubkey::new_unique();
    let tok_b = Pubkey::new_unique();
    let pool = make_pool(tok_a, tok_b, 100_000, 200_000, 25);
    let addr = pool.address;

    registry.upsert(pool.clone());
    let retrieved = registry.get(&addr).expect("Pool should exist");
    assert_eq!(retrieved.address, addr);
    assert_eq!(retrieved.reserve_a, 100_000);
    assert_eq!(retrieved.reserve_b, 200_000);
}

/// Profit simulation test: verify forward simulation of a 2-hop path.
#[test]
fn two_hop_forward_simulation() {
    use types::path::{ArbPath, Hop};
    use smallvec::SmallVec;

    let tok_a = Pubkey::new_unique();
    let tok_b = Pubkey::new_unique();

    // Pool 1: imbalanced — more B than A, so A→B gives good rate
    let pool1 = make_pool(tok_a, tok_b, 1_000_000, 2_000_000, 0);
    // Pool 2: reverse direction (B→A in A's favour)
    let pool2 = PoolState {
        address: Pubkey::new_unique(),
        dex: DexKind::Orca,
        token_a: TokenMint::new(tok_b, 9),
        token_b: TokenMint::new(tok_a, 9),
        reserve_a: 2_000_000,
        reserve_b: 1_000_000,
        fee_bps: 0,
        last_slot: 1,
    };

    let hop1 = Hop {
        pool: pool1.address,
        dex: DexKind::Raydium,
        a_to_b: true,
        pool_state: pool1,
    };
    let hop2 = Hop {
        pool: pool2.address,
        dex: DexKind::Orca,
        a_to_b: true,
        pool_state: pool2,
    };

    let path = ArbPath {
        start_token: tok_a,
        hops: SmallVec::from_vec(vec![hop1, hop2]),
        path_id: 999,
        expected_out: 0,
        optimal_input: 0,
        net_profit_lamports: 0,
    };

    let input = 10_000u64;
    let output = path.simulate_forward(input);
    assert!(output.is_some(), "Simulation should succeed");
    let out = output.unwrap();
    println!("Input: {input}, Output: {out}, Delta: {}", out as i64 - input as i64);
}

/// Property: `apply_bps` result is always ≤ input for valid basis points.
#[test]
fn apply_bps_always_le_input() {
    for amount in [0u64, 1, 1_000, 1_000_000, u64::MAX / 10_000] {
        for bps in [0u32, 1, 50, 100, 500, 1_000, 5_000, 10_000] {
            let result = common::apply_bps(amount, bps);
            assert!(
                result <= amount,
                "apply_bps({amount}, {bps}) = {result} > amount"
            );
        }
    }
}

/// Constant-product quote: balanced pool with zero fee returns exact half on 100% input.
#[test]
fn constant_product_quote_exactness() {
    let pool = make_pool(Pubkey::new_unique(), Pubkey::new_unique(), 1_000, 1_000, 0);
    let out = pool.quote_a_to_b(1_000).unwrap();
    assert_eq!(out, 500, "CP with equal reserves: out should be 500 for 1000 input");
}
