//! core — deterministic path-finding and scoring engine.
//!
//! DESIGN:
//! - Path discovery uses a modified Bellman-Ford / negative cycle detection
//!   (RICH-inspired) operating in log-space so that multiplicative profit becomes
//!   additive. This makes cycle detection equivalent to finding a negative cycle.
//! - All arithmetic is deterministic: no floats in the decision path; fixed-point
//!   log-approximations use integer arithmetic.
//! - The pool registry is a `DashMap` (lock-free concurrent hashmap) keyed by `Pubkey`.
//!   Readers never block writers.
//! - Path cache is pre-computed at startup and refreshed only when the pool set changes,
//!   not on every tick.

pub mod optimizer;
pub mod registry;
pub mod scorer;
pub mod path_finder;

pub use optimizer::InputOptimizer;
pub use registry::PoolRegistry;
pub use scorer::PathScorer;
pub use path_finder::PathFinder;
