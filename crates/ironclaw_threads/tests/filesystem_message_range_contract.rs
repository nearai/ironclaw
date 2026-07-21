//! Focused filesystem range and summary-index contract tests.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, EventRecord, FileStat, FilesystemError,
    Filter, InMemoryBackend, Page, RecordVersion, RootFilesystem, ScopedFilesystem, SeqNo,
    VersionedEntry,
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
async fn filesystem_store_bounded_read_uses_bounded_append_tail() {
    let backend = Arc::new(TailTrackingBackend::new());
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
    backend.reset_tail_observations();

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
    assert_eq!(backend.tail_calls(), 0, "bounded reads must not call tail");
    assert_eq!(backend.tail_bounded_limits(), vec![3]);
}

#[tokio::test]
async fn filesystem_store_bounded_read_deduplicates_shadowed_append_before_message_cap() {
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
        panic!("the one logical message must fit despite its stale append event");
    };
    assert_eq!(messages.messages.len(), 1);
    assert_eq!(messages.messages[0].message_id, finalized.message_id);
    assert_eq!(messages.messages[0].status, MessageStatus::Redacted);
}

#[tokio::test]
async fn filesystem_store_bounded_read_rejects_before_materializing_the_full_thread() {
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
    assert_eq!(messages.messages.len(), 3);
}

#[tokio::test]
async fn filesystem_store_range_read_returns_only_requested_sequences() {
    let fixture = RangeFixture::new("fs-range", "tenant-range").await;
    fixture.seed_messages("event", 4).await;

    assert_eq!(
        fixture.index_entry_names().await,
        vec![
            "00000000000000000001.json",
            "00000000000000000002.json",
            "00000000000000000003.json",
            "00000000000000000004.json",
        ]
    );
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

/// A finalized assistant message stored only via the append log (no
/// per-message file) must still be written into the sequence index, otherwise
/// indexed range reads — which back `list_thread_messages_range`, summaries,
/// and compaction — would silently omit it from threads that also have
/// indexed messages.
#[tokio::test]
async fn filesystem_store_range_read_includes_append_only_finalized_message() {
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

    // The append-only branch must have actually run: the finalized message has
    // no per-message file (it lives solely in the append log). Without this
    // guard the test would still pass if `append_message_event` returned false
    // and `write_new_message` created the normal per-message file instead.
    assert!(
        !fixture.message_file_exists(&finalized.message_id).await,
        "finalized assistant message must be append-only (no per-message file)"
    );

    // The append-only finalize path must have written the sequence index entry.
    assert_eq!(
        fixture.index_entry_names().await,
        vec![
            "00000000000000000001.json",
            "00000000000000000002.json",
            "00000000000000000003.json",
        ]
    );

    // The indexed range read includes the append-only finalized message (its id
    // resolves through `read_message_versioned`'s append-log fallback).
    assert_eq!(fixture.range_sequences(0, 3).await, vec![1, 2, 3]);
    assert_eq!(
        fixture.range_contents(2, 3).await,
        vec!["assistant reply".to_string()]
    );
}

/// If a finalized assistant message was appended to the log but the process
/// died before its sequence index was written, an idempotent retry (same
/// `turn_run_id`) must repair the missing index rather than returning the
/// already-finalized message with no indexed entry — otherwise a durable LLM
/// message stays invisible to indexed range reads.
#[tokio::test]
async fn filesystem_append_finalized_assistant_message_retry_repairs_missing_sequence_index() {
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
    assert_eq!(
        fixture.index_entry_names().await,
        vec!["00000000000000000001.json"]
    );

    // Simulate the partial-persistence failure: the append-log event survived,
    // but the sequence index entry is gone.
    fixture.delete_sequence_index(1).await;
    assert!(fixture.index_entry_names().await.is_empty());

    // Idempotent retry with the same turn_run_id resolves the finalized
    // message via the append-log fallback and must repair the index.
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
    assert_eq!(
        fixture.index_entry_names().await,
        vec!["00000000000000000001.json"],
        "idempotent retry must repair the missing sequence index"
    );

    // The repaired index makes the message visible to indexed range reads.
    assert_eq!(fixture.range_sequences(0, 1).await, vec![1]);
}

#[tokio::test]
async fn filesystem_store_range_read_falls_back_when_sequence_index_has_gap() {
    let fixture = RangeFixture::new("fs-range-gap", "tenant-range-gap").await;
    fixture.seed_messages("gap-event", 4).await;
    fixture.delete_sequence_index(2).await;

    assert_eq!(fixture.range_sequences(1, 3).await, vec![2, 3]);
    assert_eq!(
        fixture.range_contents(1, 3).await,
        vec!["message 2".to_string(), "message 3".to_string()]
    );
}

#[tokio::test]
async fn filesystem_store_range_read_clamps_to_thread_sequence_ceiling() {
    let fixture = RangeFixture::new("fs-range-ceiling", "tenant-range-ceiling").await;
    fixture.seed_messages("ceiling-event", 4).await;

    assert_eq!(fixture.range_sequences(0, u64::MAX).await, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn filesystem_store_range_read_errors_when_indexed_message_is_missing() {
    let fixture = RangeFixture::new("fs-range-missing", "tenant-range-missing").await;
    let message_ids = fixture.seed_messages("missing-event", 4).await;
    fixture.delete_message(message_ids[1]).await;

    let err = fixture.range_error(1, 3).await;

    assert!(matches!(
        err,
        SessionThreadError::UnknownMessage { message_id } if message_id == message_ids[1]
    ));
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
async fn filesystem_store_summary_creation_falls_back_when_sequence_index_has_gap() {
    let fixture = RangeFixture::new("fs-summary-range-gap", "tenant-summary-range-gap").await;
    fixture.seed_messages("summary-gap-event", 4).await;
    fixture.delete_sequence_index(2).await;

    let summary = fixture.create_compaction_summary(2, 3).await;

    assert_eq!(summary.start_sequence, 2);
    assert_eq!(summary.end_sequence, 3);
}

struct TailTrackingBackend {
    inner: InMemoryBackend,
    tail_calls: AtomicUsize,
    tail_bounded_limits: Mutex<Vec<usize>>,
}

impl TailTrackingBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            tail_calls: AtomicUsize::new(0),
            tail_bounded_limits: Mutex::new(Vec::new()),
        }
    }

    fn reset_tail_observations(&self) {
        self.tail_calls.store(0, Ordering::SeqCst);
        self.tail_bounded_limits.lock().unwrap().clear();
    }

    fn tail_calls(&self) -> usize {
        self.tail_calls.load(Ordering::SeqCst)
    }

    fn tail_bounded_limits(&self) -> Vec<usize> {
        self.tail_bounded_limits.lock().unwrap().clone()
    }
}

#[async_trait]
impl RootFilesystem for TailTrackingBackend {
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
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.inner.append(path, payload).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.tail_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.tail_bounded_limits.lock().unwrap().push(max_records);
        self.inner.tail_bounded(path, from, max_records).await
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

    async fn index_entry_names(&self) -> Vec<String> {
        let mut names = self
            .scoped
            .list_dir(&self.scope.to_resource_scope(), &self.sequence_root())
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
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
        self.scoped
            .delete(
                &self.scope.to_resource_scope(),
                &self.sequence_index_path(sequence),
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

    async fn range_error(&self, after_sequence: u64, through_sequence: u64) -> SessionThreadError {
        self.service
            .list_thread_messages_range(ThreadMessageRangeRequest {
                scope: self.scope.clone(),
                thread_id: self.thread_id.clone(),
                after_sequence,
                through_sequence,
            })
            .await
            .unwrap_err()
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

    fn sequence_root(&self) -> ScopedPath {
        ScopedPath::new(format!("{}/messages_by_sequence", self.thread_root())).unwrap()
    }

    fn sequence_index_path(&self, sequence: u64) -> ScopedPath {
        ScopedPath::new(format!(
            "{}/messages_by_sequence/{sequence:020}.json",
            self.thread_root()
        ))
        .unwrap()
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
