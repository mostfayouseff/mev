//! Strategy loop runner — the main coordination loop.
//!
//! Driven by `PoolUpdateEvent`s from the ingress layer.
//! On each event: update the registry, re-run path-finding, score, apply hooks,
//! and emit profitable opportunities into the safety layer.

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use tracing::{debug, info, warn};

use config::SharedConfig;
use apex_core::{
    optimizer::{InputOptimizer, OptimizerConfig},
    path_finder::PathFinder,
    registry::PoolRegistry,
    scorer::{PathScorer, ScorerConfig},
};
use apex_ingress::PoolUpdateEvent;
use metrics::metrics;

use crate::{hooks::StrategyHook, Opportunity};

pub struct StrategyRunner {
    config: SharedConfig,
    registry: Arc<PoolRegistry>,
    pool_rx: Receiver<PoolUpdateEvent>,
    opp_tx: Sender<Opportunity>,
    hooks: Vec<Box<dyn StrategyHook>>,
}

impl StrategyRunner {
    pub fn new(
        config: SharedConfig,
        registry: Arc<PoolRegistry>,
        pool_rx: Receiver<PoolUpdateEvent>,
        opp_tx: Sender<Opportunity>,
        hooks: Vec<Box<dyn StrategyHook>>,
    ) -> Self {
        Self {
            config,
            registry,
            pool_rx,
            opp_tx,
            hooks,
        }
    }

    /// Run the strategy loop until the channel is disconnected.
    pub async fn run(self) {
        info!("Strategy loop starting");
        let mut tick_count: u64 = 0;

        loop {
            let mut got_update = false;
            loop {
                match self.pool_rx.try_recv() {
                    Ok(event) => {
                        self.registry.upsert(event.pool);
                        got_update = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        info!("Pool update channel disconnected; strategy loop exiting");
                        return;
                    }
                }
            }

            if !got_update {
                tokio::time::sleep(Duration::from_micros(100)).await;
                continue;
            }

            tick_count += 1;
            metrics().strategy_ticks.inc();

            let cfg = self.config.load();
            let live_pools = self.registry.live_pools(0, cfg.max_pool_age_slots);

            if live_pools.is_empty() {
                continue;
            }

            // ── Path finding ───────────────────────────────────────────────
            let finder = PathFinder::new(cfg.max_hops, 100_000_000); // 0.1 SOL probe
            let paths = finder.find_paths(&live_pools);
            metrics().paths_evaluated.inc_by(paths.len() as u64);

            // ── Scoring ────────────────────────────────────────────────────
            let scorer = PathScorer::new(ScorerConfig {
                priority_fee_lamports: cfg.priority_fee_lamports,
                jito_tip_lamports: cfg.jito_tip_lamports,
                use_jito: cfg.use_jito,
                min_profit_lamports: cfg.min_profit_lamports,
                latency_penalty_per_hop: 500,
            });
            let scored = scorer.rank_paths(&paths);

            for sp in scored.iter().take(cfg.max_candidate_paths) {
                metrics().paths_profitable.inc();

                // ── Input optimisation ─────────────────────────────────────
                let optimizer = InputOptimizer::new(OptimizerConfig {
                    min_input: 1_000,
                    max_input: cfg.max_position_size,
                    iterations: 45,
                });
                let Some((opt_input, expected_out)) = optimizer.optimize(&sp.path) else {
                    continue;
                };

                let mut opp = Opportunity {
                    path: sp.clone(),
                    optimal_input: opt_input,
                    expected_output: expected_out,
                    created_at_ns: common::monotonic_ns(),
                };

                // ── Apply hooks ────────────────────────────────────────────
                let mut vetoed = false;
                for hook in &self.hooks {
                    match hook.on_opportunity(opp) {
                        Some(o) => opp = o,
                        None => {
                            debug!(hook = hook.name(), "Opportunity vetoed by hook");
                            vetoed = true;
                            break;
                        }
                    }
                }
                if vetoed {
                    continue;
                }

                metrics().opportunities_found.inc();

                if self.opp_tx.try_send(opp).is_err() {
                    warn!("Opportunity channel full; dropping");
                }
            }

            if tick_count % 100 == 0 {
                info!(
                    tick = tick_count,
                    pools = live_pools.len(),
                    "Strategy heartbeat"
                );
            }
        }
    }
}
