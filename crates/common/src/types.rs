use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

// =============================================================================
// TOKEN MINT
// =============================================================================

/// Opaque newtype for Solana token mint (32-byte pubkey).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenMint(pub [u8; 32]);

impl TokenMint {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[cfg(test)]
    pub fn zero() -> Self {
        Self([0u8; 32])
    }

    #[must_use]
    pub fn to_base58(&self) -> String {
        bs58::encode(self.0).into_string()
    }
}

impl fmt::Display for TokenMint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_base58())
    }
}

// Custom Serialize / Deserialize (manual impl to avoid conflict with derive)
impl Serialize for TokenMint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base58())
    }
}

impl<'de> Deserialize<'de> for TokenMint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse::<TokenMint>().map_err(serde::de::Error::custom)
    }
}

impl FromStr for TokenMint {
    type Err = bs58::decode::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = bs58::decode(s).into_vec()?;
        if bytes.len() != 32 {
            return Err(bs58::decode::Error::BufferTooSmall);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

// =============================================================================
// DEX & PATH TYPES
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dex {
    Raydium,
    Orca,
    Meteora,
    Phoenix,
    JupiterV6,
}

impl fmt::Display for Dex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raydium => write!(f, "Raydium"),
            Self::Orca => write!(f, "Orca"),
            Self::Meteora => write!(f, "Meteora"),
            Self::Phoenix => write!(f, "Phoenix"),
            Self::JupiterV6 => write!(f, "JupiterV6"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketEdge {
    pub from: TokenMint,
    pub to: TokenMint,
    pub dex: Dex,
    pub log_weight: Decimal,
    pub liquidity_lamports: u64,
    pub slot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbPath {
    pub edges: Vec<MarketEdge>,
    pub expected_profit_lamports: u64,
    pub gnn_confidence: f32,
    pub rich_color: RichColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichColor {
    White,
    Gray,
    Black,
}

#[derive(Debug, Clone)]
pub struct PriceMatrix {
    pub n: usize,
    pub data: Vec<f64>, // row-major
    pub tokens: Vec<TokenMint>,
}

impl PriceMatrix {
    #[must_use]
    pub fn new(tokens: Vec<TokenMint>) -> Self {
        let n = tokens.len();
        Self {
            n,
            data: vec![f64::INFINITY; n * n],
            tokens,
        }
    }

    #[inline]
    #[must_use]
    pub fn get(&self, row: usize, col: usize) -> Option<f64> {
        let idx = row.checked_mul(self.n)?.checked_add(col)?;
        self.data.get(idx).copied()
    }

    #[inline]
    pub fn set(&mut self, row: usize, col: usize, val: f64) -> bool {
        if let Some(idx) = row.checked_mul(self.n).and_then(|r| r.checked_add(col)) {
            if let Some(slot) = self.data.get_mut(idx) {
                *slot = val;
                return true;
            }
        }
        false
    }
}
