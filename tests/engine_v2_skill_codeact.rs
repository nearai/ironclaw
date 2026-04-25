//! Integration test: v2 engine skill activation with full CodeAct execution.
//!
//! Exercises the complete path:
//! 1. GitHub skill selected based on thread goal keywords
//! 2. LLM returns Python code calling `await http(...)` to fetch issues
//! 3. Monty VM executes the code, dispatches `http` to mock EffectExecutor
//! 4. Mock returns canned GitHub JSON response
//! 5. `FINAL(result)` terminates the code step
//! 6. Thread completes with the canned data in the response

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use ironclaw_engine::types::capability::{EffectType, LeaseId, ModelToolSurface};
use ironclaw_engine::{
    ActionDef, ActionResult, Capability, CapabilityLease, CapabilityRegistry, DocId, DocType,
    EffectExecutor, EngineError, LeaseManager, LlmBackend, LlmCallConfig, LlmOutput, LlmResponse,
    MemoryDoc, Mission, MissionId, MissionStatus, PolicyEngine, Project, ProjectId, Step, Store,
    Thread, ThreadConfig, ThreadEvent, ThreadId, ThreadManager, ThreadMessage, ThreadOutcome,
    ThreadState, ThreadType, TokenUsage,
};

use ironclaw_skills::types::ActivationCriteria;
use ironclaw_skills::v2::{CodeSnippet, SkillMetrics, V2SkillMetadata, V2SkillSource};

// ── Scripted LLM ─────────────────────────────────────────────

/// Mock LLM that returns pre-queued responses.
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

struct CapturingScriptedLlm {
    responses: std::sync::Mutex<Vec<LlmOutput>>,
    seen_messages: std::sync::Mutex<Vec<Vec<ThreadMessage>>>,
}

impl CapturingScriptedLlm {
    fn new(responses: Vec<LlmOutput>) -> Arc<Self> {
        Arc::new(Self {
            responses: std::sync::Mutex::new(responses),
            seen_messages: std::sync::Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl LlmBackend for CapturingScriptedLlm {
    async fn complete(
        &self,
        messages: &[ThreadMessage],
        _actions: &[ActionDef],
        _config: &LlmCallConfig,
    ) -> Result<LlmOutput, EngineError> {
        self.seen_messages.lock().unwrap().push(messages.to_vec());
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
        "capturing-scripted-mock"
    }
}

// ── HTTP Mock Effects ────────────────────────────────────────

/// Mock EffectExecutor that intercepts `http` calls and returns canned responses.
/// Records all calls for verification.
struct HttpMockEffects {
    /// Map from URL substring → canned response JSON
    canned_responses: HashMap<String, serde_json::Value>,
    /// Recorded action calls (name, params)
    calls: RwLock<Vec<(String, serde_json::Value)>>,
}

impl HttpMockEffects {
    fn new(canned: HashMap<String, serde_json::Value>) -> Arc<Self> {
        Arc::new(Self {
            canned_responses: canned,
            calls: RwLock::new(Vec::new()),
        })
    }

    async fn recorded_calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.read().await.clone()
    }
}

struct PausingHttpMockEffects {
    canned_responses: HashMap<String, serde_json::Value>,
    calls: RwLock<Vec<(String, serde_json::Value)>>,
    approved: RwLock<bool>,
    capabilities: Vec<ironclaw_engine::CapabilitySummary>,
}

impl PausingHttpMockEffects {
    fn new(canned: HashMap<String, serde_json::Value>) -> Arc<Self> {
        Self::with_capabilities(canned, vec![])
    }

    fn with_capabilities(
        canned: HashMap<String, serde_json::Value>,
        capabilities: Vec<ironclaw_engine::CapabilitySummary>,
    ) -> Arc<Self> {
        Arc::new(Self {
            canned_responses: canned,
            calls: RwLock::new(Vec::new()),
            approved: RwLock::new(false),
            capabilities,
        })
    }

    async fn mark_approved(&self) {
        *self.approved.write().await = true;
    }
}

#[async_trait::async_trait]
impl EffectExecutor for HttpMockEffects {
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

        // Match by URL substring in canned responses
        let url = parameters.get("url").and_then(|v| v.as_str()).unwrap_or("");

        let output = self
            .canned_responses
            .iter()
            .find(|(pattern, _)| url.contains(pattern.as_str()))
            .map(|(_, response)| response.clone())
            .unwrap_or_else(|| {
                serde_json::json!({
                    "error": "not_found",
                    "message": format!("No canned response for URL: {url}")
                })
            });

        Ok(ActionResult {
            call_id: String::new(),
            action_name: action_name.to_string(),
            output,
            is_error: false,
            duration: Duration::from_millis(1),
        })
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
        _context: &ironclaw_engine::ThreadExecutionContext,
    ) -> Result<Vec<ActionDef>, EngineError> {
        Ok(vec![ActionDef {
            name: "http".into(),
            description: "Make HTTP requests".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "method": {"type": "string"},
                    "url": {"type": "string"},
                    "headers": {"type": "array"},
                    "body": {}
                },
                "required": ["url"]
            }),
            effects: vec![EffectType::ReadExternal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }])
    }

    async fn available_capabilities(
        &self,
        _leases: &[CapabilityLease],
        _context: &ironclaw_engine::ThreadExecutionContext,
    ) -> Result<Vec<ironclaw_engine::CapabilitySummary>, EngineError> {
        Ok(vec![])
    }
}

#[async_trait::async_trait]
impl EffectExecutor for PausingHttpMockEffects {
    async fn execute_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        _lease: &CapabilityLease,
        _context: &ironclaw_engine::ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError> {
        if action_name == "http" && !*self.approved.read().await {
            return Err(EngineError::GatePaused {
                gate_name: "approval".into(),
                action_name: action_name.to_string(),
                call_id: "call_http_gate_1".into(),
                parameters: Box::new(parameters),
                resume_kind: Box::new(ironclaw_engine::ResumeKind::Approval {
                    allow_always: false,
                }),
                paused_lease: None,
                resume_output: None,
            });
        }

        self.calls
            .write()
            .await
            .push((action_name.to_string(), parameters.clone()));

        let url = parameters.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let output = self
            .canned_responses
            .iter()
            .find(|(pattern, _)| url.contains(pattern.as_str()))
            .map(|(_, response)| response.clone())
            .unwrap_or_else(|| {
                serde_json::json!({
                    "error": "not_found",
                    "message": format!("No canned response for URL: {url}")
                })
            });

        Ok(ActionResult {
            call_id: String::new(),
            action_name: action_name.to_string(),
            output,
            is_error: false,
            duration: Duration::from_millis(1),
        })
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
        _context: &ironclaw_engine::ThreadExecutionContext,
    ) -> Result<Vec<ActionDef>, EngineError> {
        Ok(vec![ActionDef {
            name: "http".into(),
            description: "Make HTTP requests".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "method": {"type": "string"},
                    "url": {"type": "string"},
                    "headers": {"type": "array"},
                    "body": {}
                },
                "required": ["url"]
            }),
            effects: vec![EffectType::ReadExternal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }])
    }

    async fn available_capabilities(
        &self,
        _leases: &[CapabilityLease],
        _context: &ironclaw_engine::ThreadExecutionContext,
    ) -> Result<Vec<ironclaw_engine::CapabilitySummary>, EngineError> {
        Ok(self.capabilities.clone())
    }
}

// ── In-Memory Store ──────────────────────────────────────────

/// Minimal in-memory Store for integration tests.
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
        self.steps.write().await.push(step.clone());
        Ok(())
    }
    async fn load_steps(&self, tid: ThreadId) -> Result<Vec<Step>, EngineError> {
        Ok(self
            .steps
            .read()
            .await
            .iter()
            .filter(|s| s.thread_id == tid)
            .cloned()
            .collect())
    }
    async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
        self.events.write().await.extend_from_slice(events);
        Ok(())
    }
    async fn load_events(&self, tid: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
        Ok(self
            .events
            .read()
            .await
            .iter()
            .filter(|e| e.thread_id == tid)
            .cloned()
            .collect())
    }
    async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
        Ok(())
    }
    async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
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
        pid: ProjectId,
        _user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self
            .docs
            .read()
            .await
            .iter()
            .filter(|d| d.project_id == pid)
            .cloned()
            .collect())
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
        self.leases.write().await.push(lease.clone());
        Ok(())
    }
    async fn load_active_leases(&self, _: ThreadId) -> Result<Vec<CapabilityLease>, EngineError> {
        Ok(vec![])
    }
    async fn revoke_lease(&self, _: LeaseId, _: &str) -> Result<(), EngineError> {
        Ok(())
    }
    async fn save_mission(&self, m: &Mission) -> Result<(), EngineError> {
        let mut missions = self.missions.write().await;
        missions.retain(|x| x.id != m.id);
        missions.push(m.clone());
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
        pid: ProjectId,
        _user_id: &str,
    ) -> Result<Vec<Mission>, EngineError> {
        Ok(self
            .missions
            .read()
            .await
            .iter()
            .filter(|m| m.project_id == pid)
            .cloned()
            .collect())
    }
    async fn update_mission_status(
        &self,
        _: MissionId,
        _: MissionStatus,
    ) -> Result<(), EngineError> {
        Ok(())
    }
}

// ── Helpers ──────────────────────────────────────────────────

fn make_github_skill_doc(project_id: ProjectId) -> MemoryDoc {
    let meta = V2SkillMetadata {
        name: "github".into(),
        version: 1,
        description: "GitHub API integration via HTTP tool".into(),
        activation: ActivationCriteria {
            keywords: vec![
                "github".into(),
                "issues".into(),
                "pull request".into(),
                "repository".into(),
            ],
            patterns: vec![
                r"(?i)(list|show|get|fetch).*issue".into(),
            ],
            tags: vec!["git".into(), "devops".into()],
            max_context_tokens: 1500,
            ..Default::default()
        },
        source: V2SkillSource::Authored,
        trust: ironclaw_skills::SkillTrust::Trusted,
        requires: Default::default(),
        code_snippets: vec![CodeSnippet {
            name: "list_github_issues".into(),
            code: r#"def list_github_issues(owner, repo, state="open"):
    result = await http(method="GET", url=f"https://api.github.com/repos/{owner}/{repo}/issues?state={state}&per_page=10")
    return result"#
                .into(),
            description: "List issues for a GitHub repository".into(),
        }],
        metrics: SkillMetrics::default(),
        parent_version: None,
        revisions: vec![],
        repairs: vec![],
        content_hash: String::new(),
        bundle_path: None,
        source_url: None,
    };

    let prompt = "\
# GitHub API Skill

Use the `http` tool to call the GitHub REST API. Credentials are injected automatically.

## Patterns

- List issues: `await http(method=\"GET\", url=\"https://api.github.com/repos/{owner}/{repo}/issues?state=open\")`
- Create issue: `await http(method=\"POST\", url=\"...issues\", body={\"title\": \"...\"})`

## Rules
- Always use HTTPS
- Do NOT set Authorization headers manually
- Default to state=open for issue queries
";

    let mut doc = MemoryDoc::new(project_id, "system", DocType::Skill, "skill:github", prompt);
    doc.metadata = serde_json::to_value(&meta).unwrap();
    doc
}

fn canned_github_issues() -> serde_json::Value {
    serde_json::json!([
        {"number": 42, "title": "Fix login bug", "state": "open", "user": {"login": "alice"}},
        {"number": 37, "title": "Add dark mode", "state": "open", "user": {"login": "bob"}},
        {"number": 15, "title": "Update docs", "state": "open", "user": {"login": "carol"}}
    ])
}

// ── Tests ────────────────────────────────────────────────────

/// Full CodeAct E2E: skill selected → LLM returns code → http() dispatched →
/// canned response returned → FINAL() terminates → thread completes.
#[tokio::test]
async fn skill_codeact_e2e_github_issues() {
    let project_id = ProjectId::new();

    // 1. Build GitHub skill doc (stored in TestStore for Python orchestrator to find)
    let skill_doc = make_github_skill_doc(project_id);

    // 2. Script the LLM: return Python code that awaits http() then FINAL()
    let python_code = r#"
result = await http(method="GET", url="https://api.github.com/repos/test-org/test-repo/issues?state=open&per_page=5")
FINAL(str(result))
"#;
    let llm = ScriptedLlm::new(vec![LlmOutput {
        response: LlmResponse::Code {
            code: python_code.to_string(),
            content: None,
        },
        usage: TokenUsage::default(),
    }]);

    // 3. Mock HTTP effects with canned GitHub response
    let mut canned = HashMap::new();
    canned.insert(
        "api.github.com/repos/test-org/test-repo/issues".to_string(),
        canned_github_issues(),
    );
    let effects = HttpMockEffects::new(canned);

    // 4. Build infrastructure — store skill doc so __list_skills__() finds it
    let store = TestStore::new();
    store.save_memory_doc(&skill_doc).await.unwrap();

    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "Available tools".into(),
        actions: vec![ActionDef {
            name: "http".into(),
            description: "Make HTTP requests".into(),
            parameters_schema: serde_json::json!({"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}),
            effects: vec![EffectType::ReadExternal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }],
        knowledge: vec![],
        policies: vec![],
    });

    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(caps),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    // 5. Spawn thread with a goal that matches the GitHub skill keywords
    // (Python orchestrator calls __list_skills__() and selects based on goal)
    let tid = mgr
        .spawn_thread(
            "show me open github issues for test-org/test-repo",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    // 6. Wait for completion
    let outcome = mgr.join_thread(tid).await.expect("join_thread");

    // 7. Verify thread completed with the canned response data
    match &outcome {
        ThreadOutcome::Completed { response } => {
            let resp = response.as_deref().unwrap_or("");
            assert!(
                resp.contains("Fix login bug") || resp.contains("42"),
                "response should contain canned issue data, got: {resp}"
            );
        }
        other => panic!("expected Completed, got: {other:?}"),
    }

    // 8. Verify the http action was called with correct parameters
    let calls = effects.recorded_calls().await;
    assert!(
        !calls.is_empty(),
        "http action should have been called at least once"
    );
    let (action_name, params) = &calls[0];
    assert_eq!(action_name, "http");
    let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        url.contains("api.github.com") && url.contains("test-org/test-repo/issues"),
        "http should be called with GitHub issues URL, got: {url}"
    );

    // 9. Verify skill content was injected into the internal working transcript.
    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let has_skill_content = thread
        .internal_messages
        .iter()
        .any(|m| m.content.contains("Active Skills") || m.content.contains("GitHub API Skill"));
    assert!(
        has_skill_content,
        "thread internal_messages should contain injected skill content"
    );
}

/// Verify selected skill provenance is persisted onto the thread for learning flows.
#[tokio::test]
async fn skill_codeact_persists_active_skill_provenance() {
    let project_id = ProjectId::new();
    let skill_doc = make_github_skill_doc(project_id);
    let skill_doc_id = skill_doc.id;

    let python_code = r#"
result = await http(method="GET", url="https://api.github.com/repos/test-org/test-repo/issues?state=open&per_page=5")
FINAL(str(result))
"#;
    let llm = ScriptedLlm::new(vec![LlmOutput {
        response: LlmResponse::Code {
            code: python_code.to_string(),
            content: None,
        },
        usage: TokenUsage::default(),
    }]);

    let mut canned = HashMap::new();
    canned.insert(
        "api.github.com/repos/test-org/test-repo/issues".to_string(),
        canned_github_issues(),
    );
    let effects = HttpMockEffects::new(canned);
    let store = TestStore::new();
    store.save_memory_doc(&skill_doc).await.unwrap();

    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "Available tools".into(),
        actions: vec![ActionDef {
            name: "http".into(),
            description: "Make HTTP requests".into(),
            parameters_schema: serde_json::json!({"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}),
            effects: vec![EffectType::ReadExternal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }],
        knowledge: vec![],
        policies: vec![],
    });

    let mgr = ThreadManager::new(
        llm,
        effects,
        store.clone() as Arc<dyn Store>,
        Arc::new(caps),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "show me open github issues for test-org/test-repo",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let outcome = mgr.join_thread(tid).await.expect("join_thread");
    assert!(
        matches!(outcome, ThreadOutcome::Completed { .. }),
        "expected Completed, got: {outcome:?}"
    );

    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let active_skills = thread.active_skills();
    let github_skill = active_skills
        .iter()
        .find(|skill| skill.doc_id == skill_doc_id)
        .unwrap_or_else(|| panic!("expected github skill provenance in {active_skills:?}"));
    assert_eq!(github_skill.name, "github");
    assert_eq!(github_skill.version, 1);
    assert_eq!(github_skill.snippet_names, vec!["list_github_issues"]);
}

/// Verify that non-matching goals don't activate skills (negative case).
#[tokio::test]
async fn non_matching_goal_skips_skill_codeact() {
    let project_id = ProjectId::new();

    let skill_doc = make_github_skill_doc(project_id);

    // LLM just returns text — no code execution needed
    let llm = ScriptedLlm::new(vec![LlmOutput {
        response: LlmResponse::Text("The weather is sunny.".into()),
        usage: TokenUsage::default(),
    }]);

    let effects = HttpMockEffects::new(HashMap::new());
    let store = TestStore::new();
    store.save_memory_doc(&skill_doc).await.unwrap();

    let mgr = ThreadManager::new(
        llm,
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(CapabilityRegistry::new()),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "what is the weather today",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let outcome = mgr.join_thread(tid).await.expect("join_thread");
    assert!(matches!(outcome, ThreadOutcome::Completed { .. }));

    // No http calls should have been made
    let calls = effects.recorded_calls().await;
    assert!(calls.is_empty(), "no http calls for weather query");

    // Skill content should NOT appear in messages (goal doesn't match)
    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let has_skill_content = thread
        .messages
        .iter()
        .any(|m| m.content.contains("Active Skills"));
    assert!(!has_skill_content, "no skills for unrelated goal");
}

#[tokio::test]
async fn skill_prompt_context_survives_pause_and_resume() {
    let project_id = ProjectId::new();
    let skill_doc = make_github_skill_doc(project_id);

    let llm = CapturingScriptedLlm::new(vec![
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_http_gate_1".into(),
                    action_name: "http".into(),
                    parameters: serde_json::json!({
                        "method": "GET",
                        "url": "https://api.github.com/repos/test-org/test-repo/issues?state=open&per_page=5"
                    }),
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

    let mut canned = HashMap::new();
    canned.insert(
        "api.github.com/repos/test-org/test-repo/issues".to_string(),
        canned_github_issues(),
    );
    let effects = PausingHttpMockEffects::new(canned);

    let store = TestStore::new();
    store.save_memory_doc(&skill_doc).await.unwrap();

    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "Available tools".into(),
        actions: vec![ActionDef {
            name: "http".into(),
            description: "Make HTTP requests".into(),
            parameters_schema: serde_json::json!({"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}),
            effects: vec![EffectType::ReadExternal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }],
        knowledge: vec![],
        policies: vec![],
    });

    let mgr = ThreadManager::new(
        llm.clone(),
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(caps),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            "show me open github issues for test-org/test-repo",
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    assert!(
        matches!(first, ThreadOutcome::GatePaused { .. }),
        "unexpected first outcome: {first:?}"
    );

    effects.mark_approved().await;
    mgr.resume_thread(
        tid,
        "test-user",
        Some(ThreadMessage::user("approved")),
        Some(("call_http_gate_1".into(), true)),
        None,
    )
    .await
    .expect("resume_thread");

    let resumed = mgr.join_thread(tid).await.expect("second join");
    assert!(matches!(resumed, ThreadOutcome::Completed { .. }));

    let seen = llm.seen_messages.lock().unwrap();
    assert!(
        seen.len() >= 2,
        "expected at least one LLM call before and after resume"
    );
    let resumed_system_prompt = &seen.last().unwrap()[0].content;
    assert!(resumed_system_prompt.contains("GitHub API Skill"));
    assert!(resumed_system_prompt.contains("Active Skills"));
    assert!(!resumed_system_prompt.contains("## Available tools (call as Python functions)"));
}

#[tokio::test]
async fn skill_prompt_context_survives_compaction_and_resume() {
    let project_id = ProjectId::new();
    let skill_doc = make_github_skill_doc(project_id);
    let long_goal = format!(
        "show me open github issues for test-org/test-repo and use /missing for comparison. {}",
        "Include the repo state in detail. ".repeat(80)
    );

    let llm = CapturingScriptedLlm::new(vec![
        LlmOutput {
            response: LlmResponse::Text("Compaction summary text".into()),
            usage: TokenUsage::default(),
        },
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![ironclaw_engine::ActionCall {
                    id: "call_http_gate_1".into(),
                    action_name: "http".into(),
                    parameters: serde_json::json!({
                        "method": "GET",
                        "url": "https://api.github.com/repos/test-org/test-repo/issues?state=open&per_page=5"
                    }),
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

    let mut canned = HashMap::new();
    canned.insert(
        "api.github.com/repos/test-org/test-repo/issues".to_string(),
        canned_github_issues(),
    );
    let effects = PausingHttpMockEffects::with_capabilities(
        canned,
        vec![ironclaw_engine::CapabilitySummary {
            name: "slack".into(),
            display_name: Some("Slack".into()),
            kind: ironclaw_engine::CapabilitySummaryKind::Provider,
            status: ironclaw_engine::CapabilityStatus::NeedsAuth,
            description: Some("Slack workspace integration".into()),
            action_preview: vec!["slack_send".into()],
            routing_hint: None,
        }],
    );

    let store = TestStore::new();
    store.save_memory_doc(&skill_doc).await.unwrap();

    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "Available tools".into(),
        actions: vec![ActionDef {
            name: "http".into(),
            description: "Make HTTP requests".into(),
            parameters_schema: serde_json::json!({"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}),
            effects: vec![EffectType::ReadExternal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }],
        knowledge: vec![],
        policies: vec![],
    });

    let mgr = ThreadManager::new(
        llm.clone(),
        effects.clone(),
        store.clone() as Arc<dyn Store>,
        Arc::new(caps),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    );

    let tid = mgr
        .spawn_thread(
            &long_goal,
            ThreadType::Foreground,
            project_id,
            ThreadConfig {
                enable_compaction: true,
                model_context_limit: 1_200,
                compaction_threshold: 0.25,
                ..ThreadConfig::default()
            },
            None,
            "test-user",
        )
        .await
        .expect("spawn_thread");

    let first = mgr.join_thread(tid).await.expect("first join");
    assert!(
        matches!(first, ThreadOutcome::GatePaused { .. }),
        "unexpected first outcome: {first:?}"
    );

    let paused_thread = store.load_thread(tid).await.unwrap().unwrap();
    assert!(
        paused_thread
            .internal_messages
            .iter()
            .any(|message| message.content == "Compaction summary text"),
        "compaction summary should be persisted into the active transcript"
    );
    assert!(
        paused_thread.internal_messages.iter().any(|message| message
            .content
            .contains("Your conversation has been compacted.")),
        "compaction notice should be persisted into the active transcript"
    );

    assert_eq!(
        paused_thread
            .internal_messages
            .iter()
            .filter(|message| message.role == ironclaw_engine::types::message::MessageRole::System)
            .count(),
        1,
        "compacted paused transcript should preserve exactly one system message"
    );

    effects.mark_approved().await;
    mgr.resume_thread(
        tid,
        "test-user",
        Some(ThreadMessage::user("approved")),
        Some(("call_http_gate_1".into(), true)),
        None,
    )
    .await
    .expect("resume_thread");

    let resumed = mgr.join_thread(tid).await.expect("second join");
    assert!(matches!(resumed, ThreadOutcome::Completed { .. }));

    let seen = llm.seen_messages.lock().unwrap();
    let summary_prompt = "Summarize progress so far in a concise but complete way.";
    let non_summary_calls: Vec<&Vec<ThreadMessage>> = seen
        .iter()
        .filter(|messages| {
            messages
                .last()
                .is_some_and(|message| !message.content.contains(summary_prompt))
        })
        .collect();
    assert_eq!(
        non_summary_calls.len(),
        2,
        "expected exactly one post-compaction call before pause and one resumed call"
    );

    let post_compaction_call = non_summary_calls[0];
    let resumed_call = non_summary_calls[1];

    let post_compaction_system_prompt = &post_compaction_call[0].content;
    assert!(post_compaction_system_prompt.contains("GitHub API Skill"));
    assert!(post_compaction_system_prompt.contains("Active Skills"));
    assert!(post_compaction_system_prompt.contains("/missing"));
    assert!(post_compaction_system_prompt.contains("`slack` [provider]"));
    assert!(post_compaction_system_prompt.contains("tool_activate(name=\"<integration>\")"));
    assert_eq!(
        post_compaction_system_prompt
            .matches("## Activatable Integrations")
            .count(),
        1
    );
    assert!(
        !post_compaction_system_prompt.contains("## Available tools (call as Python functions)")
    );
    assert!(
        post_compaction_call
            .iter()
            .any(|message| message.content == "Compaction summary text"),
        "first real post-compaction call should include the compaction summary message"
    );
    assert!(
        post_compaction_call.iter().any(|message| message
            .content
            .contains("Your conversation has been compacted.")),
        "first real post-compaction call should include the compaction notice"
    );

    let resumed_system_prompt = &resumed_call[0].content;
    assert!(resumed_system_prompt.contains("GitHub API Skill"));
    assert!(resumed_system_prompt.contains("Active Skills"));
    assert!(resumed_system_prompt.contains("/missing"));
    assert!(resumed_system_prompt.contains("`slack` [provider]"));
    assert!(resumed_system_prompt.contains("tool_activate(name=\"<integration>\")"));
    assert_eq!(
        resumed_system_prompt
            .matches("## Activatable Integrations")
            .count(),
        1
    );
    assert!(!resumed_system_prompt.contains("## Available tools (call as Python functions)"));
}
