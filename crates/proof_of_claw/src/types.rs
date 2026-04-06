//! Core data types for Proof of Claw.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Execution trace ─────────────────────────────────────────────────────────

/// An immutable record of every action an agent took during a session.
/// This is the primary input to proof generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub agent_id: String,
    pub session_id: String,
    pub timestamp: i64,
    /// Commitment to the LLM inference (0G Compute attestation or fallback hash).
    pub inference_commitment: String,
    pub tool_invocations: Vec<ToolInvocation>,
    pub policy_check_results: Vec<PolicyResult>,
    /// Final output hash — committed to the entire trace.
    pub output_commitment: String,
}

/// A single tool call within an execution trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub tool_name: String,
    /// SHA-256 of the tool input parameters.
    pub input_hash: String,
    /// SHA-256 of the tool output.
    pub output_hash: String,
    /// Content-addressable hash of the tool's capability definition.
    pub capability_hash: String,
    pub timestamp: i64,
    /// Whether this invocation was within the agent's active policy.
    pub within_policy: bool,
}

/// Result of a single policy rule check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResult {
    pub rule_id: String,
    pub severity: PolicySeverity,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PolicySeverity {
    Block,
    Warn,
    Sanitize,
    Pass,
}

// ── Agent identity / policy ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPolicy {
    pub allowed_tools: Vec<String>,
    pub endpoint_allowlist: Vec<String>,
    pub max_value_autonomous: u64,
    pub capability_root: String,
}

// ── Inter-agent messaging ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub message_type: MessageType,
    pub payload: MessagePayload,
    pub nonce: u64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    Propose,
    Accept,
    Reject,
    Execute,
    Verify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub action: String,
    pub params: HashMap<String, serde_json::Value>,
    pub trace_root_hash: Option<String>,
    pub proof_receipt: Option<String>,
    pub required_approval: Option<bool>,
}

// ── LLM inference ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponse {
    pub content: String,
    /// TEE attestation signature from 0G Compute, or a local content hash fallback.
    pub attestation_signature: String,
    pub provider: String,
}

// ── ZK proof receipt ─────────────────────────────────────────────────────────

/// Output of the RISC Zero proof generation step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofReceipt {
    /// ABI-encoded verified outputs (journal).
    pub journal: Vec<u8>,
    /// Cryptographic seal.
    pub seal: Vec<u8>,
    /// RISC Zero image ID of the proof circuit.
    pub image_id: String,
}

/// The verified outputs decoded from a proof receipt's journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedOutput {
    pub agent_id: String,
    /// Hash of the active policy at time of execution.
    pub policy_hash: String,
    pub output_commitment: String,
    /// True if no policy rule with severity `Block` fired.
    pub all_checks_passed: bool,
    /// True if the action value exceeded the autonomous threshold and requires Ledger approval.
    pub requires_ledger_approval: bool,
    /// Estimated value of the executed action in wei.
    pub action_value: u64,
}
