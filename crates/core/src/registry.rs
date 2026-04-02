//! Thread-safe, lock-free pool registry.
//!
//! Uses `DashMap` for concurrent reads/writes without a global lock.
//! The registry is the single source of truth for all pool states.

use dashmap::DashMap;
use solana_sdk::pubkey::Pubkey;
use tracing::debug;
use types::pool::PoolState;

/// Central registry of all known pool states.
/// Keyed by pool `Pubkey`. Lock-free for readers.
pub struct PoolRegistry {
    pools: DashMap<Pubkey, PoolState>,
}

impl PoolRegistry {
    pub fn new() -> Self {
        Self {
            pools: DashMap::new(),
        }
    }

    /// Update or insert a pool state. Called by the ingress layer on every account change.
    #[inline]
    pub fn upsert(&self, state: PoolState) {
        debug!(pool = %state.address, slot = state.last_slot, "Pool updated");
        self.pools.insert(state.address, state);
    }

    /// Get a snapshot of a pool state (cloned from the shard).
    #[inline]
    pub fn get(&self, addr: &Pubkey) -> Option<PoolState> {
        self.pools.get(addr).map(|e| e.clone())
    }

    /// Collect all non-stale pools.
    pub fn live_pools(&self, current_slot: u64, max_age_slots: u64) -> Vec<PoolState> {
        self.pools
            .iter()
            .filter(|e| !e.is_stale(current_slot, max_age_slots))
            .map(|e| e.clone())
            .collect()
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }
}

impl Default for PoolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
