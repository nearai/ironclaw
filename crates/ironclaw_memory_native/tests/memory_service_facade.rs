use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, ThreadId, UserId, VirtualPath};
use ironclaw_memory_native::{
    MemoryBackend, MemoryBackendCapabilities, MemoryContext, MemoryDocumentPath,
    MemorySearchRequest, MemorySearchResult, MemoryServiceErrorKind, MemoryWriteOutcome,
};
use ironclaw_memory_native::{
    MemoryContextProfileId, MemoryInteractionMessage, MemoryInteractionRole, MemoryInvocation,
    MemoryService, MemoryServiceContextRequest, MemoryServiceProfileSetRequest,
    MemoryServiceReadRequest, MemoryServiceRecordRequest, MemoryServiceSearchRequest,
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
async fn native_context_retrieve_filters_cross_scope_results_and_returns_raw_components() {
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
    // The provider returns raw, in-scope candidates with the scope/path
    // components the host needs to hash the reference; it no longer sanitizes,
    // wraps, or hashes itself (that is now host-owned).
    assert_eq!(snippets[0].text, "ordinary planning note");
    assert_eq!(snippets[0].relative_path, "allowed.md");
    assert_eq!(snippets[0].tenant_id, "tenant-native-memory");
    assert_eq!(snippets[0].user_id, "user-native-memory");
    assert_eq!(snippets[0].agent_id, None);
    assert_eq!(snippets[0].project_id, None);
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
    assert_eq!(snippets[0].text, "in scope planning note");
}

#[tokio::test]
async fn native_context_retrieve_scopes_short_term_to_active_thread() {
    // Short-term ("run-local") memory is scoped to the active conversation/thread.
    // The backend returns two in-scope, same-user docs under two different thread
    // prefixes. With `thread_id = Some(thread-a)` on the trusted invocation scope,
    // the provider must retain ONLY the active thread's doc. The long-term lane
    // (thread_id = None, the default `invocation()`) stays unfiltered and is
    // covered by the existing scope-isolation tests above.
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "threads/thread-a/note.md",
                1.0,
                "active thread planning note",
            ),
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "threads/thread-b/note.md",
                0.9,
                "other thread planning note",
            ),
        ],
        fail: false,
    }));

    let mut scoped = invocation();
    scoped.scope.thread_id = Some(ThreadId::new("thread-a").expect("valid thread"));

    let snippets = service
        .retrieve_context(
            scoped,
            MemoryServiceContextRequest {
                query: "planning".to_string(),
                max_snippets: 10,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("short-term context retrieval");

    assert_eq!(
        snippets.len(),
        1,
        "short-term retrieval must scope to the active thread"
    );
    assert_eq!(snippets[0].relative_path, "threads/thread-a/note.md");
    assert_eq!(snippets[0].text, "active thread planning note");
}

#[tokio::test]
async fn native_context_retrieve_excludes_thread_scratch_from_long_term() {
    // Long-term retrieval (no `thread_id` on the invocation scope) is the user's
    // general/durable memory; it must EXCLUDE per-thread short-term scratch
    // (anything under a `threads/<id>/` prefix). With `thread_id = None`, only the
    // general doc survives — the thread-scoped doc is dropped — so the long-term
    // and short-term lanes stay disjoint (no duplicate snippet when the run-level
    // fetch concatenates both lanes).
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "MEMORY.md",
                1.0,
                "durable planning fact",
            ),
            search_result(
                "tenant-native-memory",
                "user-native-memory",
                "threads/thread-a/note.md",
                0.9,
                "ephemeral thread planning note",
            ),
        ],
        fail: false,
    }));

    // `invocation()` carries `thread_id: None` — the long-term lane.
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
        .expect("long-term context retrieval");

    assert_eq!(
        snippets.len(),
        1,
        "long-term retrieval must exclude per-thread short-term scratch"
    );
    assert_eq!(snippets[0].relative_path, "MEMORY.md");
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
    assert_eq!(snippets[0].text, "finite score note");
}

#[tokio::test]
async fn native_context_retrieve_returns_raw_content_for_host_sanitization() {
    // Content safety (dropping path-like / secret / injection snippets) is
    // host-owned post-lift. The provider returns the raw text unchanged; the host
    // (`ironclaw_host_runtime::memory_context`) drops it during admission. This
    // test pins that the provider does NOT pre-filter content.
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

    assert_eq!(snippets.len(), 1);
    assert_eq!(
        snippets[0].text,
        "/etc/passwd should not enter model context"
    );
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
    assert_eq!(snippets[0].text, "snippet m");
    // Tied scores (0.5): path ascending, so `a-note.md` precedes `z-note.md`.
    assert_eq!(snippets[1].text, "snippet a");
    assert_eq!(snippets[2].text, "snippet z");
}

#[tokio::test]
async fn native_context_retrieve_returns_candidates_without_aggregate_byte_budget() {
    // The per-snippet + aggregate model-visible byte budgets moved to the host
    // post-lift. The provider returns every in-scope, ranked candidate up to
    // `max_snippets` (the search limit) without sanitizing, truncating, or
    // re-imposing a byte ceiling — the host
    // (`ironclaw_host_runtime::memory_context`) enforces both budgets. This pins
    // that the provider no longer caps bytes.
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
                max_snippets: 20,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("context retrieval through IronClaw memory facade");

    // All 20 in-scope candidates are returned raw and un-truncated; no provider
    // byte budget trims them.
    assert_eq!(snippets.len(), 20);
    assert!(snippets.iter().all(|snippet| snippet.text == long_text));
}

#[tokio::test]
async fn native_record_interaction_writes_thread_log_and_feeds_short_term_lane() {
    // The native provider STORES the full turn history: `record_interaction`
    // appends the exchange to the thread-scoped short-term doc at
    // `threads/<thread_id>/log.md` (the SAME `threads/<T>/` convention the
    // short-term retrieval lane filters on). A real backend (InMemoryBackend +
    // chunking indexer + FTS) proves the write feeds the read lane end to end.
    let service = NativeMemoryService::from_filesystem(Arc::new(InMemoryBackend::new()), None);

    let mut scoped = invocation();
    scoped.scope.thread_id = Some(ThreadId::new("thread-record").expect("valid thread"));

    let response = service
        .record_interaction(
            scoped.clone(),
            MemoryServiceRecordRequest {
                messages: vec![
                    MemoryInteractionMessage {
                        role: MemoryInteractionRole::User,
                        content: "remember my favorite planning color is teal".to_string(),
                    },
                    MemoryInteractionMessage {
                        role: MemoryInteractionRole::Assistant,
                        content: "noted, your favorite planning color is teal".to_string(),
                    },
                ],
                run_id: Some("run-record-1".to_string()),
                metadata: json!({}),
            },
        )
        .await
        .expect("record_interaction persists the exchange");
    assert!(
        response.recorded,
        "a thread-scoped interaction must be recorded by the native provider"
    );

    // (a) A direct read of the thread log contains BOTH messages verbatim.
    let read = service
        .read(
            scoped.clone(),
            MemoryServiceReadRequest {
                path: "threads/thread-record/log.md".to_string(),
            },
        )
        .await
        .expect("the recorded thread log reads back");
    assert!(
        read.content
            .contains("remember my favorite planning color is teal"),
        "thread log must contain the user message: {:?}",
        read.content
    );
    assert!(
        read.content
            .contains("noted, your favorite planning color is teal"),
        "thread log must contain the assistant reply: {:?}",
        read.content
    );

    // (b) The short-term retrieval lane (thread_id kept) surfaces the recorded
    //     doc — proving the write feeds the short-term read lane inside the
    //     provider, not just a raw file write.
    let snippets = service
        .retrieve_context(
            scoped,
            MemoryServiceContextRequest {
                query: "favorite planning color".to_string(),
                max_snippets: 10,
                context_profile_id: MemoryContextProfileId::new("default").unwrap(),
            },
        )
        .await
        .expect("short-term context retrieval after record");
    assert!(
        snippets.iter().any(
            |snippet| snippet.relative_path == "threads/thread-record/log.md"
                && !snippet.text.is_empty()
        ),
        "short-term lane must surface the recorded thread log: {snippets:?}"
    );
}

#[tokio::test]
async fn native_record_interaction_without_thread_is_noop() {
    // With no `thread_id` on the invocation scope there is no short-term thread
    // subtree to record under, so the native provider degrades to a no-op
    // (recorded=false) rather than erroring or writing to an unscoped path.
    let service = NativeMemoryService::from_filesystem(Arc::new(InMemoryBackend::new()), None);

    // `invocation()` carries `thread_id: None`.
    let response = service
        .record_interaction(
            invocation(),
            MemoryServiceRecordRequest {
                messages: vec![MemoryInteractionMessage {
                    role: MemoryInteractionRole::User,
                    content: "no thread to record under".to_string(),
                }],
                run_id: None,
                metadata: json!({}),
            },
        )
        .await
        .expect("threadless record_interaction must degrade, not error");
    assert!(
        !response.recorded,
        "a threadless interaction must not be recorded"
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
        MemoryBackendCapabilities {
            full_text_search: true,
            ..MemoryBackendCapabilities::default()
        }
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
        MemoryBackendCapabilities {
            file_documents: true,
            ..MemoryBackendCapabilities::default()
        }
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
