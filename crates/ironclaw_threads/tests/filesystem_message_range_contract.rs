//! Focused filesystem range and summary-index contract tests.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError, Filter,
    InMemoryBackend, IndexKey, IndexSpec, OrderedPage, Page, RecordVersion, RootFilesystem,
    ScopedFilesystem, SeqNo, StorageTxn, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, ScopedPath, TenantId,
    ThreadId, UserId, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendFinalizedAssistantMessageRequest, BoundedThreadMessages,
    BoundedThreadMessagesRequest, CreateSummaryArtifactRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, MessageContent, MessageStatus, RedactMessageRequest,
    SessionThreadError, SessionThreadService, SummaryKind, SummaryModelContextPolicy,
    ThreadMessageId, ThreadMessageRangeRequest, ThreadScope,
};

#[tokio::test]
async fn filesystem_store_bounded_read_uses_capped_query_page() {
    let backend = Arc::new(QueryTrackingBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-tail-bound", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("tail-bound");
    let thread_id = ThreadId::new("thread-tail-bound").unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    for index in 0..3 {
        service
            .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: format!("run-{index}"),
                content: MessageContent::text(format!("reply {index}")),
            })
            .await
            .unwrap();
    }
    backend.reset_query_observations();

    let result = service
        .list_thread_messages_bounded(BoundedThreadMessagesRequest {
            scope,
            thread_id,
            max_messages: 2,
            max_bytes: 1024 * 1024,
        })
        .await
        .unwrap();

    assert_eq!(result, BoundedThreadMessages::LimitExceeded);
    assert_eq!(backend.query_pages(), vec![Page::first(3)]);
}

#[tokio::test]
async fn filesystem_store_bounded_read_returns_newest_redacted_row_within_cap() {
    let fixture = RangeFixture::new("fs-bounded-shadow", "tenant-bounded-shadow").await;
    let finalized = fixture
        .service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: "run-shadow".into(),
            content: MessageContent::text("secret answer"),
        })
        .await
        .unwrap();
    fixture
        .service
        .redact_message(RedactMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            message_id: finalized.message_id,
            redaction_ref: "redaction/shadow".into(),
        })
        .await
        .unwrap();

    let result = fixture
        .service
        .list_thread_messages_bounded(BoundedThreadMessagesRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            max_messages: 1,
            max_bytes: 1024 * 1024,
        })
        .await
        .unwrap();

    let BoundedThreadMessages::Complete(messages) = result else {
        panic!("the one redacted message must fit within the logical row cap");
    };
    assert_eq!(messages.history.messages.len(), 1);
    assert_eq!(
        messages.history.messages[0].message_id,
        finalized.message_id
    );
    assert_eq!(messages.history.messages[0].status, MessageStatus::Redacted);
}

#[tokio::test]
async fn filesystem_store_bounded_read_classifies_message_and_byte_budgets() {
    let fixture = RangeFixture::new("fs-bounded", "tenant-bounded").await;
    fixture.seed_messages("event", 3).await;

    let result = fixture
        .service
        .list_thread_messages_bounded(BoundedThreadMessagesRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            max_messages: 2,
            max_bytes: 1024 * 1024,
        })
        .await
        .unwrap();

    assert_eq!(result, BoundedThreadMessages::LimitExceeded);

    let byte_limited = fixture
        .service
        .list_thread_messages_bounded(BoundedThreadMessagesRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            max_messages: 4,
            max_bytes: 1,
        })
        .await
        .unwrap();
    assert_eq!(byte_limited, BoundedThreadMessages::LimitExceeded);

    let complete = fixture
        .service
        .list_thread_messages_bounded(BoundedThreadMessagesRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            max_messages: 4,
            max_bytes: 1024 * 1024,
        })
        .await
        .unwrap();
    let BoundedThreadMessages::Complete(messages) = complete else {
        panic!("messages should fit within the export budget");
    };
    assert_eq!(messages.history.messages.len(), 3);
}

#[tokio::test]
async fn filesystem_store_bounded_read_fails_closed_without_paginated_query() {
    let backend = Arc::new(QueryTrackingBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-no-query", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("no-query");
    let thread_id = ThreadId::new("thread-no-query").unwrap();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("event-no-query".into()),
            content: MessageContent::text("message"),
        })
        .await
        .unwrap();
    backend.reject_queries();

    let error = service
        .list_thread_messages_bounded(BoundedThreadMessagesRequest {
            scope,
            thread_id,
            max_messages: 10,
            max_bytes: 1024 * 1024,
        })
        .await
        .expect_err("unsupported bounded query must remain a backend error");

    assert!(
        matches!(error, SessionThreadError::Backend(ref message) if message.contains("query")),
        "unexpected bounded query error: {error:?}",
    );
    assert_eq!(
        backend.list_dir_calls(),
        0,
        "bounded reads must not materialize an unpaged directory fallback",
    );
}

#[tokio::test]
async fn filesystem_store_range_read_returns_only_requested_sequences() {
    let fixture = RangeFixture::new("fs-range", "tenant-range").await;
    fixture.seed_messages("event", 4).await;

    fixture
        .put_malformed_message("malformed-out-of-range")
        .await;

    let range = fixture.range_sequences(1, 3).await;

    assert_eq!(range, vec![2, 3]);
    assert_eq!(
        fixture.range_contents(1, 3).await,
        vec!["message 2".to_string(), "message 3".to_string()]
    );
}

/// Finalized assistant messages use the same individual-row plus sequence
/// projection shape as every other transcript message.
#[tokio::test]
async fn filesystem_store_range_read_includes_finalized_message_row() {
    let fixture = RangeFixture::new("fs-range-append", "tenant-range-append").await;
    // Two indexed user messages (sequences 1, 2) so the index is non-empty —
    // `list_thread_messages_range_indexed` will not fall back to a full scan.
    fixture.seed_messages("event", 2).await;

    let finalized = fixture
        .service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: "run-append-only".into(),
            content: MessageContent::text("assistant reply"),
        })
        .await
        .unwrap();
    assert_eq!(finalized.sequence, 3);

    assert!(
        fixture.message_file_exists(&finalized.message_id).await,
        "finalized assistant message must be stored as an individual row"
    );

    // The indexed range read resolves the individual finalized message row.
    assert_eq!(fixture.range_sequences(0, 3).await, vec![1, 2, 3]);
    assert_eq!(
        fixture.range_contents(2, 3).await,
        vec!["assistant reply".to_string()]
    );
}

/// The finalized message and its ordered projection are one row, while the
/// run lookup keeps retries idempotent.
#[tokio::test]
async fn filesystem_finalized_message_row_and_projection_are_atomic() {
    let fixture = RangeFixture::new("fs-range-repair", "tenant-range-repair").await;

    let first = fixture
        .service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: "run-repair".into(),
            content: MessageContent::text("assistant reply"),
        })
        .await
        .unwrap();
    assert_eq!(first.sequence, 1);

    let retried = fixture
        .service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            turn_run_id: "run-repair".into(),
            content: MessageContent::text("assistant reply"),
        })
        .await
        .unwrap();
    assert_eq!(retried.message_id, first.message_id);
    assert_eq!(retried.sequence, 1);
    assert_eq!(fixture.range_sequences(0, 1).await, vec![1]);
}

#[tokio::test]
async fn filesystem_store_range_read_stays_available_until_a_gap_is_repaired() {
    let fixture = RangeFixture::new("fs-range-gap", "tenant-range-gap").await;
    fixture.seed_messages("gap-event", 4).await;
    fixture.delete_sequence_index(2).await;

    assert_eq!(fixture.range_sequences(1, 3).await, vec![3]);
    assert_eq!(
        fixture
            .service
            .migrate_transcript_indexes_for_scope(&fixture.scope)
            .await
            .unwrap(),
        4
    );
    assert_eq!(fixture.range_sequences(1, 3).await, vec![2, 3]);
}

#[tokio::test]
async fn filesystem_store_range_read_tolerates_a_leaked_sequence_without_a_message() {
    let fixture = RangeFixture::new("fs-range-leaked-gap", "tenant-range-leaked-gap").await;
    let message_ids = fixture.seed_messages("leaked-gap-event", 4).await;
    fixture.delete_sequence_index(2).await;
    fixture.delete_message(message_ids[1]).await;

    assert_eq!(fixture.range_sequences(1, 3).await, vec![3]);
}

#[tokio::test]
async fn filesystem_store_range_read_clamps_to_thread_sequence_ceiling() {
    let fixture = RangeFixture::new("fs-range-ceiling", "tenant-range-ceiling").await;
    fixture.seed_messages("ceiling-event", 4).await;

    assert_eq!(fixture.range_sequences(0, u64::MAX).await, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn filesystem_store_range_read_tolerates_a_missing_message_row() {
    let fixture = RangeFixture::new("fs-range-missing", "tenant-range-missing").await;
    let message_ids = fixture.seed_messages("missing-event", 4).await;
    fixture.delete_message(message_ids[1]).await;

    assert_eq!(fixture.range_sequences(1, 3).await, vec![3]);
}

#[tokio::test]
async fn filesystem_store_summary_creation_uses_indexed_range_validation() {
    let fixture = RangeFixture::new("fs-summary-range", "tenant-summary-range").await;
    fixture.seed_messages("summary-event", 4).await;
    fixture
        .put_malformed_message("malformed-out-of-range")
        .await;

    let summary = fixture.create_compaction_summary(2, 3).await;

    assert_eq!(summary.start_sequence, 2);
    assert_eq!(summary.end_sequence, 3);
}

#[tokio::test]
async fn filesystem_store_summary_creation_requires_complete_sequence_projection() {
    let fixture = RangeFixture::new("fs-summary-range-gap", "tenant-summary-range-gap").await;
    fixture.seed_messages("summary-gap-event", 4).await;
    fixture.delete_sequence_index(2).await;

    let error = fixture
        .service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: fixture.scope.clone(),
            thread_id: fixture.thread_id.clone(),
            start_sequence: 2,
            end_sequence: 3,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("summary"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SessionThreadError::InvalidSummaryRange {
            start_sequence: 2,
            end_sequence: 3
        }
    ));
}

struct QueryTrackingBackend {
    inner: InMemoryBackend,
    query_pages: Mutex<Vec<Page>>,
    reject_queries: AtomicBool,
    list_dir_calls: AtomicUsize,
}

impl QueryTrackingBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            query_pages: Mutex::new(Vec::new()),
            reject_queries: AtomicBool::new(false),
            list_dir_calls: AtomicUsize::new(0),
        }
    }

    fn reset_query_observations(&self) {
        self.query_pages.lock().unwrap().clear();
    }

    fn query_pages(&self) -> Vec<Page> {
        self.query_pages.lock().unwrap().clone()
    }

    fn reject_queries(&self) {
        self.reject_queries.store(true, Ordering::SeqCst);
        self.list_dir_calls.store(0, Ordering::SeqCst);
    }

    fn list_dir_calls(&self) -> usize {
        self.list_dir_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RootFilesystem for QueryTrackingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.list_dir_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.query_pages.lock().unwrap().push(page);
        if self.reject_queries.load(Ordering::SeqCst) {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::Query,
            });
        }
        self.inner.query(path, filter, page).await
    }

    async fn query_ordered(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: &OrderedPage,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query_ordered(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        self.inner.begin(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.inner.reserve_sequence(path).await
    }
}

struct RangeFixture {
    scoped: Arc<ScopedFilesystem<InMemoryBackend>>,
    service: FilesystemSessionThreadService<InMemoryBackend>,
    scope: ThreadScope,
    thread_id: ThreadId,
    label: &'static str,
}

impl RangeFixture {
    async fn new(label: &'static str, tenant: &str) -> Self {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_threads_fs_at(backend, tenant, "alice");
        let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
        let scope = scope(label);
        let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "actor-a".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        Self {
            scoped,
            service,
            scope,
            thread_id,
            label,
        }
    }

    async fn seed_messages(&self, event_prefix: &str, count: u64) -> Vec<ThreadMessageId> {
        let mut message_ids = Vec::new();
        for index in 1..=count {
            let accepted = self
                .service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: self.scope.clone(),
                    thread_id: self.thread_id.clone(),
                    actor_id: "actor-a".into(),
                    source_binding_id: None,
                    reply_target_binding_id: None,
                    external_event_id: Some(format!("{event_prefix}-{index}")),
                    content: MessageContent::text(format!("message {index}")),
                })
                .await
                .unwrap();
            message_ids.push(accepted.message_id);
        }
        message_ids
    }

    async fn put_malformed_message(&self, name: &str) {
        self.scoped
            .put(
                &self.scope.to_resource_scope(),
                &self.message_path(name),
                Entry::bytes(b"{not-json".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }

    async fn delete_sequence_index(&self, sequence: u64) {
        let range = self.list_range(0, sequence).await;
        let message_id = range
            .messages
            .iter()
            .find(|message| message.sequence == sequence)
            .map(|message| message.message_id)
            .unwrap();
        let path = self.message_path(&message_id.to_string());
        let versioned = self
            .scoped
            .get(&self.scope.to_resource_scope(), &path)
            .await
            .unwrap()
            .unwrap();
        let mut entry = versioned.entry;
        entry.indexed.remove(&IndexKey::new("sequence").unwrap());
        self.scoped
            .put(
                &self.scope.to_resource_scope(),
                &path,
                entry,
                CasExpectation::Version(versioned.version),
            )
            .await
            .unwrap();
    }

    async fn message_file_exists(&self, message_id: &ThreadMessageId) -> bool {
        self.scoped
            .get(
                &self.scope.to_resource_scope(),
                &self.message_path(&message_id.to_string()),
            )
            .await
            .unwrap()
            .is_some()
    }

    async fn delete_message(&self, message_id: ThreadMessageId) {
        self.scoped
            .delete(
                &self.scope.to_resource_scope(),
                &self.message_path(&message_id.to_string()),
            )
            .await
            .unwrap();
    }

    async fn range_sequences(&self, after_sequence: u64, through_sequence: u64) -> Vec<u64> {
        self.list_range(after_sequence, through_sequence)
            .await
            .messages
            .into_iter()
            .map(|message| message.sequence)
            .collect()
    }

    async fn range_contents(&self, after_sequence: u64, through_sequence: u64) -> Vec<String> {
        self.list_range(after_sequence, through_sequence)
            .await
            .messages
            .into_iter()
            .map(|message| message.content.unwrap_or_default())
            .collect()
    }

    async fn create_compaction_summary(
        &self,
        start_sequence: u64,
        end_sequence: u64,
    ) -> ironclaw_threads::SummaryArtifact {
        self.service
            .create_summary_artifact(CreateSummaryArtifactRequest {
                scope: self.scope.clone(),
                thread_id: self.thread_id.clone(),
                start_sequence,
                end_sequence,
                summary_kind: SummaryKind::Compaction,
                content: MessageContent::text("summary"),
                model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
            })
            .await
            .unwrap()
    }

    async fn list_range(
        &self,
        after_sequence: u64,
        through_sequence: u64,
    ) -> ironclaw_threads::ThreadMessageRange {
        self.service
            .list_thread_messages_range(ThreadMessageRangeRequest {
                scope: self.scope.clone(),
                thread_id: self.thread_id.clone(),
                after_sequence,
                through_sequence,
            })
            .await
            .unwrap()
    }

    fn thread_root(&self) -> String {
        format!(
            "/threads/agents/agent-{}/projects/project-{}/owners/user-{}/threads/thread-{}",
            self.label, self.label, self.label, self.label
        )
    }

    fn message_path(&self, name: &str) -> ScopedPath {
        ScopedPath::new(format!("{}/messages/{name}.json", self.thread_root())).unwrap()
    }
}

fn scope(label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
        agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
        project_id: Some(ProjectId::new(format!("project-{label}")).unwrap()),
        owner_user_id: Some(UserId::new(format!("user-{label}")).unwrap()),
        mission_id: None,
    }
}

fn scoped_threads_fs_at<F>(backend: Arc<F>, tenant: &str, user: &str) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let target = format!("/tenants/{tenant}/users/{user}/threads");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("alias"),
        VirtualPath::new(target).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}
