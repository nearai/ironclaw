//! Focused filesystem range and summary-index contract tests.

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, Entry, InMemoryBackend, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, ScopedPath, TenantId,
    ThreadId, UserId, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, CreateSummaryArtifactRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, MessageContent, SessionThreadError, SessionThreadService,
    SummaryKind, SummaryModelContextPolicy, ThreadMessageId, ThreadMessageRangeRequest,
    ThreadScope,
};

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
