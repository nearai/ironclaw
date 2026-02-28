//! Test harness for constructing `AgentDeps` with sensible defaults.
//!
//! Provides:
//! - [`StubLlm`]: A configurable LLM provider that returns a fixed response
//! - [`TestHarnessBuilder`]: Builder for wiring `AgentDeps` with defaults
//! - [`TestHarness`]: The assembled components ready for use in tests
//!
//! # Usage
//!
//! ```rust,no_run
//! use ironclaw::testing::TestHarnessBuilder;
//!
//! #[tokio::test]
//! async fn test_something() {
//!     let harness = TestHarnessBuilder::new().build().await;
//!     // use harness.deps, harness.db, etc.
//! }
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::agent::AgentDeps;
use crate::db::Database;
use crate::error::LlmError;
use crate::llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};
use crate::tools::ToolRegistry;

/// Create a libSQL-backed test database in a temporary directory.
///
/// Returns the database and a `TempDir` guard — the database file is
/// deleted when the guard is dropped.
#[cfg(feature = "libsql")]
pub async fn test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
    use crate::db::libsql::LibSqlBackend;

    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&path)
        .await
        .expect("failed to create test LibSqlBackend");
    backend
        .run_migrations()
        .await
        .expect("failed to run migrations");
    (Arc::new(backend) as Arc<dyn Database>, dir)
}

/// What kind of error the stub should produce when failing.
#[derive(Clone, Copy, Debug)]
pub enum StubErrorKind {
    /// Transient/retryable error (`LlmError::RequestFailed`).
    Transient,
    /// Non-transient error (`LlmError::ContextLengthExceeded`).
    NonTransient,
}

/// A configurable LLM provider stub for tests.
///
/// Supports:
/// - Fixed response content
/// - Call counting via [`calls()`](Self::calls)
/// - Runtime failure toggling via [`set_failing()`](Self::set_failing)
/// - Configurable error kinds (transient vs non-transient)
///
/// Use this in tests instead of creating ad-hoc stub implementations.
pub struct StubLlm {
    model_name: String,
    response: String,
    call_count: AtomicU32,
    should_fail: AtomicBool,
    error_kind: StubErrorKind,
}

impl StubLlm {
    /// Create a new stub that returns the given response.
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            model_name: "stub-model".to_string(),
            response: response.into(),
            call_count: AtomicU32::new(0),
            should_fail: AtomicBool::new(false),
            error_kind: StubErrorKind::Transient,
        }
    }

    /// Create a stub that always fails with a transient error.
    pub fn failing(name: impl Into<String>) -> Self {
        Self {
            model_name: name.into(),
            response: String::new(),
            call_count: AtomicU32::new(0),
            should_fail: AtomicBool::new(true),
            error_kind: StubErrorKind::Transient,
        }
    }

    /// Create a stub that always fails with a non-transient error.
    pub fn failing_non_transient(name: impl Into<String>) -> Self {
        Self {
            model_name: name.into(),
            response: String::new(),
            call_count: AtomicU32::new(0),
            should_fail: AtomicBool::new(true),
            error_kind: StubErrorKind::NonTransient,
        }
    }

    /// Set the model name.
    pub fn with_model_name(mut self, name: impl Into<String>) -> Self {
        self.model_name = name.into();
        self
    }

    /// Get the number of times `complete` or `complete_with_tools` was called.
    pub fn calls(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Toggle whether calls should fail at runtime.
    pub fn set_failing(&self, fail: bool) {
        self.should_fail.store(fail, Ordering::Relaxed);
    }

    fn make_error(&self) -> LlmError {
        match self.error_kind {
            StubErrorKind::Transient => LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: "server error".to_string(),
            },
            StubErrorKind::NonTransient => LlmError::ContextLengthExceeded {
                used: 100_000,
                limit: 50_000,
            },
        }
    }
}

impl Default for StubLlm {
    fn default() -> Self {
        Self::new("OK")
    }
}

#[async_trait]
impl LlmProvider for StubLlm {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) {
            return Err(self.make_error());
        }
        Ok(CompletionResponse {
            content: self.response.clone(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
        })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) {
            return Err(self.make_error());
        }
        Ok(ToolCompletionResponse {
            content: Some(self.response.clone()),
            tool_calls: Vec::new(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
        })
    }
}

/// Assembled test components.
pub struct TestHarness {
    /// The agent dependencies, ready for use.
    pub deps: AgentDeps,
    /// Direct reference to the database (as `Arc<dyn Database>`).
    pub db: Arc<dyn Database>,
    /// Temp directory guard — keeps the test database alive. Dropped
    /// automatically when the harness goes out of scope.
    #[cfg(feature = "libsql")]
    _temp_dir: tempfile::TempDir,
}

/// Builder for constructing a [`TestHarness`] with sensible defaults.
///
/// All defaults are designed to work without any external services:
/// - Database: libSQL in a temp directory (real SQL, FTS5, no network)
/// - LLM: `StubLlm` returning "OK"
/// - Safety: permissive config
/// - Tools: builtin tools registered
/// - Hooks: empty registry
/// - Cost guard: no limits
pub struct TestHarnessBuilder {
    db: Option<Arc<dyn Database>>,
    llm: Option<Arc<dyn LlmProvider>>,
    tools: Option<Arc<ToolRegistry>>,
}

impl TestHarnessBuilder {
    /// Create a new builder with all defaults.
    pub fn new() -> Self {
        Self {
            db: None,
            llm: None,
            tools: None,
        }
    }

    /// Override the database backend.
    pub fn with_db(mut self, db: Arc<dyn Database>) -> Self {
        self.db = Some(db);
        self
    }

    /// Override the LLM provider.
    pub fn with_llm(mut self, llm: Arc<dyn LlmProvider>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Override the tool registry.
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Build the harness with defaults applied.
    #[cfg(feature = "libsql")]
    pub async fn build(self) -> TestHarness {
        use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
        use crate::config::{SafetyConfig, SkillsConfig};
        use crate::hooks::HookRegistry;
        use crate::safety::SafetyLayer;

        let (db, temp_dir) = if let Some(db) = self.db {
            // Caller provided a DB; create a dummy temp dir to satisfy the struct.
            let dir = tempfile::tempdir().expect("failed to create temp dir");
            (db, dir)
        } else {
            test_db().await
        };

        let llm: Arc<dyn LlmProvider> = self.llm.unwrap_or_else(|| Arc::new(StubLlm::default()));

        let tools = self.tools.unwrap_or_else(|| {
            let t = Arc::new(ToolRegistry::new());
            t.register_builtin_tools();
            t
        });

        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        }));

        let hooks = Arc::new(HookRegistry::new());

        let cost_guard = Arc::new(CostGuard::new(CostGuardConfig {
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
        }));

        let deps = AgentDeps {
            store: Some(Arc::clone(&db)),
            llm,
            cheap_llm: None,
            safety,
            tools,
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks,
            cost_guard,
            wasm_router: None,
        };

        TestHarness {
            deps,
            db,
            _temp_dir: temp_dir,
        }
    }
}

impl Default for TestHarnessBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_harness_builds_with_defaults() {
        let harness = TestHarnessBuilder::new().build().await;
        assert!(harness.deps.store.is_some());
        assert_eq!(harness.deps.llm.model_name(), "stub-model");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_harness_custom_llm() {
        let custom_llm = Arc::new(StubLlm::new("custom response").with_model_name("my-model"));
        let harness = TestHarnessBuilder::new().with_llm(custom_llm).build().await;
        assert_eq!(harness.deps.llm.model_name(), "my-model");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_harness_db_works() {
        let harness = TestHarnessBuilder::new().build().await;

        let id = harness
            .db
            .create_conversation("test", "user1", None)
            .await
            .expect("create conversation");
        assert!(!id.is_nil());
    }

    // === QA Plan P1 - 2.2: Turn persistence round-trip tests ===

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_conversation_message_round_trip() {
        let harness = TestHarnessBuilder::new().build().await;
        let db = &harness.db;

        let conv_id = db
            .create_conversation("tui", "alice", None)
            .await
            .expect("create conversation");

        // Add several messages in order.
        let m1 = db
            .add_conversation_message(conv_id, "user", "Hello!")
            .await
            .expect("add msg 1");
        let m2 = db
            .add_conversation_message(conv_id, "assistant", "Hi there!")
            .await
            .expect("add msg 2");
        let m3 = db
            .add_conversation_message(conv_id, "user", "How are you?")
            .await
            .expect("add msg 3");

        // IDs must be unique.
        assert_ne!(m1, m2);
        assert_ne!(m2, m3);

        // List messages and verify content + ordering.
        let msgs = db
            .list_conversation_messages(conv_id)
            .await
            .expect("list messages");
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "Hello!");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "Hi there!");
        assert_eq!(msgs[2].role, "user");
        assert_eq!(msgs[2].content, "How are you?");

        // Timestamps should be monotonically non-decreasing.
        assert!(msgs[0].created_at <= msgs[1].created_at);
        assert!(msgs[1].created_at <= msgs[2].created_at);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_conversation_metadata_persistence() {
        let harness = TestHarnessBuilder::new().build().await;
        let db = &harness.db;

        let conv_id = db
            .create_conversation("web", "bob", None)
            .await
            .expect("create conversation");

        // Initially no metadata.
        let meta = db
            .get_conversation_metadata(conv_id)
            .await
            .expect("get metadata");
        // May be None or empty object depending on backend.
        if let Some(m) = &meta {
            assert!(m.is_null() || m.as_object().is_none_or(|o| o.is_empty()));
        }

        // Set a metadata field.
        db.update_conversation_metadata_field(
            conv_id,
            "thread_type",
            &serde_json::json!("assistant"),
        )
        .await
        .expect("set thread_type");

        // Read it back.
        let meta = db
            .get_conversation_metadata(conv_id)
            .await
            .expect("get metadata after update")
            .expect("metadata should exist");
        assert_eq!(meta["thread_type"], "assistant");

        // Update with a second field — first field should still be there.
        db.update_conversation_metadata_field(conv_id, "model", &serde_json::json!("gpt-4"))
            .await
            .expect("set model");

        let meta = db
            .get_conversation_metadata(conv_id)
            .await
            .expect("get metadata after second update")
            .expect("metadata should exist");
        assert_eq!(meta["thread_type"], "assistant");
        assert_eq!(meta["model"], "gpt-4");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_conversation_belongs_to_user() {
        let harness = TestHarnessBuilder::new().build().await;
        let db = &harness.db;

        let conv_id = db
            .create_conversation("tui", "alice", None)
            .await
            .expect("create conversation");

        // Owner check should pass.
        assert!(
            db.conversation_belongs_to_user(conv_id, "alice")
                .await
                .expect("belongs check")
        );

        // Different user should NOT own it.
        assert!(
            !db.conversation_belongs_to_user(conv_id, "mallory")
                .await
                .expect("belongs check other user")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_ensure_conversation_idempotent() {
        let harness = TestHarnessBuilder::new().build().await;
        let db = &harness.db;

        let conv_id = uuid::Uuid::new_v4();

        // ensure_conversation should create the row.
        db.ensure_conversation(conv_id, "web", "carol", None)
            .await
            .expect("ensure first");

        // Calling again with the same ID should not error.
        db.ensure_conversation(conv_id, "web", "carol", None)
            .await
            .expect("ensure second (idempotent)");

        // Should be able to add messages to it.
        let msg_id = db
            .add_conversation_message(conv_id, "user", "test message")
            .await
            .expect("add message to ensured conversation");
        assert!(!msg_id.is_nil());

        // Verify the message is there.
        let msgs = db
            .list_conversation_messages(conv_id)
            .await
            .expect("list messages");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "test message");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_paginated_messages() {
        let harness = TestHarnessBuilder::new().build().await;
        let db = &harness.db;

        let conv_id = db
            .create_conversation("tui", "dave", None)
            .await
            .expect("create conversation");

        // Add messages.
        for i in 0..5 {
            db.add_conversation_message(conv_id, "user", &format!("msg {i}"))
                .await
                .expect("add message");
        }

        // First page with limit 3, no cursor. Returns newest-first.
        let (page1, has_more) = db
            .list_conversation_messages_paginated(conv_id, None, 3)
            .await
            .expect("page 1");
        assert_eq!(page1.len(), 3, "first page should have 3 messages");
        assert!(has_more, "should indicate more messages exist");

        // Verify all messages can be retrieved with a large limit.
        let (all, _) = db
            .list_conversation_messages_paginated(conv_id, None, 100)
            .await
            .expect("all messages");
        assert_eq!(all.len(), 5);

        // Messages are returned oldest-first (ascending created_at).
        for w in all.windows(2) {
            assert!(
                w[0].created_at <= w[1].created_at,
                "messages should be in ascending created_at order"
            );
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_conversations_with_preview() {
        let harness = TestHarnessBuilder::new().build().await;
        let db = &harness.db;

        // Create two conversations for the same user.
        let c1 = db
            .create_conversation("tui", "eve", None)
            .await
            .expect("create c1");
        db.add_conversation_message(c1, "user", "First conversation opener")
            .await
            .expect("add msg to c1");

        let c2 = db
            .create_conversation("tui", "eve", None)
            .await
            .expect("create c2");
        db.add_conversation_message(c2, "user", "Second conversation opener")
            .await
            .expect("add msg to c2");

        // List with preview.
        let summaries = db
            .list_conversations_with_preview("eve", "tui", 10)
            .await
            .expect("list with preview");

        assert_eq!(summaries.len(), 2);
        // Both should have message_count >= 1.
        for s in &summaries {
            assert!(s.message_count >= 1);
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_job_action_persistence() {
        use crate::context::{ActionRecord, JobContext, JobState};

        let harness = TestHarnessBuilder::new().build().await;
        let db = &harness.db;

        let ctx = JobContext::with_user("user1", "Do something", "test task");

        let job_id = ctx.job_id;

        // Save job.
        db.save_job(&ctx).await.expect("save job");

        // Get job back.
        let fetched = db.get_job(job_id).await.expect("get job");
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.job_id, job_id);

        // Save an action.
        let action = ActionRecord {
            id: uuid::Uuid::new_v4(),
            sequence: 1,
            tool_name: "echo".to_string(),
            input: serde_json::json!({"message": "hello"}),
            output_raw: Some("hello".to_string()),
            output_sanitized: None,
            sanitization_warnings: vec![],
            cost: None,
            duration: std::time::Duration::from_millis(42),
            success: true,
            error: None,
            executed_at: chrono::Utc::now(),
        };
        db.save_action(job_id, &action).await.expect("save action");

        // Retrieve actions.
        let actions = db.get_job_actions(job_id).await.expect("get actions");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool_name, "echo");
        assert_eq!(actions[0].output_raw, Some("hello".to_string()));
        assert!(actions[0].success);
        assert_eq!(actions[0].duration, std::time::Duration::from_millis(42));

        // Update job status.
        db.update_job_status(job_id, JobState::Completed, None)
            .await
            .expect("update status");

        let updated = db
            .get_job(job_id)
            .await
            .expect("get updated job")
            .expect("job should exist");
        assert!(matches!(updated.state, JobState::Completed));
    }

    #[tokio::test]
    async fn test_stub_llm_complete() {
        let llm = StubLlm::new("hello world");
        let response = llm
            .complete(CompletionRequest::new(vec![]))
            .await
            .expect("complete");
        assert_eq!(response.content, "hello world");
        assert_eq!(response.finish_reason, FinishReason::Stop);
    }
}
