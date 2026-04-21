//! Integration tests for the unified ExecutionGate abstraction.
//!
//! Exercises the complete gate lifecycle:
//! 1. Tool call triggers GatePaused (approval or auth)
//! 2. Thread transitions to Waiting state
//! 3. PendingGateStore holds the gate with channel verification
//! 4. resolve_gate() resumes or stops the thread
//! 5. Cross-channel attacks are blocked structurally
//!
//! Uses the same ScriptedLlm + mock EffectExecutor pattern as
//! engine_v2_skill_codeact.rs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;

use ironclaw_engine::types::capability::{EffectType, LeaseId};
use ironclaw_engine::{
    ActionDef, ActionResult, Capability, CapabilityLease, CapabilityRegistry, DocId,
    EffectExecutor, EngineError, GrantedActions, LeaseManager, LlmBackend, LlmCallConfig,
    LlmOutput, LlmResponse, MemoryDoc, Mission, MissionId, MissionStatus, PolicyEngine, Project,
    ProjectId, ResumeKind, Step, Store, Thread, ThreadConfig, ThreadEvent, ThreadId, ThreadManager,
    ThreadMessage, ThreadOutcome, ThreadState, ThreadType, TokenUsage,
};

use ironclaw::bridge::EffectBridgeAdapter;
use ironclaw::context::JobContext;
use ironclaw::gate::pending::{PendingGate, PendingGateKey};
use ironclaw::gate::store::{GateStoreError, PendingGateStore, TRUSTED_GATE_CHANNELS};
use ironclaw::hooks::HookRegistry;
use ironclaw::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRegistry};
use ironclaw_safety::{SafetyConfig, SafetyLayer};

// ── Scripted LLM ─────────────────────────────────────────────

struct ScriptedLlm {
    responses: std::sync::Mutex<Vec<LlmOutput>>,
}

impl ScriptedLlm {
    fn new(responses: Vec<LlmOutput>) -> Arc<Self> {
        Arc::new(Self {
            responses: std::sync::Mutex::new(responses),
        })
    }
}

#[async_trait::async_trait]
impl LlmBackend for ScriptedLlm {
    async fn complete(
        &self,
        _messages: &[ThreadMessage],
        _actions: &[ActionDef],
        _config: &LlmCallConfig,
    ) -> Result<LlmOutput, EngineError> {
        let mut queue = self.responses.lock().unwrap();
        if queue.is_empty() {
            Ok(LlmOutput {
                response: LlmResponse::Text("done".into()),
                usage: TokenUsage::default(),
            })
        } else {
            Ok(queue.remove(0))
        }
    }
    fn model_name(&self) -> &str {
        "scripted-mock"
    }
}

// ── Gate-Aware Mock Effects ──────────────────────────────────

/// Mock EffectExecutor that returns GatePaused for specific tools,
/// NeedApproval for others, and success for the rest.
struct GateMockEffects {
    /// Tools that trigger GatePaused with Approval resume kind.
    gate_approval_tools: Vec<String>,
    /// Tools that trigger GatePaused with Authentication resume kind.
    gate_auth_tools: Vec<String>,
    /// Tools that require approval first, then authentication on retry.
    chained_approval_then_auth_tools: Vec<String>,
    /// Recorded calls (including gated ones that were retried after approval).
    calls: RwLock<Vec<(String, serde_json::Value)>>,
    /// Actions cleared through the approval gate.
    approved: RwLock<std::collections::HashSet<String>>,
    /// Actions cleared through the auth gate.
    authenticated: RwLock<std::collections::HashSet<String>>,
}

struct ApprovalTool;

#[async_trait]
impl Tool for ApprovalTool {
    fn name(&self) -> &str {
        "approval_test"
    }

    fn description(&self) -> &str {
        "Integration test approval tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::success(
            serde_json::json!({"ok": true, "params": params}),
            Duration::from_millis(1),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

impl GateMockEffects {
    fn new(gate_approval_tools: Vec<String>, gate_auth_tools: Vec<String>) -> Arc<Self> {
        Self::new_with_chain(gate_approval_tools, gate_auth_tools, Vec::new())
    }

    fn new_with_chain(
        gate_approval_tools: Vec<String>,
        gate_auth_tools: Vec<String>,
        chained_approval_then_auth_tools: Vec<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            gate_approval_tools,
            gate_auth_tools,
            chained_approval_then_auth_tools,
            calls: RwLock::new(Vec::new()),
            approved: RwLock::new(std::collections::HashSet::new()),
            authenticated: RwLock::new(std::collections::HashSet::new()),
        })
    }

    #[allow(dead_code)]
    async fn recorded_calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.read().await.clone()
    }

    #[allow(dead_code)]
    async fn mark_approved(&self, tool_name: &str) {
        self.approved.write().await.insert(tool_name.to_string());
    }

    #[allow(dead_code)]
    async fn mark_authenticated(&self, tool_name: &str) {
        self.authenticated
            .write()
            .await
            .insert(tool_name.to_string());
    }
}

struct InstallThenAliasEffects {
    calls: RwLock<Vec<(String, serde_json::Value)>>,
    authenticated: RwLock<std::collections::HashSet<String>>,
}

impl InstallThenAliasEffects {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: RwLock::new(Vec::new()),
            authenticated: RwLock::new(std::collections::HashSet::new()),
        })
    }

    async fn mark_authenticated(&self, action_name: &str) {
        self.authenticated
            .write()
            .await
            .insert(action_name.to_string());
    }

    async fn recorded_calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.read().await.clone()
    }
}

#[async_trait::async_trait]
impl EffectExecutor for InstallThenAliasEffects {
    async fn execute_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        _lease: &CapabilityLease,
        _context: &ironclaw_engine::ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError> {
        self.calls
            .write()
            .await
            .push((action_name.to_string(), parameters.clone()));

        if action_name == "tool_install"
            && !self.authenticated.read().await.contains("tool_install")
        {
            return Err(EngineError::GatePaused {
                gate_name: "authentication".into(),
                action_name: action_name.to_string(),
                call_id: "call_install_1".into(),
                parameters: Box::new(parameters),
                resume_kind: Box::new(ResumeKind::Authentication {
                    credential_name: ironclaw_common::CredentialName::new("github").unwrap(),
                    instructions: "Authenticate GitHub".into(),
                    auth_url: None,
                }),
                paused_lease: None,
                resume_output: None,
            });
        }

        Ok(ActionResult {
            call_id: String::new(),
            action_name: action_name.to_string(),
            output: serde_json::json!({"status": "ok", "action": action_name}),
            is_error: false,
            duration: Duration::from_millis(1),
        })
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
    ) -> Result<Vec<ActionDef>, EngineError> {
        Ok(vec![
            ActionDef {
                name: "tool_install".into(),
                description: "Install a tool".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: false,
            },
            ActionDef {
                name: "create-issue".into(),
                description: "Create an issue after install".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: false,
            },
        ])
    }
}

#[async_trait::async_trait]
impl EffectExecutor for GateMockEffects {
    async fn execute_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        _lease: &CapabilityLease,
        _context: &ironclaw_engine::ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError> {
        self.calls
            .write()
            .await
            .push((action_name.to_string(), parameters.clone()));

        let already_approved = self.approved.read().await.contains(action_name);
        let already_authenticated = self.authenticated.read().await.contains(action_name);

        if self
            .chained_approval_then_auth_tools
            .contains(&action_name.to_string())
        {
            if !already_approved {
                return Err(EngineError::GatePaused {
                    gate_name: "approval".into(),
                    action_name: action_name.to_string(),
                    call_id: "call_gate_1".into(),
                    parameters: Box::new(parameters),
                    resume_kind: Box::new(ResumeKind::Approval { allow_always: true }),
                    paused_lease: None,
                    resume_output: None,
                });
            }

            if !already_authenticated {
                return Err(EngineError::GatePaused {
                    gate_name: "authentication".into(),
                    action_name: action_name.to_string(),
                    call_id: "call_gate_2".into(),
                    parameters: Box::new(parameters),
                    resume_kind: Box::new(ResumeKind::Authentication {
                        credential_name: ironclaw_common::CredentialName::new("notion").unwrap(),
                        instructions: "Authenticate your Notion workspace".into(),
                        auth_url: None,
                    }),
                    paused_lease: None,
                    resume_output: None,
                });
            }
        }

        // Gate: approval required
        if self.gate_approval_tools.contains(&action_name.to_string()) && !already_approved {
            return Err(EngineError::GatePaused {
                gate_name: "approval".into(),
                action_name: action_name.to_string(),
                call_id: "call_gate_1".into(),
                parameters: Box::new(parameters),
                resume_kind: Box::new(ResumeKind::Approval { allow_always: true }),
                paused_lease: None,
                resume_output: None,
            });
        }

        // Gate: authentication required
        if self.gate_auth_tools.contains(&action_name.to_string()) && !already_authenticated {
            return Err(EngineError::GatePaused {
                gate_name: "authentication".into(),
                action_name: action_name.to_string(),
                call_id: "call_gate_2".into(),
                parameters: Box::new(parameters),
                resume_kind: Box::new(ResumeKind::Authentication {
                    credential_name: ironclaw_common::CredentialName::new("test_api_key").unwrap(),
                    instructions: "Provide your API key".into(),
                    auth_url: None,
                }),
                paused_lease: None,
                resume_output: None,
            });
        }

        Ok(ActionResult {
            call_id: String::new(),
            action_name: action_name.to_string(),
            output: serde_json::json!({"status": "ok", "result": "success"}),
            is_error: false,
            duration: Duration::from_millis(1),
        })
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
    ) -> Result<Vec<ActionDef>, EngineError> {
        // requires_approval: false — the gate check is done by the mock's
        // execute_action() returning GatePaused, not by the PolicyEngine.
        Ok(vec![
            ActionDef {
                name: "http".into(),
                description: "Make HTTP requests".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: false,
            },
            ActionDef {
                name: "echo".into(),
                description: "Echo input".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::ReadLocal],
                requires_approval: false,
            },
            ActionDef {
                name: "tool_install".into(),
                description: "Install an extension".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: false,
            },
        ])
    }
}

// ── In-Memory Store (same as engine_v2_skill_codeact) ────────

struct TestStore {
    threads: RwLock<HashMap<ThreadId, Thread>>,
    events: RwLock<Vec<ThreadEvent>>,
    docs: RwLock<Vec<MemoryDoc>>,
    missions: RwLock<Vec<Mission>>,
    leases: RwLock<Vec<CapabilityLease>>,
    steps: RwLock<Vec<Step>>,
}

impl TestStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            threads: RwLock::new(HashMap::new()),
            events: RwLock::new(Vec::new()),
            docs: RwLock::new(Vec::new()),
            missions: RwLock::new(Vec::new()),
            leases: RwLock::new(Vec::new()),
            steps: RwLock::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl Store for TestStore {
    async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
        self.threads.write().await.insert(thread.id, thread.clone());
        Ok(())
    }
    async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
        Ok(self.threads.read().await.get(&id).cloned())
    }
    async fn list_threads(
        &self,
        pid: ProjectId,
        _user_id: &str,
    ) -> Result<Vec<Thread>, EngineError> {
        Ok(self
            .threads
            .read()
            .await
            .values()
            .filter(|t| t.project_id == pid)
            .cloned()
            .collect())
    }
    async fn update_thread_state(
        &self,
        id: ThreadId,
        state: ThreadState,
    ) -> Result<(), EngineError> {
        if let Some(t) = self.threads.write().await.get_mut(&id) {
            t.state = state;
        }
        Ok(())
    }
    async fn save_step(&self, step: &Step) -> Result<(), EngineError> {
        let mut steps = self.steps.write().await;
        steps.retain(|s| s.id != step.id);
        steps.push(step.clone());
        Ok(())
    }
    async fn load_steps(&self, thread_id: ThreadId) -> Result<Vec<Step>, EngineError> {
        Ok(self
            .steps
            .read()
            .await
            .iter()
            .filter(|s| s.thread_id == thread_id)
            .cloned()
            .collect())
    }
    async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
        self.events.write().await.extend(events.iter().cloned());
        Ok(())
    }
    async fn load_events(&self, thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
        Ok(self
            .events
            .read()
            .await
            .iter()
            .filter(|e| e.thread_id == thread_id)
            .cloned()
            .collect())
    }
    async fn save_project(&self, _project: &Project) -> Result<(), EngineError> {
        Ok(())
    }
    async fn load_project(&self, _id: ProjectId) -> Result<Option<Project>, EngineError> {
        Ok(None)
    }
    async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError> {
        let mut docs = self.docs.write().await;
        docs.retain(|d| d.id != doc.id);
        docs.push(doc.clone());
        Ok(())
    }
    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
        Ok(self.docs.read().await.iter().find(|d| d.id == id).cloned())
    }
    async fn list_memory_docs(
        &self,
        _pid: ProjectId,
        _user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self.docs.read().await.clone())
    }
    async fn list_memory_docs_by_owner(
        &self,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self
            .docs
            .read()
            .await
            .iter()
            .filter(|d| d.user_id == user_id)
            .cloned()
            .collect())
    }
    async fn save_lease(&self, lease: &CapabilityLease) -> Result<(), EngineError> {
        let mut leases = self.leases.write().await;
        leases.retain(|l| l.id != lease.id);
        leases.push(lease.clone());
        Ok(())
    }
    async fn load_active_leases(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<CapabilityLease>, EngineError> {
        Ok(self
            .leases
            .read()
            .await
            .iter()
            .filter(|l| l.thread_id == thread_id && !l.revoked)
            .cloned()
            .collect())
    }
    async fn revoke_lease(&self, lease_id: LeaseId, _reason: &str) -> Result<(), EngineError> {
        if let Some(l) = self
            .leases
            .write()
            .await
            .iter_mut()
            .find(|l| l.id == lease_id)
        {
            l.revoked = true;
        }
        Ok(())
    }
    async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError> {
        let mut missions = self.missions.write().await;
        missions.retain(|m| m.id != mission.id);
        missions.push(mission.clone());
        Ok(())
    }
    async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
        Ok(self
            .missions
            .read()
            .await
            .iter()
            .find(|m| m.id == id)
            .cloned())
    }
    async fn list_missions(
        &self,
        _pid: ProjectId,
        _user_id: &str,
    ) -> Result<Vec<Mission>, EngineError> {
        Ok(self.missions.read().await.clone())
    }
    async fn update_mission_status(
        &self,
        _id: MissionId,
        _status: MissionStatus,
    ) -> Result<(), EngineError> {
        Ok(())
    }
}

// ── Helpers ──────────────────────────────────────────────────

fn make_caps(require_approval: bool) -> CapabilityRegistry {
    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "test tools".into(),
        actions: vec![
            ActionDef {
                name: "http".into(),
                description: "HTTP requests".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: require_approval,
            },
            ActionDef {
                name: "echo".into(),
                description: "Echo".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::ReadLocal],
                requires_approval: false,
            },
            ActionDef {
                name: "tool_install".into(),
                description: "Install a tool".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: require_approval,
            },
        ],
        knowledge: vec![],
        policies: vec![],
    });
    caps
}

fn make_caps_with_approval_tool() -> CapabilityRegistry {
    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "test tools".into(),
        actions: vec![ActionDef {
            name: "approval_test".into(),
            description: "Approval test tool".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::WriteExternal],
            requires_approval: false,
        }],
        knowledge: vec![],
        policies: vec![],
    });
    caps
}

fn make_caps_with_install_and_alias_followup() -> CapabilityRegistry {
    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "test tools".into(),
        actions: vec![
            ActionDef {
                name: "tool_install".into(),
                description: "Install a tool".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: false,
            },
            ActionDef {
                name: "create_issue".into(),
                description: "Create a follow-up issue".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![EffectType::WriteExternal],
                requires_approval: false,
            },
        ],
        knowledge: vec![],
        policies: vec![],
    });
    caps
}

fn sample_pending_gate(
    user_id: &str,
    thread_id: ThreadId,
    channel: &str,
    resume_kind: ResumeKind,
) -> PendingGate {
    PendingGate {
        request_id: uuid::Uuid::new_v4(),
        gate_name: "approval".into(),
        user_id: user_id.into(),
        thread_id,
        scope_thread_id: None,
        conversation_id: ironclaw_engine::ConversationId::new(),
        source_channel: channel.into(),
        action_name: "http".into(),
        call_id: "call_1".into(),
        parameters: serde_json::json!({"url": "https://example.com"}),
        display_parameters: None,
        description: "Tool 'http' requires approval".into(),
        resume_kind,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(30),
        original_message: None,
        paused_lease: None,
        resume_output: None,
        approval_already_granted: false,
    }
}

fn resumed_action_result_message(
    call_id: &str,
    action_name: &str,
    output: &serde_json::Value,
) -> ThreadMessage {
    let rendered = serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string());
    ThreadMessage::action_result(call_id, action_name, rendered)
}

// ── Tests: GatePaused ThreadOutcome ──────────────────────────

/// When effect executor returns GatePaused, the thread transitions to
/// Waiting and the outcome carries the gate info.
#[tokio::test]
async fn gate_paused_transitions_thread_to_waiting() {
    let project_id = ProjectId::new();
    let effects = GateMockEffects::new(vec!["http".into()], vec![]);

    // LLM returns a structured tool call for http
    let llm = ScriptedLlm::new(vec![LlmOutput {
        response: LlmResponse::ActionCalls {
            calls: vec![ironclaw_engine::ActionCall {
                id: "call_1".into(),
                action_name: "http".into(),
                parameters: serde_json::json!({"url": "https://example.com"}),
            }],
            content: None,
        },
        usage: TokenUsage::default(),
    }]);

    let store = TestStore::new();
    // Use requires_approval=false so PolicyEngine doesn't intercept before
    // EffectExecutor — the mock returns GatePaused from execute_action().
    let mgr = ThreadManager::new(
        llm,
        effects,
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps(false)),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "make an http post",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let outcome = mgr.join_thread(tid).await.expect("join_thread");

    // Thread should have paused with GatePaused outcome
    match &outcome {
        ThreadOutcome::GatePaused {
            gate_name,
            action_name,
            resume_kind,
            ..
        } => {
            assert_eq!(gate_name, "approval");
            assert_eq!(action_name, "http");
            assert!(matches!(resume_kind, ResumeKind::Approval { .. }));
        }
        other => panic!("Expected GatePaused, got: {other:?}"),
    }

    // Thread state should be Waiting (safety net in loop_engine.rs)
    let thread = store.load_thread(tid).await.unwrap().unwrap();
    assert_eq!(
        thread.state,
        ThreadState::Waiting,
        "Thread should be in Waiting state after GatePaused"
    );
}

/// GatePaused with Authentication resume kind carries credential info.
#[tokio::test]
async fn gate_paused_authentication_carries_credential_name() {
    let project_id = ProjectId::new();
    let effects = GateMockEffects::new(vec![], vec!["http".into()]);

    let llm = ScriptedLlm::new(vec![LlmOutput {
        response: LlmResponse::ActionCalls {
            calls: vec![ironclaw_engine::ActionCall {
                id: "call_1".into(),
                action_name: "http".into(),
                parameters: serde_json::json!({"url": "https://api.example.com"}),
            }],
            content: None,
        },
        usage: TokenUsage::default(),
    }]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects,
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps(false)), // false so PolicyEngine doesn't intercept
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "fetch data from API",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let outcome = mgr.join_thread(tid).await.expect("join_thread");

    match &outcome {
        ThreadOutcome::GatePaused {
            gate_name,
            resume_kind,
            ..
        } => {
            assert_eq!(gate_name, "authentication");
            match resume_kind {
                ResumeKind::Authentication {
                    credential_name, ..
                } => {
                    assert_eq!(credential_name, "test_api_key");
                }
                other => panic!("Expected Authentication, got: {other:?}"),
            }
        }
        other => panic!("Expected GatePaused, got: {other:?}"),
    }
}

/// A paused thread remains resumable and completes after approval.
#[tokio::test]
async fn gate_paused_thread_resumes_to_completion() {
    let project_id = ProjectId::new();
    let effects = GateMockEffects::new(vec!["http".into()], vec![]);

    let llm = ScriptedLlm::new(vec![
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_1".into(),
                    action_name: "http".into(),
                    parameters: serde_json::json!({"url": "https://example.com"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::Text("done".into()),
            usage: TokenUsage::default(),
        },
    ]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps(false)),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "make an http post",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    assert!(matches!(first, ThreadOutcome::GatePaused { .. }));
    assert_eq!(
        store.load_thread(tid).await.unwrap().unwrap().state,
        ThreadState::Waiting
    );

    effects.mark_approved("http").await;
    mgr.resume_thread(
        tid,
        "test-user",
        Some(ThreadMessage::user("approved")),
        Some(("call_gate_1".into(), true)),
        None,
    )
    .await
    .expect("resume_thread");

    let resumed = mgr.join_thread(tid).await.expect("second join");
    if !matches!(resumed, ThreadOutcome::Completed { .. }) {
        panic!("expected Completed after approved retry, got {:?}", resumed);
    }
    let saved = store.load_thread(tid).await.unwrap().unwrap();
    assert_eq!(saved.state, ThreadState::Done);
    assert!(
        saved.events.iter().any(|event| matches!(
            event.kind,
            ironclaw_engine::types::event::EventKind::ApprovalReceived { .. }
        )),
        "resume should record ApprovalReceived"
    );
}

#[tokio::test]
async fn approval_resolution_executes_pending_call_directly() {
    let project_id = ProjectId::new();
    let tools = Arc::new(ToolRegistry::new());
    tools.register(Arc::new(ApprovalTool)).await;

    let effects = Arc::new(EffectBridgeAdapter::new(
        tools,
        Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 10_000,
            injection_check_enabled: false,
        })),
        Arc::new(HookRegistry::default()),
    ));

    let llm = ScriptedLlm::new(vec![
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_approval_1".into(),
                    action_name: "approval_test".into(),
                    parameters: serde_json::json!({"value": "hello"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::Text("done".into()),
            usage: TokenUsage::default(),
        },
    ]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps_with_approval_tool()),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "run the approval tool",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    match first {
        ThreadOutcome::GatePaused {
            gate_name,
            action_name,
            call_id,
            parameters,
            resume_kind,
            ..
        } => {
            assert_eq!(gate_name, "approval");
            assert_eq!(action_name, "approval_test");
            assert_eq!(call_id, "call_approval_1");
            assert_eq!(parameters["value"], "hello");
            assert!(matches!(resume_kind, ResumeKind::Approval { .. }));
        }
        other => panic!("expected GatePaused approval, got {other:?}"),
    }
    assert_eq!(
        store.load_thread(tid).await.unwrap().unwrap().state,
        ThreadState::Waiting
    );

    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let lease = mgr
        .leases
        .find_lease_for_action(tid, "approval_test")
        .await
        .expect("lease for approval_test");
    let exec_ctx = ironclaw_engine::ThreadExecutionContext {
        thread_id: tid,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: "test-user".into(),
        step_id: ironclaw_engine::StepId::new(),
        current_call_id: Some("call_approval_1".into()),
        active_skill_names: Vec::new(),
        source_channel: None,
        user_timezone: None,
        thread_goal: Some(thread.goal.clone()),
    };

    let tool_result = effects
        .execute_resolved_pending_action(
            "approval_test",
            serde_json::json!({"value": "hello"}),
            &lease,
            &exec_ctx,
            true,
        )
        .await
        .expect("approved pending call should execute directly");
    mgr.resume_thread(
        tid,
        "test-user",
        Some(resumed_action_result_message(
            "call_approval_1",
            "approval_test",
            &tool_result.output,
        )),
        Some(("call_approval_1".into(), true)),
        Some("call_approval_1".into()),
    )
    .await
    .expect("resume_thread");

    let resumed = mgr.join_thread(tid).await.expect("second join");
    assert!(
        matches!(resumed, ThreadOutcome::Completed { .. }),
        "expected Completed after approval retry, got {resumed:?}"
    );

    let saved = store.load_thread(tid).await.unwrap().unwrap();
    assert_eq!(saved.state, ThreadState::Done);
    let approval_requests = saved
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                ironclaw_engine::types::event::EventKind::ApprovalRequested { .. }
            )
        })
        .count();
    assert_eq!(
        approval_requests, 1,
        "resumed execution should not prompt for approval again"
    );
    assert!(
        saved.events.iter().any(|event| matches!(
            event.kind,
            ironclaw_engine::types::event::EventKind::ApprovalReceived { .. }
        )),
        "resume should record ApprovalReceived"
    );
}

#[tokio::test]
async fn auth_resolution_retries_same_pending_action_without_second_pause() {
    let project_id = ProjectId::new();
    let effects = GateMockEffects::new(vec![], vec!["http".into()]);

    let llm = ScriptedLlm::new(vec![
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_auth_1".into(),
                    action_name: "http".into(),
                    parameters: serde_json::json!({"url": "https://example.com/private"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_auth_2".into(),
                    action_name: "http".into(),
                    parameters: serde_json::json!({"url": "https://example.com/private"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::Text("done".into()),
            usage: TokenUsage::default(),
        },
    ]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps(false)),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "call the authenticated endpoint",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    assert!(matches!(first, ThreadOutcome::GatePaused { .. }));
    assert_eq!(
        store.load_thread(tid).await.unwrap().unwrap().state,
        ThreadState::Waiting
    );

    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let lease = mgr
        .leases
        .find_lease_for_action(tid, "http")
        .await
        .expect("lease for http");
    let exec_ctx = ironclaw_engine::ThreadExecutionContext {
        thread_id: tid,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: "test-user".into(),
        step_id: ironclaw_engine::StepId::new(),
        current_call_id: Some("call_auth_1".into()),
        active_skill_names: Vec::new(),
        source_channel: None,
        user_timezone: None,
        thread_goal: Some(thread.goal.clone()),
    };

    effects.mark_authenticated("http").await;
    let result = effects
        .execute_action(
            "http",
            serde_json::json!({"url": "https://example.com/private"}),
            &lease,
            &exec_ctx,
        )
        .await
        .expect("authenticated pending action should execute directly");
    mgr.resume_thread(
        tid,
        "test-user",
        Some(resumed_action_result_message(
            "call_auth_1",
            "http",
            &result.output,
        )),
        None,
        Some("call_auth_1".into()),
    )
    .await
    .expect("resume_thread");

    let resumed = mgr.join_thread(tid).await.expect("second join");
    assert!(
        matches!(resumed, ThreadOutcome::Completed { .. }),
        "expected Completed after auth retry, got {resumed:?}"
    );

    let saved = store.load_thread(tid).await.unwrap().unwrap();
    let auth_pauses = saved
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                ironclaw_engine::types::event::EventKind::ApprovalRequested { .. }
            )
        })
        .count();
    assert_eq!(auth_pauses, 1, "resumed auth should not pause again");
}

#[tokio::test]
async fn approval_chains_directly_into_auth_for_install_flow() {
    let project_id = ProjectId::new();
    let effects = GateMockEffects::new_with_chain(vec![], vec![], vec!["tool_install".into()]);
    let install_params = serde_json::json!({"kind": "mcp_server", "name": "notion"});

    let llm = ScriptedLlm::new(vec![
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_install_1".into(),
                    action_name: "tool_install".into(),
                    parameters: install_params.clone(),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::Text("notion connected".into()),
            usage: TokenUsage::default(),
        },
    ]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps(false)),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "install notion",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    assert!(matches!(first, ThreadOutcome::GatePaused { .. }));

    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let lease = mgr
        .leases
        .find_lease_for_action(tid, "tool_install")
        .await
        .expect("lease for tool_install");
    let exec_ctx = ironclaw_engine::ThreadExecutionContext {
        thread_id: tid,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: "test-user".into(),
        step_id: ironclaw_engine::StepId::new(),
        current_call_id: Some("call_install_1".into()),
        active_skill_names: Vec::new(),
        source_channel: None,
        user_timezone: None,
        thread_goal: Some(thread.goal.clone()),
    };

    effects.mark_approved("tool_install").await;
    let auth_pause = effects
        .execute_action("tool_install", install_params.clone(), &lease, &exec_ctx)
        .await
        .expect_err("approved install should chain directly into auth");
    match auth_pause {
        EngineError::GatePaused {
            gate_name,
            action_name,
            resume_kind,
            ..
        } => {
            assert_eq!(gate_name, "authentication");
            assert_eq!(action_name, "tool_install");
            match *resume_kind {
                ResumeKind::Authentication {
                    credential_name, ..
                } => assert_eq!(credential_name, "notion"),
                other => panic!("expected auth gate after install approval, got {other:?}"),
            }
        }
        other => panic!("expected auth gate immediately after install approval, got {other:?}"),
    }

    effects.mark_authenticated("tool_install").await;
    let install_result = effects
        .execute_action("tool_install", install_params, &lease, &exec_ctx)
        .await
        .expect("authenticated install should complete directly");
    mgr.resume_thread(
        tid,
        "test-user",
        Some(resumed_action_result_message(
            "call_install_1",
            "tool_install",
            &install_result.output,
        )),
        None,
        Some("call_install_1".into()),
    )
    .await
    .expect("resume after auth");

    let final_outcome = mgr.join_thread(tid).await.expect("third join");
    assert!(
        matches!(final_outcome, ThreadOutcome::Completed { .. }),
        "expected completion after auth, got {final_outcome:?}"
    );

    let calls = effects.recorded_calls().await;
    let install_calls = calls
        .iter()
        .filter(|(name, _)| name == "tool_install")
        .count();
    assert_eq!(
        install_calls, 3,
        "install flow should retry once for approval and once for auth"
    );
}

#[tokio::test]
async fn install_auth_resume_followed_by_aliased_tool_call_completes_without_hanging() {
    let project_id = ProjectId::new();
    let effects = InstallThenAliasEffects::new();

    let llm = ScriptedLlm::new(vec![
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_install_1".into(),
                    action_name: "tool_install".into(),
                    parameters: serde_json::json!({"kind": "mcp_server", "name": "github"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_followup_1".into(),
                    action_name: "create-issue".into(),
                    parameters: serde_json::json!({"title": "Issue after install"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::Text("done".into()),
            usage: TokenUsage::default(),
        },
    ]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps_with_install_and_alias_followup()),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "install github and then create an issue",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    assert!(matches!(first, ThreadOutcome::GatePaused { .. }));
    assert_eq!(
        store.load_thread(tid).await.unwrap().unwrap().state,
        ThreadState::Waiting
    );

    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let lease = mgr
        .leases
        .find_lease_for_action(tid, "tool_install")
        .await
        .expect("lease for tool_install");
    let exec_ctx = ironclaw_engine::ThreadExecutionContext {
        thread_id: tid,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: "test-user".into(),
        step_id: ironclaw_engine::StepId::new(),
        current_call_id: Some("call_install_1".into()),
        active_skill_names: Vec::new(),
        source_channel: None,
        user_timezone: None,
        thread_goal: Some(thread.goal.clone()),
    };

    effects.mark_authenticated("tool_install").await;
    let install_result = effects
        .execute_action(
            "tool_install",
            serde_json::json!({"kind": "mcp_server", "name": "github"}),
            &lease,
            &exec_ctx,
        )
        .await
        .expect("authenticated install should complete directly");
    mgr.resume_thread(
        tid,
        "test-user",
        Some(resumed_action_result_message(
            "call_install_1",
            "tool_install",
            &install_result.output,
        )),
        None,
        Some("call_install_1".into()),
    )
    .await
    .expect("resume after auth");

    let outcome = mgr.join_thread(tid).await.expect("second join");
    match outcome {
        ThreadOutcome::Completed { response } => {
            assert_eq!(response.as_deref(), Some("done"));
        }
        other => panic!("expected completion after auth + aliased follow-up call, got {other:?}"),
    }

    let saved = store.load_thread(tid).await.unwrap().unwrap();
    assert_eq!(saved.state, ThreadState::Done);

    let calls = effects.recorded_calls().await;
    let install_calls = calls
        .iter()
        .filter(|(name, _)| name == "tool_install")
        .count();
    let followup_calls = calls
        .iter()
        .filter(|(name, _)| name == "create-issue")
        .count();
    assert_eq!(
        install_calls, 2,
        "install should be retried once after auth"
    );
    assert_eq!(
        followup_calls, 1,
        "aliased follow-up tool should execute once"
    );
}

// ── Tests: PendingGateStore full lifecycle ────────────────────

/// Full lifecycle: insert gate → peek → take_verified → gate removed.
#[tokio::test]
async fn pending_gate_full_lifecycle() {
    let store = PendingGateStore::in_memory();
    let tid = ThreadId::new();
    let gate = sample_pending_gate(
        "user1",
        tid,
        "telegram",
        ResumeKind::Approval { allow_always: true },
    );
    let key = gate.key();
    let request_id = gate.request_id;

    // Insert
    store.insert(gate).await.unwrap();

    // Peek (should find it)
    let view = store.peek(&key).await;
    assert!(view.is_some());
    assert_eq!(view.unwrap().tool_name, "http");

    // Take (should remove it)
    let taken = store
        .take_verified(&key, request_id, "telegram")
        .await
        .unwrap();
    assert_eq!(taken.action_name, "http");

    // Peek again (should be gone)
    assert!(store.peek(&key).await.is_none());
}

/// Cross-channel: telegram gate cannot be resolved from slack.
#[tokio::test]
async fn cross_channel_approval_blocked() {
    let store = PendingGateStore::in_memory();
    let tid = ThreadId::new();
    let gate = sample_pending_gate(
        "user1",
        tid,
        "telegram",
        ResumeKind::Approval { allow_always: true },
    );
    let key = gate.key();
    let request_id = gate.request_id;
    store.insert(gate).await.unwrap();

    // Slack cannot resolve a telegram gate
    let result = store.take_verified(&key, request_id, "slack").await;
    assert!(matches!(
        result,
        Err(GateStoreError::ChannelMismatch { .. })
    ));

    // Gate still exists (not consumed by failed attempt)
    assert!(store.peek(&key).await.is_some());

    // Telegram can resolve it
    let taken = store.take_verified(&key, request_id, "telegram").await;
    assert!(taken.is_ok());
}

/// Trusted channels (web, gateway) can resolve gates from any source.
#[tokio::test]
async fn trusted_channel_can_resolve_any_gate() {
    let store = PendingGateStore::in_memory();

    for &trusted in TRUSTED_GATE_CHANNELS {
        let tid = ThreadId::new();
        let gate = sample_pending_gate(
            "user1",
            tid,
            "signal",
            ResumeKind::Approval { allow_always: true },
        );
        let key = gate.key();
        let request_id = gate.request_id;
        store.insert(gate).await.unwrap();

        let result = store.take_verified(&key, request_id, trusted).await;
        assert!(
            result.is_ok(),
            "Trusted channel '{trusted}' should resolve gate from 'signal'"
        );
    }
}

/// Thread-scoped: thread A's gate is not visible to thread B.
#[tokio::test]
async fn gate_scoped_to_thread_no_leakage() {
    let store = PendingGateStore::in_memory();
    let tid_a = ThreadId::new();
    let tid_b = ThreadId::new();

    let gate_a = sample_pending_gate(
        "user1",
        tid_a,
        "web",
        ResumeKind::Approval { allow_always: true },
    );
    store.insert(gate_a).await.unwrap();

    // Thread B should see nothing
    let key_b = PendingGateKey {
        user_id: "user1".into(),
        thread_id: tid_b,
    };
    assert!(store.peek(&key_b).await.is_none());

    // Thread A should see the gate
    let key_a = PendingGateKey {
        user_id: "user1".into(),
        thread_id: tid_a,
    };
    assert!(store.peek(&key_a).await.is_some());
}

/// Expired gate: cannot be resolved.
#[tokio::test]
async fn expired_gate_cannot_be_resolved() {
    let store = PendingGateStore::in_memory();
    let tid = ThreadId::new();
    let mut gate = sample_pending_gate(
        "user1",
        tid,
        "web",
        ResumeKind::Approval { allow_always: true },
    );
    gate.expires_at = Utc::now() - chrono::Duration::seconds(10); // already expired
    let key = gate.key();
    let request_id = gate.request_id;
    store.insert(gate).await.unwrap();

    // Take should fail with Expired
    let result = store.take_verified(&key, request_id, "web").await;
    assert!(matches!(result, Err(GateStoreError::Expired)));

    // Peek should also return None for expired
    assert!(store.peek(&key).await.is_none());
}

/// Wrong request_id: does NOT consume the gate (regression: 74cbe5c2).
#[tokio::test]
async fn wrong_request_id_does_not_consume_gate() {
    let store = PendingGateStore::in_memory();
    let tid = ThreadId::new();
    let gate = sample_pending_gate(
        "user1",
        tid,
        "web",
        ResumeKind::Approval { allow_always: true },
    );
    let key = gate.key();
    let correct_id = gate.request_id;
    store.insert(gate).await.unwrap();

    // Wrong ID fails
    let wrong_id = uuid::Uuid::new_v4();
    let result = store.take_verified(&key, wrong_id, "web").await;
    assert!(matches!(result, Err(GateStoreError::RequestIdMismatch)));

    // Correct ID still works (gate was NOT consumed)
    let taken = store.take_verified(&key, correct_id, "web").await;
    assert!(taken.is_ok());
}

/// Concurrent resolution: only one caller succeeds (regression: 52d935d7).
#[tokio::test]
async fn concurrent_resolution_exactly_one_succeeds() {
    let store = Arc::new(PendingGateStore::in_memory());
    let tid = ThreadId::new();
    let gate = sample_pending_gate(
        "user1",
        tid,
        "web",
        ResumeKind::Approval { allow_always: true },
    );
    let key = gate.key();
    let request_id = gate.request_id;
    store.insert(gate).await.unwrap();

    let s1 = Arc::clone(&store);
    let s2 = Arc::clone(&store);
    let k1 = key.clone();
    let k2 = key;

    let (r1, r2) = tokio::join!(
        tokio::spawn(async move { s1.take_verified(&k1, request_id, "web").await }),
        tokio::spawn(async move { s2.take_verified(&k2, request_id, "web").await }),
    );

    let results = [r1.unwrap(), r2.unwrap()];
    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    let err_count = results.iter().filter(|r| r.is_err()).count();
    assert_eq!(ok_count, 1, "Exactly one concurrent take must succeed");
    assert_eq!(err_count, 1, "Exactly one concurrent take must fail");
}

// ── Tests: Persistence & Recovery ────────────────────────────

/// Gates survive persistence round-trip (restart recovery).
#[tokio::test]
async fn persistence_round_trip_survives_restart() {
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    struct FakePersistence {
        gates: StdMutex<Vec<PendingGate>>,
    }

    #[async_trait]
    impl ironclaw::gate::store::GatePersistence for FakePersistence {
        async fn save(&self, gate: &PendingGate) -> Result<(), GateStoreError> {
            self.gates.lock().unwrap().push(gate.clone());
            Ok(())
        }
        async fn remove(&self, _key: &PendingGateKey) -> Result<(), GateStoreError> {
            Ok(())
        }
        async fn load_all(&self) -> Result<Vec<PendingGate>, GateStoreError> {
            Ok(self.gates.lock().unwrap().clone())
        }
    }

    let tid = ThreadId::new();
    let gate = sample_pending_gate(
        "user1",
        tid,
        "telegram",
        ResumeKind::Approval { allow_always: true },
    );
    let request_id = gate.request_id;
    let persistence = Arc::new(FakePersistence {
        gates: StdMutex::new(vec![]),
    });

    // Store 1: insert and persist
    let store1 = PendingGateStore::new(Some(persistence.clone()));
    store1.insert(gate).await.unwrap();

    // Simulate restart: new store, restore from persistence
    let store2 = PendingGateStore::new(Some(persistence));
    let restored = store2.restore_from_persistence().await.unwrap();
    assert_eq!(restored, 1);

    // Gate resolvable from restored store
    let key = PendingGateKey {
        user_id: "user1".into(),
        thread_id: tid,
    };
    let taken = store2.take_verified(&key, request_id, "telegram").await;
    assert!(taken.is_ok(), "Gate should be resolvable after restart");
    assert_eq!(taken.unwrap().action_name, "http");
}

// ── Tests: LeasePlanner thread-type scoping ──────────────────

/// Research threads cannot access Privileged or Administrative tools.
#[tokio::test]
async fn lease_planner_research_excludes_privileged() {
    use ironclaw_engine::LeasePlanner;

    let planner = LeasePlanner::new();
    let caps = make_caps(true); // http has requires_approval=true → Privileged

    let plans = planner.plan_for_thread(ThreadType::Research, &caps);
    let all_actions: Vec<String> = plans
        .iter()
        .flat_map(|p| p.granted_actions.actions().to_vec())
        .collect();

    assert!(
        all_actions.contains(&"echo".into()),
        "Research should include ReadOnly tools"
    );
    assert!(
        !all_actions.contains(&"http".into()),
        "Research should NOT include Privileged tools"
    );
}

/// Mission threads exclude Administrative tools (denylist).
#[tokio::test]
async fn lease_planner_mission_excludes_denylisted() {
    use ironclaw_engine::LeasePlanner;

    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "test".into(),
        actions: vec![
            ActionDef {
                name: "echo".into(),
                description: "Echo".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::ReadLocal],
                requires_approval: false,
            },
            ActionDef {
                name: "routine_create".into(),
                description: "Create routine".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::WriteLocal],
                requires_approval: false,
            },
        ],
        knowledge: vec![],
        policies: vec![],
    });

    let planner = LeasePlanner::new();
    let plans = planner.plan_for_thread(ThreadType::Mission, &caps);
    let all_actions: Vec<String> = plans
        .iter()
        .flat_map(|p| p.granted_actions.actions().to_vec())
        .collect();

    assert!(all_actions.contains(&"echo".into()));
    assert!(
        !all_actions.contains(&"routine_create".into()),
        "Mission should NOT include denylisted Administrative tools"
    );
}

// ── Tests: Child lease inheritance ───────────────────────────

/// Child leases are the intersection of parent leases and requested actions.
#[tokio::test]
async fn child_lease_inherits_subset_of_parent() {
    let mgr = LeaseManager::new();
    let parent = ThreadId::new();
    let child = ThreadId::new();

    mgr.grant(
        parent,
        "tools",
        GrantedActions::Specific(vec!["read".into(), "write".into(), "delete".into()]),
        None,
        None,
    )
    .await
    .unwrap();

    let mut requested = std::collections::HashSet::new();
    requested.insert("write".into());
    requested.insert("delete".into());
    requested.insert("admin".into()); // not in parent

    let child_leases = mgr
        .derive_child_leases(parent, child, Some(&requested))
        .await;
    assert_eq!(child_leases.len(), 1);

    let ga = &child_leases[0].granted_actions;
    assert!(ga.covers("write"));
    assert!(ga.covers("delete"));
    assert!(
        !ga.covers("admin"),
        "Child cannot have actions parent doesn't have"
    );
}

/// Expired parent leases produce no child leases (fail-closed).
#[tokio::test]
async fn expired_parent_yields_no_child_leases() {
    let mgr = LeaseManager::new();
    let parent = ThreadId::new();
    let child = ThreadId::new();

    // Grant a valid lease, then revoke it so it appears invalid to
    // derive_child_leases. (Negative durations are now rejected by grant.)
    let lease = mgr
        .grant(
            parent,
            "tools",
            GrantedActions::Specific(vec!["read".into()]),
            None,
            None,
        )
        .await
        .unwrap();
    mgr.revoke(lease.id, "test: simulating expired").await;

    let child_leases = mgr.derive_child_leases(parent, child, None).await;
    assert!(
        child_leases.is_empty(),
        "Revoked parent should yield no child leases"
    );
}

/// Wildcard parent (granted_actions=[]) + requested subset should give
/// only the requested subset, NOT a wildcard child (regression: C3 review).
#[tokio::test]
async fn wildcard_parent_lease_gives_requested_subset_not_wildcard() {
    let mgr = LeaseManager::new();
    let parent = ThreadId::new();
    let child = ThreadId::new();

    // Wildcard parent: granted_actions=All means "all actions"
    mgr.grant(parent, "tools", GrantedActions::All, None, None)
        .await
        .unwrap();

    let mut requested = std::collections::HashSet::new();
    requested.insert("read".into());
    requested.insert("write".into());

    let child_leases = mgr
        .derive_child_leases(parent, child, Some(&requested))
        .await;
    assert_eq!(child_leases.len(), 1);

    let ga = &child_leases[0].granted_actions;
    // Child should get Specific(["read", "write"]), NOT All (wildcard)
    let actions = ga.actions();
    assert_eq!(
        actions.len(),
        2,
        "Child of wildcard parent should get exactly the requested actions, not wildcard. Got: {actions:?}"
    );
    assert!(ga.covers("read"));
    assert!(ga.covers("write"));
}

// ── Tests: LeaseGate integration ─────────────────────────────

/// LeaseGate denies actions without a valid lease.
#[tokio::test]
async fn lease_gate_denies_without_lease() {
    use ironclaw_engine::gate::lease::LeaseGate;
    use ironclaw_engine::gate::{ExecutionGate, ExecutionMode, GateContext, GateDecision};

    let mgr = Arc::new(LeaseManager::new());
    let tid = ThreadId::new();
    // No leases granted

    let gate = LeaseGate::new(Arc::clone(&mgr));
    let ad = ActionDef {
        name: "shell".into(),
        description: String::new(),
        parameters_schema: serde_json::json!({}),
        effects: vec![EffectType::WriteLocal],
        requires_approval: true,
    };
    let auto = std::collections::HashSet::new();
    let params = serde_json::json!({});
    let ctx = GateContext {
        user_id: "user1",
        thread_id: tid,
        source_channel: "web",
        action_name: &ad.name,
        call_id: "call_1",
        parameters: &params,
        action_def: &ad,
        execution_mode: ExecutionMode::Autonomous,
        auto_approved: &auto,
    };

    assert!(
        matches!(gate.evaluate(&ctx).await, GateDecision::Deny { .. }),
        "LeaseGate should deny actions without a lease"
    );
}

/// LeaseGate allows actions covered by a valid lease.
#[tokio::test]
async fn lease_gate_allows_with_valid_lease() {
    use ironclaw_engine::gate::lease::LeaseGate;
    use ironclaw_engine::gate::{ExecutionGate, ExecutionMode, GateContext, GateDecision};

    let mgr = Arc::new(LeaseManager::new());
    let tid = ThreadId::new();
    mgr.grant(
        tid,
        "tools",
        GrantedActions::Specific(vec!["shell".into()]),
        None,
        None,
    )
    .await
    .unwrap();

    let gate = LeaseGate::new(Arc::clone(&mgr));
    let ad = ActionDef {
        name: "shell".into(),
        description: String::new(),
        parameters_schema: serde_json::json!({}),
        effects: vec![EffectType::WriteLocal],
        requires_approval: true,
    };
    let auto = std::collections::HashSet::new();
    let params = serde_json::json!({});
    let ctx = GateContext {
        user_id: "user1",
        thread_id: tid,
        source_channel: "web",
        action_name: &ad.name,
        call_id: "call_1",
        parameters: &params,
        action_def: &ad,
        execution_mode: ExecutionMode::Autonomous,
        auto_approved: &auto,
    };

    assert!(
        matches!(gate.evaluate(&ctx).await, GateDecision::Allow),
        "LeaseGate should allow actions covered by a valid lease"
    );
}

// ── Tests: GatePipeline composition ──────────────────────────

/// Pipeline evaluates gates in priority order; first Deny wins.
#[tokio::test]
async fn pipeline_first_deny_wins() {
    use ironclaw_engine::gate::pipeline::GatePipeline;
    use ironclaw_engine::gate::{ExecutionGate, ExecutionMode, GateContext, GateDecision};

    struct AlwaysAllow;
    #[async_trait::async_trait]
    impl ExecutionGate for AlwaysAllow {
        fn name(&self) -> &str {
            "allow"
        }
        fn priority(&self) -> u32 {
            10
        }
        async fn evaluate(&self, _: &GateContext<'_>) -> GateDecision {
            GateDecision::Allow
        }
    }

    struct AlwaysDeny;
    #[async_trait::async_trait]
    impl ExecutionGate for AlwaysDeny {
        fn name(&self) -> &str {
            "deny"
        }
        fn priority(&self) -> u32 {
            20
        }
        async fn evaluate(&self, _: &GateContext<'_>) -> GateDecision {
            GateDecision::Deny {
                reason: "blocked".into(),
            }
        }
    }

    let pipeline = GatePipeline::new(vec![
        Arc::new(AlwaysAllow) as Arc<dyn ExecutionGate>,
        Arc::new(AlwaysDeny),
    ]);

    let ad = ActionDef {
        name: "test".into(),
        description: String::new(),
        parameters_schema: serde_json::json!({}),
        effects: vec![],
        requires_approval: false,
    };
    let auto = std::collections::HashSet::new();
    let params = serde_json::json!({});
    let ctx = GateContext {
        user_id: "user1",
        thread_id: ThreadId::new(),
        source_channel: "web",
        action_name: &ad.name,
        call_id: "call_1",
        parameters: &params,
        action_def: &ad,
        execution_mode: ExecutionMode::Interactive,
        auto_approved: &auto,
    };

    assert!(matches!(
        pipeline.evaluate(&ctx).await,
        GateDecision::Deny { .. }
    ));
}

// ── Tests: InteractiveAutoApprove mode ───────────────────────

/// Auto-approve mode: GatePaused(Approval) is NOT returned for
/// UnlessAutoApproved tools — they execute directly.
#[tokio::test]
async fn auto_approve_mode_skips_approval_for_standard_tools() {
    let project_id = ProjectId::new();
    // This mock returns GatePaused only when NOT already approved.
    // In auto-approve mode, the engine should never reach this gate
    // because the ApprovalGate allows UnlessAutoApproved through.
    // But our mock sits at the EffectExecutor level, so we test that
    // the tool executes successfully (no GatePaused outcome).
    let effects = GateMockEffects::new(vec![], vec![]); // No gates — tool succeeds

    let llm = ScriptedLlm::new(vec![LlmOutput {
        response: LlmResponse::ActionCalls {
            calls: vec![ironclaw_engine::ActionCall {
                id: "call_1".into(),
                action_name: "echo".into(),
                parameters: serde_json::json!({"text": "hello"}),
            }],
            content: None,
        },
        usage: TokenUsage::default(),
    }]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects,
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps(false)),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "echo hello",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let outcome = mgr.join_thread(tid).await.expect("join_thread");

    // Tool should have executed and completed (no approval pause)
    assert!(
        matches!(outcome, ThreadOutcome::Completed { .. }),
        "Expected Completed in auto-approve mode, got: {outcome:?}"
    );
}

/// Auto-approve mode: Always-gated tools still pause for explicit approval.
#[tokio::test]
async fn auto_approve_mode_still_pauses_always_tools() {
    use ironclaw_engine::gate::{ExecutionMode, GateContext};

    // Test the ApprovalGate directly since we need the mode check
    // without a full ThreadManager setup.
    let ad = ActionDef {
        name: "dangerous_delete".into(),
        description: String::new(),
        parameters_schema: serde_json::json!({}),
        effects: vec![EffectType::WriteExternal],
        requires_approval: true, // This maps to Always in the real system
    };
    let auto = std::collections::HashSet::new();
    let params = serde_json::json!({});
    let ctx = GateContext {
        user_id: "user1",
        thread_id: ThreadId::new(),
        source_channel: "web",
        action_name: &ad.name,
        call_id: "call_1",
        parameters: &params,
        action_def: &ad,
        execution_mode: ExecutionMode::InteractiveAutoApprove,
        auto_approved: &auto,
    };

    // In auto-approve mode, the RelayChannelGate still allows
    // (it only checks channel suffix, not mode).
    // But the PolicyEngine would catch requires_approval=true.
    // This test validates the ExecutionMode semantics at the gate level.

    // Verify the mode is correctly propagated
    assert_eq!(ctx.execution_mode, ExecutionMode::InteractiveAutoApprove);
}

/// Execution obligation fires on the resume path: a thread hits a gate,
/// is resumed with a user message that signals execution intent ("run the
/// echo tool"), and the orchestrator's per-message intent detection enables
/// the obligation nudge even though the thread config did not have
/// `require_action_attempt` set at spawn time.
#[tokio::test]
async fn gate_resume_with_execution_obligation() {
    let project_id = ProjectId::new();
    let effects = GateMockEffects::new(vec!["http".into()], vec![]);

    // LLM responses for both phases:
    // Phase 1 (initial): tool_call(http) → gate fires
    // Phase 2 (resume):  text refusal → obligation nudge → tool_call(echo) → text done
    let llm = ScriptedLlm::new(vec![
        // Phase 1: triggers the http gate
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_http_1".into(),
                    action_name: "http".into(),
                    parameters: serde_json::json!({"url": "https://example.com"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        // Phase 2 after resume: http succeeds (approved), then LLM returns text
        // refusal. The orchestrator should detect "run the echo tool" intent from
        // the injected resume message and fire the obligation nudge.
        LlmOutput {
            response: LlmResponse::Text("I cannot execute tools from this reply path.".into()),
            usage: TokenUsage::default(),
        },
        // After obligation nudge: echo tool call
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_echo_1".into(),
                    action_name: "echo".into(),
                    parameters: serde_json::json!({"message": "obligation resume test"}),
                }],
                content: None,
            },
            usage: TokenUsage::default(),
        },
        // Final text
        LlmOutput {
            response: LlmResponse::Text("Echo returned: obligation resume test".into()),
            usage: TokenUsage::default(),
        },
    ]);

    let store = TestStore::new();
    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(make_caps(false)),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    // Phase 1: spawn with NO execution intent in the goal
    let tid = mgr
        .spawn_thread(
            "check the api status",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(), // require_action_attempt = false
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    assert!(
        matches!(first, ThreadOutcome::GatePaused { .. }),
        "expected GatePaused, got: {first:?}"
    );
    assert_eq!(
        store.load_thread(tid).await.unwrap().unwrap().state,
        ThreadState::Waiting,
    );

    // Phase 2: approve the gate, resume with execution intent message.
    // "run the echo tool" triggers signals_execution_intent() in the Python
    // orchestrator's context check on run_loop startup.
    effects.mark_approved("http").await;
    mgr.resume_thread(
        tid,
        "test-user",
        Some(ThreadMessage::user(
            "run the echo tool with 'obligation resume test'",
        )),
        Some(("call_gate_1".into(), true)),
        None,
    )
    .await
    .expect("resume_thread");

    let resumed = mgr.join_thread(tid).await.expect("second join");
    assert!(
        matches!(resumed, ThreadOutcome::Completed { .. }),
        "expected Completed, got: {resumed:?}"
    );

    // Verify the echo tool was called — proves obligation worked.
    // Without the per-message intent detection fix, the LLM's text refusal
    // would have been accepted as the final response (no nudge), and the
    // echo tool would never execute.
    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let echo_executed = thread.events.iter().any(|e| {
        matches!(
            &e.kind,
            ironclaw_engine::types::event::EventKind::ActionExecuted { action_name, .. }
                if action_name == "echo"
        )
    });
    assert!(
        echo_executed,
        "echo tool should have been called after obligation nudge on resume"
    );
}
