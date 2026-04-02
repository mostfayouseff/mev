//! Transaction builder — assembles the full versioned transaction for a validated arb.
//!
//! DESIGN:
//! - Uses VersionedTransaction (v0) with Address Lookup Tables (ALTs) to pack more
//!   accounts per transaction (Solana's ALT feature reduces instruction size).
//! - Priority fee instruction is prepended so validators prioritise our tx.
//! - The transaction is a single atomic unit — all hops succeed or none do.

use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    hash::Hash,
    instruction::Instruction,
    message::{v0::Message, VersionedMessage},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::VersionedTransaction,
};
use tracing::debug;

use common::ApexError;
use safety::ValidatedOpportunity;

pub struct TransactionBuilder {
    payer: Keypair,
    priority_fee_lamports: u64,
    compute_unit_limit: u32,
}

impl TransactionBuilder {
    /// `keypair_bytes` is the raw 64-byte keypair (not base58).
    pub fn new(
        keypair_bytes: &[u8],
        priority_fee_lamports: u64,
        compute_unit_limit: u32,
    ) -> Result<Self, ApexError> {
        let payer = Keypair::from_bytes(keypair_bytes)
            .map_err(|e| ApexError::Config(format!("Invalid keypair: {e}")))?;
        Ok(Self {
            payer,
            priority_fee_lamports,
            compute_unit_limit,
        })
    }

    pub fn pubkey(&self) -> Pubkey {
        self.payer.pubkey()
    }

    /// Build a signed VersionedTransaction from the validated opportunity.
    pub fn build(
        &self,
        opp: &ValidatedOpportunity,
        recent_blockhash: Hash,
        extra_ixs: Vec<Instruction>, // e.g. Jito tip instruction
    ) -> Result<VersionedTransaction, ApexError> {
        let mut instructions: Vec<Instruction> = Vec::new();

        // ── Compute budget ─────────────────────────────────────────────────
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(
            self.compute_unit_limit,
        ));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
            self.priority_fee_lamports,
        ));

        // ── Extra instructions (Jito tip, etc.) ────────────────────────────
        instructions.extend(extra_ixs);

        // ── Hop swap instructions ─────────────────────────────────────────
        // In production: each hop yields an Instruction via the adapter registry.
        // For now, include a placeholder comment indicating where they'd go.
        // Replace this section with real swap instruction construction per DEX adapter.
        for (i, hop) in opp.opportunity.path.path.hops.iter().enumerate() {
            debug!(
                hop = i,
                pool = %hop.pool,
                dex = %hop.dex,
                a_to_b = hop.a_to_b,
                "Building swap instruction (stub)"
            );
            // production: instructions.push(adapter.build_swap_ix(hop, &self.payer.pubkey(), ...));
        }

        // ── Build v0 message ───────────────────────────────────────────────
        // ALT loading would go here in production (fetch ALT accounts from RPC).
        let message = Message::try_compile(
            &self.payer.pubkey(),
            &instructions,
            &[], // Address lookup tables (empty for now)
            recent_blockhash,
        )
        .map_err(|e| ApexError::Execution(format!("Message compile failed: {e}")))?;

        let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&self.payer])
            .map_err(|e| ApexError::Execution(format!("Transaction sign failed: {e}")))?;

        Ok(tx)
    }
}
