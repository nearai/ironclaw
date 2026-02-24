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
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
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
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
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
