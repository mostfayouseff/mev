use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async_tls_with_config,   // ← Added this
    tungstenite::Message,
};
use tracing::{error, info, warn};

pub const DEX_PROGRAMS: &[&str] = &[
    "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",
    "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK",
    "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3sFjJ37",
    "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo",
    "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB",
    "PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY",
    "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
];

const RECONNECT_BASE_DELAY_MS: u64 = 500;
const RECONNECT_MAX_DELAY_MS: u64 = 30_000;

#[derive(Deserialize, Debug)]
struct WsMessage {
    method: Option<String>,
    params: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    id: Option<u64>,
    error: Option<WsError>,
}

#[derive(Deserialize, Debug)]
struct WsError {
    code: i64,
    message: String,
}

pub struct ShredEvent {
    pub slot: u64,
    pub index: u32,
    pub data: Bytes,
}

pub struct HeliusTransactionStream;

impl HeliusTransactionStream {
    #[must_use]
    pub fn spawn(api_key: String) -> mpsc::Receiver<ShredEvent> {
        let (tx, rx) = mpsc::channel(8192);
        tokio::spawn(Self::run(tx, api_key));
        rx
    }

    async fn run(tx: mpsc::Sender<ShredEvent>, api_key: String) {
        let url = format!("wss://mainnet.helius-rpc.com/?api-key={api_key}");
        let mut delay = RECONNECT_BASE_DELAY_MS;
        let mut attempts = 0u64;

        info!("LIVE DATA SOURCE: HELIUS — starting primary WebSocket stream");

        loop {
            attempts += 1;
            info!(attempt = attempts, "Helius WS: connecting");

            match Self::connect_and_stream(&url, &tx).await {
                Ok(()) => info!("Helius WS: stream ended cleanly — reconnecting"),
                Err(e) => warn!(error = %e, delay_ms = delay, "Helius WS: connection failed"),
            }

            if tx.is_closed() {
                return;
            }

            sleep(Duration::from_millis(delay)).await;
            delay = (delay * 2).min(RECONNECT_MAX_DELAY_MS);
        }
    }

    async fn connect_and_stream(url: &str, tx: &mpsc::Sender<ShredEvent>) -> anyhow::Result<()> {
        let (mut ws, _) = connect_async_tls_with_config(url, None, false, None).await?;
        info!("Helius WS: connected — subscribing to DEX programs");

        for (i, &prog) in DEX_PROGRAMS.iter().enumerate() {
            Self::subscribe_logs(&mut ws, (i + 1) as u64, prog).await?;
        }

        let mut event_count: u64 = 0;

        while let Some(msg_result) = ws.next().await {
            let msg = msg_result?;
            let text = match msg {
                Message::Text(t) => t,
                Message::Ping(data) => {
                    let _ = ws.send(Message::Pong(data)).await;
                    continue;
                }
                Message::Close(_) => return Ok(()),
                _ => continue,
            };

            let notification: WsMessage = match serde_json::from_str(&text) {
                Ok(n) => n,
                Err(_) => continue,
            };

            if let Some(err) = notification.error {
                error!(code = err.code, "Helius subscription error: {}", err.message);
                continue;
            }

            if notification.method.as_deref() == Some("logsNotification")
                || notification.method.as_deref() == Some("transactionNotification")
            {
                let slot = extract_slot(&notification.params);
                event_count += 1;

                let event = ShredEvent {
                    slot,
                    index: (event_count & 0xFFFF_FFFF) as u32,
                    data: Bytes::from_static(b"helius_trigger"),
                };

                if tx.send(event).await.is_err() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    async fn subscribe_logs(
        ws: &mut (impl futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
        id: u64,
        program: &str,
    ) -> anyhow::Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "logsSubscribe",
            "params": [
                { "mentions": [program] },
                { "commitment": "processed" }
            ]
        });
        ws.send(Message::Text(msg.to_string())).await?;
        Ok(())
    }
}

fn extract_slot(params: &Option<serde_json::Value>) -> u64 {
    params
        .as_ref()
        .and_then(|p| p.get("result"))
        .and_then(|r| r.get("context"))
        .and_then(|c| c.get("slot"))
        .and_then(|s| s.as_u64())
        .unwrap_or(0)
}
