use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId, VirtualPath};
use ironclaw_memory_native::{
    MemoryBackend, MemoryBackendCapabilities, MemoryContext, MemoryDocumentPath,
    MemorySearchRequest, MemorySearchResult, MemoryServiceErrorKind, MemoryWriteOutcome,
};
use ironclaw_memory_native::{
    MemoryContextProfileId, MemoryInvocation, MemoryService, MemoryServiceContextRequest,
    MemoryServiceProfileSetRequest, MemoryServiceReadRequest, MemoryServiceSearchRequest,
    MemoryServiceTreeRequest, MemoryServiceWriteRequest, NativeMemoryService,
};
use serde_json::{Value, json};

fn invocation() -> MemoryInvocation {
    MemoryInvocation {
        scope: ResourceScope {
            tenant_id: TenantId::new("tenant-native-memory").unwrap(),
            user_id: UserId::new("user-native-memory").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        correlation_id: ironclaw_host_api::CorrelationId::new(),
    }
}

#[tokio::test]
async fn native_provider_reads_writes_lists_and_searches_through_memory_service() {
    let service = NativeMemoryService::from_filesystem(Arc::new(InMemoryBackend::new()), None);
    let invocation = invocation();

    let write = service
        .write(
            invocation.clone(),
            MemoryServiceWriteRequest {
                target: "notes/alpha.md".to_string(),
                content: "alpha native IronClaw memory marker".to_string(),
                append: false,
                old_string: None,
                new_string: None,
                replace_all: false,
                metadata: None,
                timezone: None,
            },
        )
        .await
        .expect("write through IronClaw memory facade");
    assert_eq!(write.path, "notes/alpha.md");

    let read = service
        .read(
            invocation.clone(),
            MemoryServiceReadRequest {
                path: "notes/alpha.md".to_string(),
            },
        )
        .await
        .expect("read through IronClaw memory facade");
    assert_eq!(read.content, "alpha native IronClaw memory marker");

    let tree = service
        .tree(
            invocation.clone(),
            MemoryServiceTreeRequest {
                path: String::new(),
                depth: 2,
            },
        )
        .await
        .expect("tree through IronClaw memory facade");
    assert!(
        serde_json::to_string(&tree.entries)
            .expect("tree serializes")
            .contains("alpha.md")
    );

    let search = service
        .search(
            invocation,
            MemoryServiceSearchRequest {
                query: "native IronClaw memory marker".to_string(),
                limit: 5,
            },
        )
        .await
        .expect("search through IronClaw memory facade");
    assert_eq!(search.results.len(), 1);
    assert_eq!(search.results[0].path, "notes/alpha.md");
}

#[tokio::test]
async fn native_context_retrieve_filters_cross_scope_results_and_hashes_snippet_refs() {
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "allowed.md",
                1.0,
                "ordinary planning note",
            ),
            search_result(
                "other-tenant",
                "user-native-memory",
                "leak.md",
                0.9,
                "tenant leak",
            ),
        ],
        fail: false,
    }));

    let snippets = service
        .retrieve_context(
            invocation(),
            MemoryServiceContextRequest {
                query: "planning".to_string(),
                max_snippets: 10,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("context retrieval through IronClaw memory facade");

    assert_eq!(snippets.len(), 1);
    assert_eq!(
        snippets[0].safe_summary,
        "Untrusted memory content: ordinary planning note"
    );
    assert_eq!(snippets[0].snippet_ref, "memory-snippet:cb96ed00b13e6ae4");
}

#[tokio::test]
async fn native_context_retrieve_filters_out_of_scope_tenant_user_agent_and_project() {
    // The request scope is (tenant-native-memory, user-native-memory, no agent,
    // no project) from `invocation()`. The backend returns one in-scope result
    // plus four results that each differ on exactly one scope axis. The
    // provider-side `retain` in `retrieve_context` is solely responsible for
    // dropping every cross-scope result; if it were removed, all five would
    // survive and the `len() == 1` assertion below would fail.
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "allowed.md",
                1.0,
                "in scope planning note",
            ),
            // Different tenant — must be dropped.
            search_result(
                "other-tenant",
                "user-native-memory",
                "wrong-tenant.md",
                0.95,
                "tenant leak",
            ),
            // Different user — must be dropped.
            search_result(
                "tenant-native-memory",
                "other-user",
                "wrong-user.md",
                0.9,
                "user leak",
            ),
            // Different agent (request has none) — must be dropped.
            search_result_with_agent(
                "tenant-native-memory",
                "user-native-memory",
                Some("agent-other"),
                None,
                "wrong-agent.md",
                0.85,
                "agent leak",
            ),
            // Different project (request has none) — must be dropped.
            search_result_with_agent(
                "tenant-native-memory",
                "user-native-memory",
                None,
                Some("project-other"),
                "wrong-project.md",
                0.8,
                "project leak",
            ),
        ],
        fail: false,
    }));

    let snippets = service
        .retrieve_context(
            invocation(),
            MemoryServiceContextRequest {
                query: "planning".to_string(),
                max_snippets: 10,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("context retrieval through IronClaw memory facade");

    // Only the exactly-in-scope result survives the scope-isolation filter.
    assert_eq!(snippets.len(), 1);
    assert_eq!(
        snippets[0].safe_summary,
        "Untrusted memory content: in scope planning note"
    );
}

#[tokio::test]
async fn native_context_retrieve_filters_non_finite_scores_before_ordering() {
    // The backend returns three in-scope results: two with non-finite scores
    // (NaN and +inf) and one finite. The provider-side `retain` in
    // `retrieve_context` drops the non-finite ones via `score.is_finite()`;
    // if that predicate were removed, all three would survive (and NaN ordering
    // would be ill-defined), so the `len() == 1` assertion below depends on it.
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "nan.md",
                f32::NAN,
                "nan score note",
            ),
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "inf.md",
                f32::INFINITY,
                "infinite score note",
            ),
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "finite.md",
                0.5,
                "finite score note",
            ),
        ],
        fail: false,
    }));

    let snippets = service
        .retrieve_context(
            invocation(),
            MemoryServiceContextRequest {
                query: "score".to_string(),
                max_snippets: 10,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("context retrieval through IronClaw memory facade");

    // Only the result with a finite score survives.
    assert_eq!(snippets.len(), 1);
    assert_eq!(
        snippets[0].safe_summary,
        "Untrusted memory content: finite score note"
    );
}

#[tokio::test]
async fn native_context_retrieve_drops_path_like_snippets() {
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![search_result(
            "tenant-native-memory",
            "user-native-memory",
            "path.md",
            1.0,
            "/etc/passwd should not enter model context",
        )],
        fail: false,
    }));

    let snippets = service
        .retrieve_context(
            invocation(),
            MemoryServiceContextRequest {
                query: "path".to_string(),
                max_snippets: 10,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("context retrieval through IronClaw memory facade");

    assert!(snippets.is_empty());
}

#[tokio::test]
async fn native_context_retrieve_orders_score_desc_then_path_asc() {
    // Ordering facade test, ported from the pre-lift
    // `deterministic_ordering_score_desc_then_path_asc`. It drives
    // `retrieve_context`, whose `results.sort_by(compare_memory_search_results)`
    // is solely responsible for the ordering. Two of the three in-scope results
    // share the same score (0.5) to force the path-ascending tie-break; if the
    // sort were removed or its key inverted, the assertions below would fail.
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![
            // Deliberately seeded out of final order so the sort has work to do.
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "z-note.md",
                0.5,
                "snippet z",
            ),
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "a-note.md",
                0.5,
                "snippet a",
            ),
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "m-note.md",
                0.9,
                "snippet m",
            ),
        ],
        fail: false,
    }));

    let snippets = service
        .retrieve_context(
            invocation(),
            MemoryServiceContextRequest {
                query: "snippet".to_string(),
                max_snippets: 10,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("context retrieval through IronClaw memory facade");

    assert_eq!(snippets.len(), 3);
    // Highest score first.
    assert_eq!(
        snippets[0].safe_summary,
        "Untrusted memory content: snippet m"
    );
    // Tied scores (0.5): path ascending, so `a-note.md` precedes `z-note.md`.
    assert_eq!(
        snippets[1].safe_summary,
        "Untrusted memory content: snippet a"
    );
    assert_eq!(
        snippets[2].safe_summary,
        "Untrusted memory content: snippet z"
    );
}

#[tokio::test]
async fn native_context_retrieve_caps_aggregate_safe_summary_bytes() {
    // Aggregate-budget facade test, ported from the pre-lift
    // `aggregate_safe_summary_bytes_are_bounded`. It drives `retrieve_context`,
    // which calls `collect_context_snippets(.., MAX_TOTAL_SAFE_SUMMARY_BYTES)`.
    // Twenty in-scope results each carry a long snippet (~512 bytes after the
    // per-snippet cap), so the cumulative safe-summary bytes blow past the 4 KiB
    // aggregate ceiling well before `max_snippets`. The aggregate cap — not
    // `max_snippets` — must stop collection. If the byte budget were removed,
    // all 20 would be returned and both assertions below would fail.
    let long_text = "b".repeat(1000);
    let results = (0..20)
        .map(|index| {
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                &format!("note-{index:02}.md"),
                1.0,
                &long_text,
            )
        })
        .collect();
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results,
        fail: false,
    }));

    let snippets = service
        .retrieve_context(
            invocation(),
            MemoryServiceContextRequest {
                query: "budget".to_string(),
                // High enough that the aggregate byte budget, not max_snippets,
                // is what truncates the returned set.
                max_snippets: 20,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("context retrieval through IronClaw memory facade");

    let total_bytes: usize = snippets
        .iter()
        .map(|snippet| snippet.safe_summary.len())
        .sum();
    assert!(
        total_bytes <= 4 * 1024,
        "aggregate safe_summary bytes must stay within the 4 KiB ceiling, got {total_bytes}"
    );
    assert!(
        snippets.len() < 20,
        "aggregate byte budget must cap snippets before max_snippets, got {}",
        snippets.len()
    );
}

#[tokio::test]
async fn native_profile_set_persists_profile_document() {
    let service = NativeMemoryService::from_filesystem(Arc::new(InMemoryBackend::new()), None);
    service
        .profile_set(
            invocation(),
            profile_request(json!({
                "timezone": "America/Toronto",
                "locale": "en-CA",
                "location": "Toronto"
            })),
        )
        .await
        .expect("profile_set persists profile");

    let profile = read_profile(&service).await;
    assert_eq!(profile["timezone"], json!("America/Toronto"));
    assert_eq!(profile["locale"], json!("en-CA"));
    assert_eq!(profile["location"], json!("Toronto"));
}

#[tokio::test]
async fn native_profile_set_merges_without_clobbering_existing_fields() {
    let service = NativeMemoryService::from_filesystem(Arc::new(InMemoryBackend::new()), None);
    service
        .profile_set(
            invocation(),
            profile_request(json!({
                "timezone": "America/Toronto",
                "locale": "en-CA"
            })),
        )
        .await
        .expect("initial profile_set persists profile");
    service
        .profile_set(
            invocation(),
            profile_request(json!({
                "location": "Toronto"
            })),
        )
        .await
        .expect("second profile_set merges profile");

    let profile = read_profile(&service).await;
    assert_eq!(profile["timezone"], json!("America/Toronto"));
    assert_eq!(profile["locale"], json!("en-CA"));
    assert_eq!(profile["location"], json!("Toronto"));
}

#[tokio::test]
async fn native_profile_set_rejects_non_json_profile_document() {
    let service = NativeMemoryService::from_filesystem(Arc::new(InMemoryBackend::new()), None);
    write_raw_profile(&service, "not json").await;

    let error = service
        .profile_set(invocation(), profile_request(json!({"locale": "en-CA"})))
        .await
        .expect_err("non-json profile must fail closed");

    assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);
}

#[tokio::test]
async fn native_profile_set_rejects_corrupt_known_profile_fields() {
    let service = NativeMemoryService::from_filesystem(Arc::new(InMemoryBackend::new()), None);
    write_raw_profile(&service, r#"{"timezone":42,"nickname":"Ben"}"#).await;

    let error = service
        .profile_set(invocation(), profile_request(json!({"locale": "en-CA"})))
        .await
        .expect_err("corrupt known profile fields must fail closed");

    assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);
}

#[tokio::test]
async fn native_profile_set_returns_operation_error_after_cas_exhaustion() {
    let service = NativeMemoryService::new(Arc::new(AlwaysConflictProfileBackend));

    let error = service
        .profile_set(invocation(), profile_request(json!({"locale": "en-CA"})))
        .await
        .expect_err("CAS exhaustion must fail closed");

    assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);
}

struct MockSearchBackend {
    results: Vec<MemorySearchResult>,
    fail: bool,
}

/// Minimal `tree`-only backend: returns an arbitrary set of
/// `MemoryDocumentPath`s from `list_documents` so the test can prove that
struct AlwaysConflictProfileBackend;

#[async_trait]
impl MemoryBackend for MockSearchBackend {
    fn capabilities(&self) -> MemoryBackendCapabilities {
        MemoryBackendCapabilities::default().set_full_text_search(true)
    }

    async fn search(
        &self,
        _context: &MemoryContext,
        _request: MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        if self.fail {
            return Err(FilesystemError::Backend {
                path: VirtualPath::new("/memory").unwrap(),
                operation: FilesystemOperation::ReadFile,
                reason: "search failed".to_string(),
            });
        }
        Ok(self.results.clone())
    }
}

#[async_trait]
impl MemoryBackend for AlwaysConflictProfileBackend {
    fn capabilities(&self) -> MemoryBackendCapabilities {
        MemoryBackendCapabilities::default().set_file_documents(true)
    }

    async fn read_document(
        &self,
        _context: &MemoryContext,
        _path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        Ok(None)
    }

    async fn compare_and_write_document_with_backend_options(
        &self,
        _context: &MemoryContext,
        _path: &MemoryDocumentPath,
        _expected_previous_hash: Option<&str>,
        _bytes: &[u8],
        _backend_options: &ironclaw_memory_native::MemoryBackendWriteOptions,
    ) -> Result<MemoryWriteOutcome, FilesystemError> {
        Ok(MemoryWriteOutcome::Conflict)
    }
}

fn search_result(
    tenant: &str,
    user: &str,
    path: &str,
    score: f32,
    snippet: &str,
) -> MemorySearchResult {
    search_result_with_agent(tenant, user, None, None, path, score, snippet)
}

fn search_result_with_agent(
    tenant: &str,
    user: &str,
    agent: Option<&str>,
    project: Option<&str>,
    path: &str,
    score: f32,
    snippet: &str,
) -> MemorySearchResult {
    MemorySearchResult {
        path: MemoryDocumentPath::new_with_agent(tenant, user, agent, project, path).unwrap(),
        score,
        snippet: snippet.to_string(),
        full_text_rank: Some(1),
        vector_rank: None,
    }
}

fn profile_request(input: Value) -> MemoryServiceProfileSetRequest {
    MemoryServiceProfileSetRequest::from_tool_input(&input).expect("valid profile input")
}

async fn read_profile(service: &NativeMemoryService) -> Value {
    let profile = service
        .read(
            invocation(),
            MemoryServiceReadRequest {
                path: "context/profile.json".to_string(),
            },
        )
        .await
        .expect("profile document reads");
    serde_json::from_str(&profile.content).expect("profile is json")
}

async fn write_raw_profile(service: &NativeMemoryService, content: &str) {
    service
        .write(
            invocation(),
            MemoryServiceWriteRequest {
                target: "context/profile.json".to_string(),
                content: content.to_string(),
                append: false,
                old_string: None,
                new_string: None,
                replace_all: false,
                metadata: None,
                timezone: None,
            },
        )
        .await
        .expect("raw profile document writes");
}
