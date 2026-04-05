// =============================================================================
// MOCK YELLOWSTONE — Slot Update Stream
// =============================================================================

use rand::{Rng, SeedableRng};
use rand::rngs::SmallRng;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotUpdate {
    pub slot: u64,
    pub parent: u64,
    pub commitment: String,
    pub timestamp_ms: u64,
}

pub struct MockYellowstoneStream;

impl MockYellowstoneStream {
    #[must_use]
    pub fn spawn(slots_per_second: u64) -> mpsc::Receiver<SlotUpdate> {
        let (tx, rx) = mpsc::channel(1024);
        tokio::spawn(Self::run(tx, slots_per_second));
        rx
    }

    async fn run(tx: mpsc::Sender<SlotUpdate>, slots_per_second: u64) {
        let delay = 1000 / slots_per_second.max(1);
        let mut slot: u64 = 300_000_000;
        let mut rng = SmallRng::from_entropy();
        let commitments = ["processed", "confirmed", "finalized"];

        loop {
            for &commitment in &commitments {
                let update = SlotUpdate {
                    slot,
                    parent: slot.saturating_sub(1),
                    commitment: commitment.to_string(),
                    timestamp_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                };

                debug!(slot, commitment, "Yellowstone slot update");

                if tx.send(update).await.is_err() {
                    warn!("Yellowstone receiver dropped");
                    return;
                }

                let jitter: u64 = rng.gen_range(0..5);
                tokio::time::sleep(std::time::Duration::from_millis(delay + jitter)).await;
            }
            slot = slot.wrapping_add(1);
        }
    }
}
