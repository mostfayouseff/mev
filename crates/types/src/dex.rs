//! DEX protocol identifiers and pool-type metadata.

use serde::{Deserialize, Serialize};

/// Enumeration of all supported DEX protocols.
/// New DEXes are added here first; downstream match arms must be updated too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DexKind {
    Raydium,
    RaydiumClmm,
    Orca,
    OrcaWhirlpool,
    Meteora,
    Phoenix,
    JupiterV6,
    Unknown,
}

impl std::fmt::Display for DexKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raydium => write!(f, "Raydium"),
            Self::RaydiumClmm => write!(f, "RaydiumCLMM"),
            Self::Orca => write!(f, "Orca"),
            Self::OrcaWhirlpool => write!(f, "OrcaWhirlpool"),
            Self::Meteora => write!(f, "Meteora"),
            Self::Phoenix => write!(f, "Phoenix"),
            Self::JupiterV6 => write!(f, "JupiterV6"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

impl DexKind {
    /// Estimated average swap latency in microseconds for each DEX.
    /// Used by the scorer to penalise slow legs.
    pub fn estimated_latency_us(&self) -> u64 {
        match self {
            Self::Raydium | Self::RaydiumClmm => 200,
            Self::Orca | Self::OrcaWhirlpool => 220,
            Self::Meteora => 250,
            Self::Phoenix => 180,
            Self::JupiterV6 => 400, // aggregator has higher overhead
            Self::Unknown => 1000,
        }
    }
}
