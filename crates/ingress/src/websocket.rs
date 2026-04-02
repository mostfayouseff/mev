//! WebSocket subscription to the private RPC for account-change notifications.
//!
//! DESIGN:
//! - Uses `tokio-tungstenite` for async WebSocket.
//! - Subscribes to `accountSubscribe` for all tracked pool addresses.
//! - On reconnect, re-subscribes automatically (exponential backoff).
//! - The receive loop runs entirely in one Tokio task — no locks needed.
//! - Raw bytes are dispatched to `AccountParser` before being pushed into
//!   the bounded channel.

use std::time::Duration;

use crossbeam_channel::Sender;
use serde_json::Value;
use solana_sdk::pubkey::Pubkey;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use tracing::{error, info, warn};

use crate::parser::AccountParser;
use crate::PoolUpdateEvent;
use common::monotonic_ns;

/// Handle for the WebSocket ingress pipeline.
pub struct IngressWs {
    ws_url: String,
    tx: Sender<PoolUpdateEvent>,
}

impl IngressWs {
    pub fn new(ws_url: String, tx: Sender<PoolUpdateEvent>) -> Self {
        Self { ws_url, tx }
    }

    /// Start the ingress loop in a background Tokio task.
    /// Reconnects on error with exponential backoff (max 30s).
    pub fn start(self, pool_addresses: Vec<Pubkey>) {
        tokio::spawn(async move {
            let mut backoff_ms = 500u64;
            loop {
                info!(url = %self.ws_url, "Connecting to RPC WebSocket");
                match self.run_once(&pool_addresses).await {
                    Ok(()) => {
                        info!("WebSocket connection closed gracefully");
                    }
                    Err(e) => {
                        error!(?e, "WebSocket error; reconnecting in {backoff_ms}ms");
                    }
                }
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(30_000);
            }
        });
    }

    async fn run_once(
        &self,
        pool_addresses: &[Pubkey],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = url::Url::parse(&self.ws_url)?;
        let (ws_stream, _) = connect_async(url).await?;
        let (mut write, mut read) = ws_stream.split();

        // Subscribe to slot notifications
        let slot_sub = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "slotSubscribe"
        });
        write
            .send(Message::Text(slot_sub.to_string().into()))
            .await?;

        // Subscribe to each pool account
        for (idx, addr) in pool_addresses.iter().enumerate() {
            let sub = serde_json::json!({
                "jsonrpc": "2.0",
                "id": idx + 1,
                "method": "accountSubscribe",
                "params": [
                    addr.to_string(),
                    { "encoding": "base64", "commitment": "processed" }
                ]
            });
            write.send(Message::Text(sub.to_string().into())).await?;
        }

        info!(pool_count = pool_addresses.len(), "Subscribed to pool accounts");

        while let Some(msg) = read.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                self.handle_message(&text);
            }
        }
        Ok(())
    }

    fn handle_message(&self, text: &str) {
        if !text.contains("\"method\"") {
            return;
        }

        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };

        let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

        match method {
            "accountNotification" => self.handle_account_notification(&v),
            "slotNotification" => {} // Could update an atomic slot counter
            _ => {}
        }
    }

    fn handle_account_notification(&self, v: &Value) {
        let result = v
            .get("params")
            .and_then(|p| p.get("result"))
            .and_then(|r| r.get("value"));

        let Some(result) = result else { return };

        let slot = v
            .get("params")
            .and_then(|p| p.get("context"))
            .and_then(|c| c.get("slot"))
            .and_then(|s| s.as_u64())
            .unwrap_or(0);

        // Decode base64 account data
        let data_b64 = result
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|s| s.as_str())
            .unwrap_or("");

        let Ok(data) = decode_base64(data_b64) else {
            return;
        };

        let owner_str = result
            .get("owner")
            .and_then(|o| o.as_str())
            .unwrap_or("");
        let Ok(owner) = owner_str.parse::<Pubkey>() else {
            return;
        };

        // In production, track subscription id → pubkey mapping.
        let pool_pubkey = Pubkey::default();

        let Some(pool_state) = AccountParser::parse(pool_pubkey, &owner, &data, slot) else {
            return;
        };

        let event = PoolUpdateEvent {
            pool: pool_state,
            slot,
            parsed_at_ns: monotonic_ns(),
        };

        if self.tx.try_send(event).is_err() {
            warn!("Ingress channel full; dropping pool update");
        }
    }
}

/// Minimal base64 decoder (standard alphabet, padded).
/// In production, use the `base64` crate for full correctness and speed.
fn decode_base64(input: &str) -> Result<Vec<u8>, &'static str> {
    const TABLE: &[u8; 128] = &{
        let mut t = [255u8; 128];
        let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0usize;
        while i < 64 {
            t[alpha[i] as usize] = i as u8;
            i += 1;
        }
        t
    };

    let bytes = input.as_bytes();
    let len = bytes.len();
    if len % 4 != 0 {
        return Err("invalid base64 length");
    }

    let mut out = Vec::with_capacity(len * 3 / 4);
    let mut i = 0;
    while i < len {
        let get = |c: u8| -> Result<u8, &'static str> {
            if c >= 128 {
                return Err("invalid base64 char");
            }
            let v = TABLE[c as usize];
            if v == 255 && c != b'=' {
                return Err("invalid base64 char");
            }
            Ok(if c == b'=' { 0 } else { v })
        };

        let b0 = get(bytes[i])?;
        let b1 = get(bytes[i + 1])?;
        let b2 = get(bytes[i + 2])?;
        let b3 = get(bytes[i + 3])?;

        out.push((b0 << 2) | (b1 >> 4));
        if bytes[i + 2] != b'=' {
            out.push((b1 << 4) | (b2 >> 2));
        }
        if bytes[i + 3] != b'=' {
            out.push((b2 << 6) | b3);
        }
        i += 4;
    }
    Ok(out)
}
