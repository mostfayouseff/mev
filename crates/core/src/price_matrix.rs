use common::types::{MarketEdge, PriceMatrix, TokenMint};
use rustc_hash::FxHashMap;
use tracing::warn;

const STALE_SLOTS: u64 = 200;

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
        // Register all tokens first
        for edge in edges {
            self.register_token(&edge.from);
            self.register_token(&edge.to);
        }

        let mut matrix = PriceMatrix::new(self.tokens.clone());

        for edge in edges {
            let age = self.current_slot.saturating_sub(edge.slot);
            if age > STALE_SLOTS {
                continue; // silently skip stale edges in production
            }

            let Some(&i) = self.token_index.get(&edge.from.0) else { continue };
            let Some(&j) = self.token_index.get(&edge.to.0) else { continue };

            // Decimal → f64 conversion (done once)
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
