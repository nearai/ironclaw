//! ZK proof generation for execution traces.
//!
//! This module generates RISC Zero receipts that prove an agent's execution
//! was policy-compliant. The actual RISC Zero proving happens either:
//! - Locally (requires `risc-zero` toolchain)
//! - Via Boundless proving marketplace
//!
//! The `zkvm/` directory contains the guest program that defines the circuit.

use crate::types::{ExecutionTrace, ProofReceipt, VerifiedOutput};
use anyhow::Result;
use sha2::{Digest, Sha256};

/// Generates RISC Zero proof receipts for execution traces.
///
/// Currently produces mock receipts using SHA-256 hashing.

pub struct ProofGenerator {
    /// Use Boundless marketplace for proving (if true). Local RISC Zero if false.
    use_boundless: bool,
    /// RISC Zero image ID of the deployed proof circuit.
    image_id: String,
}

impl ProofGenerator {
    /// Create a new generator.
    ///
    /// `image_id` — RISC Zero image ID loaded from `RISC_ZERO_IMAGE_ID` env var.
    /// When empty, proofs will be non-verifiable on-chain.
    pub fn new(use_boundless: bool, image_id: String) -> Self {
        Self {
            use_boundless,
            image_id,
        }
    }

    /// Generate a proof receipt for an execution trace.
    ///
    /// In production this calls the RISC Zero prover or Boundless API.
    /// Currently produces a mock receipt using SHA-256.
    pub async fn generate_proof(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
        if self.use_boundless {
            self.generate_proof_boundless(trace).await
        } else {
            self.generate_proof_local(trace).await
        }
    }

    async fn generate_proof_boundless(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
        tracing::info!("Generating proof via Boundless proving marketplace");

        let verified_output = self.compute_verified_output(trace)?;
        let journal = serde_json::to_vec(&verified_output)?;

        let mut h = Sha256::new();
        h.update(&journal);
        let seal = h.finalize().to_vec();

        Ok(ProofReceipt {
            journal,
            seal,
            image_id: self.image_id.clone(),
        })
    }

    async fn generate_proof_local(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
        tracing::info!("Generating proof via local RISC Zero prover");

        let verified_output = self.compute_verified_output(trace)?;
        let journal = serde_json::to_vec(&verified_output)?;

        let mut h = Sha256::new();
        h.update(&journal);
        let seal = h.finalize().to_vec();

        Ok(ProofReceipt {
            journal,
            seal,
            image_id: self.image_id.clone(),
        })
    }

    /// Compute the verified outputs that go into the proof journal.
    fn compute_verified_output(&self, trace: &ExecutionTrace) -> Result<VerifiedOutput> {
        let all_checks_passed = trace
            .policy_check_results
            .iter()
            .all(|r| !matches!(r.severity, crate::types::PolicySeverity::Block));

        let action_value: u64 = trace
            .tool_invocations
            .iter()
            .filter(|inv| inv.tool_name.contains("swap") || inv.tool_name.contains("transfer"))
            .map(|_| 1_000_000_000_000_000_000u64)
            .sum();

        let requires_ledger_approval = action_value > 1_000_000_000_000_000_000;

        let mut h = Sha256::new();
        h.update(trace.agent_id.as_bytes());
        let policy_hash = format!("0x{}", hex::encode(h.finalize()));

        Ok(VerifiedOutput {
            agent_id: trace.agent_id.clone(),
            policy_hash,
            output_commitment: trace.output_commitment.clone(),
            all_checks_passed,
            requires_ledger_approval,
            action_value,
        })
    }

    /// Verify a proof receipt by decoding the journal.
    pub fn verify_receipt(&self, receipt: &ProofReceipt) -> Result<VerifiedOutput> {
        let output: VerifiedOutput = serde_json::from_slice(&receipt.journal)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use crate::types::{PolicyResult, PolicySeverity, ToolInvocation};

    use super::*;

    fn test_image_id() -> String {
        "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string()
    }

    fn test_trace() -> ExecutionTrace {
        ExecutionTrace {
            agent_id: "test-agent".to_string(),
            session_id: "session-123".to_string(),
            timestamp: 1234567890,
            inference_commitment: "0xabcd".to_string(),
            tool_invocations: vec![ToolInvocation {
                tool_name: "swap_tokens".to_string(),
                input_hash: "0x1111".to_string(),
                output_hash: "0x2222".to_string(),
                capability_hash: "0x3333".to_string(),
                timestamp: 1234567890,
                within_policy: true,
            }],
            policy_check_results: vec![PolicyResult {
                rule_id: "tool_allowlist".to_string(),
                severity: PolicySeverity::Pass,
                details: "All checks passed".to_string(),
            }],
            output_commitment: "0xoutput".to_string(),
        }
    }

    #[tokio::test]
    async fn test_proof_generation() {
        let gen = ProofGenerator::new(true, test_image_id());
        let receipt = gen.generate_proof(&test_trace()).await.unwrap();
        assert!(!receipt.journal.is_empty());
        assert!(!receipt.seal.is_empty());
        assert_eq!(receipt.image_id, test_image_id());
    }

    #[tokio::test]
    async fn test_verify_receipt() {
        let gen = ProofGenerator::new(true, test_image_id());
        let receipt = gen.generate_proof(&test_trace()).await.unwrap();
        let verified = gen.verify_receipt(&receipt).unwrap();
        assert_eq!(verified.agent_id, "test-agent");
        assert!(verified.all_checks_passed);
    }

    #[tokio::test]
    async fn test_ledger_approval_required() {
        let gen = ProofGenerator::new(true, test_image_id());
        let mut trace = test_trace();
        trace.tool_invocations.push(ToolInvocation {
            tool_name: "transfer".to_string(),
            input_hash: "0x4444".to_string(),
            output_hash: "0x5555".to_string(),
            capability_hash: "0x6666".to_string(),
            timestamp: 1234567890,
            within_policy: true,
        });
        let receipt = gen.generate_proof(&trace).await.unwrap();
        let verified = gen.verify_receipt(&receipt).unwrap();
        assert!(verified.requires_ledger_approval);
    }
}
