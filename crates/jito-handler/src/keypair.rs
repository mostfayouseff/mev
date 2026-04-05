// =============================================================================
// APEX KEYPAIR — Clean & Secure
// =============================================================================

use anyhow::{Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use tracing::info;

/// Operator keypair loaded from JSON file.
pub struct ApexKeypair {
    signing_key: SigningKey,
    /// Base58-encoded public key (safe for logging)
    pub pubkey_b58: String,
    /// Raw 32-byte public key
    pub pubkey_bytes: [u8; 32],
}

impl ApexKeypair {
    /// Load keypair from Solana JSON file (64-byte array).
    pub fn load(path: &str) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read keypair file: {}", path))?;

        let bytes: Vec<u8> = serde_json::from_str(&raw)
            .with_context(|| format!("Keypair file is not a valid JSON byte array: {}", path))?;

        if bytes.len() != 64 {
            return Err(anyhow::anyhow!(
                "Keypair must contain exactly 64 bytes, got {}",
                bytes.len()
            ));
        }

        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes[0..32]);

        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();
        let pubkey_bytes = verifying_key.to_bytes();
        let pubkey_b58 = bs58::encode(pubkey_bytes).into_string();

        info!(pubkey = %pubkey_b58, "Keypair loaded successfully");

        Ok(Self {
            signing_key,
            pubkey_b58,
            pubkey_bytes,
        })
    }

    /// Mock keypair for simulation mode (zero signatures).
    pub fn mock() -> Self {
        let secret = [0u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();
        let pubkey_bytes = verifying_key.to_bytes();
        let pubkey_b58 = bs58::encode(pubkey_bytes).into_string();

        info!(pubkey = %pubkey_b58, "Using mock keypair (simulation mode)");

        Self {
            signing_key,
            pubkey_b58,
            pubkey_bytes,
        }
    }

    /// Sign a message (e.g. transaction message bytes).
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    /// Verify a signature (sanity check).
    #[must_use]
    pub fn verify(&self, message: &[u8], sig: &[u8; 64]) -> bool {
        let signature = Signature::from_bytes(sig);
        self.signing_key.verifying_key().verify(message, &signature).is_ok()
    }
}

// ── Transaction helpers ─────────────────────────────────────────────────────

/// Extract the message bytes that need to be signed from a serialized Solana transaction.
pub fn extract_message_bytes(tx_bytes: &[u8]) -> Result<&[u8]> {
    if tx_bytes.is_empty() {
        return Err(anyhow::anyhow!("Transaction buffer is empty"));
    }

    let mut pos = 0usize;

    // Skip versioned tx prefix (0x80+)
    if tx_bytes[0] >= 0x80 {
        pos += 1;
    }

    let (num_sigs, compact_len) = decode_compact_u16(&tx_bytes[pos..]);
    pos += compact_len + (num_sigs as usize * 64);

    if pos > tx_bytes.len() {
        return Err(anyhow::anyhow!("Transaction truncated — cannot extract message"));
    }

    Ok(&tx_bytes[pos..])
}

/// Inject a signature into the correct slot of a serialized transaction.
pub fn inject_signature(tx_bytes: &mut Vec<u8>, sig_index: usize, sig: &[u8; 64]) -> Result<()> {
    if tx_bytes.is_empty() {
        return Err(anyhow::anyhow!("Transaction buffer is empty"));
    }

    let mut pos = 0usize;
    if tx_bytes[0] >= 0x80 {
        pos += 1;
    }

    let (num_sigs, compact_len) = decode_compact_u16(&tx_bytes[pos..]);
    pos += compact_len;

    if sig_index >= num_sigs as usize {
        return Err(anyhow::anyhow!("Signature index {} out of range (only {} signatures)", sig_index, num_sigs));
    }

    let sig_start = pos + sig_index * 64;
    let sig_end = sig_start + 64;

    if sig_end > tx_bytes.len() {
        return Err(anyhow::anyhow!("Transaction too short for signature slot {}", sig_index));
    }

    tx_bytes[sig_start..sig_end].copy_from_slice(sig);
    Ok(())
}

/// Decode Solana compact-u16 encoding.
fn decode_compact_u16(bytes: &[u8]) -> (u16, usize) {
    if bytes.is_empty() {
        return (0, 0);
    }

    let b0 = bytes[0];
    if b0 & 0x80 == 0 {
        return (b0 as u16, 1);
    }
    if bytes.len() < 2 {
        return ((b0 & 0x7f) as u16, 1);
    }

    let b1 = bytes[1];
    if b1 & 0x80 == 0 {
        return (((b0 & 0x7f) as u16) | ((b1 as u16) << 7), 2);
    }
    if bytes.len() < 3 {
        return (((b0 & 0x7f) as u16) | ((b1 as u16) << 7), 2);
    }

    let b2 = bytes[2];
    let val = ((b0 & 0x7f) as u16) | (((b1 & 0x7f) as u16) << 7) | ((b2 as u16) << 14);
    (val, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_keypair_works() {
        let kp = ApexKeypair::mock();
        assert!(!kp.pubkey_b58.is_empty());
        assert_eq!(kp.pubkey_bytes.len(), 32);
    }

    #[test]
    fn sign_verify_roundtrip() {
        let kp = ApexKeypair::mock();
        let msg = b"test message";
        let sig = kp.sign(msg);
        assert!(kp.verify(msg, &sig));
        assert!(!kp.verify(b"wrong message", &sig));
    }

    #[test]
    fn compact_u16_decode() {
        assert_eq!(decode_compact_u16(&[0x01]), (1, 1));
        assert_eq!(decode_compact_u16(&[0xAC, 0x02]), (300, 2));
    }
}
