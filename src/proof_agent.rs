//! Proof of Claw agent runtime — wires the proof_of_claw crate into an
//! async agent loop with state management for the HTTP API.

use anyhow::Result;
use proof_of_claw::{
    AgentConfig, AgentMessage, ExecutionTrace, InferenceRequest, InferenceResponse,
    MessagePayload, MessageType, PolicyConfig, PolicyEngine, PolicyResult, PolicySeverity,
    ProofGenerator, ProofReceipt, ToolInvocation, ZeroGCompute, ZeroGStorage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Shared mutable state for the HTTP API.
#[derive(Debug, Clone)]
pub struct AgentState {
    pub activity: Arc<RwLock<Vec<ActivityEntry>>>,
    pub proofs: Arc<RwLock<Vec<ProofEntry>>>,
    /// Full proof receipts (journal + seal) keyed by proof_id.
    /// Required for browser-side ZK verification.
    pub receipts: Arc<RwLock<std::collections::HashMap<String, ProofReceipt>>>,
    pub messages: Arc<RwLock<Vec<MessageEntry>>>,
    pub stats: Arc<RwLock<AgentStats>>,
    /// Broadcast channel for SSE trace stream.
    /// Broadcasting<TraceEvent> — clone a sender per session.
    pub trace_broadcaster: Arc<tokio::sync::broadcast::Sender<TraceEvent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub id: String,
    pub timestamp: i64,
    pub action: String,
    pub details: String,
    pub within_policy: bool,
    pub tool: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofEntry {
    pub id: String,
    pub timestamp: i64,
    pub session_id: String,
    pub agent_id: String,
    pub policy_hash: String,
    pub all_checks_passed: bool,
    pub requires_ledger: bool,
    pub action_value_wei: u64,
    pub output_commitment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEntry {
    pub id: String,
    pub timestamp: i64,
    pub direction: String, // "inbound" or "outbound"
    pub from: String,
    pub to: String,
    pub content: String,
    pub message_type: String,
}

/// Runtime stats for the /api/status endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentStats {
    pub total_requests: u64,
    pub total_actions: u64,
    pub proofs_generated: u64,
    pub uptime_secs: u64,
    pub start_time: i64,
}

/// Event emitted over SSE to drive the Kanban board in real-time.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "event")]
#[serde(rename_all = "snake_case")]
pub enum TraceEvent {
    /// Fired when a single tool invocation completes (pre-proof).
    ToolInvocation {
        tool_name: String,
        input_hash: String,
        output_hash: String,
        capability_hash: String,
        within_policy: bool,
        timestamp: i64,
    },
    /// Fired when the full execution trace is assembled and proof generation starts.
    TraceComplete {
        session_id: String,
        num_invocations: usize,
    },
    /// Fired when the ZK proof receipt is ready.
    ProofReceipt {
        proof_id: String,
        session_id: String,
        journal_b64: String,
        seal_b64: String,
        image_id: String,
    },
}

impl AgentState {
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(1024);
        Self {
            activity: Arc::new(RwLock::new(Vec::new())),
            proofs: Arc::new(RwLock::new(Vec::new())),
            receipts: Arc::new(RwLock::new(std::collections::HashMap::new())),
            messages: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(AgentStats::default())),
            trace_broadcaster: Arc::new(tx),
        }
    }
}

impl TraceEvent {
    /// Returns the session_id if the event belongs to a session.
    pub fn session_id(&self) -> Option<String> {
        match self {
            TraceEvent::ToolInvocation { .. } => None,
            TraceEvent::TraceComplete { session_id, .. } => Some(session_id.clone()),
            TraceEvent::ProofReceipt { session_id, .. } => Some(session_id.clone()),
        }
    }
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}

/// The Proof of Claw agent — orchestrates inference, policy checks, and proof generation.
#[derive(Debug)]
pub struct ProofOfClawAgent {
    pub id: String,
    pub config: AgentConfig,
    zero_g: ZeroGCompute,
    zero_g_storage: ZeroGStorage,
    policy_engine: PolicyEngine,
    proof_generator: ProofGenerator,
    state: AgentState,
    session_counter: Arc<std::sync::atomic::AtomicU64>,
}

impl ProofOfClawAgent {
    /// Create a new agent from the given config.
    pub async fn new(config: AgentConfig) -> Result<Self> {
        let zero_g = ZeroGCompute::new(&config).await?;
        let zero_g_storage = ZeroGStorage::new(&config).await?;
        let policy_engine = PolicyEngine::new(config.policy.clone());
        let proof_generator = ProofGenerator::new(false, config.risc_zero_image_id.clone().unwrap_or_default());

        let state = AgentState::new();
        // Record start time
        {
            let mut stats = state.stats.write().await;
            stats.start_time = chrono::Utc::now().timestamp();
        }

        Ok(Self {
            id: config.agent_id.clone(),
            config,
            zero_g,
            zero_g_storage,
            policy_engine,
            proof_generator,
            state,
            session_counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        })
    }

    /// Create a new agent in mock mode — uses default config values.
    pub async fn mock() -> Result<Self> {
        Self::new(AgentConfig::mock()?).await
    }

    // ── Public API (called by the HTTP layer) ──────────────────────────────

    /// Handle a chat message end-to-end:
    /// 1. Injection detection
    /// 2. Policy check
    /// 3. 0G Compute inference
    /// 4. Execution trace + proof generation
    /// 5. Record in state
    pub async fn chat(&self, input: ChatInput) -> Result<ChatOutput> {
        let session_id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().timestamp();

        // ── 1. Injection detection ───────────────────────────────────────────
        let injection_check = proof_of_claw::InjectionDetector::new()
            .map(|detector| detector.check(&input.message))
            .unwrap_or(());
        let injection_passed = injection_check.map(|c| c.is_safe()).unwrap_or(true);

        if !injection_passed {
            let entry = ActivityEntry {
                id: Uuid::new_v4().to_string(),
                timestamp,
                action: "injection_check".to_string(),
                details: "Prompt injection detected — request rejected".to_string(),
                within_policy: false,
                tool: None,
            };
            self.state.activity.write().await.push(entry);
            return Err(anyhow::anyhow!("Prompt injection detected — request rejected"));
        }

        // ── 2. Policy check ─────────────────────────────────────────────────
        let agent_msg = AgentMessage {
            message_type: MessageType::Execute,
            payload: MessagePayload {
                action: input.action.clone().unwrap_or_else(|| "chat".to_string()),
                params: serde_json::json!({ "message": input.message }),
                trace_root_hash: None,
                proof_receipt: None,
                required_approval: None,
            },
            nonce: 0,
            timestamp,
        };

        let policy_result = self.policy_engine.check(&agent_msg, &InferenceResponse {
            content: String::new(),
            attestation_signature: String::new(),
            provider: String::new(),
        });

        // Record policy check
        {
            let mut stats = self.state.stats.write().await;
            stats.total_requests += 1;
            stats.total_actions += 1;
        }

        // ── 3. 0G Compute inference ─────────────────────────────────────────
        let inference_resp = self.zero_g.inference(&InferenceRequest {
            system_prompt: input.system_prompt.clone().unwrap_or_else(|| {
                "You are a provable AI agent. Every action you take is logged and cryptographically verifiable.".to_string()
            }),
            user_prompt: input.message.clone(),
            model: input.model.clone(),
        }).await?;

        // ── 4. Build execution trace ─────────────────────────────────────────
        let session_nonce = self.session_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let trace = ExecutionTrace {
            agent_id: self.id.clone(),
            session_id: session_id.clone(),
            timestamp,
            inference_commitment: inference_resp.attestation_signature.clone(),
            tool_invocations: vec![ToolInvocation {
                tool_name: agent_msg.payload.action.clone(),
                input_hash: format!("0x{}", hex::encode(Sha256::digest(&input.message))),
                output_hash: format!("0x{}", hex::encode(Sha256::digest(&inference_resp.content))),
                capability_hash: format!("0x{}", hex::encode(Sha256::digest(agent_msg.payload.action.as_bytes()))),
                timestamp,
                within_policy: policy_result.severity != PolicySeverity::Block,
            }],
            policy_check_results: vec![policy_result.clone()],
            output_commitment: format!("0x{}", hex::encode(Sha256::digest(&inference_resp.content))),
        };

        // ── 5. Emit tool_invocation SSE event ──────────────────────────────
        let _ = self.state.trace_broadcaster.send(TraceEvent::ToolInvocation {
            tool_name: agent_msg.payload.action.clone(),
            input_hash: format!("0x{}", hex::encode(Sha256::digest(&input.message))),
            output_hash: format!("0x{}", hex::encode(Sha256::digest(&inference_resp.content))),
            capability_hash: format!("0x{}", hex::encode(Sha256::digest(agent_msg.payload.action.as_bytes()))),
            within_policy: policy_result.severity != PolicySeverity::Block,
            timestamp,
        });

        // ── 6. Generate proof ───────────────────────────────────────────────
        let proof_receipt = self.proof_generator.generate_proof(&trace).await?;
        let verified = self.proof_generator.verify_receipt(&proof_receipt)?;

        // ── 7. Upload trace to 0G Storage ────────────────────────────────────
        let trace_root = self.zero_g_storage.store_trace(&trace).await?;
        let proof_id = Uuid::new_v4().to_string();

        // Store raw receipt for browser-side ZK verification
        self.state.receipts.write().await.insert(
            proof_id.clone(),
            proof_receipt.clone(),
        );

        // Emit trace_complete + proof_receipt SSE events
        let num_invocations = trace.tool_invocations.len();
        let _ = self.state.trace_broadcaster.send(TraceEvent::TraceComplete {
            session_id: session_id.clone(),
            num_invocations,
        });

        use base64::Engine as _;
        let journal_b64 = base64::engine::general_purpose::STANDARD.encode(&proof_receipt.journal);
        let seal_b64 = base64::engine::general_purpose::STANDARD.encode(&proof_receipt.seal);
        let _ = self.state.trace_broadcaster.send(TraceEvent::ProofReceipt {
            proof_id: proof_id.clone(),
            session_id: session_id.clone(),
            journal_b64,
            seal_b64,
            image_id: proof_receipt.image_id.clone(),
        });

        // ── 8. Record in state ───────────────────────────────────────────────
        let activity_entry = ActivityEntry {
            id: Uuid::new_v4().to_string(),
            timestamp,
            action: agent_msg.payload.action.clone(),
            details: format!(
                "Policy: {:?} | Ledger required: {}",
                policy_result.severity, verified.requires_ledger_approval
            ),
            within_policy: policy_result.severity != PolicySeverity::Block,
            tool: Some(agent_msg.payload.action),
        };
        self.state.activity.write().await.push(activity_entry);

        let proof_entry = ProofEntry {
            id: proof_id.clone(),
            timestamp,
            session_id: session_id.clone(),
            agent_id: self.id.clone(),
            policy_hash: verified.policy_hash,
            all_checks_passed: verified.all_checks_passed,
            requires_ledger: verified.requires_ledger_approval,
            action_value_wei: verified.action_value,
            output_commitment: trace.output_commitment,
        };
        self.state.proofs.write().await.push(proof_entry.clone());

        {
            let mut stats = self.state.stats.write().await;
            stats.proofs_generated += 1;
        }

        Ok(ChatOutput {
            session_id,
            response: inference_resp.content,
            attestation: inference_resp.attestation_signature,
            proof_id,
            trace_root,
            policy_result,
            requires_ledger: verified.requires_ledger_approval,
            session_nonce,
        })
    }

    /// Returns a snapshot of the current API state.
    pub async fn get_state(&self) -> AgentState {
        self.state.clone()
    }

    /// Record an outbound message.
    pub async fn record_message(&self, direction: &str, from: &str, to: &str, content: &str) {
        let entry = MessageEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            direction: direction.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            content: content.to_string(),
            message_type: "chat".to_string(),
        };
        self.state.messages.write().await.push(entry);
    }
}

// ── Request / response types ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatInput {
    pub message: String,
    pub action: Option<String>,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatOutput {
    pub session_id: String,
    pub response: String,
    pub attestation: String,
    pub proof_id: String,
    pub trace_root: String,
    pub policy_result: PolicyResult,
    pub requires_ledger: bool,
    pub session_nonce: u64,
}
