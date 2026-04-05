// =============================================================================
// APEX-MEV Neural Core 3.0 — LIVE TRADING MODE
// =============================================================================

mod pnl;

use anyhow::{Context, Result};
use common::{ApexConfig, ApexMetrics};
use apex_core::MatrixBuilder;
use ingress::{
    build_ultra_client, get_best_route_and_transaction, AlchemyTransactionStream,
    HeliusTransactionStream, JupiterMonitor, MockShredStream, MockYellowstoneStream,
};
use jito_handler::{build_flash_loan_tx, ApexKeypair, JitoBundleHandler, SolanaRpcClient};
use pnl::{make_record, AtomicPnL, SessionStats};
use risk_oracle::{AnomalyDetector, CircuitBreaker, SelfOptimizer, TradingParams};
use safety::{AtomicRevertGuard, PreSimulator};
use solana_program_apex::instruction::dex_fee_bps;
use std::sync::Arc;
use strategy::{ArbitrageStrategy, SolendFlashLoan};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use ingress::ShredEvent;

// ─────────────────────────────────────────────────────────────────────────────
// IMPORTANT: Rustls 0.23+ CryptoProvider Fix
// This must be called VERY EARLY — before any TLS connection (Helius WS, Jupiter, etc.)
// ─────────────────────────────────────────────────────────────────────────────
fn install_rustls_crypto_provider() {
    // Option 1 (recommended): aws-lc-rs — modern, performant, default in recent rustls
    match rustls::crypto::aws_lc_rs::default_provider().install_default() {
        Ok(_) => info!("Rustls crypto provider installed: aws-lc-rs"),
        Err(_) => {
            // This can happen if it was already installed (safe to ignore)
            info!("Rustls crypto provider already installed (aws-lc-rs)");
        }
    }

    // Alternative (if you prefer ring or have build issues with aws-lc-rs):
    // rustls::crypto::ring::default_provider()
    //     .install_default()
    //     .expect("Failed to install ring crypto provider");
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install crypto provider BEFORE any logging or async runtime starts
    install_rustls_crypto_provider();

    // ── Logging ───────────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_env("RUST_LOG")
                .add_directive("apex_mev=info".parse().unwrap()),
        )
        .init();

    info!("╔══════════════════════════════════════════════════════════════╗");
    info!("║ APEX-MEV Neural Core 3.0 — LIVE MAINNET TRADING ║");
    info!("╚══════════════════════════════════════════════════════════════╝");

    // ── Configuration ─────────────────────────────────────────────────────────
    let config = ApexConfig::from_env()
        .context("Failed to load configuration — check env vars")?;

    info!(
        simulation_only = config.simulation_only,
        rpc_url = %config.rpc_url,
        http_rpc_url = %config.http_rpc_url,
        helius_active = config.helius_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false),
        alchemy_active = config.alchemy_api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false),
        jupiter_key = config.jupiter_api_key.is_some(),
        min_profit = config.min_profit_lamports,
        max_hops = config.max_hops,
        flash_loan = config.flash_loan_enabled,
        "Configuration loaded"
    );

    // ── HTTP RPC connectivity check ────────────────────────────────────────────
    info!(url = %config.http_rpc_url, "Solana RPC: verifying connectivity (getSlot)");
    match SolanaRpcClient::new(&config.http_rpc_url) {
        Ok(rpc_check) => match rpc_check.get_slot().await {
            Ok(slot) => {
                info!(slot, url = %config.http_rpc_url, "Solana RPC: CONNECTED — getSlot OK");
            }
            Err(e) => {
                warn!(
                    error = %e,
                    url = %config.http_rpc_url,
                    "Solana RPC: getSlot FAILED — will retry in background. Continuing."
                );
            }
        },
        Err(e) => {
            warn!(error = %e, "Solana RPC: client init failed — continuing anyway");
        }
    }

    // ── Background slot poller ─────────────────────────────────────────────────
    {
        let rpc_url = config.http_rpc_url.clone();
        tokio::spawn(async move {
            let rpc = match SolanaRpcClient::new(&rpc_url) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Slot poller: could not build RPC client");
                    return;
                }
            };
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                match rpc.get_slot().await {
                    Ok(slot) => info!(slot, "Solana RPC: slot poll OK"),
                    Err(e) => warn!(error = %e, "Solana RPC: slot poll failed — will retry"),
                }
            }
        });
    }

    // ── Metrics ────────────────────────────────────────────────────────────────
    let metrics = Arc::new(
        ApexMetrics::register().context("Failed to register Prometheus metrics")?,
    );
    info!("Prometheus metrics registered");

    // ── P&L tracking ──────────────────────────────────────────────────────────
    let pnl = AtomicPnL::new();
    let mut session_stats = SessionStats::new();
    info!("P&L tracker initialised (session start: {})", session_stats.start_time);

    // ── Self-Optimizer ─────────────────────────────────────────────────────────
    let mut self_optimizer = SelfOptimizer::new(
        TradingParams::new(
            config.slippage_bps,
            config.min_profit_lamports,
            40,
        ),
        config.auto_optimize,
    );
    info!(
        auto_optimize = config.auto_optimize,
        initial_slippage = config.slippage_bps,
        initial_min_profit = config.min_profit_lamports,
        "Self-optimizer initialised"
    );

    // ── Flash loan builder ─────────────────────────────────────────────────────
    let flash_loan = SolendFlashLoan::new_sol();
    if config.flash_loan_enabled {
        info!(
            "Flash loans: ENABLED (Solend, 9bps fee) — atomic borrow+swap+repay per trade"
        );
    }

    // ── Risk subsystem ─────────────────────────────────────────────────────────
    let circuit_breaker = CircuitBreaker::new(
        config.circuit_breaker_consecutive_losses,
        config.circuit_breaker_threshold_lamports,
    );
    metrics.circuit_breaker_state.set(1.0);
    let anomaly_detector = Arc::new(Mutex::new(AnomalyDetector::new(200)));

    // ── Core engine ────────────────────────────────────────────────────────────
    let mut matrix_builder = MatrixBuilder::new();
    let strategy = Arc::new(
        ArbitrageStrategy::new(
            config.max_hops,
            config.min_profit_lamports,
            0.30,
            config.max_position_lamports,
        )
        .context("Failed to initialise arbitrage strategy")?,
    );

    // ── Safety layer ───────────────────────────────────────────────────────────
    let pre_sim = PreSimulator::new(config.min_profit_lamports);

    // ── Jito handler ───────────────────────────────────────────────────────────
    let flash_keypair: Option<Arc<ApexKeypair>>;
    let jito = if config.simulation_only {
        info!("Jito: SIMULATION mode — bundles logged but NOT submitted");
        flash_keypair = None;
        JitoBundleHandler::new(config.jito_url.clone())
    } else {
        info!("Jito: LIVE mode — loading keypair from {}", config.keypair_path);
        let keypair = ApexKeypair::load(&config.keypair_path).with_context(|| {
            format!("Failed to load keypair from {} — ensure the file exists", config.keypair_path)
        })?;

        // Log wallet balance
        if let Ok(rpc) = jito_handler::SolanaRpcClient::new(&config.http_rpc_url) {
            match rpc.get_balance(&keypair.pubkey_b58).await {
                Ok(bal) => {
                    info!(
                        pubkey = %keypair.pubkey_b58,
                        balance = format!("{:.9} SOL ({} lamports)", bal as f64 / 1e9, bal),
                        "Operator wallet: balance logged — flash loans do NOT require pre-funded balance"
                    );
                    if bal < 5_000_000 {
                        warn!(
                            balance_lamports = bal,
                            "Operator wallet balance is low (< 0.005 SOL) — ensure enough for transaction fees."
                        );
                    }
                }
                Err(e) => warn!("Could not fetch operator balance: {e} — continuing"),
            }
        }

        let fkp = if config.flash_loan_enabled {
            match ApexKeypair::load(&config.keypair_path) {
                Ok(kp) => {
                    info!(pubkey = %kp.pubkey_b58, "Flash loan signing keypair loaded");
                    Some(Arc::new(kp))
                }
                Err(e) => {
                    warn!(error = %e, "Flash loan keypair load failed — flash loans disabled for this session");
                    None
                }
            }
        } else {
            None
        };
        flash_keypair = fkp;

        JitoBundleHandler::new_live(
            config.jito_url.clone(),
            &config.http_rpc_url,
            keypair,
        )
        .context("Failed to initialise live Jito handler")?
    };

    // ── Jupiter Ultra API client ───────────────────────────────────────────────
    let ultra_client = build_ultra_client()
        .context("Failed to build Jupiter Ultra HTTP client")?;
    info!(
        endpoint = "https://api.jup.ag/ultra/v1/order",
        has_key = config.jupiter_api_key.is_some(),
        "Jupiter Ultra API client ready"
    );

    // ── Ingress streams ────────────────────────────────────────────────────────
    let ingress_source: &str;
    let mut shred_rx: Receiver<ShredEvent> =
        if let Some(ref helius_key) = config.helius_api_key {
            if !helius_key.is_empty() {
                ingress_source = "HELIUS (PRIMARY)";
                info!(
                    endpoint = "wss://mainnet.helius-rpc.com",
                    dex_programs = ingress::DEX_PROGRAMS.len(),
                    "LIVE DATA SOURCE: HELIUS — connecting to primary WebSocket stream"
                );
                HeliusTransactionStream::spawn(helius_key.clone())
            } else {
                ingress_source = "MOCK (no Helius key)";
                warn!("Helius API key is empty — using MockShredStream.");
                MockShredStream::spawn(400)
            }
        } else if let Some(ref alchemy_key) = config.alchemy_api_key {
            if !alchemy_key.is_empty() {
                ingress_source = "ALCHEMY (FALLBACK)";
                info!(
                    endpoint = "wss://solana-mainnet.g.alchemy.com",
                    "LIVE DATA SOURCE: ALCHEMY (FALLBACK)"
                );
                AlchemyTransactionStream::spawn(alchemy_key.clone())
            } else {
                ingress_source = "MOCK (no WS keys)";
                warn!("No WS keys set — using MockShredStream.");
                MockShredStream::spawn(400)
            }
        } else {
            ingress_source = "MOCK (no WS keys configured)";
            warn!("No Helius or Alchemy keys — using MockShredStream.");
            MockShredStream::spawn(400)
        };

    info!(ingress = ingress_source, "Ingress stream configured");
    let mut slot_rx = MockYellowstoneStream::spawn(2);

    // ── Self-healing Jupiter Price Monitor ────────────────────────────────────
    info!(
        tokens = ingress::TOKENS.len(),
        poll_ms = 1500,
        "Starting self-healing Jupiter price monitor"
    );
    let mut jupiter_rx = JupiterMonitor::spawn_with_key(config.jupiter_api_key.clone());

    info!("All subsystems initialised — entering LIVE hot loop");
    info!(
        mode = if config.simulation_only { "SIMULATION" } else { "LIVE TRADING" },
        ingress = ingress_source,
        flash_loans = config.flash_loan_enabled,
        min_profit = config.min_profit_lamports,
        "System ready"
    );

    // ── Hot loop ───────────────────────────────────────────────────────────────
    let mut live_edges: Option<Vec<common::types::MarketEdge>> = None;
    let mut iteration: u64 = 0;
    let mut last_stats_report = std::time::Instant::now();
    const STATS_REPORT_INTERVAL_SECS: u64 = 60;

    loop {
        if let Ok(slot_update) = slot_rx.try_recv() {
            matrix_builder.set_slot(slot_update.slot);
        }

        while let Ok(edges) = jupiter_rx.try_recv() {
            info!(
                edges = edges.len(),
                source = "JUPITER/LIVE",
                "LIVE DATA SOURCE: JUPITER — price matrix updated"
            );
            live_edges = Some(edges);
        }

        let t_hot_start = std::time::Instant::now();
        if let Ok(shred) = shred_rx.try_recv() {
            if !filter_accepts(&shred) {
                tokio::task::yield_now().await;
                continue;
            }

            iteration += 1;
            metrics.paths_evaluated.inc();

            if circuit_breaker.check_allow_trade().is_err() {
                warn!("Circuit breaker OPEN — halting trades temporarily");
                metrics.circuit_breaker_state.set(0.0);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }

            let (edges_ref, edge_source): (&[common::types::MarketEdge], &str) =
                match live_edges.as_deref() {
                    Some(live) if !live.is_empty() => (live, "JUPITER/LIVE"),
                    _ => {
                        if iteration % 500 == 0 {
                            warn!("No live price data yet — waiting for Jupiter price monitor.");
                        }
                        tokio::task::yield_now().await;
                        continue;
                    }
                };

            let t_matrix = std::time::Instant::now();
            let matrix = matrix_builder.build(edges_ref);
            let matrix_us = t_matrix.elapsed().as_micros();

            let t_rich = std::time::Instant::now();
            let matrix_clone = matrix.clone();
            let strategy_arc = strategy.clone();
            let approved_trades = tokio::task::spawn_blocking(move || {
                strategy_arc.evaluate(&matrix_clone)
            })
            .await
            .unwrap_or_default();

            let rich_us = t_rich.elapsed().as_micros();
            let n_approved = approved_trades.len();
            metrics.paths_profitable.inc_by(n_approved as f64);

            for trade in approved_trades {
                // ... (your entire trade processing logic remains unchanged)
                // I kept it exactly as you had it for brevity in this response.
                // Paste your original trade loop code here (from "let dex_path..." to the end of the for loop).
                // No changes were needed inside the hot loop.
            }

            let _updated_params = self_optimizer.maybe_optimize();
            metrics.observe_hot_path(t_hot_start);
        }

        if last_stats_report.elapsed().as_secs() >= STATS_REPORT_INTERVAL_SECS {
            session_stats.log_summary();
            let opt_params = self_optimizer.params();
            info!(
                iterations = iteration,
                live_price_active = live_edges.is_some(),
                pnl_lamports = pnl.total_lamports(),
                pnl_sol = format!("{:+.9}", pnl.total_sol()),
                current_slippage = opt_params.slippage_bps,
                current_min_profit = opt_params.min_profit_lamports,
                current_tip_pct = opt_params.tip_fraction_pct,
                ingress_source = ingress_source,
                "Periodic status — LIVE TRADING ENGINE"
            );
            last_stats_report = std::time::Instant::now();
        }

        tokio::task::yield_now().await;
    }
}

#[inline(always)]
fn filter_accepts(shred: &ShredEvent) -> bool {
    !shred.data.is_empty() && shred.data.len() <= 65536
}
