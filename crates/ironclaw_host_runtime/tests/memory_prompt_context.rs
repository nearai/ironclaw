//! Production adapter tests for [`ProductionMemoryPromptContextService`].
//!
//! Uses a mock [`MemoryBackend`] to test scope enforcement, ordering,
//! truncation, error handling, and safe summary sanitization.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId, VirtualPath};
use ironclaw_memory::{
    MemoryBackend, MemoryBackendCapabilities, MemoryContext, MemoryDocumentPath,
    MemoryDocumentScope, MemorySearchRequest, MemorySearchResult,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostErrorKind, ContextProfileId, MemoryPromptContextRequest,
    MemoryPromptContextService,
};
use ironclaw_turns::scope::{TurnActor, TurnScope};

use ironclaw_host_runtime::memory_context::ProductionMemoryPromptContextService;

// ─── Mock MemoryBackend ──────────────────────────────────────────────────

#[derive(Clone)]
enum MockSearchBehavior {
    Results(Vec<MemorySearchResult>),
    Error,
}

struct MockMemoryBackend {
    behavior: MockSearchBehavior,
    /// Records the scope from each search call for assertion.
    captured_scopes: Mutex<Vec<MemoryDocumentScope>>,
}

impl MockMemoryBackend {
    fn with_results(results: Vec<MemorySearchResult>) -> Self {
        Self {
            behavior: MockSearchBehavior::Results(results),
            captured_scopes: Mutex::new(Vec::new()),
        }
    }

    fn with_error() -> Self {
        Self {
            behavior: MockSearchBehavior::Error,
            captured_scopes: Mutex::new(Vec::new()),
        }
    }

    fn captured_scopes(&self) -> Vec<MemoryDocumentScope> {
        self.captured_scopes.lock().unwrap().clone()
    }
}

#[async_trait]
impl MemoryBackend for MockMemoryBackend {
    fn capabilities(&self) -> MemoryBackendCapabilities {
        MemoryBackendCapabilities {
            full_text_search: true,
            vector_search: true,
            ..MemoryBackendCapabilities::default()
        }
    }

    async fn search(
        &self,
        context: &MemoryContext,
        _request: MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        self.captured_scopes
            .lock()
            .unwrap()
            .push(context.scope().clone());

        match &self.behavior {
            MockSearchBehavior::Results(results) => Ok(results.clone()),
            MockSearchBehavior::Error => Err(FilesystemError::Backend {
                path: VirtualPath::new("/memory").unwrap(),
                operation: FilesystemOperation::ReadFile,
                reason: "internal DB error: connection refused at 10.0.0.5:5432".to_string(),
            }),
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

fn make_result(tenant: &str, user: &str, rel_path: &str, score: f32, snippet: &str) -> MemorySearchResult {
    MemorySearchResult {
        path: MemoryDocumentPath::new(tenant, user, None, rel_path).unwrap(),
        score,
        snippet: snippet.to_string(),
        full_text_rank: Some(1),
        vector_rank: None,
    }
}

fn test_request(
    tenant: &str,
    user: &str,
    agent: Option<&str>,
    project: Option<&str>,
    max_snippets: usize,
) -> MemoryPromptContextRequest {
    MemoryPromptContextRequest {
        scope: TurnScope::new(
            TenantId::new(tenant).unwrap(),
            agent.map(|a| AgentId::new(a).unwrap()),
            project.map(|p| ProjectId::new(p).unwrap()),
            ThreadId::new("thread-1").unwrap(),
        ),
        actor: TurnActor::new(UserId::new(user).unwrap()),
        query: "test query".to_string(),
        max_snippets,
        context_profile_id: ContextProfileId::new("default").unwrap(),
    }
}

fn make_service(backend: MockMemoryBackend) -> ProductionMemoryPromptContextService {
    ProductionMemoryPromptContextService::new(Arc::new(backend))
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_memory_returns_empty_snippets() {
    let service = make_service(MockMemoryBackend::with_results(vec![]));
    let result = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn unavailable_backend_returns_host_error_without_leaking_details() {
    let service = make_service(MockMemoryBackend::with_error());
    let err = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap_err();
    assert_eq!(err.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(err.safe_summary, "memory context unavailable");
    // Must not contain raw backend details
    assert!(!err.safe_summary.contains("connection refused"));
    assert!(!err.safe_summary.contains("10.0.0.5"));
    assert!(!err.safe_summary.contains("5432"));
}

#[tokio::test]
async fn cross_tenant_isolation_scope_passed_to_backend() {
    let backend = MockMemoryBackend::with_results(vec![]);
    let backend = Arc::new(backend);
    let service = ProductionMemoryPromptContextService::new(backend.clone());

    // Call with tenant-A
    service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap();

    // Call with tenant-B
    service
        .load_memory_snippets(test_request("tenant-b", "user-x", None, None, 10))
        .await
        .unwrap();

    let scopes = backend.captured_scopes();
    assert_eq!(scopes.len(), 2);
    assert_eq!(scopes[0].tenant_id(), "tenant-a");
    assert_eq!(scopes[1].tenant_id(), "tenant-b");
    assert_ne!(scopes[0], scopes[1], "different tenants must produce different scopes");
}

#[tokio::test]
async fn cross_user_isolation_scope_passed_to_backend() {
    let backend = MockMemoryBackend::with_results(vec![]);
    let backend = Arc::new(backend);
    let service = ProductionMemoryPromptContextService::new(backend.clone());

    service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap();

    service
        .load_memory_snippets(test_request("tenant-a", "user-y", None, None, 10))
        .await
        .unwrap();

    let scopes = backend.captured_scopes();
    assert_eq!(scopes.len(), 2);
    assert_eq!(scopes[0].user_id(), "user-x");
    assert_eq!(scopes[1].user_id(), "user-y");
    assert_ne!(scopes[0], scopes[1], "different users must produce different scopes");
}

#[tokio::test]
async fn agent_and_project_scope_enforcement() {
    let backend = MockMemoryBackend::with_results(vec![]);
    let backend = Arc::new(backend);
    let service = ProductionMemoryPromptContextService::new(backend.clone());

    service
        .load_memory_snippets(test_request(
            "tenant-a",
            "user-x",
            Some("agent-1"),
            Some("project-1"),
            10,
        ))
        .await
        .unwrap();

    let scopes = backend.captured_scopes();
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].agent_id(), Some("agent-1"));
    assert_eq!(scopes[0].project_id(), Some("project-1"));
}

#[tokio::test]
async fn deterministic_ordering_score_desc_then_path_asc() {
    let results = vec![
        make_result("t", "u", "z-note.md", 0.5, "snippet z"),
        make_result("t", "u", "a-note.md", 0.5, "snippet a"),
        make_result("t", "u", "m-note.md", 0.9, "snippet m"),
    ];
    let service = make_service(MockMemoryBackend::with_results(results));

    // Run twice and compare
    let first = service
        .load_memory_snippets(test_request("t", "u", None, None, 10))
        .await
        .unwrap();
    let second = service
        .load_memory_snippets(test_request("t", "u", None, None, 10))
        .await
        .unwrap();

    assert_eq!(first.len(), 3);
    assert_eq!(first, second, "ordering must be deterministic across calls");

    // Highest score first
    assert_eq!(first[0].snippet_ref, "memory:m-note.md");
    // Tied scores: path ascending
    assert_eq!(first[1].snippet_ref, "memory:a-note.md");
    assert_eq!(first[2].snippet_ref, "memory:z-note.md");
}

#[tokio::test]
async fn snippet_truncation_respects_max_snippets() {
    let results = (0..20)
        .map(|i| make_result("t", "u", &format!("note-{i:02}.md"), 1.0 - i as f32 * 0.01, &format!("snippet {i}")))
        .collect();
    let service = make_service(MockMemoryBackend::with_results(results));

    let snippets = service
        .load_memory_snippets(test_request("t", "u", None, None, 5))
        .await
        .unwrap();
    assert!(snippets.len() <= 5);
}

#[tokio::test]
async fn safe_summary_does_not_contain_control_characters() {
    let results = vec![make_result(
        "t",
        "u",
        "note.md",
        1.0,
        "clean\x00text\twith\nnewlines and normal words",
    )];
    let service = make_service(MockMemoryBackend::with_results(results));

    let snippets = service
        .load_memory_snippets(test_request("t", "u", None, None, 10))
        .await
        .unwrap();

    assert_eq!(snippets.len(), 1);
    let summary = &snippets[0].safe_summary;
    assert!(
        !summary.chars().any(|c| c.is_control()),
        "safe_summary must not contain control characters: {summary:?}"
    );
}

#[tokio::test]
async fn safe_summary_does_not_contain_raw_filesystem_paths() {
    // LoopSafeSummary rejects `/` and `\` characters
    let results = vec![make_result(
        "t",
        "u",
        "note.md",
        1.0,
        "/etc/passwd secret file",
    )];
    let service = make_service(MockMemoryBackend::with_results(results));

    let snippets = service
        .load_memory_snippets(test_request("t", "u", None, None, 10))
        .await
        .unwrap();

    // Snippet with path delimiters should be silently dropped
    assert!(
        snippets.is_empty(),
        "snippets with filesystem paths must be filtered out"
    );
}

#[tokio::test]
async fn safe_summary_length_is_bounded() {
    let long_text = "a".repeat(2000);
    let results = vec![make_result("t", "u", "note.md", 1.0, &long_text)];
    let service = make_service(MockMemoryBackend::with_results(results));

    let snippets = service
        .load_memory_snippets(test_request("t", "u", None, None, 10))
        .await
        .unwrap();

    assert_eq!(snippets.len(), 1);
    assert!(
        snippets[0].safe_summary.len() <= 512,
        "safe_summary must be bounded to 512 bytes, got {}",
        snippets[0].safe_summary.len()
    );
}

#[tokio::test]
async fn snippet_ref_has_memory_prefix() {
    let results = vec![make_result("t", "u", "my-note.md", 1.0, "some content")];
    let service = make_service(MockMemoryBackend::with_results(results));

    let snippets = service
        .load_memory_snippets(test_request("t", "u", None, None, 10))
        .await
        .unwrap();

    assert_eq!(snippets.len(), 1);
    assert!(
        snippets[0].snippet_ref.starts_with("memory:"),
        "snippet_ref must start with 'memory:' prefix"
    );
    assert_eq!(snippets[0].snippet_ref, "memory:my-note.md");
}
