//! Dynamic tip strategy — adjusts Jito tip based on opportunity value.
//!
//! STRATEGY:
//! - Tip = max(base_tip, profit * tip_fraction).
//! - `tip_fraction` starts at 10% and is adjusted based on recent landing rate.
//! - A poor landing rate → increase tip fraction; good rate → decrease.
//! - Tip is always capped at `max_tip_lamports` (risk control).

use std::collections::VecDeque;
use std::time::Duration;

use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_program,
};
use tracing::debug;

/// Jito tip accounts (one chosen randomly per bundle for load balancing).
/// Source: https://jito-foundation.gitbook.io/mev/searchers/bundles/tip-payment
pub const JITO_TIP_ACCOUNTS: [&str; 8] = [
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

/// Build a SOL tip transfer instruction to a Jito tip account.
/// Uses a round-robin selection of tip accounts for load balancing.
pub fn build_tip_instruction(
    payer: &Pubkey,
    tip_lamports: u64,
    tip_account_idx: usize,
) -> Instruction {
    let tip_account_str = JITO_TIP_ACCOUNTS[tip_account_idx % JITO_TIP_ACCOUNTS.len()];
    let tip_account = tip_account_str.parse::<Pubkey>().expect("Invalid Jito tip account");

    // SOL transfer via system program
    solana_sdk::system_instruction::transfer(payer, &tip_account, tip_lamports)
}

/// Adaptive tip calculator.
pub struct TipStrategy {
    base_tip: u64,
    max_tip: u64,
    /// Fraction of profit to tip (0.0–1.0).
    tip_fraction: f64,
    /// Ring buffer of recent landing results for rate tracking.
    landing_history: VecDeque<bool>,
    history_window: usize,
    /// Counter for round-robin tip account selection.
    account_cursor: usize,
}

impl TipStrategy {
    pub fn new(base_tip: u64, max_tip: u64) -> Self {
        Self {
            base_tip,
            max_tip,
            tip_fraction: 0.10, // Start at 10%
            landing_history: VecDeque::new(),
            history_window: 20,
            account_cursor: 0,
        }
    }

    /// Calculate the tip for an opportunity with the given profit.
    pub fn calculate_tip(&self, profit_lamports: i64) -> u64 {
        if profit_lamports <= 0 {
            return self.base_tip;
        }
        let profit_based = (profit_lamports as f64 * self.tip_fraction) as u64;
        self.max_tip.min(self.base_tip.max(profit_based))
    }

    /// Record whether the last bundle landed.
    pub fn record_result(&mut self, landed: bool) {
        self.landing_history.push_back(landed);
        if self.landing_history.len() > self.history_window {
            self.landing_history.pop_front();
        }
        self.adapt();
    }

    /// Adapt tip fraction based on recent landing rate.
    fn adapt(&mut self) {
        if self.landing_history.len() < 5 {
            return; // Not enough data yet
        }
        let lands = self.landing_history.iter().filter(|&&l| l).count();
        let rate = lands as f64 / self.landing_history.len() as f64;

        if rate < 0.5 {
            // Poor landing rate: increase tip
            self.tip_fraction = (self.tip_fraction * 1.1).min(0.5);
        } else if rate > 0.8 {
            // Good landing rate: decrease tip (save money)
            self.tip_fraction = (self.tip_fraction * 0.95).max(0.05);
        }

        debug!(rate, tip_fraction = self.tip_fraction, "Tip strategy adapted");
    }

    /// Next tip account address (round-robin).
    pub fn next_tip_account_idx(&mut self) -> usize {
        let idx = self.account_cursor;
        self.account_cursor = (self.account_cursor + 1) % JITO_TIP_ACCOUNTS.len();
        idx
    }
}
