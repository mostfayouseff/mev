//! Risk checks — position size limits and borrow ratio enforcement.

use common::ApexError;

pub struct RiskChecker {
    max_position_size: u64,
    max_borrow_ratio: f64,
    wallet_balance: u64,
}

impl RiskChecker {
    pub fn new(max_position_size: u64, max_borrow_ratio: f64, wallet_balance: u64) -> Self {
        Self {
            max_position_size,
            max_borrow_ratio,
            wallet_balance,
        }
    }

    /// Update the wallet balance (called after each confirmed trade).
    pub fn update_balance(&mut self, balance: u64) {
        self.wallet_balance = balance;
    }

    /// Fail if the requested position size exceeds the configured maximum.
    pub fn check_position_size(&self, amount: u64) -> Result<(), ApexError> {
        if amount > self.max_position_size {
            return Err(ApexError::Safety(format!(
                "Position size {amount} exceeds max {}", self.max_position_size
            )));
        }
        Ok(())
    }

    /// Fail if the amount to borrow would exceed `max_borrow_ratio * wallet_balance`.
    pub fn check_borrow_ratio(&self, borrow_amount: u64) -> Result<(), ApexError> {
        let max_borrow = (self.wallet_balance as f64 * self.max_borrow_ratio) as u64;
        if borrow_amount > max_borrow {
            return Err(ApexError::Safety(format!(
                "Borrow {borrow_amount} > max allowed {max_borrow} (ratio {:.2})",
                self.max_borrow_ratio
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_size_limit() {
        let r = RiskChecker::new(100_000_000, 0.8, 500_000_000);
        assert!(r.check_position_size(50_000_000).is_ok());
        assert!(r.check_position_size(200_000_000).is_err());
    }

    #[test]
    fn borrow_ratio_limit() {
        let r = RiskChecker::new(500_000_000, 0.8, 100_000_000);
        assert!(r.check_borrow_ratio(79_000_000).is_ok());
        assert!(r.check_borrow_ratio(90_000_000).is_err());
    }
}
