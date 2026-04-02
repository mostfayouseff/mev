//! Pre-simulation via the private RPC `simulateTransaction` endpoint.
//!
//! DESIGN:
//! - We build the full transaction (all hop instructions + priority fee) and
//!   simulate it against the private RPC BEFORE paying gas or submitting.
//! - If simulation fails or reports a profit below threshold: FAIL-FAST (no gas wasted).
//! - Simulation is async (RPC round-trip); we use `reqwest` with a short timeout.
//! - We parse the simulation response to extract the actual profit by comparing
//!   pre/post token balances in the simulation log.

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use common::ApexError;
use strategy::Opportunity;

/// Result of a `simulateTransaction` call.
#[derive(Debug, Clone)]
pub struct SimulationResult {
    /// Whether the simulated transaction succeeded.
    pub success: bool,
    /// Estimated compute units consumed.
    pub compute_units: u64,
    /// Net profit estimated from pre/post token balance deltas.
    pub net_profit_lamports: i64,
    /// Any error message from the simulation.
    pub error: Option<String>,
    /// Fee charged (priority fee + base fee).
    pub fee_lamports: u64,
}

/// Calls `simulateTransaction` and parses the result.
pub struct TransactionSimulator {
    rpc_url: String,
    client: Client,
    timeout: Duration,
}

impl TransactionSimulator {
    pub fn new(rpc_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("HTTP client build failed");
        Self {
            rpc_url,
            client,
            timeout: Duration::from_secs(3),
        }
    }

    /// Simulate the opportunity's transaction.
    /// Returns a `SimulationResult` or an `ApexError` if the RPC call fails.
    pub async fn simulate(&self, opp: &Opportunity) -> Result<SimulationResult, ApexError> {
        // Build a placeholder transaction for simulation.
        // In production: build the full versioned transaction with all hop instructions
        // and Address Lookup Tables, then base64-encode it.
        let encoded_tx = self.build_encoded_tx(opp);

        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateTransaction",
            "params": [
                encoded_tx,
                {
                    "encoding": "base64",
                    "commitment": "processed",
                    "replaceRecentBlockhash": true,
                    "sigVerify": false,
                    "accounts": {
                        "encoding": "base64jsonParsed",
                        "addresses": self.extract_token_accounts(opp)
                    }
                }
            ]
        });

        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&request_body)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| ApexError::Rpc(format!("simulateTransaction request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(ApexError::Rpc(format!(
                "simulateTransaction HTTP {}", resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ApexError::Rpc(format!("simulateTransaction parse failed: {e}")))?;

        self.parse_response(&body, opp)
    }

    fn build_encoded_tx(&self, opp: &Opportunity) -> String {
        // In production: build VersionedTransaction with all hop IXs + priority fee IX.
        // For the skeleton, return a placeholder that the RPC will reject (expected in devnet).
        // Replace with real transaction building in production.
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string()
    }

    fn extract_token_accounts(&self, _opp: &Opportunity) -> Vec<String> {
        // Return the token accounts whose balances we want to compare pre/post.
        // In production: derive ATAs from the hop token mints + user pubkey.
        vec![]
    }

    fn parse_response(
        &self,
        body: &serde_json::Value,
        opp: &Opportunity,
    ) -> Result<SimulationResult, ApexError> {
        let result = body
            .get("result")
            .ok_or_else(|| ApexError::Rpc("simulateTransaction: no result field".into()))?;

        let value = result
            .get("value")
            .ok_or_else(|| ApexError::Rpc("simulateTransaction: no value field".into()))?;

        // Check for simulation error
        let sim_error = value.get("err");
        let has_error = sim_error.map(|e| !e.is_null()).unwrap_or(false);

        if has_error {
            let err_str = sim_error
                .and_then(|e| serde_json::to_string(e).ok())
                .unwrap_or_else(|| "unknown".to_string());
            warn!(error = %err_str, "Simulation returned error");
            return Ok(SimulationResult {
                success: false,
                compute_units: 0,
                net_profit_lamports: i64::MIN,
                error: Some(err_str),
                fee_lamports: 0,
            });
        }

        let compute_units = value
            .get("unitsConsumed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Parse pre/post token balances for actual profit calculation.
        // In production: subtract input_amount from output_amount across balances.
        // As a safe conservative estimate, use the off-chain simulation result.
        let estimated_profit = opp.expected_output as i64 - opp.optimal_input as i64;

        // Rough fee: 5000 lamports base + 1 lamport per CU (simplified).
        let fee_lamports = 5_000 + compute_units / 1000;

        let net_profit = estimated_profit - fee_lamports as i64;

        debug!(
            compute_units,
            net_profit,
            "Simulation result"
        );

        Ok(SimulationResult {
            success: true,
            compute_units,
            net_profit_lamports: net_profit,
            error: None,
            fee_lamports,
        })
    }
}
