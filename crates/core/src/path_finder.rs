//! Deterministic path-finding using RICH-inspired negative cycle detection.
//!
//! ALGORITHM:
//! 1. Build a directed graph where nodes = token mints, edges = pool swaps.
//! 2. Each edge weight is `-log(exchange_rate)` (negative so that Bellman-Ford
//!    finds negative cycles = profitable arbitrage).
//! 3. Run Bellman-Ford for V iterations over all edges.
//! 4. On the (V+1)th relaxation, any edge that still relaxes is part of a
//!    negative cycle (i.e. a profitable arbitrage path).
//! 5. Extract the cycle and convert back to a sequence of `Hop`s.
//!
//! DETERMINISM: edge ordering is fixed by iterating `IndexMap` (insertion-order).
//! Same pool set → same paths discovered every time.
//!
//! COMPLEXITY: O(V × E) where V = distinct token mints, E = 2 × pool_count.
//! For 500 pools / 100 tokens: 100 × 1000 = 100k iterations, sub-millisecond.

use std::collections::HashMap;

use indexmap::IndexMap;
use smallvec::SmallVec;
use solana_sdk::pubkey::Pubkey;
use tracing::{debug, trace};
use types::{
    dex::DexKind,
    path::{ArbPath, Hop, MAX_HOPS},
    pool::PoolState,
};

/// Node index type (fits in u16 for small graphs, usize for safety).
type NodeIdx = usize;

/// Directed edge in the token exchange graph.
#[derive(Debug, Clone)]
struct Edge {
    from: NodeIdx,
    to: NodeIdx,
    /// Negative log of exchange rate (fixed-point * SCALE for integer arithmetic).
    /// Smaller = better rate.
    neg_log_rate: i64,
    pool: Pubkey,
    dex: DexKind,
    a_to_b: bool,
    pool_state: PoolState,
}

/// Scale factor for fixed-point log approximation.
/// log(x) ≈ (x - 1) * SCALE for x near 1; we use a richer approximation.
const LOG_SCALE: i64 = 1_000_000;

/// Compute a scaled negative log of the exchange rate `out / in`.
/// This uses a fast integer approximation of ln() suitable for comparison.
/// For exact profit calculation we fall back to `simulate_forward`.
fn neg_log_rate(amount_in: u64, amount_out: u64) -> Option<i64> {
    if amount_in == 0 || amount_out == 0 {
        return None;
    }
    // ln(out/in) ≈ (out - in) / in for values near 1; multiply by scale.
    // Using i128 to avoid overflow.
    let num = (amount_out as i128 - amount_in as i128) * LOG_SCALE as i128;
    let den = amount_in as i128;
    // Negate: negative log means profit (negative cycle = arb).
    Some(-(num / den) as i64)
}

/// Builds the graph, runs Bellman-Ford, and returns all discovered `ArbPath`s.
pub struct PathFinder {
    max_hops: usize,
    probe_amount: u64, // amount used to estimate edge rates; 1 SOL equivalent.
}

impl PathFinder {
    pub fn new(max_hops: usize, probe_amount: u64) -> Self {
        Self {
            max_hops: max_hops.min(MAX_HOPS),
            probe_amount,
        }
    }

    /// Discover all profitable circular paths in the given pool set.
    /// Returns paths sorted by estimated profit (best first).
    pub fn find_paths(&self, pools: &[PoolState]) -> Vec<ArbPath> {
        if pools.is_empty() {
            return vec![];
        }

        // ── Build token→index mapping ──────────────────────────────────────
        let mut token_index: IndexMap<Pubkey, NodeIdx> = IndexMap::new();
        for pool in pools {
            let na = token_index.len();
            token_index.entry(pool.token_a.address).or_insert(na);
            let nb = token_index.len();
            token_index.entry(pool.token_b.address).or_insert(nb);
        }
        let n = token_index.len();

        // ── Build edge list ────────────────────────────────────────────────
        let mut edges: Vec<Edge> = Vec::with_capacity(pools.len() * 2);
        for pool in pools {
            let &from_a = token_index.get(&pool.token_a.address).unwrap();
            let &from_b = token_index.get(&pool.token_b.address).unwrap();

            // Direction A→B
            if let Some(out) = pool.quote_a_to_b(self.probe_amount) {
                if let Some(w) = neg_log_rate(self.probe_amount, out) {
                    edges.push(Edge {
                        from: from_a,
                        to: from_b,
                        neg_log_rate: w,
                        pool: pool.address,
                        dex: pool.dex,
                        a_to_b: true,
                        pool_state: pool.clone(),
                    });
                }
            }
            // Direction B→A
            if let Some(out) = pool.quote_b_to_a(self.probe_amount) {
                if let Some(w) = neg_log_rate(self.probe_amount, out) {
                    edges.push(Edge {
                        from: from_b,
                        to: from_a,
                        neg_log_rate: w,
                        pool: pool.address,
                        dex: pool.dex,
                        a_to_b: false,
                        pool_state: pool.clone(),
                    });
                }
            }
        }

        // ── Bellman-Ford: relax edges up to max_hops times ─────────────────
        // dist[v] = shortest path weight from source to v (in log-rate units).
        // We run from every node as source (all tokens are potential starts).
        let mut discovered: Vec<ArbPath> = Vec::new();

        for source in 0..n {
            let paths = self.bellman_ford_from(source, n, &edges, &token_index);
            discovered.extend(paths);
        }

        // Deduplicate by path_id (same cycle discovered from different starts).
        discovered.sort_by(|a, b| b.net_profit_lamports.cmp(&a.net_profit_lamports));
        discovered.dedup_by_key(|p| p.path_id);

        debug!(found = discovered.len(), "Path discovery complete");
        discovered
    }

    fn bellman_ford_from(
        &self,
        source: NodeIdx,
        n: usize,
        edges: &[Edge],
        token_index: &IndexMap<Pubkey, NodeIdx>,
    ) -> Vec<ArbPath> {
        let inf = i64::MAX / 2;
        let mut dist = vec![inf; n];
        let mut pred: Vec<Option<(NodeIdx, usize)>> = vec![None; n]; // (from_node, edge_idx)
        dist[source] = 0;

        // Relax edges (max_hops) times
        for _ in 0..self.max_hops {
            let mut updated = false;
            for (eidx, edge) in edges.iter().enumerate() {
                if dist[edge.from] < inf {
                    let new_dist = dist[edge.from].saturating_add(edge.neg_log_rate);
                    if new_dist < dist[edge.to] {
                        dist[edge.to] = new_dist;
                        pred[edge.to] = Some((edge.from, eidx));
                        updated = true;
                    }
                }
            }
            if !updated {
                break; // Converged early
            }
        }

        // Detect negative cycles: if any edge can still be relaxed, it's in a cycle.
        let mut paths = Vec::new();
        let mut in_cycle = vec![false; n];

        for (eidx, edge) in edges.iter().enumerate() {
            if dist[edge.from] < inf {
                let new_dist = dist[edge.from].saturating_add(edge.neg_log_rate);
                if new_dist < dist[edge.to] && edge.to == source {
                    // Found a negative cycle starting and ending at `source`.
                    if !in_cycle[source] {
                        in_cycle[source] = true;
                        if let Some(path) = self.extract_path(source, eidx, edges, &pred, token_index) {
                            paths.push(path);
                        }
                    }
                }
            }
        }

        paths
    }

    /// Trace back predecessor edges to reconstruct the arbitrage path.
    fn extract_path(
        &self,
        source: NodeIdx,
        last_edge_idx: usize,
        edges: &[Edge],
        pred: &[Option<(NodeIdx, usize)>],
        token_index: &IndexMap<Pubkey, NodeIdx>,
    ) -> Option<ArbPath> {
        let mut hop_edges: SmallVec<[&Edge; MAX_HOPS]> = SmallVec::new();
        let last_edge = &edges[last_edge_idx];
        hop_edges.push(last_edge);

        // Walk predecessor chain, stopping at source or when max_hops exceeded.
        let mut cursor = last_edge.from;
        let mut visited = vec![false; token_index.len()];
        visited[source] = true;

        while cursor != source && hop_edges.len() < self.max_hops {
            if visited[cursor] {
                break; // Prevent infinite loops on malformed graphs
            }
            visited[cursor] = true;
            let Some((prev_node, eidx)) = pred[cursor] else { break };
            hop_edges.push(&edges[eidx]);
            cursor = prev_node;
        }

        if cursor != source || hop_edges.len() < 2 {
            return None; // Not a complete cycle or too short
        }

        // Reverse so hops are in execution order (start → ... → start).
        hop_edges.reverse();

        // Get the start token pubkey
        let start_token = token_index
            .get_index(source)
            .map(|(k, _)| *k)?;

        // Compute a deterministic path ID from all pool pubkeys.
        let path_id = compute_path_id(hop_edges.iter().map(|e| &e.pool));

        let hops: SmallVec<[Hop; MAX_HOPS]> = hop_edges
            .iter()
            .map(|e| Hop {
                pool: e.pool,
                dex: e.dex,
                a_to_b: e.a_to_b,
                pool_state: e.pool_state.clone(),
            })
            .collect();

        // Quick forward simulation to get estimated output
        let arb = ArbPath {
            start_token,
            hops,
            path_id,
            expected_out: 0,
            optimal_input: self.probe_amount,
            net_profit_lamports: 0,
        };

        // Compute gross profit using probe amount
        let gross = arb.gross_profit(self.probe_amount)?;
        if gross <= 0 {
            return None; // No profit at probe level; skip
        }

        Some(ArbPath {
            expected_out: (self.probe_amount as i64 + gross) as u64,
            net_profit_lamports: gross,
            ..arb
        })
    }
}

/// Compute a deterministic u64 path ID from pool pubkeys.
/// Simple FNV-1a hash — fast, deterministic, no randomness.
fn compute_path_id<'a>(pubkeys: impl Iterator<Item = &'a Pubkey>) -> u64 {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    let mut hash = FNV_OFFSET;
    for key in pubkeys {
        for byte in key.as_ref() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::token::TokenMint;

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

    #[test]
    fn no_arb_in_balanced_pools() {
        let ta = Pubkey::new_unique();
        let tb = Pubkey::new_unique();
        // Two balanced pools with fees — no profitable cycle.
        let pools = vec![
            make_pool(ta, tb, 1_000_000, 1_000_000, 30),
            make_pool(tb, ta, 1_000_000, 1_000_000, 30),
        ];
        let finder = PathFinder::new(4, 100_000);
        let paths = finder.find_paths(&pools);
        // Balanced pools with fees should yield no profitable paths.
        for p in &paths {
            assert!(p.net_profit_lamports <= 0);
        }
    }

    #[test]
    fn fnv_path_id_is_deterministic() {
        let k1 = Pubkey::new_unique();
        let k2 = Pubkey::new_unique();
        let id1 = compute_path_id([k1, k2].iter());
        let id2 = compute_path_id([k1, k2].iter());
        assert_eq!(id1, id2);
    }
}
