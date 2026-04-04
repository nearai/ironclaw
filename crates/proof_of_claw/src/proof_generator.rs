//! ZK proof generation for execution traces.
//!
//! This module generates RISC Zero receipts that prove an agent's execution
//! was policy-compliant. The actual RISC Zero proving happens either:
//! - **Locally** (`--features zk`): requires `cargo install cargo-risczero && risczero install`
//!   The guest ELF is compiled to `zkvm/guest/target/riscv32im-risc0-zkvm-elf/release/proof-of-claw-guest`
//! - **Via Boundless** proving marketplace (set `ZERO_G_COMPUTE_ENDPOINT`)
//!
//! Without the `zk` feature, proofs are SHA-256 mocks — suitable for development and CI
//! but **not cryptographically verifiable**.

use crate::types::{ExecutionTrace, PolicySeverity, ProofReceipt, VerifiedOutput};
use anyhow::Result;
use sha2::{Digest, Sha256};

/// Generates RISC Zero proof receipts for execution traces.
pub struct ProofGenerator {
    /// Use Boundless marketplace for proving (if true). Local RISC Zero if false.
    use_boundless: bool,
    /// RISC Zero image ID (hex string, "0x" prefix).
    /// Computed at build time from the guest ELF when `zk` feature is enabled.
    image_id: String,
}

impl ProofGenerator {
    /// Create a new generator.
    ///
    /// `image_id` — RISC Zero image ID loaded from `RISC_ZERO_IMAGE_ID` env var,
    /// or computed from the guest ELF when the `zk` feature is enabled.
    pub fn new(use_boundless: bool, image_id: String) -> Self {
        Self {
            use_boundless,
            image_id,
        }
    }

    /// Generate a proof receipt for an execution trace.
    ///
    /// With `zk` feature: calls the real RISC Zero prover.
    /// Without: produces a mock SHA-256 receipt (dev/CI only).
    pub async fn generate_proof(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
        if self.use_boundless {
            self.generate_proof_boundless(trace).await
        } else {
            self.generate_proof_local(trace).await
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Local / ZK feature — real RISC Zero proving (~30–60s per proof)
    // ─────────────────────────────────────────────────────────────────────────

    #[cfg(feature = "zk")]
    async fn generate_proof_local(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
        use risc0_zkvm::{default_prover, ExecutorEnv};

        tracing::info!(
            "Generating REAL RISC Zero ZK proof — this takes ~30–60s (proving is compute-heavy)"
        );

        let verified_output = self.compute_verified_output(trace)?;
        let journal = serde_json::to_vec(&verified_output)?;

        let env = ExecutorEnv::builder()
            .write(trace)?
            .write(&crate::zk::policy_for_prover())?
            .build()?;

        let prover = default_prover();
        let receipt = prover.prove(env, crate::zk::guest_elf())?;

        tracing::info!(
            "ZK proof generated! journal_len={} seal_len={}",
            receipt.journal.bytes.len(),
            receipt.seal.len()
        );

        Ok(ProofReceipt {
            journal: receipt.journal.bytes,
            seal: receipt.seal,
            image_id: self.image_id.clone(),
        })
    }

    #[cfg(not(feature = "zk"))]
    async fn generate_proof_local(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
        tracing::warn!(
            "ZK proving disabled — build with `--features zk` + RISC Zero toolchain for real proofs"
        );
        Ok(self.generate_mock_receipt(trace)?)
    }

    async fn generate_proof_boundless(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
        #[cfg(feature = "zk")]
        {
            if let Some(receipt) = self.try_boundless_proof(trace).await? {
                return Ok(receipt);
            }
            tracing::info!("Boundless proving unavailable — falling back to local prover");
            return self.generate_proof_local(trace).await;
        }
        #[cfg(not(feature = "zk"))]
        {
            let _ = trace;
            tracing::warn!(
                "Boundless proof requested but `zk` feature not enabled — using mock receipt"
            );
            Ok(self.generate_mock_receipt(trace)?)
        }
    }

    /// Attempt Boundless marketplace proving. Returns `Some(receipt)` on success.
    #[cfg(feature = "zk")]
    async fn try_boundless_proof(&self, _trace: &ExecutionTrace) -> Result<Option<ProofReceipt>> {
        // TODO: POST to ZERO_G_COMPUTE_ENDPOINT / Boundless API
        // Parse { journal: base64, seal: base64, image_id: "0x..." }
        // Return None to fall back to local proving.
        Ok(None)
    }

    fn generate_mock_receipt(&self, trace: &ExecutionTrace) -> Result<ProofReceipt> {
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
    /// Identical logic in both ZK and mock modes.
    fn compute_verified_output(&self, trace: &ExecutionTrace) -> Result<VerifiedOutput> {
        let all_checks_passed = trace
            .policy_check_results
            .iter()
            .all(|r| !matches!(r.severity, PolicySeverity::Block));

        let action_value: u64 = trace
            .tool_invocations
            .iter()
            .filter(|inv| {
                let n = inv.tool_name.to_lowercase();
                n.contains("swap") || n.contains("transfer")
            })
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

    /// Verify a proof receipt.
    ///
    /// With `zk` feature: calls `risc0_zkvm::verify()` — cryptographically verifies
    /// the seal against the journal using the image ID. This is the call made in
    /// the browser via the WASM verifier.
    ///
    /// Without `zk`: decodes the journal as JSON (mock verification — dev/CI only).
    pub fn verify_receipt(&self, receipt: &ProofReceipt) -> Result<VerifiedOutput> {
        #[cfg(feature = "zk")]
        {
            use risc0_zkvm::verify;

            let image_id_bytes: [u8; 32] = hex::decode(&receipt.image_id[2..])?
                .try_into()
                .map_err(|_| anyhow::anyhow!("image_id must be 32 bytes"))?;

            verify(&image_id_bytes, &receipt.seal, &receipt.journal)
                .map_err(|e| anyhow::anyhow!("ZK verification failed: {}", e))?;

            let output: VerifiedOutput = serde_json::from_slice(&receipt.journal)?;
            Ok(output)
        }

        #[cfg(not(feature = "zk"))]
        {
            let output: VerifiedOutput = serde_json::from_slice(&receipt.journal)?;
            Ok(output)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// zk — conditionally compiled submodule for real RISC Zero proving
// ─────────────────────────────────────────────────────────────────────────────
// Only compiled when `--features zk` is set AND the RISC Zero toolchain is installed.
// Provides guest ELF bytes and the computed image ID.

#[cfg(feature = "zk")]
mod zk {
    use sha2::{Digest, Sha256};

    /// Embed the guest ELF via risc0-build.
    /// The guest must be built first: cd zkvm/guest && cargo build --release
    risc0_build::embed_methods!(
        path = "../../../../zkvm/guest",
        profile = "release"
    );

    /// Returns the guest ELF bytes for the RISC Zero prover.
    pub fn guest_elf() -> &'static [u8] {
        &Methods::ELFS[0]
    }

    /// Computes the RISC Zero image ID = SHA-256(guest_elf).
    pub fn compute_image_id() -> String {
        let hash = Sha256::digest(guest_elf());
        format!("0x{}", hex::encode(hash))
    }

    /// Minimal AgentPolicy for the prover — mirrors zkvm/guest/src/main.rs.
    #[derive(serde::Serialize)]
    pub struct PolicyForProver {
        pub allowed_tools: Vec<String>,
        pub endpoint_allowlist: Vec<String>,
        pub max_value_autonomous: u64,
        pub capability_root: [u8; 32],
    }

    pub fn policy_for_prover() -> PolicyForProver {
        PolicyForProver {
            allowed_tools: vec![
                "swap_tokens".into(),
                "transfer".into(),
                "query".into(),
                "read_file".into(),
                "write_file".into(),
                "edit_file".into(),
                "glob_search".into(),
                "grep_search".into(),
                "bash".into(),
                "web_search".into(),
                "web_fetch".into(),
            ],
            endpoint_allowlist: vec![
                "https://api.uniswap.org".into(),
                "https://api.coingecko.com".into(),
            ],
            max_value_autonomous: 100_000_000_000_000_000_000u64,
            capability_root: [0u8; 32],
        }
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
    async fn test_proof_generation_mock() {
        let gen = ProofGenerator::new(true, test_image_id());
        let receipt = gen.generate_proof(&test_trace()).await.unwrap();
        assert!(!receipt.journal.is_empty());
        assert!(!receipt.seal.is_empty());
        assert_eq!(receipt.image_id, test_image_id());
    }

    #[tokio::test]
    async fn test_verify_receipt_mock() {
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
