//! eBPF-style fast filter — runs before any deserialisation.
//!
//! The filter is a set of `Pubkey` allow-lists stored in `ahash::AHashSet` (O(1) lookup,
//! faster than `std::HashSet` because it uses a non-cryptographic hash).
//! Only transactions touching known pool program IDs or token-mint pairs pass through.

use ahash::AHashSet;
use solana_sdk::pubkey::Pubkey;

/// Program IDs for all supported DEX protocols.
/// Populated at startup from config + known constants.
pub struct CandidateFilter {
    /// DEX program IDs that we care about.
    program_ids: AHashSet<Pubkey>,
    /// Known pool addresses (populated from seed list + on-chain discovery).
    pool_addresses: AHashSet<Pubkey>,
}

impl CandidateFilter {
    pub fn new(program_ids: Vec<Pubkey>, pool_addresses: Vec<Pubkey>) -> Self {
        Self {
            program_ids: program_ids.into_iter().collect(),
            pool_addresses: pool_addresses.into_iter().collect(),
        }
    }

    /// Add a newly discovered pool address.
    #[inline]
    pub fn add_pool(&mut self, addr: Pubkey) {
        self.pool_addresses.insert(addr);
    }

    /// Returns `true` if any account key in `account_keys` belongs to a tracked program
    /// or pool. This is the hot-path decision gate — must stay branch-predictor friendly.
    #[inline]
    pub fn is_candidate(&self, account_keys: &[Pubkey]) -> bool {
        // Iterate manually (no allocations) and short-circuit on first match.
        for key in account_keys {
            if self.program_ids.contains(key) || self.pool_addresses.contains(key) {
                return true;
            }
        }
        false
    }

    pub fn program_count(&self) -> usize {
        self.program_ids.len()
    }

    pub fn pool_count(&self) -> usize {
        self.pool_addresses.len()
    }
}

// Well-known program IDs for supported DEXes (mainnet).
pub mod program_ids {
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    pub fn raydium_amm_v4() -> Pubkey {
        Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap()
    }
    pub fn raydium_clmm() -> Pubkey {
        Pubkey::from_str("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK").unwrap()
    }
    pub fn orca_whirlpool() -> Pubkey {
        Pubkey::from_str("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc").unwrap()
    }
    pub fn meteora_dlmm() -> Pubkey {
        Pubkey::from_str("LBUZKhRxPF3XUpBCjp4YofRA8eggHQpQbzNX9PaNNYFp").unwrap()
    }
    pub fn phoenix() -> Pubkey {
        Pubkey::from_str("PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY").unwrap()
    }

    /// Returns all known DEX program IDs.
    pub fn all() -> Vec<Pubkey> {
        vec![
            raydium_amm_v4(),
            raydium_clmm(),
            orca_whirlpool(),
            meteora_dlmm(),
            phoenix(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_matches_program_id() {
        let prog = Pubkey::new_unique();
        let filter = CandidateFilter::new(vec![prog], vec![]);
        let keys = vec![Pubkey::new_unique(), prog, Pubkey::new_unique()];
        assert!(filter.is_candidate(&keys));
    }

    #[test]
    fn filter_rejects_unknown() {
        let filter = CandidateFilter::new(vec![Pubkey::new_unique()], vec![]);
        let keys = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        assert!(!filter.is_candidate(&keys));
    }
}
