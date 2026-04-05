use common::types::{ArbPath, Dex, MarketEdge, PriceMatrix, RichColor};
use tracing::{debug, trace};

const REF_POSITION_LAMPORTS: f64 = 100_000_000.0; // 0.1 SOL reference

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

            // Bellman-Ford relaxation
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

            // Detect negative cycles
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
                        if let Some(path) = self.reconstruct_cycle(matrix, weights, src, u, v) {
                            results.push(RichResult {
                                negative_cycle: true,
                                total_log_weight: path.edges.iter()
                                    .map(|e| e.log_weight.to_f64().unwrap_or(0.0))
                                    .sum(),
                                path,
                            });
                        }
                    }
                }
            }
        }

        results
    }

    fn reconstruct_cycle(
        &self,
        matrix: &PriceMatrix,
        weights: &[f64],
        src: usize,
        start: usize,
        end: usize,
    ) -> Option<ArbPath> {
        // Simplified reconstruction using predecessor simulation (for stub)
        // In full version you'd keep real predecessors
        let mut edges = Vec::new();
        let mut current = end;
        let mut steps = 0;

        while steps < self.max_hops && current != src {
            let from_idx = if steps == 0 { start } else { current };
            let to_idx = if current == end { src } else { /* logic */ current }; // placeholder

            if let (Some(from_tok), Some(to_tok)) = (
                matrix.tokens.get(from_idx),
                matrix.tokens.get(to_idx),
            ) {
                let w = weights.get(from_idx * matrix.n + to_idx).copied().unwrap_or(0.0);
                let log_weight = rust_decimal::Decimal::from_f64_retain(w).unwrap_or_default();

                edges.push(MarketEdge {
                    from: from_tok.clone(),
                    to: to_tok.clone(),
                    dex: Dex::Raydium, // stub
                    log_weight,
                    liquidity_lamports: 1_000_000_000,
                    slot: 0,
                });
            }
            steps += 1;
            if steps > 10 {
                break;
            }
        }

        if edges.len() < 2 {
            return None;
        }

        let total_log: f64 = edges.iter()
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
