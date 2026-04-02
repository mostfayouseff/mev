//! Transaction submitter — coordinates execution: build → [Jito bundle] → submit → confirm.
//!
//! This is the final stage. Every trade that reaches here has been:
//! 1. Scored (core engine)
//! 2. Hook-filtered (strategy)
//! 3. Pre-simulated (safety)
//! 4. Profit-verified (safety)
//! 5. Rate-limited (execution)

use std::sync::Arc;
use std::time::Instant;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use tracing::{error, info, warn};

use common::ApexError;
use config::SharedConfig;
use jito_integration::{
    bundle::JitoClient,
    tip::{build_tip_instruction, TipStrategy},
    JITO_BLOCK_ENGINE_DEVNET, JITO_BLOCK_ENGINE_URL,
};
use metrics::metrics;
use safety::{circuit_breaker::CircuitBreaker, ValidatedOpportunity};

use crate::{builder::TransactionBuilder, rate_limiter::RateLimiter, ExecutionOutcome};

pub struct Submitter {
    config: SharedConfig,
    builder: Arc<TransactionBuilder>,
    rpc: Arc<RpcClient>,
    jito: Option<JitoClient>,
    rate_limiter: RateLimiter,
    circuit_breaker: Arc<CircuitBreaker>,
    tip_strategy: parking_lot::Mutex<TipStrategy>,
}

impl Submitter {
    pub fn new(
        config: SharedConfig,
        builder: Arc<TransactionBuilder>,
        circuit_breaker: Arc<CircuitBreaker>,
    ) -> Self {
        let cfg = config.load();
        let rpc = Arc::new(RpcClient::new_with_commitment(
            cfg.rpc_url.clone(),
            CommitmentConfig::confirmed(),
        ));

        let jito = if cfg.use_jito {
            let endpoint = if cfg.rpc_url.contains("devnet") {
                JITO_BLOCK_ENGINE_DEVNET
            } else {
                JITO_BLOCK_ENGINE_URL
            };
            Some(JitoClient::new(endpoint))
        } else {
            None
        };

        let rate_limiter = RateLimiter::new(cfg.max_trades_per_minute);
        let tip_strategy = TipStrategy::new(cfg.jito_tip_lamports, cfg.jito_tip_lamports * 10);

        Self {
            config,
            builder,
            rpc,
            jito,
            rate_limiter,
            circuit_breaker,
            tip_strategy: parking_lot::Mutex::new(tip_strategy),
        }
    }

    /// Execute a validated opportunity end-to-end.
    pub async fn execute(
        &self,
        opp: ValidatedOpportunity,
    ) -> Result<ExecutionOutcome, ApexError> {
        let cfg = self.config.load();
        let start = Instant::now();

        // ── Rate limit ────────────────────────────────────────────────────
        if !self.rate_limiter.allow() {
            return Err(ApexError::Execution("Rate limit exceeded".into()));
        }

        // ── Dry run gate ──────────────────────────────────────────────────
        if cfg.dry_run {
            info!(
                profit = opp.confirmed_profit_lamports,
                input = opp.opportunity.optimal_input,
                hops = opp.opportunity.path.path.len(),
                "[DRY RUN] Would execute opportunity"
            );
            metrics().trades_submitted.inc();
            return Ok(ExecutionOutcome {
                success: true,
                signature: Some("DRY_RUN".to_string()),
                profit_lamports: opp.confirmed_profit_lamports,
                latency_ms: start.elapsed().as_millis() as u64,
                error: None,
            });
        }

        // ── Gate: execute_trades must be true ────────────────────────────
        if !cfg.execute_trades {
            return Err(ApexError::Execution(
                "execute_trades=false; set to true to enable live trading".into(),
            ));
        }

        // ── Get recent blockhash ──────────────────────────────────────────
        let blockhash = self
            .rpc
            .get_latest_blockhash()
            .await
            .map_err(|e| ApexError::Rpc(format!("get_latest_blockhash: {e}")))?;

        // ── Build transaction ─────────────────────────────────────────────
        let tip_ix = if cfg.use_jito {
            let profit = opp.confirmed_profit_lamports;
            let mut ts = self.tip_strategy.lock();
            let tip = ts.calculate_tip(profit);
            let tip_account_idx = ts.next_tip_account_idx();
            Some(build_tip_instruction(&self.builder.pubkey(), tip, tip_account_idx))
        } else {
            None
        };

        let extra_ixs: Vec<_> = tip_ix.into_iter().collect();
        let tx = self.builder.build(&opp, blockhash, extra_ixs)?;

        metrics().trades_submitted.inc();

        // ── Submit ────────────────────────────────────────────────────────
        let outcome = if cfg.use_jito {
            self.submit_via_jito(tx, &opp).await?
        } else {
            self.submit_direct(tx, &opp).await?
        };

        // ── Post-execution ─────────────────────────────────────────────────
        if outcome.success {
            self.circuit_breaker.record_success();
            metrics().trades_confirmed.inc();
            metrics()
                .profit_lamports_total
                .inc_by(opp.confirmed_profit_lamports.max(0) as u64);
            info!(
                sig = outcome.signature.as_deref().unwrap_or("unknown"),
                profit = opp.confirmed_profit_lamports,
                latency_ms = outcome.latency_ms,
                "Trade confirmed"
            );
        } else {
            self.circuit_breaker.record_loss();
            metrics().trades_failed.inc();
            warn!(
                error = ?outcome.error,
                "Trade failed"
            );
        }

        metrics()
            .trade_latency_ms
            .with_label_values(&[if outcome.success { "success" } else { "failure" }])
            .observe(outcome.latency_ms as f64);

        Ok(ExecutionOutcome {
            latency_ms: start.elapsed().as_millis() as u64,
            ..outcome
        })
    }

    async fn submit_via_jito(
        &self,
        tx: solana_sdk::transaction::VersionedTransaction,
        opp: &ValidatedOpportunity,
    ) -> Result<ExecutionOutcome, ApexError> {
        let jito = self.jito.as_ref().expect("Jito client not initialised");
        let bundle_result = jito.submit_bundle(&[tx]).await?;

        if !bundle_result.submitted {
            return Ok(ExecutionOutcome {
                success: false,
                signature: None,
                profit_lamports: 0,
                latency_ms: 0,
                error: bundle_result.error,
            });
        }

        let landed = jito
            .wait_for_bundle(&bundle_result.bundle_id, std::time::Duration::from_secs(30))
            .await
            .unwrap_or(false);

        self.tip_strategy.lock().record_result(landed);

        Ok(ExecutionOutcome {
            success: landed,
            signature: Some(bundle_result.bundle_id),
            profit_lamports: if landed { opp.confirmed_profit_lamports } else { 0 },
            latency_ms: 0,
            error: if !landed { Some("Bundle did not land".into()) } else { None },
        })
    }

    async fn submit_direct(
        &self,
        tx: solana_sdk::transaction::VersionedTransaction,
        opp: &ValidatedOpportunity,
    ) -> Result<ExecutionOutcome, ApexError> {
        let sig = self
            .rpc
            .send_and_confirm_transaction(&tx)
            .await
            .map_err(|e| ApexError::Rpc(format!("send_and_confirm_transaction: {e}")))?;

        Ok(ExecutionOutcome {
            success: true,
            signature: Some(sig.to_string()),
            profit_lamports: opp.confirmed_profit_lamports,
            latency_ms: 0,
            error: None,
        })
    }
}
