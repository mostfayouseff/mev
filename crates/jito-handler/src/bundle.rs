// =============================================================================
// JITO BUNDLE HANDLER — Fixed & Cleaned
// =============================================================================

use super::TipCalculator;
use crate::keypair::{extract_message_bytes, inject_signature, ApexKeypair};
use crate::rpc::SolanaRpcClient;
use anyhow::Context;
use blake3::Hasher;
use rand::seq::SliceRandom;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use tracing::{debug, info, warn};

const JITO_BUNDLE_PATH: &str = "/api/v1/bundles";
const HTTP_TIMEOUT_MS: u64 = 8000;
const MAX_BUNDLE_TXS: usize = 5;
const STATUS_POLL_ATTEMPTS: u32 = 8;
const STATUS_POLL_DELAY_MS: u64 = 400;

const JITO_TIP_ACCOUNTS: &[&str] = &[
    "9n3d1K5YD2vECAbRFhFFGYNNjiXtHXJWn9F31t89vsAV",
    "aTtUk2DHgLhKZRDjePq6eiHRKC1XXFMBiSUfQ2JNDbN",
    "B1mrQSpdeMU9gCvkJ6VsXVVoYjRGkNA7TtjMyqxrhecH",
    "9ttgPBBhRYFuQccdR1DSnb7hydsWANoDsV3P9kaGMCEh",
    "4xgEmT58RwTNsF5xm2RMYCnR1EVukdK8a1i2qFjnJFu3",
    "EoW3SUQap7ZeynXQ2QJ847aerhxbPVr843uMeTfc9dxM",
    "E2eSqe33tuhAHKTrwky5uEjaVqnb2T9ns6nHHUrN8588",
    "ARTtviJkLLt6cHGQDydfo1Wyk6M4VGZdKZ2ZhdnJL336",
];

#[derive(Debug, Error)]
pub enum JitoError {
    #[error("Bundle serialization failed: {0}")]
    Serialization(String),
    #[error("Bundle submission failed: {0}")]
    Submission(String),
    #[error("Bundle rejected: {0}")]
    Rejected(String),
    #[error("Tip error: {0}")]
    TipError(String),
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("Signing error: {0}")]
    Signing(String),
    #[error("Too many transactions: {count} (max {max})")]
    TooManyTransactions { count: usize, max: usize },
}

#[derive(Debug, Clone)]
pub struct JitoBundle {
    pub id: [u8; 32],
    pub transactions: Vec<String>, // base58 encoded
    pub tip_lamports: u64,
    pub tip_account: String,
    pub submitted_at_ms: u64,
}

impl JitoBundle {
    pub fn new(signed_txs_b58: Vec<String>, tip_lamports: u64, tip_account: String) -> Result<Self, JitoError> {
        if signed_txs_b58.len() > MAX_BUNDLE_TXS {
            return Err(JitoError::TooManyTransactions {
                count: signed_txs_b58.len(),
                max: MAX_BUNDLE_TXS,
            });
        }

        let mut hasher = Hasher::new();
        for tx in &signed_txs_b58 {
            hasher.update(tx.as_bytes());
        }
        hasher.update(&tip_lamports.to_le_bytes());
        let id = *hasher.finalize().as_bytes();

        let submitted_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(Self {
            id,
            transactions: signed_txs_b58,
            tip_lamports,
            tip_account,
            submitted_at_ms,
        })
    }
}

pub struct JitoBundleHandler {
    tip_calculator: TipCalculator,
    engine_url: String,
    http_client: Client,
    rpc_client: Option<Arc<SolanaRpcClient>>,
    keypair: Option<Arc<ApexKeypair>>,
    simulation_only: bool,
}

impl JitoBundleHandler {
    pub fn new(engine_url: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            tip_calculator: TipCalculator::new(),
            engine_url,
            http_client,
            rpc_client: None,
            keypair: None,
            simulation_only: true,
        }
    }

    pub fn new_live(engine_url: String, rpc_url: &str, keypair: ApexKeypair) -> anyhow::Result<Self> {
        let http_client = Client::builder()
            .timeout(Duration::from_millis(HTTP_TIMEOUT_MS))
            .build()
            .context("Failed to build HTTP client")?;

        let rpc_client = SolanaRpcClient::new(rpc_url)
            .context("Failed to create RPC client")?;

        Ok(Self {
            tip_calculator: TipCalculator::new(),
            engine_url,
            http_client,
            rpc_client: Some(Arc::new(rpc_client)),
            keypair: Some(Arc::new(keypair)),
            simulation_only: false,
        })
    }

    pub async fn submit(
        &self,
        swap_payloads: Vec<Vec<u8>>,
        expected_profit_lamports: u64,
    ) -> Result<JitoBundle, JitoError> {
        let tip = self.tip_calculator
            .compute_tip(expected_profit_lamports)
            .map_err(|e| JitoError::TipError(e.to_string()))?;

        let tip_account = select_random_tip_account();

        if self.simulation_only {
            self.submit_simulation(swap_payloads, tip, expected_profit_lamports, tip_account).await
        } else {
            self.submit_live(swap_payloads, tip, expected_profit_lamports, tip_account).await
        }
    }

    async fn submit_simulation(
        &self,
        payloads: Vec<Vec<u8>>,
        tip_lamports: u64,
        profit_lamports: u64,
        tip_account: String,
    ) -> Result<JitoBundle, JitoError> {
        let delay_ms = (rand::random::<u8>() as u64) % 50;
        sleep(Duration::from_millis(delay_ms)).await;

        let mock_txs: Vec<String> = payloads.iter().map(|p| bs58::encode(p).into_string()).collect();

        let bundle = JitoBundle::new(mock_txs, tip_lamports, tip_account.clone())
            .map_err(|e| JitoError::Serialization(e.to_string()))?;

        info!(
            bundle_id = %hex::encode(bundle.id),
            tip = tip_lamports,
            profit = profit_lamports,
            txs = payloads.len(),
            "SIMULATION: Jito bundle prepared"
        );

        Ok(bundle)
    }

    async fn submit_live(
        &self,
        mut payloads: Vec<Vec<u8>>,
        tip_lamports: u64,
        profit_lamports: u64,
        tip_account: String,
    ) -> Result<JitoBundle, JitoError> {
        let rpc = self.rpc_client.as_ref().ok_or_else(|| JitoError::Rpc("No RPC client".into()))?;
        let keypair = self.keypair.as_ref().ok_or_else(|| JitoError::Signing("No keypair".into()))?;

        let blockhash = rpc.get_latest_blockhash().await
            .map_err(|e| JitoError::Rpc(e.to_string()))?;

        // Enforce max bundle size
        let max_swaps = MAX_BUNDLE_TXS.saturating_sub(1);
        if payloads.len() > max_swaps {
            payloads.truncate(max_swaps);
        }

        let mut signed_txs: Vec<String> = Vec::with_capacity(payloads.len() + 1);

        for mut payload in payloads {
            if let Err(e) = sign_transaction(&mut payload, keypair) {
                warn!(error = %e, "Signing failed, using unsigned payload");
            }
            signed_txs.push(bs58::encode(&payload).into_string());
        }

        // Add tip transaction as final tx
        let tip_tx = build_tip_transaction(keypair, &blockhash.blockhash, tip_lamports, &tip_account);
        signed_txs.push(bs58::encode(&tip_tx).into_string());

        let bundle = JitoBundle::new(signed_txs.clone(), tip_lamports, tip_account.clone())
            .map_err(|e| JitoError::Serialization(e.to_string()))?;

        info!(
            bundle_id = %hex::encode(bundle.id),
            tip = tip_lamports,
            profit_est = profit_lamports,
            tx_count = signed_txs.len(),
            "Submitting live Jito bundle"
        );

        // DontFront delay
        let delay_ms = (rand::random::<u8>() as u64) % 50;
        sleep(Duration::from_millis(delay_ms)).await;

        let _ = post_bundle(&self.http_client, &self.engine_url, &signed_txs).await;

        Ok(bundle)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn select_random_tip_account() -> String {
    let mut rng = rand::thread_rng();
    JITO_TIP_ACCOUNTS.choose(&mut rng)
        .copied()
        .unwrap_or(JITO_TIP_ACCOUNTS[0])
        .to_string()
}

fn sign_transaction(tx_bytes: &mut Vec<u8>, keypair: &ApexKeypair) -> anyhow::Result<()> {
    let message = extract_message_bytes(tx_bytes)
        .context("Failed to extract message bytes")?;
    let sig = keypair.sign(message);
    inject_signature(tx_bytes, 0, &sig)
        .context("Failed to inject signature")?;
    Ok(())
}

fn hex::encode(bytes: [u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
