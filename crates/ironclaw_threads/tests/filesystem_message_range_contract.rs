//! Focused filesystem range and summary-index contract tests.

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, Entry, InMemoryBackend, IndexKey, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, ScopedPath, TenantId,
    ThreadId, UserId, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendFinalizedAssistantMessageRequest,
    CreateSummaryArtifactRequest, EnsureThreadRequest, FilesystemSessionThreadService,
    MessageContent, SessionThreadError, SessionThreadService, SummaryKind,
    SummaryModelContextPolicy, ThreadMessageId, ThreadMessageRangeRequest, ThreadScope,
};

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
