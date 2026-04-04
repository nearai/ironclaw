//! IronClaw adapter — wires Proof of Claw into ironclaw's lifecycle hook system.
//!
//! Registers hooks at the following points:
//! - `BeforeInbound` — prompt injection detection (FailClosed)
//! - `BeforeToolCall` — policy engine checks (FailClosed)
//! - `OnSessionEnd` — store trace on 0G + generate ZK proof

use crate::{
    config::AgentConfig,
    injection_detector::InjectionDetector,
    policy_engine::PolicyEngine,
    proof_generator::ProofGenerator,
    types::{
        AgentMessage, ExecutionTrace, PolicyResult, PolicySeverity, ToolInvocation,
    },
    zero_g::{ZeroGCompute, ZeroGStorage},
};
use anyhow::Result;
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Full adapter — holds all POC components needed by the hooks.
pub struct IronClawAdapter {
    pub config: AgentConfig,
    pub proof_generator: ProofGenerator,
    pub zero_g_compute: ZeroGCompute,
    pub zero_g_storage: ZeroGStorage,
    pub injection_detector: InjectionDetector,
    pub policy_engine: PolicyEngine,
}

impl IronClawAdapter {
    pub async fn new(config: AgentConfig) -> Result<Self> {
        let zero_g_compute = ZeroGCompute::new(&config).await?;
        let zero_g_storage = ZeroGStorage::new(&config).await?;

        let image_id = config
            .risc_zero_image_id
            .clone()
            .unwrap_or_else(|| {
                tracing::warn!(
                    "RISC_ZERO_IMAGE_ID not set — proofs will not be verifiable on-chain"
                );
                String::new()
            });

        let proof_generator = ProofGenerator::new(true, image_id);

        Ok(Self {
            config: config.clone(),
            proof_generator,
            zero_g_compute,
            zero_g_storage,
            injection_detector: InjectionDetector::new(),
            policy_engine: PolicyEngine::new(config.policy.clone()),
        })
    }

    /// Convert an ironclaw tool call + policy results into a POC ExecutionTrace.
    pub fn build_trace(
        &self,
        session_id: &str,
        tool_calls: Vec<(String, serde_json::Value, serde_json::Value, bool)>,
        policy_results: Vec<PolicyResult>,
        inference_attestation: Option<String>,
    ) -> ExecutionTrace {
        let timestamp = chrono::Utc::now().timestamp();

        let tool_invocations: Vec<ToolInvocation> = tool_calls
            .into_iter()
            .map(|(name, input, output, allowed)| {
                let input_hash = sha256_json(&input);
                let output_hash = sha256_json(&output);
                ToolInvocation {
                    tool_name: name,
                    input_hash,
                    output_hash,
                    capability_hash: String::new(),
                    timestamp,
                    within_policy: allowed,
                }
            })
            .collect();

        let output_commitment = {
            let mut h = Sha256::new();
            // Simplified — in production serialize the full trace
            h.update(format!("{}:{}", session_id, timestamp).as_bytes());
            format!("0x{}", hex::encode(h.finalize()))
        };

        ExecutionTrace {
            agent_id: self.config.agent_id.clone(),
            session_id: session_id.to_string(),
            timestamp,
            inference_commitment: inference_attestation.unwrap_or_default(),
            tool_invocations,
            policy_check_results: policy_results,
            output_commitment,
        }
    }

    /// Store trace on 0G Storage and generate ZK proof.
    pub async fn process_trace(&self, trace: &ExecutionTrace) -> Result<String> {
        let trace_hash = self.zero_g_storage.store_trace(trace).await?;
        tracing::info!("Trace stored on 0G Storage: {trace_hash}");

        let receipt = self.proof_generator.generate_proof(trace).await?;
        tracing::info!(
            "ZK proof generated: image_id={}, journal_len={}",
            receipt.image_id,
            receipt.journal.len()
        );

        Ok(trace_hash)
    }
}

// ── SHA-256 helpers ──────────────────────────────────────────────────────────

fn sha256_json(value: &serde_json::Value) -> String {
    let s = serde_json::to_string(value).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("0x{}", hex::encode(h.finalize()))
}

// ── Ironclaw hook implementations ────────────────────────────────────────────
//
// These implement ironclaw's `Hook` trait and delegate to the adapter.

use ironclaw::hooks::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};

/// Hook at `BeforeInbound` — rejects prompt injection attempts.
pub struct InjectionDetectionHook {
    detector: InjectionDetector,
}

impl InjectionDetectionHook {
    pub fn new(detector: InjectionDetector) -> Self {
        Self { detector }
    }
}

#[async_trait]
impl Hook for InjectionDetectionHook {
    fn name(&self) -> &str {
        "proof_of_claw::injection_detection"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::BeforeInbound]
    }

    fn failure_mode(&self) -> HookFailureMode {
        HookFailureMode::FailClosed
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let content = match event {
            HookEvent::Inbound { content, .. } => content,
            _ => return Ok(HookOutcome::ok()),
        };

        if self.detector.detect(content) {
            tracing::warn!(
                "proof_of_claw: rejected injection attempt: {}",
                &content[..content.len().min(100)]
            );
            return Ok(HookOutcome::reject(
                "Prompt injection detected by Proof of Claw safety layer",
            ));
        }

        Ok(HookOutcome::ok())
    }
}

/// Hook at `BeforeToolCall` — checks tool against policy and value thresholds.
pub struct PolicyEnforcementHook {
    engine: PolicyEngine,
}

impl PolicyEnforcementHook {
    pub fn new(engine: PolicyEngine) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Hook for PolicyEnforcementHook {
    fn name(&self) -> &str {
        "proof_of_claw::policy_enforcement"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::BeforeToolCall]
    }

    fn failure_mode(&self) -> HookFailureMode {
        HookFailureMode::FailClosed
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let (tool_name, parameters) = match event {
            HookEvent::ToolCall {
                tool_name,
                parameters,
                ..
            } => (tool_name, parameters),
            _ => return Ok(HookOutcome::ok()),
        };

        // Convert serde_json::Value to HashMap for MessagePayload
        let params: std::collections::HashMap<String, serde_json::Value> = parameters
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let dummy_message = AgentMessage {
            message_type: crate::types::MessageType::Execute,
            payload: crate::types::MessagePayload {
                action: tool_name.clone(),
                params,
                trace_root_hash: None,
                proof_receipt: None,
                required_approval: None,
            },
            nonce: 0,
            timestamp: chrono::Utc::now().timestamp(),
        };
        let dummy_inference = crate::types::InferenceResponse {
            content: String::new(),
            attestation_signature: String::new(),
            provider: "0g-compute".to_string(),
        };

        let result = self.engine.check(&dummy_message, &dummy_inference);

        match result.severity {
            PolicySeverity::Block => {
                tracing::warn!(
                    "proof_of_claw: blocked tool '{tool_name}': {}",
                    result.details
                );
                Ok(HookOutcome::reject(format!(
                    "Policy violation: {}. Tool '{}' is not permitted.",
                    result.details, tool_name
                )))
            }
            PolicySeverity::Warn => {
                tracing::warn!(
                    "proof_of_claw: warning for tool '{tool_name}': {}",
                    result.details
                );
                Ok(HookOutcome::ok()) // Warn but don't block
            }
            PolicySeverity::Sanitize | PolicySeverity::Pass => Ok(HookOutcome::ok()),
        }
    }
}

/// Hook at `OnSessionEnd` — stores execution trace and generates ZK proof.
pub struct ProofGenerationHook {
    adapter: Arc<IronClawAdapter>,
}

impl ProofGenerationHook {
    pub fn new(adapter: Arc<IronClawAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl Hook for ProofGenerationHook {
    fn name(&self) -> &str {
        "proof_of_claw::proof_generation"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::OnSessionEnd]
    }

    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30)
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let session_id = match event {
            HookEvent::SessionEnd { session_id, .. } => session_id,
            HookEvent::SessionStart { session_id, .. } => session_id,
            _ => return Ok(HookOutcome::ok()),
        };

        // Build an empty trace for now; ironclaw's session context will
        // populate actual tool_calls when full integration is done.
        let trace = self.adapter.build_trace(
            session_id,
            vec![], // tool_calls populated by full ironclaw integration
            vec![],
            None,
        );

        match self.adapter.process_trace(&trace).await {
            Ok(trace_hash) => {
                tracing::info!(
                    "proof_of_claw: session {session_id} trace committed: {trace_hash}"
                );
                Ok(HookOutcome::ok())
            }
            Err(e) => {
                tracing::error!("proof_of_claw: failed to process trace: {e}");
                // Fail open — don't block session end for proof failures
                Ok(HookOutcome::ok())
            }
        }
    }
}
