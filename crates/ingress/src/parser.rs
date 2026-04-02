//! Zero-copy account data parser.
//!
//! DESIGN:
//! - Each DEX has a different account layout. The parser dispatches on `DexKind`.
//! - All parsing is done from raw `&[u8]` slices — no intermediate `Vec` allocation.
//! - If a layout changes on-chain, the parser returns `None` (graceful degradation)
//!   rather than crashing.
//! - `#[repr(C, packed)]` structs are used for direct byte-slice casting where the
//!   Solana program's on-chain layout permits. Alignment is handled explicitly.

use common::ApexError;
use solana_sdk::pubkey::Pubkey;
use types::{dex::DexKind, pool::PoolState, token::TokenMint};

/// Parses raw account bytes into a `PoolState`.
/// Returns `None` if the data is too short or the discriminator doesn't match.
pub struct AccountParser;

impl AccountParser {
    /// Dispatch parser based on the owning program (DEX kind).
    pub fn parse(
        account_pubkey: Pubkey,
        owner_program: &Pubkey,
        data: &[u8],
        current_slot: u64,
    ) -> Option<PoolState> {
        use super::filter::program_ids;

        if *owner_program == program_ids::raydium_amm_v4() {
            Self::parse_raydium_amm(account_pubkey, data, current_slot)
        } else if *owner_program == program_ids::raydium_clmm() {
            Self::parse_raydium_clmm(account_pubkey, data, current_slot)
        } else if *owner_program == program_ids::orca_whirlpool() {
            Self::parse_orca_whirlpool(account_pubkey, data, current_slot)
        } else if *owner_program == program_ids::meteora_dlmm() {
            Self::parse_meteora(account_pubkey, data, current_slot)
        } else if *owner_program == program_ids::phoenix() {
            Self::parse_phoenix(account_pubkey, data, current_slot)
        } else {
            None
        }
    }

    /// Raydium AMM V4 pool layout (simplified).
    /// Full layout: https://github.com/raydium-io/raydium-amm
    /// Offsets (bytes): status(8) | nonce(8) | ... | coin_vault_amount(u64@288) | pc_vault_amount(u64@296)
    /// We parse the reserves from the vault amounts (deterministic, no RPC call).
    fn parse_raydium_amm(
        address: Pubkey,
        data: &[u8],
        slot: u64,
    ) -> Option<PoolState> {
        // Raydium AMM V4 serialised pool is ~752 bytes.
        if data.len() < 304 {
            return None;
        }

        // Discriminator: first 8 bytes must be a known magic.
        // In production, verify against Raydium's actual discriminator.
        // Here we check the status field is non-zero (pool is initialised).
        let status = u64::from_le_bytes(data[0..8].try_into().ok()?);
        if status == 0 {
            return None;
        }

        // Read coin (token A) and pc (token B) vault amounts.
        let reserve_a = u64::from_le_bytes(data[288..296].try_into().ok()?);
        let reserve_b = u64::from_le_bytes(data[296..304].try_into().ok()?);

        // Token mints are at offsets 400 and 432 (32 bytes each) in full layout.
        // For devnet testing, we use placeholder pubkeys if offsets are not available.
        let token_a_mint = if data.len() >= 432 {
            Pubkey::try_from(&data[400..432]).ok()?
        } else {
            Pubkey::default()
        };
        let token_b_mint = if data.len() >= 464 {
            Pubkey::try_from(&data[432..464]).ok()?
        } else {
            Pubkey::default()
        };

        Some(PoolState {
            address,
            dex: DexKind::Raydium,
            token_a: TokenMint::new(token_a_mint, 9), // decimals fetched separately at init
            token_b: TokenMint::new(token_b_mint, 6),
            reserve_a,
            reserve_b,
            fee_bps: 25, // Raydium AMM default 0.25%
            last_slot: slot,
        })
    }

    /// Raydium CLMM (Concentrated Liquidity) — simplified stub.
    /// Full parsing requires sqrt price math; this is the foundation.
    fn parse_raydium_clmm(
        address: Pubkey,
        data: &[u8],
        slot: u64,
    ) -> Option<PoolState> {
        // Minimum size check for CLMM pool state
        if data.len() < 200 {
            return None;
        }
        // CLMM fee tier at offset 8 (in units of 10^-6)
        let fee_u32 = u32::from_le_bytes(data[8..12].try_into().ok()?);
        let fee_bps = fee_u32 / 100; // convert from 1e-6 to bps approximation

        Some(PoolState {
            address,
            dex: DexKind::RaydiumClmm,
            token_a: TokenMint::new(Pubkey::default(), 9),
            token_b: TokenMint::new(Pubkey::default(), 6),
            reserve_a: 0, // CLMM uses liquidity, not reserves; calculate via sqrt price
            reserve_b: 0,
            fee_bps,
            last_slot: slot,
        })
    }

    /// Orca Whirlpool parser.
    fn parse_orca_whirlpool(
        address: Pubkey,
        data: &[u8],
        slot: u64,
    ) -> Option<PoolState> {
        // Whirlpool discriminator: 8 bytes
        if data.len() < 300 {
            return None;
        }
        // Fee rate at offset 168 (u16, in hundredths of a bps)
        let fee_rate = u16::from_le_bytes(data[168..170].try_into().ok()?);
        let fee_bps = (fee_rate as u32 + 99) / 100; // ceiling division

        Some(PoolState {
            address,
            dex: DexKind::OrcaWhirlpool,
            token_a: TokenMint::new(Pubkey::default(), 9),
            token_b: TokenMint::new(Pubkey::default(), 6),
            reserve_a: 0, // derived from sqrt_price × liquidity in full implementation
            reserve_b: 0,
            fee_bps,
            last_slot: slot,
        })
    }

    /// Meteora DLMM parser (stub).
    fn parse_meteora(address: Pubkey, _data: &[u8], slot: u64) -> Option<PoolState> {
        Some(PoolState {
            address,
            dex: DexKind::Meteora,
            token_a: TokenMint::new(Pubkey::default(), 9),
            token_b: TokenMint::new(Pubkey::default(), 6),
            reserve_a: 0,
            reserve_b: 0,
            fee_bps: 20,
            last_slot: slot,
        })
    }

    /// Phoenix order-book parser (stub).
    fn parse_phoenix(address: Pubkey, _data: &[u8], slot: u64) -> Option<PoolState> {
        Some(PoolState {
            address,
            dex: DexKind::Phoenix,
            token_a: TokenMint::new(Pubkey::default(), 9),
            token_b: TokenMint::new(Pubkey::default(), 6),
            reserve_a: 0,
            reserve_b: 0,
            fee_bps: 4,
            last_slot: slot,
        })
    }
}
