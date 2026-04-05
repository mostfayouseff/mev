// =============================================================================
// PRE-SIMULATOR — Multi-Hop Swap Simulation with Realistic Fees & Exchange Rates
// =============================================================================

use tracing::{debug, info, warn};

/// Result of pre-simulation.
#[derive(Debug)]
pub struct SimulationResult {
    pub success: bool,
    pub final_balance: u64,
    pub expected_profit_lamports: u64,
    pub error: Option<String>,
}

impl SimulationResult {
    #[must_use]
    pub fn is_profitable(&self, min_profit: u64) -> bool {
        self.success && self.expected_profit_lamports >= min_profit
    }
}

/// Pre-simulation engine for multi-hop arbitrage paths.
pub struct PreSimulator {
    min_profit_lamports: u64,
}

impl PreSimulator {
    #[must_use]
    pub fn new(min_profit_lamports: u64) -> Self {
        Self { min_profit_lamports }
    }

    /// Simulate a multi-hop swap chain.
    ///
    /// Each hop: `amount_in → amount_out = amount_in * exchange_rate - fee`
    pub fn simulate_swap(
        &self,
        initial_balance: u64,
        hops: &[(u64, u64, u16, f64)], // (amount_in, min_out, fee_bps, exchange_rate)
        min_profit_lamports: u64,
    ) -> SimulationResult {
        let mut current_balance = initial_balance;
        let mut total_profit = 0i64;

        for (i, &(amount_in, min_out, fee_bps, exchange_rate)) in hops.iter().enumerate() {
            if current_balance < amount_in {
                return SimulationResult {
                    success: false,
                    final_balance: current_balance,
                    expected_profit_lamports: 0,
                    error: Some(format!("Insufficient balance at hop {}", i)),
                };
            }

            // Apply exchange rate
            let mut out_amount = (amount_in as f64 * exchange_rate) as u64;

            // Apply DEX fee
            let fee_lamports = (out_amount as u64 * fee_bps as u64) / 10_000;
            out_amount = out_amount.saturating_sub(fee_lamports);

            // Enforce per-hop minimum output (if set)
            if min_out > 0 && out_amount < min_out {
                return SimulationResult {
                    success: false,
                    final_balance: current_balance,
                    expected_profit_lamports: 0,
                    error: Some(format!("Hop {} failed min_out check", i)),
                };
            }

            current_balance = current_balance.saturating_sub(amount_in) + out_amount;
            total_profit = current_balance as i64 - initial_balance as i64;

            debug!(
                hop = i,
                in_amount = amount_in,
                out_amount,
                fee_lamports,
                exchange_rate,
                current_balance,
                "Hop simulation"
            );
        }

        let profit_lamports = current_balance.saturating_sub(initial_balance);

        info!(
            initial_lamports = initial_balance,
            final_lamports = current_balance,
            profit_lamports = profit_lamports,
            profit_sol = format!("{:.6}", profit_lamports as f64 / 1e9),
            n_hops = hops.len(),
            "PRE-SIMULATION COMPLETE"
        );

        SimulationResult {
            success: true,
            final_balance: current_balance,
            expected_profit_lamports: profit_lamports,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_rate_no_profit() {
        let sim = PreSimulator::new(5_000);
        let result = sim.simulate_swap(
            1_000_000,
            &[(1_000_000, 0, 30, 1.0)],
            0,
        );
        assert!(result.success);
        assert_eq!(result.expected_profit_lamports, 0); // fees eat profit
    }

    #[test]
    fn favourable_rate_generates_profit() {
        let sim = PreSimulator::new(5_000);
        let rate = (-(-0.08_f64)).exp(); // ~1.083287

        let result = sim.simulate_swap(
            100_000_000,
            &[(100_000_000, 0, 30, rate)],
            5_000,
        );

        assert!(result.success);
        assert!(result.expected_profit_lamports > 5_000);
    }

    #[test]
    fn multi_hop_arb_simulation() {
        let sim = PreSimulator::new(10_000);
        let r1 = (-(-0.08_f64)).exp();
        let r2 = (-(-0.05_f64)).exp();
        let r3 = (-(-0.06_f64)).exp();

        let result = sim.simulate_swap(
            100_000_000,
            &[
                (100_000_000, 0, 30, r1),
                (100_000_000, 0, 25, r2),
                (100_000_000, 0, 30, r3),
            ],
            10_000,
        );

        assert!(result.success);
        assert!(result.expected_profit_lamports > 10_000_000);
    }

    #[test]
    fn insufficient_balance_fails() {
        let sim = PreSimulator::new(0);
        let result = sim.simulate_swap(500_000, &[(1_000_000, 0, 30, 1.0)], 0);
        assert!(!result.success);
    }

    #[test]
    fn is_profitable_check() {
        let result = SimulationResult {
            success: true,
            final_balance: 1_050_000,
            expected_profit_lamports: 50_000,
            error: None,
        };
        assert!(result.is_profitable(10_000));
        assert!(!result.is_profitable(100_000));
    }
}
