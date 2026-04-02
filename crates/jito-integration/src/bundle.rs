//! Jito bundle client — submits transaction bundles to the Jito block engine.
//!
//! BUNDLE FORMAT (Jito v1):
//! - Up to 5 transactions per bundle.
//! - First transaction is the tip transaction (transfers SOL to Jito tip account).
//! - Remaining transactions are the arbitrage hops (or a single versioned tx).
//! - All transactions share the same recent blockhash.
//!
//! ATOMIC GUARANTEE: Jito processes the bundle atomically — if any tx fails,
//! the entire bundle reverts. This is our "atomic revert" safety property.

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    signature::Signature,
    transaction::VersionedTransaction,
};
use tracing::{debug, error, info, warn};

use common::ApexError;
use config::BotConfig;

/// Result of a bundle submission.
#[derive(Debug, Clone)]
pub struct BundleResult {
    pub bundle_id: String,
    pub submitted: bool,
    pub error: Option<String>,
}

pub struct JitoClient {
    endpoint: String,
    client: Client,
}

impl JitoClient {
    pub fn new(endpoint: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(2)
            .build()
            .expect("Jito HTTP client build failed");
        Self {
            endpoint: endpoint.to_string(),
            client,
        }
    }

    /// Submit a bundle of transactions to Jito.
    /// `transactions` is ordered: [tip_tx, arb_tx_1, ..., arb_tx_n].
    pub async fn submit_bundle(
        &self,
        transactions: &[VersionedTransaction],
    ) -> Result<BundleResult, ApexError> {
        if transactions.is_empty() || transactions.len() > 5 {
            return Err(ApexError::Jito(format!(
                "Bundle must have 1-5 transactions, got {}",
                transactions.len()
            )));
        }

        // Serialize each transaction to base58.
        let encoded_txs: Result<Vec<String>, _> = transactions
            .iter()
            .map(|tx| {
                bincode::encode_to_vec(tx, bincode::config::standard())
                    .map(|bytes| bs58::encode(bytes).into_string())
            })
            .collect();

        let encoded_txs = encoded_txs
            .map_err(|e| ApexError::Serialization(format!("Bundle serialization: {e}")))?;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [encoded_txs]
        });

        debug!(tx_count = transactions.len(), "Submitting Jito bundle");

        let resp = self
            .client
            .post(&self.endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| ApexError::Jito(format!("Bundle submission HTTP error: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ApexError::Jito(format!("Bundle response parse error: {e}")))?;

        if !status.is_success() {
            let err = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown Jito error");
            error!(http_status = %status, error = %err, "Jito bundle rejected");
            return Ok(BundleResult {
                bundle_id: String::new(),
                submitted: false,
                error: Some(err.to_string()),
            });
        }

        let bundle_id = body
            .get("result")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();

        info!(bundle_id = %bundle_id, "Jito bundle submitted");
        metrics::metrics().jito_bundles_sent.inc();

        Ok(BundleResult {
            bundle_id,
            submitted: true,
            error: None,
        })
    }

    /// Poll bundle status until landed or timeout.
    pub async fn wait_for_bundle(
        &self,
        bundle_id: &str,
        timeout: Duration,
    ) -> Result<bool, ApexError> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(500);

        loop {
            if start.elapsed() > timeout {
                warn!(bundle_id, "Bundle landing timeout");
                return Ok(false);
            }

            let status = self.get_bundle_status(bundle_id).await?;
            match status.as_str() {
                "Landed" => {
                    info!(bundle_id, "Bundle landed");
                    metrics::metrics().jito_bundles_landed.inc();
                    return Ok(true);
                }
                "Failed" | "Invalid" | "Dropped" => {
                    warn!(bundle_id, status = %status, "Bundle did not land");
                    return Ok(false);
                }
                _ => {
                    // Pending or unknown — keep polling
                    tokio::time::sleep(poll_interval).await;
                }
            }
        }
    }

    async fn get_bundle_status(&self, bundle_id: &str) -> Result<String, ApexError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBundleStatuses",
            "params": [[bundle_id]]
        });

        let resp = self
            .client
            .post(&self.endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| ApexError::Jito(format!("getBundleStatuses error: {e}")))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ApexError::Jito(format!("getBundleStatuses parse: {e}")))?;

        let status = body
            .pointer("/result/value/0/confirmation_status")
            .and_then(|s| s.as_str())
            .unwrap_or("Unknown")
            .to_string();

        Ok(status)
    }
}
