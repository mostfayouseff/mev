//! config — hot-reloadable, strongly-typed configuration loaded from .env and env vars.
//!
//! DESIGN:
//! - `ArcSwap<BotConfig>` allows zero-downtime hot-reload without a lock in the read path.
//! - All numeric fields are validated on load; invalid configs cause immediate startup failure.
//! - `HOT_RELOAD_CONFIG=true` watches for SIGHUP and reloads from disk — useful for
//!   adjusting `MIN_PROFIT_LAMPORTS` or `SLIPPAGE_BPS` without restarting the bot.

use std::sync::Arc;

use arc_swap::ArcSwap;
use dotenvy::dotenv;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Missing required environment variable: {0}")]
    MissingVar(String),

    #[error("Invalid value for {field}: {reason}")]
    InvalidValue { field: String, reason: String },

    #[error("Environment loading error: {0}")]
    EnvError(String),
}

/// Master bot configuration. All fields are validated on construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    // ─── RPC ──────────────────────────────────────────────────────────────────
    /// Private RPC endpoint (required for pre-simulation).
    pub rpc_url: String,
    /// WebSocket endpoint for account/slot notifications.
    pub ws_url: String,

    // ─── Keypair ──────────────────────────────────────────────────────────────
    /// Base58-encoded private key. NEVER log this value.
    pub private_key: String,

    // ─── Execution ────────────────────────────────────────────────────────────
    /// Slippage tolerance in basis points.
    pub slippage_bps: u32,
    /// Minimum net profit in lamports for a trade to proceed.
    pub min_profit_lamports: i64,
    /// Maximum single-trade position in lamports (risk limit).
    pub max_position_size: u64,
    /// Base priority fee per compute unit in lamports.
    pub priority_fee_lamports: u64,
    /// Whether to actually submit transactions (false = paper trading).
    pub execute_trades: bool,
    /// Rate limit: max trades per 60-second window.
    pub max_trades_per_minute: u32,
    /// Number of consecutive losses before circuit breaker trips.
    pub circuit_breaker_threshold: u32,

    // ─── Jito ─────────────────────────────────────────────────────────────────
    /// Enable Jito bundle submission.
    pub use_jito: bool,
    /// Jito tip in lamports added to bundles when `use_jito = true`.
    pub jito_tip_lamports: u64,

    // ─── Safety ───────────────────────────────────────────────────────────────
    /// If true, simulate transactions but never submit.
    pub dry_run: bool,
    /// Maximum fraction of our balance to borrow in flash-loan scenarios (0.0–1.0).
    pub max_borrow_ratio: f64,

    // ─── Observability ────────────────────────────────────────────────────────
    pub log_metrics: bool,
    pub hot_reload_config: bool,
    pub performance_monitoring: bool,

    // ─── Path finding ─────────────────────────────────────────────────────────
    /// Maximum hops to consider in path discovery.
    pub max_hops: usize,
    /// Maximum number of candidate paths to evaluate per tick.
    pub max_candidate_paths: usize,
    /// Maximum pool staleness (in slots) before the path is skipped.
    pub max_pool_age_slots: u64,
}

impl BotConfig {
    /// Load configuration from environment (after `dotenv()` has been called).
    pub fn from_env() -> Result<Self, ConfigError> {
        // Helper closures
        let require = |key: &str| -> Result<String, ConfigError> {
            std::env::var(key).map_err(|_| ConfigError::MissingVar(key.to_string()))
        };
        let parse_u64 = |key: &str, default: u64| -> Result<u64, ConfigError> {
            match std::env::var(key) {
                Ok(v) => v.parse::<u64>().map_err(|e| ConfigError::InvalidValue {
                    field: key.to_string(),
                    reason: e.to_string(),
                }),
                Err(_) => Ok(default),
            }
        };
        let parse_i64 = |key: &str, default: i64| -> Result<i64, ConfigError> {
            match std::env::var(key) {
                Ok(v) => v.parse::<i64>().map_err(|e| ConfigError::InvalidValue {
                    field: key.to_string(),
                    reason: e.to_string(),
                }),
                Err(_) => Ok(default),
            }
        };
        let parse_u32 = |key: &str, default: u32| -> Result<u32, ConfigError> {
            match std::env::var(key) {
                Ok(v) => v.parse::<u32>().map_err(|e| ConfigError::InvalidValue {
                    field: key.to_string(),
                    reason: e.to_string(),
                }),
                Err(_) => Ok(default),
            }
        };
        let parse_bool = |key: &str, default: bool| -> bool {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<bool>().ok())
                .unwrap_or(default)
        };
        let parse_f64 = |key: &str, default: f64| -> Result<f64, ConfigError> {
            match std::env::var(key) {
                Ok(v) => v.parse::<f64>().map_err(|e| ConfigError::InvalidValue {
                    field: key.to_string(),
                    reason: e.to_string(),
                }),
                Err(_) => Ok(default),
            }
        };
        let parse_usize = |key: &str, default: usize| -> Result<usize, ConfigError> {
            match std::env::var(key) {
                Ok(v) => v.parse::<usize>().map_err(|e| ConfigError::InvalidValue {
                    field: key.to_string(),
                    reason: e.to_string(),
                }),
                Err(_) => Ok(default),
            }
        };

        let cfg = BotConfig {
            rpc_url: require("RPC_URL_PRIVATE")?,
            ws_url: require("WS_URL_PRIVATE")?,
            private_key: require("PRIVATE_KEY")?,
            slippage_bps: parse_u32("SLIPPAGE_BPS", 50)?,
            min_profit_lamports: parse_i64("MIN_PROFIT_LAMPORTS", 10_000)?,
            max_position_size: parse_u64("MAX_POSITION_SIZE", 200_000_000)?,
            priority_fee_lamports: parse_u64("PRIORITY_FEE_LAMPORTS", 5_000)?,
            execute_trades: parse_bool("EXECUTE_TRADES", false),
            max_trades_per_minute: parse_u32("MAX_TRADES_PER_MINUTE", 10)?,
            circuit_breaker_threshold: parse_u32("CIRCUIT_BREAKER_THRESHOLD", 5)?,
            use_jito: parse_bool("USE_JITO", false),
            jito_tip_lamports: parse_u64("JITO_TIP_LAMPORTS", 10_000)?,
            dry_run: parse_bool("DRY_RUN", true),
            max_borrow_ratio: parse_f64("MAX_BORROW_RATIO", 0.8)?,
            log_metrics: parse_bool("LOG_METRICS", true),
            hot_reload_config: parse_bool("HOT_RELOAD_CONFIG", true),
            performance_monitoring: parse_bool("PERFORMANCE_MONITORING", true),
            max_hops: parse_usize("MAX_HOPS", 6)?,
            max_candidate_paths: parse_usize("MAX_CANDIDATE_PATHS", 512)?,
            max_pool_age_slots: parse_u64("MAX_POOL_AGE_SLOTS", 32)?,
        };

        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.slippage_bps > 10_000 {
            return Err(ConfigError::InvalidValue {
                field: "SLIPPAGE_BPS".into(),
                reason: "must be ≤ 10_000".into(),
            });
        }
        if self.max_borrow_ratio <= 0.0 || self.max_borrow_ratio > 1.0 {
            return Err(ConfigError::InvalidValue {
                field: "MAX_BORROW_RATIO".into(),
                reason: "must be in (0, 1]".into(),
            });
        }
        if self.max_hops > 6 {
            return Err(ConfigError::InvalidValue {
                field: "MAX_HOPS".into(),
                reason: "must be ≤ 6".into(),
            });
        }
        if self.private_key.contains("INSERT") || self.private_key.is_empty() {
            return Err(ConfigError::MissingVar("PRIVATE_KEY (real value required)".into()));
        }
        Ok(())
    }
}

/// Shared, hot-swappable config handle. Readers pay no lock cost.
pub type SharedConfig = Arc<ArcSwap<BotConfig>>;

/// Initialise config from `.env` file + environment variables.
/// Returns an `ArcSwap`-wrapped config for zero-cost reads in the hot path.
pub fn load_config() -> Result<SharedConfig, ConfigError> {
    // Load .env if it exists; ignore errors (env vars may already be set).
    let _ = dotenv();

    let cfg = BotConfig::from_env()?;
    info!(
        slippage_bps = cfg.slippage_bps,
        min_profit_lamports = cfg.min_profit_lamports,
        execute_trades = cfg.execute_trades,
        dry_run = cfg.dry_run,
        "Configuration loaded"
    );
    Ok(Arc::new(ArcSwap::from_pointee(cfg)))
}

/// Hot-reload config in-place without restarting the process.
/// Called on SIGHUP when `HOT_RELOAD_CONFIG=true`.
pub fn reload_config(shared: &SharedConfig) -> Result<(), ConfigError> {
    let _ = dotenv();
    let new_cfg = BotConfig::from_env()?;
    shared.store(Arc::new(new_cfg));
    info!("Configuration hot-reloaded");
    Ok(())
}
