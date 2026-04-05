// =============================================================================
// eBPF-STYLE PACKET FILTER — Hot-path pre-filter
// =============================================================================

use super::ShredEvent;

/// Filtering rule for incoming shreds/events.
#[derive(Debug, Clone)]
pub enum FilterRule {
    MinDataLen(usize),
    MaxDataLen(usize),
    MagicPrefix(Vec<u8>),
    SlotRange { min: u64, max: u64 },
}

/// Lightweight eBPF-style filter. Runs before heavy processing.
pub struct EbpfFilter {
    rules: Vec<FilterRule>,
}

impl EbpfFilter {
    #[must_use]
    pub fn new(rules: Vec<FilterRule>) -> Self {
        Self { rules }
    }

    /// Default filter optimized for Solana DEX arbitrage events.
    #[must_use]
    pub fn default_arb_filter() -> Self {
        Self::new(vec![
            FilterRule::MinDataLen(32),
            FilterRule::MaxDataLen(1200),
        ])
    }

    /// Returns `true` if the event passes all rules.
    #[must_use]
    pub fn accepts(&self, event: &ShredEvent) -> bool {
        for rule in &self.rules {
            if !Self::evaluate_rule(rule, event) {
                return false;
            }
        }
        true
    }

    #[inline]
    fn evaluate_rule(rule: &FilterRule, event: &ShredEvent) -> bool {
        match rule {
            FilterRule::MinDataLen(min) => event.data.len() >= *min,
            FilterRule::MaxDataLen(max) => event.data.len() <= *max,
            FilterRule::MagicPrefix(prefix) => event.data.starts_with(prefix),
            FilterRule::SlotRange { min, max } => event.slot >= *min && event.slot <= *max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn make_event(slot: u64, data: Vec<u8>) -> ShredEvent {
        ShredEvent {
            slot,
            index: 0,
            data: Bytes::from(data),
        }
    }

    #[test]
    fn accepts_valid_event() {
        let filter = EbpfFilter::default_arb_filter();
        let event = make_event(100, vec![0u8; 64]);
        assert!(filter.accepts(&event));
    }

    #[test]
    fn rejects_too_small() {
        let filter = EbpfFilter::default_arb_filter();
        let event = make_event(100, vec![0u8; 10]);
        assert!(!filter.accepts(&event));
    }

    #[test]
    fn rejects_too_large() {
        let filter = EbpfFilter::default_arb_filter();
        let event = make_event(100, vec![0u8; 1201]);
        assert!(!filter.accepts(&event));
    }
}
