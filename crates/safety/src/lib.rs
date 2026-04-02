//! safety — pre-simulation, circuit breakers, and position-size risk controls.
//!
//! DESIGN:
//! - **Pre-simulation ALWAYS runs before execution.** If simulateTransaction fails or
//!   returns a profit below `MIN_PROFIT_LAMPORTS`, the opportunity is CANCELLED here.
//!   Nothing reaches the execution layer without passing safety.
//! - The circuit breaker is a simple atomic loss counter. When losses exceed
//!   `CIRCUIT_BREAKER_THRESHOLD`, all execution is blocked until manual reset.
//! - Borrow validation: only allows borrowing if the simulated profit covers the
//!   borrow cost + principal repayment.
//! - All checks are deterministic (same inputs → same decision).

pub mod circuit_breaker;
pub mod pre_sim;
pub mod risk;

pub use circuit_breaker::CircuitBreaker;
pub use pre_sim::{SimulationResult, TransactionSimulator};
pub use risk::RiskChecker;

use common::ApexError;
use strategy::Opportunity;

/// A validated opportunity — only created after passing all safety checks.
#[derive(Debug, Clone)]
pub struct ValidatedOpportunity {
    pub opportunity: Opportunity,
    pub sim_result: SimulationResult,
    /// Actual expected profit confirmed by simulation (may differ from estimate).
    pub confirmed_profit_lamports: i64,
}

/// Run all safety checks on an opportunity.
/// Returns `Ok(ValidatedOpportunity)` on pass, `Err` with reason on fail.
pub async fn validate_opportunity(
    opp: Opportunity,
    simulator: &TransactionSimulator,
    risk: &RiskChecker,
    breaker: &CircuitBreaker,
    min_profit_lamports: i64,
) -> Result<ValidatedOpportunity, ApexError> {
    // ── 1. Circuit breaker ───────────────────────────────────────────────────
    if breaker.is_tripped() {
        return Err(ApexError::CircuitBreaker("Circuit breaker is open".into()));
    }

    // ── 2. Risk check ────────────────────────────────────────────────────────
    risk.check_position_size(opp.optimal_input)?;
    risk.check_borrow_ratio(opp.optimal_input)?;

    // ── 3. Pre-simulation ────────────────────────────────────────────────────
    metrics::metrics().sim_attempts.inc();

    let sim_result = simulator
        .simulate(&opp)
        .await
        .map_err(|e| {
            metrics::metrics().sim_failures.inc();
            e
        })?;

    // ── 4. Profit threshold after simulation ─────────────────────────────────
    if sim_result.net_profit_lamports < min_profit_lamports {
        metrics::metrics().fail_fast_cancels.inc();
        return Err(ApexError::InsufficientProfit {
            got_lamports: sim_result.net_profit_lamports,
            min_lamports: min_profit_lamports,
        });
    }

    Ok(ValidatedOpportunity {
        confirmed_profit_lamports: sim_result.net_profit_lamports,
        opportunity: opp,
        sim_result,
    })
}
