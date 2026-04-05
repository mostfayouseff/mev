// =============================================================================
// APEX ON-CHAIN INSTRUCTION — Fixed & Realistic Multi-Hop Simulation
// =============================================================================

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum ProgramError {
    #[error("Instruction data too short")]
    DataTooShort,
    #[error("Unknown instruction discriminator: {0}")]
    UnknownDiscriminator(u8),
    #[error("Profit guard violated: final={final_balance} < initial={initial_balance} + min={min_profit}")]
    ProfitGuardViolation {
        final_balance: u64,
        initial_balance: u64,
        min_profit: u64,
    },
    #[error("Per-swap check failed at hop {hop}: got={got} < min={min}")]
    PerSwapCheckFailed { hop: usize, got: u64, min: u64 },
    #[error("Arithmetic overflow")]
    ArithmeticOverflow,
}

// Discriminators (single byte, zero-selector style)
const DISC_MULTI_HOP_SWAP: u8 = 0x01;
const DISC_EMERGENCY_REVERT: u8 = 0xFF;

/// On-chain Apex instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApexInstruction {
    MultiHopSwap(MultiHopSwapParams),
    EmergencyRevert,
}

/// Parameters for a multi-hop arbitrage swap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiHopSwapParams {
    pub initial_balance: u64,
    pub min_profit_lamports: u64,
    pub hops: Vec<HopParam>,
    pub lookup_table_indices: Vec<u8>, // ALT v0 stub
}

/// Single hop parameters (Phase 5+).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HopParam {
    pub amount_in: u64,
    pub min_amount_out: u64,
    pub pool_index: u8,
    pub fee_bps: u16,
    #[serde(default = "default_exchange_rate")]
    pub exchange_rate: f64,
}

fn default_exchange_rate() -> f64 {
    1.0
}

impl ApexInstruction {
    /// Parse instruction from raw bytes.
    pub fn from_bytes(data: &Bytes) -> Result<Self, ProgramError> {
        let discriminator = *data.first().ok_or(ProgramError::DataTooShort)?;

        match discriminator {
            DISC_MULTI_HOP_SWAP => parse_multi_hop_swap(data),
            DISC_EMERGENCY_REVERT => Ok(Self::EmergencyRevert),
            other => Err(ProgramError::UnknownDiscriminator(other)),
        }
    }

    /// Simulate execution on-chain (used by pre-simulation).
    pub fn execute_simulated(&self, initial_balance: u64) -> Result<u64, ProgramError> {
        match self {
            Self::EmergencyRevert => Ok(initial_balance),
            Self::MultiHopSwap(params) => simulate_multi_hop(params, initial_balance),
        }
    }
}

// ── Parsing ───────────────────────────────────────────────────────────────────

fn parse_multi_hop_swap(data: &Bytes) -> Result<ApexInstruction, ProgramError> {
    if data.len() < 18 {
        return Err(ProgramError::DataTooShort);
    }

    let initial_balance = read_u64(data, 1)?;
    let min_profit_lamports = read_u64(data, 9)?;
    let n_hops = *data.get(17).ok_or(ProgramError::DataTooShort)? as usize;

    let mut hops = Vec::with_capacity(n_hops);
    let mut offset = 18usize;

    for hop_idx in 0..n_hops {
        if offset + 19 > data.len() {
            return Err(ProgramError::DataTooShort);
        }

        let amount_in = read_u64(data, offset)?;
        let min_amount_out = read_u64(data, offset + 8)?;
        let pool_index = *data.get(offset + 16).unwrap();
        let fee_bps = u16::from_le_bytes([
            *data.get(offset + 17).unwrap_or(&0),
            *data.get(offset + 18).unwrap_or(&0),
        ]);

        hops.push(HopParam {
            amount_in,
            min_amount_out,
            pool_index,
            fee_bps,
            exchange_rate: 1.0, // conservative default when parsed from on-chain bytes
        });

        offset += 19;
    }

    Ok(ApexInstruction::MultiHopSwap(MultiHopSwapParams {
        initial_balance,
        min_profit_lamports,
        hops,
        lookup_table_indices: Vec::new(),
    }))
}

// ── Simulation ───────────────────────────────────────────────────────────────

fn simulate_multi_hop(params: &MultiHopSwapParams, initial_balance: u64) -> Result<u64, ProgramError> {
    let mut running_amount = initial_balance;

    for (hop_idx, hop) in params.hops.iter().enumerate() {
        let fee_bps = if hop.fee_bps == 0 { 30 } else { hop.fee_bps }; // conservative default

        // Realistic hop simulation: out = in * exchange_rate * (1 - fee/10000)
        let after_fee = (running_amount as f64 * (10_000.0 - fee_bps as f64)) / 10_000.0;
        running_amount = (after_fee * hop.exchange_rate) as u64;

        // Per-hop minimum output guard
        if hop.min_amount_out > 0 && running_amount < hop.min_amount_out {
            return Err(ProgramError::PerSwapCheckFailed {
                hop: hop_idx,
                got: running_amount,
                min: hop.min_amount_out,
            });
        }
    }

    // Global profit guard — critical on-chain revert condition
    let required = params.initial_balance.saturating_add(params.min_profit_lamports);
    if running_amount < required {
        return Err(ProgramError::ProfitGuardViolation {
            final_balance: running_amount,
            initial_balance: params.initial_balance,
            min_profit: params.min_profit_lamports,
        });
    }

    Ok(running_amount)
}

/// Fee-only simulation (used in tests and conservative estimates).
#[inline]
pub fn simulate_amm_swap(amount_in: u64, fee_bps: u16) -> u64 {
    simulate_amm_swap_with_rate(amount_in, fee_bps, 1.0)
}

#[inline]
fn simulate_amm_swap_with_rate(amount_in: u64, fee_bps: u16, exchange_rate: f64) -> u64 {
    let fee_factor = (10_000.0 - fee_bps as f64) / 10_000.0;
    (amount_in as f64 * fee_factor * exchange_rate) as u64
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn read_u64(data: &Bytes, offset: usize) -> Result<u64, ProgramError> {
    let end = offset.checked_add(8).ok_or(ProgramError::ArithmeticOverflow)?;
    let slice = data.get(offset..end).ok_or(ProgramError::DataTooShort)?;
    let arr: [u8; 8] = slice.try_into().map_err(|_| ProgramError::DataTooShort)?;
    Ok(u64::from_le_bytes(arr))
}

/// DEX fee lookup (used by off-chain strategy).
#[must_use]
pub fn dex_fee_bps(dex_name: &str) -> u16 {
    match dex_name {
        "Raydium" => 25,
        "Orca" => 30,
        "Meteora" => 20,
        "Phoenix" => 10,
        "JupiterV6" => 30,
        _ => 30, // conservative fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_rate_simulation() {
        let params = MultiHopSwapParams {
            initial_balance: 1_000_000,
            min_profit_lamports: 0,
            hops: vec![HopParam {
                amount_in: 1_000_000,
                min_amount_out: 0,
                pool_index: 0,
                fee_bps: 30,
                exchange_rate: 1.0,
            }],
            lookup_table_indices: vec![],
        };
        let instr = ApexInstruction::MultiHopSwap(params);
        let result = instr.execute_simulated(1_000_000);
        assert!(result.is_ok());
    }

    #[test]
    fn profitable_arb_simulation() {
        let rate = (-(-0.08_f64)).exp(); // ≈1.0833

        let params = MultiHopSwapParams {
            initial_balance: 100_000_000,
            min_profit_lamports: 5_000,
            hops: vec![HopParam {
                amount_in: 100_000_000,
                min_amount_out: 0,
                pool_index: 0,
                fee_bps: 30,
                exchange_rate: rate,
            }],
            lookup_table_indices: vec![],
        };

        let instr = ApexInstruction::MultiHopSwap(params);
        let result = instr.execute_simulated(100_000_000).unwrap();
        assert!(result > 100_000_000 + 5_000);
    }

    #[test]
    fn profit_guard_triggers() {
        let params = MultiHopSwapParams {
            initial_balance: 1_000_000,
            min_profit_lamports: 1_000_000,
            hops: vec![HopParam {
                amount_in: 1_000_000,
                min_amount_out: 0,
                pool_index: 0,
                fee_bps: 30,
                exchange_rate: 1.0,
            }],
            lookup_table_indices: vec![],
        };

        let instr = ApexInstruction::MultiHopSwap(params);
        let result = instr.execute_simulated(1_000_000);
        assert!(matches!(result, Err(ProgramError::ProfitGuardViolation { .. })));
    }

    #[test]
    fn dex_fee_lookup() {
        assert_eq!(dex_fee_bps("Raydium"), 25);
        assert_eq!(dex_fee_bps("Phoenix"), 10);
        assert_eq!(dex_fee_bps("Unknown"), 30);
    }
}
