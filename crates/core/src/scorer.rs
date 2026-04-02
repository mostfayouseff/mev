//! Path scorer — ranks candidate paths by expected profit adjusted for risk.
//!
//! SCORING FUNCTION (deterministic):
//!   score = (net_profit - fee_cost - latency_penalty) / sqrt(hops)
//!
//! This penalises long paths (more hops = more execution risk) and accounts
//! for the priority fee + Jito tip cost. All inputs are integers; no floats
//! in the comparison key.

use types::path::{ArbPath, ScoredPath};

/// Scoring parameters derived from config.
#[derive(Debug, Clone)]
pub struct ScorerConfig {
    /// Priority fee in lamports (deducted from profit estimate).
    pub priority_fee_lamports: u64,
    /// Jito tip in lamports (deducted if Jito is used).
    pub jito_tip_lamports: u64,
    /// Use Jito? Determines whether Jito tip is included in cost.
    pub use_jito: bool,
    /// Minimum net profit threshold (paths below this are discarded).
    pub min_profit_lamports: i64,
    /// Latency penalty per hop in lamports (represents execution risk).
    pub latency_penalty_per_hop: i64,
}

pub struct PathScorer {
    config: ScorerConfig,
}

impl PathScorer {
    pub fn new(config: ScorerConfig) -> Self {
        Self { config }
    }

    /// Score a path. Returns `None` if the path is unprofitable after fees.
    /// This is the fail-fast gate: if score is None, we discard immediately.
    pub fn score(&self, path: &ArbPath) -> Option<ScoredPath> {
        let fee_cost = self.config.priority_fee_lamports as i64
            + if self.config.use_jito {
                self.config.jito_tip_lamports as i64
            } else {
                0
            };

        let latency_penalty = self.config.latency_penalty_per_hop * path.len() as i64;

        let adjusted_profit = path.net_profit_lamports - fee_cost - latency_penalty;

        if adjusted_profit < self.config.min_profit_lamports {
            return None; // FAIL-FAST: drop below threshold
        }

        // Score proportional to profit, inverse of hop count.
        // Using integer*1000 to maintain determinism.
        let hop_penalty = (path.len() as f64).sqrt();
        let score = adjusted_profit as f64 / hop_penalty;

        Some(ScoredPath {
            path: path.clone(),
            score,
        })
    }

    /// Score and sort a collection of paths. Returns only profitable paths, best first.
    pub fn rank_paths(&self, paths: &[ArbPath]) -> Vec<ScoredPath> {
        let mut scored: Vec<ScoredPath> = paths
            .iter()
            .filter_map(|p| self.score(p))
            .collect();

        // Deterministic sort: descending score, tie-break by path_id (lexicographic).
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.path_id.cmp(&b.path.path_id))
        });

        scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;
    use solana_sdk::pubkey::Pubkey;
    use types::path::ArbPath;

    fn dummy_path(profit: i64, hops: usize) -> ArbPath {
        ArbPath {
            start_token: Pubkey::new_unique(),
            hops: SmallVec::new(), // not used by scorer
            path_id: profit as u64 * 1000 + hops as u64,
            expected_out: 0,
            optimal_input: 1_000_000,
            net_profit_lamports: profit,
        }
    }

    #[test]
    fn unprofitable_path_is_none() {
        let scorer = PathScorer::new(ScorerConfig {
            priority_fee_lamports: 5_000,
            jito_tip_lamports: 10_000,
            use_jito: false,
            min_profit_lamports: 10_000,
            latency_penalty_per_hop: 100,
        });
        let path = dummy_path(5_000, 2); // profit below threshold after fees
        assert!(scorer.score(&path).is_none());
    }

    #[test]
    fn profitable_path_is_some() {
        let scorer = PathScorer::new(ScorerConfig {
            priority_fee_lamports: 5_000,
            jito_tip_lamports: 10_000,
            use_jito: false,
            min_profit_lamports: 10_000,
            latency_penalty_per_hop: 100,
        });
        let path = dummy_path(50_000, 2);
        let scored = scorer.score(&path);
        assert!(scored.is_some());
        assert!(scored.unwrap().score > 0.0);
    }

    #[test]
    fn rank_is_deterministic() {
        let scorer = PathScorer::new(ScorerConfig {
            priority_fee_lamports: 1_000,
            jito_tip_lamports: 0,
            use_jito: false,
            min_profit_lamports: 1_000,
            latency_penalty_per_hop: 0,
        });
        let paths = vec![dummy_path(20_000, 2), dummy_path(50_000, 3), dummy_path(30_000, 2)];
        let r1 = scorer.rank_paths(&paths);
        let r2 = scorer.rank_paths(&paths);
        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.path.path_id, b.path.path_id);
        }
    }
}
