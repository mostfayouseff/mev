//! Strategy hooks — pluggable extension points for custom strategies.
//!
//! Hooks are called synchronously in the strategy loop before the opportunity
//! is emitted to the safety layer. They can veto, modify, or augment opportunities.

use crate::Opportunity;

/// A hook that can inspect or modify an opportunity before it proceeds to simulation.
pub trait StrategyHook: Send + Sync {
    fn name(&self) -> &'static str;

    /// Called with the candidate opportunity.
    /// Return `Some(opportunity)` to pass it through (optionally modified).
    /// Return `None` to veto (drop) the opportunity.
    fn on_opportunity(&self, opp: Opportunity) -> Option<Opportunity>;
}

/// Hook: reject opportunities whose path traverses more than `max_hops` DEXes.
pub struct MaxHopsFilter {
    pub max_hops: usize,
}

impl StrategyHook for MaxHopsFilter {
    fn name(&self) -> &'static str {
        "MaxHopsFilter"
    }
    fn on_opportunity(&self, opp: Opportunity) -> Option<Opportunity> {
        if opp.path.path.len() > self.max_hops {
            None
        } else {
            Some(opp)
        }
    }
}

/// Hook: reject opportunities where the expected profit per hop is below a threshold.
pub struct MinProfitPerHopFilter {
    pub min_lamports_per_hop: i64,
}

impl StrategyHook for MinProfitPerHopFilter {
    fn name(&self) -> &'static str {
        "MinProfitPerHopFilter"
    }
    fn on_opportunity(&self, opp: Opportunity) -> Option<Opportunity> {
        let hops = opp.path.path.len() as i64;
        let profit_per_hop = opp.path.path.net_profit_lamports / hops.max(1);
        if profit_per_hop < self.min_lamports_per_hop {
            None
        } else {
            Some(opp)
        }
    }
}

/// Hook: launch-sniping — only pass through opportunities detected within
/// `max_age_ns` nanoseconds of pool update (freshness guard).
pub struct FreshnessGuard {
    pub max_age_ns: u64,
}

impl StrategyHook for FreshnessGuard {
    fn name(&self) -> &'static str {
        "FreshnessGuard"
    }
    fn on_opportunity(&self, opp: Opportunity) -> Option<Opportunity> {
        let age_ns = common::monotonic_ns().saturating_sub(opp.created_at_ns);
        if age_ns > self.max_age_ns {
            None
        } else {
            Some(opp)
        }
    }
}
