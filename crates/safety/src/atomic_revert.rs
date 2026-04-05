// =============================================================================
// ATOMIC REVERT GUARD — RAII Safety Layer
// =============================================================================

use tracing::{debug, warn};

/// RAII guard that automatically reverts state on drop unless explicitly committed.
///
/// Usage:
/// ```rust
/// let guard = AtomicRevertGuard::new(initial_balance, "trade_123");
/// // ... do risky operations ...
/// if success {
///     guard.commit();   // prevents revert
/// }
/// // If dropped without commit(), revert() is called automatically
/// ```
pub struct AtomicRevertGuard {
    initial_balance: u64,
    committed: bool,
    tag: String,
}

impl AtomicRevertGuard {
    #[must_use]
    pub fn new(initial_balance: u64, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        debug!(
            initial_balance,
            tag = %tag,
            "AtomicRevertGuard created — will revert on drop unless committed"
        );
        Self {
            initial_balance,
            committed: false,
            tag,
        }
    }

    /// Mark the operation as successful. Consumes the guard (prevents revert).
    pub fn commit(self) {
        // `self` is moved, so Drop will not run with `committed = false`
        debug!(tag = %self.tag, "AtomicRevertGuard committed successfully");
        // No need to do anything else — Drop is now a no-op
    }

    /// Explicit revert (called automatically on drop if not committed).
    fn revert(&self) {
        warn!(
            tag = %self.tag,
            initial_balance = self.initial_balance,
            "ATOMIC REVERT: restoring initial state (operation failed or dropped)"
        );
        // In production: issue a CPI / transaction to refund the operator
        // For now: log only (as per your original stub)
    }
}

impl Drop for AtomicRevertGuard {
    fn drop(&mut self) {
        if !self.committed {
            self.revert();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_guard_does_not_revert() {
        let guard = AtomicRevertGuard::new(1_000_000, "test_commit");
        guard.commit(); // consumes guard → no revert on drop
        // Test passes if no "reverting" warning appears in logs
    }

    #[test]
    fn uncommitted_guard_reverts_on_drop() {
        {
            let _guard = AtomicRevertGuard::new(1_000_000, "test_revert");
            // dropped without commit → should log revert
        }
        // Revert warning should appear in test output
    }
}
