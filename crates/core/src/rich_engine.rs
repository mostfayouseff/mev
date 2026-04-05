// crates/core/src/rich_engine.rs
use common::types::{ArbPath, Dex, MarketEdge, PriceMatrix, RichColor, TokenMint};
use rust_decimal::prelude::ToPrimitive;
use rustc_hash::FxHashMap;
use tracing::{debug, warn};

const REF_POSITION_LAMPORTS: f64 = 100_000_000.0; // 0.1 SOL reference
const STALE_SLOTS: u64 = 200;

#[derive(Debug)]
pub struct RichResult {
    pub path: ArbPath,
    pub negative_cycle: bool,
    pub total_log_weight: f64,
}

pub struct RichEngine {
    max_hops: usize,
}

impl RichEngine {
    pub fn new(max_hops: usize) -> Result<Self, crate::path_finder::PathError> {
        if !(2..=6).contains(&max_hops) {
            return Err(crate::path_finder::PathError::InvalidHops(max_hops));
        }
        Ok(Self { max_hops })
    }

    /// Detect negative cycles using Bellman-Ford on the log-weight graph.
    pub fn detect_cycles(&self, matrix: &PriceMatrix) -> Vec<RichResult> {
        let n = matrix.n;
        if n < 2 {
            return Vec::new();
        }

        let weights = &matrix.data;
        let mut results = Vec::new();

        for src in 0..n {
            let mut dist = vec![f64::INFINITY; n];
            dist[src] = 0.0;

            // Bellman-Ford relaxation (n-1 iterations)
            for _ in 0..n - 1 {
                for u in 0..n {
                    if dist[u].is_infinite() {
                        continue;
                    }
                    let row_start = u * n;
                    for v in 0..n {
                        if u == v {
                            continue;
                        }
                        let w = weights[row_start + v];
                        if w.is_finite() {
                            let new_dist = dist[u] + w;
                            if new_dist < dist[v] {
                                dist[v] = new_dist;
                            }
                        }
                    }
                }
            }

            // Check for negative cycles reachable from src
            for u in 0..n {
                if dist[u].is_infinite() {
                    continue;
                }
                let row_start = u * n;
                for v in 0..n {
                    if u == v {
                        continue;
                    }
                    let w = weights[row_start + v];
                    if w.is_finite() && dist[u] + w < dist[v] {
                        // Negative cycle detected
                        if let Some(path) = self.reconstruct_cycle(matrix, src, u, v) {
                            let total_log_weight: f64 = path
                                .edges
                                .iter()
                                .map(|e| e.log_weight.to_f64().unwrap_or(0.0))
                                .sum();

                            results.push(RichResult {
                                path,
                                negative_cycle: true,
                                total_log_weight,
                            });
                        }
                    }
                }
            }
        }

        debug!(detected = results.len(), "RICH engine found negative cycles");
        results
    }

    /// Simplified cycle reconstruction (stub version).
    /// In a production version you should maintain predecessor arrays during Bellman-Ford.
    fn reconstruct_cycle(
        &self,
        matrix: &PriceMatrix,
        _src: usize,
        start: usize,
        end: usize,
    ) -> Option<ArbPath> {
        let mut edges = Vec::new();
        let mut current = end;
        let mut steps = 0;

        while steps < self.max_hops && steps < 10 {
            let from_idx = if steps == 0 { start } else { current };
            let to_idx = if steps == 0 { end } else { (current + 1) % matrix.n }; // very naive cycle

            if let (Some(from_tok), Some(to_tok)) = (
                matrix.tokens.get(from_idx),
                matrix.tokens.get(to_idx),
            ) {
                // Get weight from matrix
                let w = matrix.get(from_idx, to_idx).unwrap_or(0.0);
                let log_weight = rust_decimal::Decimal::from_f64_retain(w).unwrap_or_default();

                edges.push(MarketEdge {
                    from: from_tok.clone(),
                    to: to_tok.clone(),
                    dex: Dex::Raydium, // stub — in real version use actual DEX from edge data
                    log_weight,
                    liquidity_lamports: 1_000_000_000,
                    slot: 0,
                });
            }

            steps += 1;
            current = to_idx;
        }

        if edges.len() < 2 {
            return None;
        }

        // Calculate expected profit from total log return
        let total_log: f64 = edges
            .iter()
            .map(|e| e.log_weight.to_f64().unwrap_or(0.0))
            .sum();

        let expected_profit = if total_log < 0.0 {
            let rate = ((-total_log).exp() - 1.0).clamp(0.0, 0.5);
            (rate * REF_POSITION_LAMPORTS) as u64
        } else {
            0
        };

        Some(ArbPath {
            edges,
            expected_profit_lamports: expected_profit,
            gnn_confidence: 0.0,
            rich_color: RichColor::Gray,
        })
    }
}

// =============================================================================
// MATRIX BUILDER
// =============================================================================

pub struct MatrixBuilder {
    token_index: FxHashMap<[u8; 32], usize>,
    tokens: Vec<TokenMint>,
    current_slot: u64,
}

impl MatrixBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            token_index: FxHashMap::with_capacity_and_hasher(128, Default::default()),
            tokens: Vec::with_capacity(64),
            current_slot: 0,
        }
    }

    pub fn set_slot(&mut self, slot: u64) {
        self.current_slot = slot;
    }

    fn register_token(&mut self, mint: &TokenMint) -> usize {
        *self.token_index.entry(mint.0).or_insert_with(|| {
            let idx = self.tokens.len();
            self.tokens.push(mint.clone());
            idx
        })
    }

    pub fn build(&mut self, edges: &[MarketEdge]) -> PriceMatrix {
        // Register all unique tokens
        for edge in edges {
            self.register_token(&edge.from);
            self.register_token(&edge.to);
        }

        let mut matrix = PriceMatrix::new(self.tokens.clone());

        for edge in edges {
            let age = self.current_slot.saturating_sub(edge.slot);
            if age > STALE_SLOTS {
                continue;
            }

            let Some(&i) = self.token_index.get(&edge.from.0) else { continue };
            let Some(&j) = self.token_index.get(&edge.to.0) else { continue };

            let w: f64 = edge.log_weight.to_f64().unwrap_or(f64::INFINITY);

            if w.is_finite() {
                matrix.set(i, j, w);
            }
        }

        matrix
    }
}

impl Default for MatrixBuilder {
    fn default() -> Self {
        Self::new()
    }
}
