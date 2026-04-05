// =============================================================================
// SOLANA RPC CLIENT — Minimal & Reliable
// =============================================================================

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

const RPC_TIMEOUT_MS: u64 = 5000;
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_MS: u64 = 500;

/// Recent blockhash with validity info.
#[derive(Debug, Clone)]
pub struct RecentBlockhash {
    pub blockhash: String,
    pub last_valid_block_height: u64,
}

// ── JSON-RPC Types ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct RpcRequest<'a, T: Serialize> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: T,
}

#[derive(Deserialize, Debug)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize, Debug)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Deserialize, Debug)]
struct BlockhashResult {
    value: BlockhashValue,
}

#[derive(Deserialize, Debug)]
struct BlockhashValue {
    blockhash: String,
    #[serde(rename = "lastValidBlockHeight")]
    last_valid_block_height: u64,
}

#[derive(Deserialize, Debug)]
struct SlotResult(pub u64);

#[derive(Deserialize, Debug)]
struct BalanceResult {
    value: u64,
}

#[derive(Deserialize, Debug)]
struct SimulateResult {
    value: SimulateValue,
}

#[derive(Deserialize, Debug)]
struct SimulateValue {
    err: Option<serde_json::Value>,
    logs: Option<Vec<String>>,
}

// ── Client ───────────────────────────────────────────────────────────────────

pub struct SolanaRpcClient {
    client: Client,
    url: String,
}

impl SolanaRpcClient {
    pub fn new(rpc_url: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(RPC_TIMEOUT_MS))
            .build()
            .context("Failed to build RPC HTTP client")?;

        Ok(Self {
            client,
            url: rpc_url.to_string(),
        })
    }

    pub async fn get_slot(&self) -> Result<u64> {
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getSlot",
            params: serde_json::json!([{"commitment": "confirmed"}]),
        };

        let result: SlotResult = self.post(&request).await?;
        debug!(slot = result.0, "getSlot OK");
        Ok(result.0)
    }

    pub async fn get_latest_blockhash(&self) -> Result<RecentBlockhash> {
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "getLatestBlockhash",
            params: serde_json::json!([{"commitment": "confirmed"}]),
        };

        for attempt in 1..=MAX_RETRIES {
            match self.post::<BlockhashResult>(&request).await {
                Ok(result) => {
                    let bh = RecentBlockhash {
                        blockhash: result.value.blockhash,
                        last_valid_block_height: result.value.last_valid_block_height,
                    };
                    debug!(blockhash = %bh.blockhash, last_valid = bh.last_valid_block_height, "Latest blockhash fetched");
                    return Ok(bh);
                }
                Err(e) if attempt < MAX_RETRIES => {
                    warn!(attempt, error = %e, "getLatestBlockhash failed — retrying");
                    tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                }
                Err(e) => return Err(e),
            }
        }

        Err(anyhow::anyhow!("getLatestBlockhash failed after {} retries", MAX_RETRIES))
    }

    pub async fn get_balance(&self, pubkey_b58: &str) -> Result<u64> {
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 2,
            method: "getBalance",
            params: serde_json::json!([pubkey_b58, {"commitment": "confirmed"}]),
        };

        let result: BalanceResult = self.post(&request).await?;
        debug!(pubkey = %pubkey_b58, balance = result.value, "Balance fetched");
        Ok(result.value)
    }

    pub async fn simulate_transaction(&self, tx_b64: &str) -> Result<bool> {
        let request = RpcRequest {
            jsonrpc: "2.0",
            id: 3,
            method: "simulateTransaction",
            params: serde_json::json!([tx_b64, {"commitment": "confirmed", "encoding": "base64"}]),
        };

        let result: SimulateResult = self.post(&request).await?;
        let success = result.value.err.is_none();

        if let Some(logs) = result.value.logs {
            for log in logs.iter().take(5) {
                debug!(log = %log, "Simulation log");
            }
        }

        Ok(success)
    }

    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        request: &(impl Serialize + ?Sized),
    ) -> Result<T> {
        let resp = self.client
            .post(&self.url)
            .json(request)
            .send()
            .await
            .context("RPC HTTP request failed")?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("RPC HTTP error: {}", resp.status()));
        }

        let rpc_resp: RpcResponse<T> = resp.json().await
            .context("Failed to parse RPC response")?;

        if let Some(err) = rpc_resp.error {
            return Err(anyhow::anyhow!("RPC error {}: {}", err.code, err.message));
        }

        rpc_resp.result.ok_or_else(|| anyhow::anyhow!("RPC response missing 'result' field"))
    }
}
