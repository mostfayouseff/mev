// =============================================================================
// APEX-MEV CONFIGURATION
// =============================================================================

use anyhow::Result;
use serde::Deserialize;
use tracing::info;

#[derive(Debug, Deserialize, Clone)]
pub struct ApexConfig {
    // ── Network ──────────────────────────────────────────────────────────────
    pub rpc_url: String,
    pub http_rpc_url: String,
    pub keypair_path: String,

    // ── API Keys ─────────────────────────────────────────────────────────────
    pub helius_api_key: Option<String>,
    pub alchemy_api_key: Option<String>,
    pub jupiter_api_key: Option<String>,

    // ── Strategy ─────────────────────────────────────────────────────────────
    pub min_profit_lamports: u64,
    pub max_hops: usize,
    pub max_position_lamports: u64,
    pub slippage_bps: u16,

    // ── Execution ────────────────────────────────────────────────────────────
    pub flash_loan_enabled: bool,
    pub jito_url: String,

    // ── Risk Management ──────────────────────────────────────────────────────
    pub circuit_breaker_threshold_lamports: u64,
    pub circuit_breaker_consecutive_losses: u32,
    pub max_daily_loss_lamports: u64,

    // ── Optimization ─────────────────────────────────────────────────────────
    pub auto_optimize: bool,

    // ── Mode ─────────────────────────────────────────────────────────────────
    pub simulation_only: bool,
}

impl ApexConfig {
    pub fn from_env() -> Result<Self> {
        let env_name = std::env::var("APEX_ENV").unwrap_or_else(|_| "development".to_string());
        if env_name != "production" {
            let _ = dotenvy::dotenv();
            info!("Loaded .env file (environment: {env_name})");
        }

        let helius_api_key = optional_env("HELIUS_API_KEY");
        let alchemy_api_key = optional_env("ALCHEMY_API_KEY");
        let jupiter_api_key = optional_env("JUPITER_API_KEY");

        let simulation_only = parse_env_bool("APEX_SIMULATION_ONLY", false)?;

        let helius_http = helius_api_key
            .as_deref()
            .map(|k| format!("https://mainnet.helius-rpc.com/?api-key={k}"))
            .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string());

        let helius_ws = helius_api_key
            .as_deref()
            .map(|k| format!("wss://mainnet.helius-rpc.com/?api-key={k}"))
            .unwrap_or_else(|| "wss://api.mainnet-beta.solana.com".to_string());

        let cfg = Self {
            rpc_url: optional_env("APEX_RPC_URL").unwrap_or_else(|| helius_ws.clone()),
            http_rpc_url: optional_env("APEX_HTTP_RPC_URL").unwrap_or_else(|| helius_http.clone()),
            keypair_path: optional_env("APEX_KEYPAIR_PATH")
                .unwrap_or_else(|| "live-key.json".to_string()),

            helius_api_key: helius_api_key.clone(),
            alchemy_api_key,
            jupiter_api_key,

            min_profit_lamports: parse_env_u64("APEX_MIN_PROFIT_LAMPORTS", 0)?,
            max_hops: parse_env_usize("APEX_MAX_HOPS", 4)?,
            max_position_lamports: parse_env_u64("APEX_MAX_POSITION_LAMPORTS", 1_000_000_000)?,
            slippage_bps: parse_env_u16("APEX_SLIPPAGE_BPS", 50)?,

            flash_loan_enabled: parse_env_bool("APEX_FLASH_LOAN_ENABLED", true)?,
            jito_url: optional_env("APEX_JITO_URL")
                .unwrap_or_else(|| "https://mainnet.block-engine.jito.wtf".to_string()),

            circuit_breaker_threshold_lamports: parse_env_u64("APEX_CB_THRESHOLD_LAMPORTS", 5_000_000_000)?,
            circuit_breaker_consecutive_losses: parse_env_u32("APEX_CB_CONSECUTIVE_LOSSES", 10)?,
            max_daily_loss_lamports: parse_env_u64("APEX_MAX_DAILY_LOSS_LAMPORTS", 500_000_000)?,

            auto_optimize: parse_env_bool("APEX_AUTO_OPTIMIZE", true)?,
            simulation_only,
        };

        info!(
            simulation_only = cfg.simulation_only,
            flash_loan_enabled = cfg.flash_loan_enabled,
            auto_optimize = cfg.auto_optimize,
            slippage_bps = cfg.slippage_bps,
            min_profit_lamports = cfg.min_profit_lamports,
            max_position_sol = cfg.max_position_lamports as f64 / 1e9,
            helius_active = cfg.helius_api_key.as_ref().map_or(false, |k| !k.is_empty()),
            "ApexConfig loaded successfully"
        );

        Ok(cfg)
    }
}

// ─── Helper parsers ───────────────────────────────────────────────────────────

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn parse_env_u64(key: &str, default: u64) -> Result<u64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Ok(default), Ok)
}

fn parse_env_u16(key: &str, default: u16) -> Result<u16> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Ok(default), Ok)
}

fn parse_env_usize(key: &str, default: usize) -> Result<usize> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Ok(default), Ok)
}

fn parse_env_u32(key: &str, default: u32) -> Result<u32> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(Ok(default), Ok)
}

fn parse_env_bool(key: &str, default: bool) -> Result<bool> {
    match std::env::var(key) {
        Ok(val) => match val.to_lowercase().trim() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            other => anyhow::bail!("Invalid boolean value for {key}: '{other}'"),
        },
        Err(_) => Ok(default),
    }
}
