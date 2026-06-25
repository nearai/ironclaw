//! Production adapter tests for [`ProductionMemoryPromptContextService`].
//!
//! These tests intentionally drive the loop-facing caller and assert that it
//! delegates to the memory service facade with host-derived scope.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_memory::{
    MemoryInvocation, MemoryService, MemoryServiceContextRequest, MemoryServiceContextSnippet,
    MemoryServiceError,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostErrorKind, ContextProfileId, MemoryPromptContextRequest,
    MemoryPromptContextService,
};
use ironclaw_turns::scope::{TurnActor, TurnScope};

use ironclaw_host_runtime::memory_context::ProductionMemoryPromptContextService;

#[derive(Clone)]
enum MockMemoryBehavior {
    Snippets(Vec<MemoryServiceContextSnippet>),
    Error,
}

struct MockMemoryService {
    behavior: MockMemoryBehavior,
    captured: Mutex<Vec<(MemoryInvocation, MemoryServiceContextRequest)>>,
}

impl MockMemoryService {
    fn with_snippets(snippets: Vec<MemoryServiceContextSnippet>) -> Self {
        Self {
            behavior: MockMemoryBehavior::Snippets(snippets),
            captured: Mutex::new(Vec::new()),
        }
    }

    fn with_error() -> Self {
        Self {
            behavior: MockMemoryBehavior::Error,
            captured: Mutex::new(Vec::new()),
        }
    }

    fn captured(&self) -> Vec<(MemoryInvocation, MemoryServiceContextRequest)> {
        self.captured.lock().unwrap().clone()
    }
}

#[async_trait]
impl MemoryService for MockMemoryService {
    async fn retrieve_context(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        self.captured.lock().unwrap().push((invocation, request));
        match &self.behavior {
            MockMemoryBehavior::Snippets(snippets) => Ok(snippets.clone()),
            MockMemoryBehavior::Error => Err(MemoryServiceError::unavailable()),
        }
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

fn make_service(memory_service: Arc<MockMemoryService>) -> ProductionMemoryPromptContextService {
    ProductionMemoryPromptContextService::new(memory_service)
}

#[tokio::test]
async fn empty_memory_returns_empty_snippets() {
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![]));
    let service = make_service(memory_service);
    let result = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn max_snippets_zero_returns_empty_without_memory_service_call() {
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![snippet(
        "memory-snippet:abc",
        "Untrusted memory content: snippet",
    )]));
    let service = make_service(memory_service.clone());

    let snippets = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 0))
        .await
        .unwrap();

    assert!(snippets.is_empty());
    assert!(
        memory_service.captured().is_empty(),
        "max_snippets=0 must not call IronClaw memory"
    );
}

#[tokio::test]
async fn memory_disabled_context_profile_returns_empty_without_memory_service_call() {
    // A memory-disabled context profile must short-circuit to empty at the host,
    // before any provider/memory-service call (privacy + no-op invariant). This
    // restores the pre-lift coverage for the host-side disabled-profile guard.
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![snippet(
        "memory-snippet:abc",
        "Untrusted memory content: snippet",
    )]));
    let service = make_service(memory_service.clone());

    let mut request = test_request("tenant-a", "user-x", None, None, 10);
    request.context_profile_id = ContextProfileId::new("memory_disabled").unwrap();

    let snippets = service.load_memory_snippets(request).await.unwrap();

    assert!(snippets.is_empty());
    assert!(
        memory_service.captured().is_empty(),
        "memory-disabled profile must not call the memory service"
    );
}

#[tokio::test]
async fn unavailable_memory_service_returns_host_error_without_leaking_details() {
    let service = make_service(Arc::new(MockMemoryService::with_error()));
    let err = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap_err();
    assert_eq!(err.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(err.safe_summary, "memory context unavailable");
    assert!(!err.safe_summary.contains("connection refused"));
}

#[tokio::test]
async fn host_derived_scope_is_passed_to_memory_service() {
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![]));
    let service = make_service(memory_service.clone());

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

    let captured = memory_service.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].0.scope.tenant_id.as_str(), "tenant-a");
    assert_eq!(captured[0].0.scope.user_id.as_str(), "user-x");
    assert_eq!(
        captured[0].0.scope.agent_id.as_ref().map(|id| id.as_str()),
        Some("agent-1")
    );
    assert_eq!(
        captured[0]
            .0
            .scope
            .project_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("project-1")
    );
    assert_eq!(captured[0].1.query, "test query");
    assert_eq!(captured[0].1.max_snippets, 10);
    // The caller's context profile must cross the facade unchanged so
    // profile-routing regressions are caught at the request boundary.
    assert_eq!(captured[0].1.context_profile_id.as_str(), "default");
}

#[tokio::test]
async fn memory_service_snippets_are_mapped_to_loop_context_snippets() {
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![snippet(
        "memory-snippet:abc",
        "Untrusted memory content: ordinary planning note",
    )]));
    let service = make_service(memory_service);

    let snippets = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap();

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].snippet_ref, "memory-snippet:abc");
    assert_eq!(
        snippets[0].safe_summary,
        "Untrusted memory content: ordinary planning note"
    );
    assert_eq!(
        snippets[0].model_content,
        "Untrusted memory content: ordinary planning note"
    );
}

#[tokio::test]
async fn adapter_enforces_max_snippets_after_memory_service_returns() {
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![
        snippet("memory-snippet:one", "Untrusted memory content: one"),
        snippet("memory-snippet:two", "Untrusted memory content: two"),
    ]));
    let service = make_service(memory_service);

    let snippets = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 1))
        .await
        .unwrap();

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].snippet_ref, "memory-snippet:one");
}

#[tokio::test]
async fn adapter_drops_unwrapped_or_unsafe_memory_service_snippets() {
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![
        snippet("memory-snippet:clean", "Untrusted memory content: visible"),
        snippet("memory-snippet:raw", "raw provider text"),
        snippet(
            "memory-snippet:path",
            "Untrusted memory content: /etc/passwd should not enter",
        ),
        snippet("memory/snippet:bad", "Untrusted memory content: bad ref"),
        MemoryServiceContextSnippet {
            snippet_ref: "memory-snippet:mismatch".to_string(),
            safe_summary: "Untrusted memory content: safe".to_string(),
            model_content: "Untrusted memory content: different".to_string(),
        },
    ]));
    let service = make_service(memory_service);

    let snippets = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap();

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].snippet_ref, "memory-snippet:clean");
    assert_eq!(
        snippets[0].model_content,
        "Untrusted memory content: visible"
    );
}

#[tokio::test]
async fn adapter_drops_oversized_memory_service_snippets() {
    let memory_service = Arc::new(MockMemoryService::with_snippets(vec![
        snippet(
            "memory-snippet:too-big",
            &format!("Untrusted memory content: {}", "a".repeat(600)),
        ),
        snippet("memory-snippet:small", "Untrusted memory content: small"),
    ]));
    let service = make_service(memory_service);

    let snippets = service
        .load_memory_snippets(test_request("tenant-a", "user-x", None, None, 10))
        .await
        .unwrap();

    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].snippet_ref, "memory-snippet:small");
}

fn snippet(snippet_ref: &str, content: &str) -> MemoryServiceContextSnippet {
    MemoryServiceContextSnippet {
        snippet_ref: snippet_ref.to_string(),
        safe_summary: content.to_string(),
        model_content: content.to_string(),
    }
}
