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
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use ironclaw_engine::types::capability::{EffectType, LeaseId};
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
        }])
    }
}

/// Generic EffectExecutor for scripted CodeAct benchmark scenarios.
/// Returns queued action results in order and records every canonical action call.
struct SequenceMockEffects {
    actions: Vec<ActionDef>,
    results: RwLock<Vec<Result<ActionResult, EngineError>>>,
    calls: RwLock<Vec<(String, serde_json::Value)>>,
}

impl SequenceMockEffects {
    fn new(actions: Vec<ActionDef>, results: Vec<Result<ActionResult, EngineError>>) -> Arc<Self> {
        Arc::new(Self {
            actions,
            results: RwLock::new(results),
            calls: RwLock::new(Vec::new()),
        })
    }

    async fn recorded_calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.read().await.clone()
    }
}

#[async_trait::async_trait]
impl EffectExecutor for SequenceMockEffects {
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

        let mut results = self.results.write().await;
        if results.is_empty() {
            Ok(ActionResult {
                call_id: String::new(),
                action_name: action_name.to_string(),
                output: serde_json::json!({"ok": true}),
                is_error: false,
                duration: Duration::from_millis(1),
            })
        } else {
            results.remove(0)
        }
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
    ) -> Result<Vec<ActionDef>, EngineError> {
        Ok(self.actions.clone())
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

fn scripted_usage_for_code(code: &str) -> TokenUsage {
    TokenUsage {
        input_tokens: 80,
        output_tokens: ((code.len() as u64) / 4).max(1),
        ..TokenUsage::default()
    }
}

fn benchmark_action(name: &str) -> ActionDef {
    let effects = match name {
        "read_file" | "list_dir" | "glob" => vec![EffectType::ReadLocal],
        "http" => vec![EffectType::ReadExternal],
        "shell" | "write_file" => vec![EffectType::WriteLocal],
        _ => vec![EffectType::ReadLocal],
    };
    ActionDef {
        name: name.into(),
        description: format!("Benchmark action {name}"),
        parameters_schema: serde_json::json!({"type": "object"}),
        effects,
        requires_approval: false,
    }
}

#[derive(Debug)]
struct CodeactBenchMetrics {
    scenario_id: &'static str,
    variant: &'static str,
    wall_time_ms: u128,
    total_tokens_used: u64,
    step_count: usize,
    action_count: usize,
    action_names: Vec<String>,
    final_response: String,
    code_chars: usize,
}

async fn run_codeact_benchmark_scenario(
    scenario_id: &'static str,
    variant: &'static str,
    goal: &str,
    code: &str,
    actions: Vec<ActionDef>,
    results: Vec<Result<ActionResult, EngineError>>,
) -> CodeactBenchMetrics {
    let project_id = ProjectId::new();
    let llm = ScriptedLlm::new(vec![LlmOutput {
        response: LlmResponse::Code {
            code: code.to_string(),
            content: None,
        },
        usage: scripted_usage_for_code(code),
    }]);

    let effects = SequenceMockEffects::new(actions.clone(), results);
    let store = TestStore::new();

    let mut caps = CapabilityRegistry::new();
    caps.register(Capability {
        name: "tools".into(),
        description: "Available tools".into(),
        actions,
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

    let start = Instant::now();
    let tid = mgr
        .spawn_thread(
            goal,
            ThreadType::Foreground,
            project_id,
            ThreadConfig::default(),
            None,
            "bench-user",
        )
        .await
        .expect("spawn_thread");

    let outcome = mgr.join_thread(tid).await.expect("join_thread");
    let elapsed = start.elapsed();

    let final_response = match outcome {
        ThreadOutcome::Completed { response } => response.unwrap_or_default(),
        other => panic!("expected completed outcome, got {other:?}"),
    };

    let thread = store.load_thread(tid).await.unwrap().unwrap();
    let calls = effects.recorded_calls().await;

    CodeactBenchMetrics {
        scenario_id,
        variant,
        wall_time_ms: elapsed.as_millis(),
        total_tokens_used: thread.total_tokens_used,
        step_count: thread.step_count,
        action_count: calls.len(),
        action_names: calls.into_iter().map(|(name, _)| name).collect(),
        final_response,
        code_chars: code.len(),
    }
}

/// Shim-vs-raw benchmark inspired by the milestone0 replay/blackbox patterns
/// in PR #2761, but aimed at the CodeAct path where the LLM emits Python code.
#[tokio::test]
#[ignore]
async fn benchmark_codeact_raw_vs_shim_scenarios() {
    let read_file_output = serde_json::json!({
        "content": "     1│ {\"answer\": 42}",
        "total_lines": 1,
        "lines_shown": 1,
        "truncated_by_default": false,
        "path": "data.json"
    });
    let list_dir_output = serde_json::json!({
        "path": "src",
        "entries": ["bin/", "main.rs (1.0KB)"],
        "count": 2,
        "truncated": false
    });
    let glob_output = serde_json::json!({
        "files": ["src/lib.rs", "src/main.rs"],
        "count": 2,
        "truncated": false,
        "duration_ms": 1
    });
    let notes_read_output = serde_json::json!({
        "content": "     1│ alpha",
        "total_lines": 1,
        "lines_shown": 1,
        "truncated_by_default": false,
        "path": "notes.txt"
    });
    let write_output = serde_json::json!({
        "path": "notes.txt",
        "bytes_written": 10,
        "success": true
    });

    let raw_read_json = r#"
import json
raw = await read_file(path="data.json")
parts = []
for line in raw["content"].splitlines():
    rest = line.split("│", 1)[1]
    if rest.startswith(" "):
        rest = rest[1:]
    parts.append(rest)
text = "\n".join(parts)
data = json.loads(text)
FINAL(str(data["answer"]))
"#;
    let shim_read_json = r#"
data = await read_json("data.json")
FINAL(str(data["answer"]))
"#;

    let raw_list_entries = r#"
listing = await list_dir(path="src")
entries = listing["entries"]
FINAL(entries[0] + "|" + str(len(entries)))
"#;
    let shim_list_entries = r#"
entries = await list_entries("src")
FINAL(entries[0] + "|" + str(len(entries)))
"#;

    let raw_find_files = r#"
result = await glob(pattern="*.rs", path="src")
files = result["files"]
FINAL(files[1] + "|" + str(len(files)))
"#;
    let shim_find_files = r#"
files = await find_files("*.rs", "src")
FINAL(files[1] + "|" + str(len(files)))
"#;

    let raw_append_text = r#"
raw = await read_file(path="notes.txt", offset=1)
parts = []
for line in raw["content"].splitlines():
    rest = line.split("│", 1)[1]
    if rest.startswith(" "):
        rest = rest[1:]
    parts.append(rest)
current = "\n".join(parts)
result = await write_file(path="notes.txt", content=current + "\nbeta")
FINAL(str(result["success"]) + "|" + str(result["bytes_written"]))
"#;
    let shim_append_text = r#"
result = await append_text("notes.txt", "\nbeta")
FINAL(str(result["ok"]) + "|" + str(result["bytes_written"]))
"#;

    let raw_read_json_metrics = run_codeact_benchmark_scenario(
        "read_json",
        "raw",
        "read json benchmark",
        raw_read_json,
        vec![benchmark_action("read_file")],
        vec![Ok(ActionResult {
            call_id: String::new(),
            action_name: "read_file".into(),
            output: read_file_output.clone(),
            is_error: false,
            duration: Duration::from_millis(1),
        })],
    )
    .await;
    let shim_read_json_metrics = run_codeact_benchmark_scenario(
        "read_json",
        "shim",
        "read json benchmark",
        shim_read_json,
        vec![benchmark_action("read_file")],
        vec![Ok(ActionResult {
            call_id: String::new(),
            action_name: "read_file".into(),
            output: read_file_output.clone(),
            is_error: false,
            duration: Duration::from_millis(1),
        })],
    )
    .await;

    let raw_list_entries_metrics = run_codeact_benchmark_scenario(
        "list_entries",
        "raw",
        "list entries benchmark",
        raw_list_entries,
        vec![benchmark_action("list_dir")],
        vec![Ok(ActionResult {
            call_id: String::new(),
            action_name: "list_dir".into(),
            output: list_dir_output.clone(),
            is_error: false,
            duration: Duration::from_millis(1),
        })],
    )
    .await;
    let shim_list_entries_metrics = run_codeact_benchmark_scenario(
        "list_entries",
        "shim",
        "list entries benchmark",
        shim_list_entries,
        vec![benchmark_action("list_dir")],
        vec![Ok(ActionResult {
            call_id: String::new(),
            action_name: "list_dir".into(),
            output: list_dir_output.clone(),
            is_error: false,
            duration: Duration::from_millis(1),
        })],
    )
    .await;

    let raw_find_files_metrics = run_codeact_benchmark_scenario(
        "find_files",
        "raw",
        "find files benchmark",
        raw_find_files,
        vec![benchmark_action("glob")],
        vec![Ok(ActionResult {
            call_id: String::new(),
            action_name: "glob".into(),
            output: glob_output.clone(),
            is_error: false,
            duration: Duration::from_millis(1),
        })],
    )
    .await;
    let shim_find_files_metrics = run_codeact_benchmark_scenario(
        "find_files",
        "shim",
        "find files benchmark",
        shim_find_files,
        vec![benchmark_action("glob")],
        vec![Ok(ActionResult {
            call_id: String::new(),
            action_name: "glob".into(),
            output: glob_output.clone(),
            is_error: false,
            duration: Duration::from_millis(1),
        })],
    )
    .await;

    let raw_append_text_metrics = run_codeact_benchmark_scenario(
        "append_text",
        "raw",
        "append text benchmark",
        raw_append_text,
        vec![
            benchmark_action("read_file"),
            benchmark_action("write_file"),
        ],
        vec![
            Ok(ActionResult {
                call_id: String::new(),
                action_name: "read_file".into(),
                output: notes_read_output.clone(),
                is_error: false,
                duration: Duration::from_millis(1),
            }),
            Ok(ActionResult {
                call_id: String::new(),
                action_name: "write_file".into(),
                output: write_output.clone(),
                is_error: false,
                duration: Duration::from_millis(1),
            }),
        ],
    )
    .await;
    let shim_append_text_metrics = run_codeact_benchmark_scenario(
        "append_text",
        "shim",
        "append text benchmark",
        shim_append_text,
        vec![
            benchmark_action("read_file"),
            benchmark_action("write_file"),
        ],
        vec![
            Ok(ActionResult {
                call_id: String::new(),
                action_name: "read_file".into(),
                output: notes_read_output,
                is_error: false,
                duration: Duration::from_millis(1),
            }),
            Ok(ActionResult {
                call_id: String::new(),
                action_name: "write_file".into(),
                output: write_output,
                is_error: false,
                duration: Duration::from_millis(1),
            }),
        ],
    )
    .await;

    let pairs = vec![
        (raw_read_json_metrics, shim_read_json_metrics),
        (raw_list_entries_metrics, shim_list_entries_metrics),
        (raw_find_files_metrics, shim_find_files_metrics),
        (raw_append_text_metrics, shim_append_text_metrics),
    ];

    for (raw, shim) in pairs {
        assert_eq!(
            raw.final_response, shim.final_response,
            "scenario {} should preserve final response",
            raw.scenario_id
        );
        assert_eq!(
            raw.action_names, shim.action_names,
            "scenario {} should preserve canonical action sequence",
            raw.scenario_id
        );
        assert!(
            shim.total_tokens_used < raw.total_tokens_used,
            "scenario {} should reduce scripted token proxy: raw={} shim={}",
            raw.scenario_id,
            raw.total_tokens_used,
            shim.total_tokens_used
        );
        eprintln!(
            "codeact_shim_bench scenario={} raw_variant={} shim_variant={} raw_ms={} shim_ms={} raw_tokens={} shim_tokens={} raw_actions={} shim_actions={} raw_steps={} shim_steps={} raw_chars={} shim_chars={}",
            raw.scenario_id,
            raw.variant,
            shim.variant,
            raw.wall_time_ms,
            shim.wall_time_ms,
            raw.total_tokens_used,
            shim.total_tokens_used,
            raw.action_count,
            shim.action_count,
            raw.step_count,
            shim.step_count,
            raw.code_chars,
            shim.code_chars,
        );
    }
}
