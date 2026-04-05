// crates/core/src/gnn_stub.rs
use common::types::ArbPath;

/// Simple stub for the GNN Oracle until the real model is integrated.
#[derive(Debug, Default)]
pub struct GnnOracle;

impl GnnOracle {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Returns a confidence score for the arbitrage path.
    /// In production this will call a real GNN / ML model.
    #[must_use]
    pub fn infer(&self, _path: &ArbPath) -> f32 {
        0.82 // placeholder value > typical threshold
    }
}

/// Error type used by RichEngine (moved here to avoid module issues)
#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("Invalid max_hops: {0}. Must be between 2 and 6")]
    InvalidHops(usize),
}
