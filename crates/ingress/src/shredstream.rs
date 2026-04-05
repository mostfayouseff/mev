// =============================================================================
// SHRED STREAM — Mock + Parser for Solana shreds (Jito-style)
// =============================================================================

use bytes::Bytes;
use rand::{Rng, SeedableRng};
use rand::rngs::SmallRng;
use tokio::sync::mpsc;
use tracing::{debug, warn};

const MAX_SHRED_BYTES: usize = 1228;
const MIN_SHRED_BYTES: usize = 15;

/// Parsed shred event delivered to the hot loop.
#[derive(Debug, Clone)]
pub struct ShredEvent {
    pub slot: u64,
    pub index: u32,
    pub data: Bytes,        // zero-copy payload
}

pub struct MockShredStream;

impl MockShredStream {
    /// Spawn mock shred producer at given rate (Hz).
    #[must_use]
    pub fn spawn(rate_hz: u64) -> mpsc::Receiver<ShredEvent> {
        let (tx, rx) = mpsc::channel(8192);
        tokio::spawn(Self::run(tx, rate_hz));
        rx
    }

    async fn run(tx: mpsc::Sender<ShredEvent>, rate_hz: u64) {
        let delay = std::time::Duration::from_micros(1_000_000 / rate_hz.max(1));
        let mut slot: u64 = 300_000_000;
        let mut rng = SmallRng::from_entropy();

        loop {
            for index in 0..400 {
                let raw = Self::mock_raw_shred(&mut rng, slot, index);

                match parse_shred(&raw) {
                    Ok(event) => {
                        debug!(slot = event.slot, index = event.index, data_len = event.data.len(), "Mock shred parsed");
                        if tx.send(event).await.is_err() {
                            return; // receiver dropped
                        }
                    }
                    Err(e) => warn!(error = %e, "Shred parse failed"),
                }

                tokio::time::sleep(delay).await;
            }
            slot = slot.wrapping_add(1);
        }
    }

    fn mock_raw_shred(rng: &mut SmallRng, slot: u64, index: u32) -> Vec<u8> {
        let data_len = rng.gen_range(64..=512);
        let mut buf = Vec::with_capacity(14 + data_len);

        buf.extend_from_slice(&slot.to_le_bytes());
        buf.extend_from_slice(&index.to_le_bytes());
        buf.extend_from_slice(&(data_len as u16).to_le_bytes());
        buf.resize(14 + data_len, 0);
        rng.fill(&mut buf[14..]);

        buf
    }
}

/// Zero-copy shred parser.
pub fn parse_shred(raw: &[u8]) -> anyhow::Result<ShredEvent> {
    if raw.len() > MAX_SHRED_BYTES {
        return Err(anyhow::anyhow!("Packet too large: {} > {}", raw.len(), MAX_SHRED_BYTES));
    }
    if raw.len() < MIN_SHRED_BYTES {
        return Err(anyhow::anyhow!("Packet too small: {}", raw.len()));
    }

    // slot (0..8)
    let slot = u64::from_le_bytes(
        raw.get(0..8)
            .ok_or_else(|| anyhow::anyhow!("Failed to read slot"))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid slot bytes"))?,
    );

    // index (8..12)
    let index = u32::from_le_bytes(
        raw.get(8..12)
            .ok_or_else(|| anyhow::anyhow!("Failed to read index"))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid index bytes"))?,
    );

    // data_len (12..14)
    let data_len = u16::from_le_bytes(
        raw.get(12..14)
            .ok_or_else(|| anyhow::anyhow!("Failed to read data_len"))?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid data_len bytes"))?,
    ) as usize;

    // payload (14..14+data_len)
    let data_end = 14usize
        .checked_add(data_len)
        .ok_or_else(|| anyhow::anyhow!("data_len overflow"))?;

    let payload = raw
        .get(14..data_end)
        .ok_or_else(|| anyhow::anyhow!("data_len exceeds packet size"))?;

    Ok(ShredEvent {
        slot,
        index,
        data: Bytes::copy_from_slice(payload),   // zero-copy view
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raw(slot: u64, index: u32, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&slot.to_le_bytes());
        buf.extend_from_slice(&index.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn parse_valid_shred() {
        let raw = make_raw(12345, 99, &[0xAA; 64]);
        let event = parse_shred(&raw).expect("parse should succeed");
        assert_eq!(event.slot, 12345);
        assert_eq!(event.index, 99);
        assert_eq!(event.data.len(), 64);
    }

    #[test]
    fn rejects_too_large() {
        let raw = vec![0u8; MAX_SHRED_BYTES + 1];
        assert!(parse_shred(&raw).is_err());
    }
}
