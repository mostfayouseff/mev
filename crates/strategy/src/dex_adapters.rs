//! Per-DEX adapter traits and implementations.
//!
//! Each DEX adapter knows how to build a swap instruction for its protocol.
//! Adapters are stateless — all state lives in `PoolState`.

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    pubkey,
};
use types::{dex::DexKind, path::Hop};

// ── Known DEX program IDs (same as ingress::filter::program_ids) ─────────────
pub mod program_ids {
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    pub fn raydium_amm_v4() -> Pubkey {
        Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap()
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
}

/// Trait implemented by each DEX adapter.
pub trait DexAdapter: Send + Sync {
    fn dex(&self) -> DexKind;

    /// Build the swap instruction for a single hop.
    fn build_swap_ix(
        &self,
        hop: &Hop,
        user: &Pubkey,
        amount_in: u64,
        min_amount_out: u64,
    ) -> Option<Instruction>;
}

// ─── Raydium AMM V4 adapter ───────────────────────────────────────────────────

pub struct RaydiumAmmAdapter;

impl DexAdapter for RaydiumAmmAdapter {
    fn dex(&self) -> DexKind {
        DexKind::Raydium
    }

    fn build_swap_ix(
        &self,
        hop: &Hop,
        user: &Pubkey,
        amount_in: u64,
        min_amount_out: u64,
    ) -> Option<Instruction> {
        // Raydium AMM swap instruction discriminator = 9 (SwapBaseIn)
        let mut data = vec![9u8];
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&min_amount_out.to_le_bytes());

        Some(Instruction {
            program_id: program_ids::raydium_amm_v4(),
            accounts: vec![
                AccountMeta::new(hop.pool, false),
                AccountMeta::new_readonly(*user, true),
            ],
            data,
        })
    }
}

// ─── Orca Whirlpool adapter ────────────────────────────────────────────────────

pub struct OrcaWhirlpoolAdapter;

impl DexAdapter for OrcaWhirlpoolAdapter {
    fn dex(&self) -> DexKind {
        DexKind::OrcaWhirlpool
    }

    fn build_swap_ix(
        &self,
        hop: &Hop,
        user: &Pubkey,
        amount_in: u64,
        min_amount_out: u64,
    ) -> Option<Instruction> {
        // Whirlpool swap discriminator (first 8 bytes of sha256("global:swap"))
        let discriminator: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];
        let mut data = discriminator.to_vec();
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&min_amount_out.to_le_bytes());
        data.push(hop.a_to_b as u8);

        Some(Instruction {
            program_id: program_ids::orca_whirlpool(),
            accounts: vec![
                AccountMeta::new(hop.pool, false),
                AccountMeta::new_readonly(*user, true),
            ],
            data,
        })
    }
}

// ─── Meteora DLMM adapter ─────────────────────────────────────────────────────

pub struct MeteoraAdapter;

impl DexAdapter for MeteoraAdapter {
    fn dex(&self) -> DexKind {
        DexKind::Meteora
    }

    fn build_swap_ix(
        &self,
        hop: &Hop,
        user: &Pubkey,
        amount_in: u64,
        min_amount_out: u64,
    ) -> Option<Instruction> {
        // Meteora swap discriminator (placeholder — use real discriminator in production)
        let mut data = vec![0xau8, 0xbu8, 0xcu8, 0xdu8, 0xeu8, 0xfu8, 0x00u8, 0x01u8];
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&min_amount_out.to_le_bytes());

        Some(Instruction {
            program_id: program_ids::meteora_dlmm(),
            accounts: vec![
                AccountMeta::new(hop.pool, false),
                AccountMeta::new_readonly(*user, true),
            ],
            data,
        })
    }
}

/// Registry of all active DEX adapters, keyed by `DexKind`.
pub struct AdapterRegistry {
    adapters: std::collections::HashMap<DexKind, Box<dyn DexAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        let mut adapters: std::collections::HashMap<DexKind, Box<dyn DexAdapter>> =
            std::collections::HashMap::new();
        adapters.insert(DexKind::Raydium, Box::new(RaydiumAmmAdapter));
        adapters.insert(DexKind::OrcaWhirlpool, Box::new(OrcaWhirlpoolAdapter));
        adapters.insert(DexKind::Meteora, Box::new(MeteoraAdapter));
        Self { adapters }
    }

    pub fn get(&self, dex: &DexKind) -> Option<&dyn DexAdapter> {
        self.adapters.get(dex).map(|a| a.as_ref())
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}
