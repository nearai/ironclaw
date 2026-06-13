//! Contract tests for [`FilesystemSessionThreadService`].
//!
//! Drives the production filesystem-backed store over an
//! [`InMemoryBackend`] composed under a `/threads` mount alias whose
//! `VirtualPath` target encodes a tenant/user prefix. Mirrors the shape of
//! the run-state and processes filesystem contract suites — see
//! `crates/ironclaw_run_state/tests/run_state_contract.rs` and
//! `crates/ironclaw_processes/tests/process_store_contract.rs`.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
    ProjectId, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AttachmentKind, AttachmentRef,
    CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewEnvelopeInput,
    CapabilityDisplayPreviewStatus, CreateSummaryArtifactRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, LoadContextMessagesRequest, LoadContextWindowRequest,
    MessageContent, MessageKind, MessageStatus, RedactMessageRequest,
    ReplayAcceptedInboundMessageRequest, SessionThreadError, SessionThreadService, SummaryKind,
    SummaryModelContextPolicy, ThreadHistoryRequest, ThreadScope, UpdateAssistantDraftRequest,
};

#[tokio::test]
async fn filesystem_delete_thread_removes_owned_thread_and_hides_missing_or_wrong_scope() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-delete", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let owned_scope = scope("delete-owned");
    let wrong_scope = scope("delete-wrong");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: owned_scope.clone(),
            thread_id: Some(ThreadId::new("thread-delete-owned").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let wrong_scope_error = service
        .delete_thread(&wrong_scope, &thread.thread_id)
        .await
        .expect_err("wrong-scope delete should hide thread existence");
    assert_unknown_thread(wrong_scope_error, &thread.thread_id);

    service
        .read_thread(ThreadHistoryRequest {
            scope: owned_scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .expect("wrong-scope delete must not remove owned thread");

    service
        .delete_thread(&owned_scope, &thread.thread_id)
        .await
        .expect("owned delete succeeds");

    let deleted_error = service
        .read_thread(ThreadHistoryRequest {
            scope: owned_scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .expect_err("deleted thread should no longer be readable");
    assert_unknown_thread(deleted_error, &thread.thread_id);

    let repeat_error = service
        .delete_thread(&owned_scope, &thread.thread_id)
        .await
        .expect_err("repeat delete should be non-enumerating missing shape");
    assert_unknown_thread(repeat_error, &thread.thread_id);

    let missing = ThreadId::new("thread-delete-missing").unwrap();
    let missing_error = service
        .delete_thread(&owned_scope, &missing)
        .await
        .expect_err("missing delete should be non-enumerating");
    assert_unknown_thread(missing_error, &missing);
}

#[tokio::test]
async fn filesystem_delete_thread_removes_inbound_idempotency_records() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-delete-idempotency", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("delete-idempotency");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-delete-idempotency").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding-delete-idempotency".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-delete-idempotency".into()),
            content: MessageContent::text("delete me"),
        })
        .await
        .unwrap();

    service
        .delete_thread(&request_scope, &thread.thread_id)
        .await
        .expect("owned delete succeeds");

    let replay = service
        .replay_accepted_inbound_message(ReplayAcceptedInboundMessageRequest {
            scope: request_scope,
            actor_id: "actor-a".into(),
            source_binding_id: "binding-delete-idempotency".into(),
            external_event_id: "event-delete-idempotency".into(),
        })
        .await
        .expect("deleted thread must not leave stale idempotency records");

    assert!(replay.is_none());
}

#[tokio::test]
async fn durable_history_round_trips_through_filesystem_store() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let label = "fs-round-trip";
    let thread_id = durable_history_flow(&service, label).await;

    // Restart-equivalent: drop the service + scoped fs, build a new pair
    // pointed at the same backend with the same MountView. Records must
    // rehydrate without loss.
    let scoped = scoped_threads_fs_at(backend, "tenant-a", "alice");
    let reopened = FilesystemSessionThreadService::new(scoped);
    assert_reopened_history(&reopened, label, thread_id).await;
}

#[tokio::test]
async fn filesystem_store_rejects_wrong_scope_history_reads() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-a", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("rejected");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-rejected").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let wrong_scope = scope("rejected-other");

    let err = service
        .list_thread_history(ThreadHistoryRequest {
            scope: wrong_scope,
            thread_id: thread.thread_id,
        })
        .await;

    assert!(err.is_err(), "wrong-scope history lookup must fail closed");
}

#[tokio::test]
async fn filesystem_store_persists_preview_history_while_hiding_it_from_context() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-preview", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-preview");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-preview").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("run a tool"),
        })
        .await
        .unwrap();

    let invocation_id = InvocationId::new();
    let first = service
        .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            preview: preview_envelope(invocation_id),
        })
        .await
        .unwrap();
    let duplicate = service
        .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            preview: preview_envelope(invocation_id),
        })
        .await
        .unwrap();
    assert_eq!(first.message_id, duplicate.message_id);

    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("summary must not replace preview range"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .map(|message| message.kind)
            .collect::<Vec<_>>(),
        vec![MessageKind::User, MessageKind::CapabilityDisplayPreview]
    );

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            max_messages: 10,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 1);
    assert_eq!(context.messages[0].kind, MessageKind::User);

    let direct_context = service
        .load_context_messages(LoadContextMessagesRequest {
            scope,
            thread_id: thread.thread_id,
            message_ids: vec![first.message_id],
        })
        .await
        .unwrap();
    assert!(direct_context.messages.is_empty());
}

#[tokio::test]
async fn filesystem_store_exact_compaction_replacement_summary_replay_is_idempotent() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-summary", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let thread_scope = scope("fs-summary");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    for text in ["one", "two"] {
        service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(text),
            })
            .await
            .unwrap();
    }

    let first = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();
    let replay = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();
    assert_eq!(replay.summary_id, first.summary_id);

    let changed_content = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("different summary"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await;
    assert!(matches!(
        changed_content,
        Err(SessionThreadError::OverlappingSummaryRange { .. })
    ));

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.summary_artifacts.len(), 1);
    assert_eq!(history.summary_artifacts[0].summary_id, first.summary_id);
}

#[tokio::test]
async fn filesystem_store_overlapping_replacement_summaries_are_rejected() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-overlap", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let thread_scope = scope("fs-overlap");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: thread_scope.clone(),
            thread_id: None,
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    for text in ["one", "two", "three"] {
        service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-a".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(text),
            })
            .await
            .unwrap();
    }
    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("one and two summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    let overlapping = service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope,
            thread_id: thread.thread_id,
            start_sequence: 2,
            end_sequence: 3,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("two and three summarized"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await;

    assert!(matches!(
        overlapping,
        Err(SessionThreadError::OverlappingSummaryRange { .. })
    ));
}

#[tokio::test]
async fn filesystem_preview_append_retries_converge_on_one_message() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-preview-race", "alice");
    let service = Arc::new(FilesystemSessionThreadService::new(scoped));
    let scope = scope("fs-preview-race");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-fs-preview-race").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    let invocation_id = InvocationId::new();

    let left = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        async move {
            service
                .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                    scope,
                    thread_id,
                    turn_run_id: "run-race".into(),
                    preview: preview_envelope(invocation_id),
                })
                .await
        }
    };
    let right = {
        let service = Arc::clone(&service);
        let scope = scope.clone();
        let thread_id = thread.thread_id.clone();
        async move {
            service
                .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                    scope,
                    thread_id,
                    turn_run_id: "run-race".into(),
                    preview: preview_envelope(invocation_id),
                })
                .await
        }
    };

    let (left, right) = tokio::join!(left, right);
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.message_id, right.message_id);

    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    let preview_count = history
        .messages
        .iter()
        .filter(|message| message.kind == MessageKind::CapabilityDisplayPreview)
        .count();
    assert_eq!(preview_count, 1);
}

/// Regression for the ScopedFilesystem migration: two stores share one
/// underlying [`RootFilesystem`] but each is constructed with a
/// [`MountView`] whose `/threads` alias resolves to a different
/// tenant-scoped [`VirtualPath`] subtree. Writing the same
/// `(agent_id, project_id, owner_user_id, thread_id)` tuple on tenant A's
/// store must NOT make the record visible from tenant B's store. Before
/// this migration the legacy SQL stores held a raw `Arc<libsql::Database>`
/// / `deadpool_postgres::Pool` and encoded scope identity inside a single
/// shared table — any composition layer that forgot to scope the
/// `Database`/`Pool` to a tenant prefix would leak across tenants, with
/// the type system saying nothing. The structural fix routes every op
/// through `ScopedFilesystem`, so two MountViews over the same backend
/// cannot see each other's data.
#[tokio::test]
async fn filesystem_session_thread_service_isolates_two_tenants_with_same_user_project_ids() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped_a = scoped_threads_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let scoped_b = scoped_threads_fs_at(backend, "tenant-b", "alice");
    let service_a = FilesystemSessionThreadService::new(scoped_a);
    let service_b = FilesystemSessionThreadService::new(scoped_b);

    // Identical within-tenant axes on both scopes — only `tenant_id`
    // differs. The MountView's per-tenant rewriting is the only thing
    // keeping the two stores apart on the shared backend.
    let scope_a = ThreadScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        agent_id: AgentId::new("agent-x").unwrap(),
        project_id: Some(ProjectId::new("project-1").unwrap()),
        owner_user_id: Some(UserId::new("alice").unwrap()),
        mission_id: None,
    };
    let scope_b = ThreadScope {
        tenant_id: TenantId::new("tenant-b").unwrap(),
        ..scope_a.clone()
    };
    let thread_id = ThreadId::new("thread-shared-id").unwrap();

    service_a
        .ensure_thread(EnsureThreadRequest {
            scope: scope_a.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "actor-a".into(),
            title: Some("Tenant A".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service_a
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope_a.clone(),
            thread_id: thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-a".into()),
            content: MessageContent::text("tenant a payload"),
        })
        .await
        .unwrap();

    // Tenant A sees its thread.
    let history_a = service_a
        .list_thread_history(ThreadHistoryRequest {
            scope: scope_a,
            thread_id: thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history_a.thread.title.as_deref(), Some("Tenant A"));
    assert_eq!(history_a.messages.len(), 1);

    // Tenant B does NOT see tenant A's thread despite identical
    // (agent_id, project_id, owner_user_id, thread_id).
    let history_b = service_b
        .list_thread_history(ThreadHistoryRequest {
            scope: scope_b.clone(),
            thread_id: thread_id.clone(),
        })
        .await;
    assert!(
        history_b.is_err(),
        "tenant B must NOT see tenant A's thread history (cross-tenant path leak)"
    );

    // And tenant B's replay lookup for tenant A's external event must
    // come back as None — the idempotency record under tenant A's mount
    // is invisible from tenant B.
    let replay = service_b
        .replay_accepted_inbound_message(ironclaw_threads::ReplayAcceptedInboundMessageRequest {
            scope: scope_b,
            actor_id: "actor-a".into(),
            source_binding_id: "binding".into(),
            external_event_id: "event-a".into(),
        })
        .await
        .unwrap();
    assert!(
        replay.is_none(),
        "tenant B must NOT replay tenant A's inbound idempotency record"
    );
}

#[tokio::test]
async fn filesystem_store_rejects_cross_actor_duplicate_external_event_replay() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-a", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let request_scope = scope("actor-replay");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: request_scope.clone(),
            thread_id: Some(ThreadId::new("thread-actor-replay").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("binding".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-actor-check".into()),
            content: MessageContent::text("actor a event"),
        })
        .await
        .unwrap();

    let replay = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: request_scope,
            thread_id: thread.thread_id,
            actor_id: "actor-b".into(),
            source_binding_id: Some("binding".into()),
            reply_target_binding_id: None,
            external_event_id: Some("event-actor-check".into()),
            content: MessageContent::text("actor b must not replay actor a"),
        })
        .await;

    assert!(matches!(
        replay,
        Err(SessionThreadError::IdempotentReplayActorMismatch { .. })
    ));
}

/// Mirrors the legacy `durable_history_flow` from the old SQL contract
/// suite. Drives every transition the service exposes and returns the
/// thread id so a downstream restart-equivalent test can confirm the
/// records rehydrated identically.
async fn durable_history_flow(service: &impl SessionThreadService, label: &str) -> ThreadId {
    let scope = scope(label);
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new(format!("thread-{label}")).unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("Durable thread".into()),
            metadata_json: Some("{\"source\":\"contract\"}".into()),
        })
        .await
        .unwrap();

    let first = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-1".into()),
            content: MessageContent::text("secret token"),
        })
        .await
        .unwrap();
    let duplicate = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: Some("telegram-thread-1".into()),
            reply_target_binding_id: Some("telegram-thread-1".into()),
            external_event_id: Some("telegram-event-1".into()),
            content: MessageContent::text("retry payload ignored"),
        })
        .await
        .unwrap();
    assert_eq!(first.message_id, duplicate.message_id);
    assert!(duplicate.idempotent_replay);

    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: None,
            content: MessageContent::text("safe follow-up"),
        })
        .await
        .unwrap();

    service
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 2,
            summary_kind: SummaryKind::Compaction,
            content: MessageContent::text("summary that mentions secret token"),
            model_context_policy: Some(SummaryModelContextPolicy::ReplaceRangeWhenSelected),
        })
        .await
        .unwrap();

    service
        .redact_message(RedactMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: first.message_id,
            redaction_ref: "redaction/audit/1".into(),
        })
        .await
        .unwrap();

    let draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("partial"),
        })
        .await
        .unwrap();
    let duplicate_draft = service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            turn_run_id: "run-1".into(),
            content: MessageContent::text("retry partial ignored"),
        })
        .await
        .unwrap();
    assert_eq!(draft.message_id, duplicate_draft.message_id);
    service
        .update_assistant_draft(UpdateAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: draft.message_id,
            content: MessageContent::text("partial plus more"),
        })
        .await
        .unwrap();
    service
        .finalize_assistant_message(
            &scope,
            &thread.thread_id,
            draft.message_id,
            MessageContent::text("final answer"),
        )
        .await
        .unwrap();

    thread.thread_id
}

async fn assert_reopened_history(
    service: &impl SessionThreadService,
    label: &str,
    thread_id: ThreadId,
) {
    let thread_scope = scope(label);
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.thread.title.as_deref(), Some("Durable thread"));
    assert_eq!(history.messages.len(), 3);
    assert_eq!(history.messages[0].sequence, 1);
    assert_eq!(history.messages[0].status, MessageStatus::Redacted);
    assert!(history.messages[0].content.is_none());
    assert_eq!(
        history.messages[1].content.as_deref(),
        Some("safe follow-up")
    );
    assert_eq!(history.messages[2].kind, MessageKind::Assistant);
    assert_eq!(history.messages[2].status, MessageStatus::Finalized);
    assert_eq!(history.messages[2].content.as_deref(), Some("final answer"));
    assert_eq!(history.summary_artifacts.len(), 1);
    assert_eq!(history.summary_artifacts[0].content, "[redacted]");

    let context = service
        .load_context_window(LoadContextWindowRequest {
            scope: thread_scope,
            thread_id: thread_id.clone(),
            max_messages: 16,
        })
        .await
        .unwrap();
    assert_eq!(context.messages.len(), 2);
    assert_eq!(context.messages[0].content, "safe follow-up");
    assert_eq!(context.messages[1].content, "final answer");

    let wrong_scope = service
        .list_thread_history(ThreadHistoryRequest {
            scope: scope(&format!("{label}-wrong")),
            thread_id,
        })
        .await;
    assert!(wrong_scope.is_err());
}

#[tokio::test]
async fn filesystem_list_threads_for_scope_is_scope_filtered_and_paginated() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-host", "alice");
    let service = FilesystemSessionThreadService::new(scoped);

    let scope_a = scope("a");
    let scope_b = scope("b");

    // Empty store → empty list, no cursor (matches the missing-root
    // is_not_found arm in `list_dir`).
    let initial = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    assert!(initial.threads.is_empty(), "fresh store must be empty");
    assert!(initial.next_cursor.is_none());

    // Seed: 3 threads in scope A with deterministic ids so the
    // pagination assertion is stable. 1 thread in scope B that the
    // scope-A enumeration must not see — because the path layout
    // encodes scope axes, this also verifies the directory walk
    // doesn't leak across `(agent, project, owner)` cells.
    for id in ["t-a-001", "t-a-002", "t-a-003"] {
        service
            .ensure_thread(EnsureThreadRequest {
                scope: scope_a.clone(),
                thread_id: Some(ThreadId::new(id).unwrap()),
                created_by_actor_id: "actor-a".into(),
                title: Some(id.into()),
                metadata_json: None,
            })
            .await
            .unwrap();
    }
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope_b.clone(),
            thread_id: Some(ThreadId::new("t-b-001").unwrap()),
            created_by_actor_id: "actor-b".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // Scope filter: A sees only A's threads, sorted deterministically.
    let scope_a_all = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids: Vec<&str> = scope_a_all
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(ids, ["t-a-001", "t-a-002", "t-a-003"]);
    assert!(
        scope_a_all.next_cursor.is_none(),
        "no more pages when page size > total",
    );

    // Pagination: limit=2 → first page is [001, 002] with cursor=002.
    let page_1 = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: Some(2),
            cursor: None,
        })
        .await
        .unwrap();
    let page_1_ids: Vec<&str> = page_1
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(page_1_ids, ["t-a-001", "t-a-002"]);
    assert_eq!(page_1.next_cursor.as_deref(), Some("t-a-002"));

    // Follow-up: cursor=002 → next page is [003] with no further cursor.
    let page_2 = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_a.clone(),
            limit: Some(2),
            cursor: page_1.next_cursor.clone(),
        })
        .await
        .unwrap();
    let page_2_ids: Vec<&str> = page_2
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(page_2_ids, ["t-a-003"]);
    assert!(page_2.next_cursor.is_none());

    // Cross-scope safety: scope B sees only its own thread, never A's.
    // For the filesystem backend this is structurally guaranteed by the
    // per-scope directory layout — `scope_axes_string` puts A and B at
    // different paths, so `list_dir` on B's root cannot return A's ids.
    let scope_b_all = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope_b,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();
    let ids_b: Vec<&str> = scope_b_all
        .threads
        .iter()
        .map(|record| record.thread_id.as_str())
        .collect();
    assert_eq!(ids_b, ["t-b-001"]);
}

#[tokio::test]
async fn filesystem_list_threads_for_scope_derives_title_from_first_user_message() {
    use ironclaw_threads::ListThreadsForScopeRequest;

    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-title-fs", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let scope = scope("fs-title");

    // Thread #1: title-less, assistant speaks before the first user message.
    // Derivation must skip assistant records and pick the first non-empty
    // trimmed line from the earliest user message.
    let derived = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("t-derived").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: scope.clone(),
            thread_id: derived.thread_id.clone(),
            turn_run_id: "run-derived-1".into(),
            content: MessageContent::text("assistant text must not become the title"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: derived.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("evt-derived-1".into()),
            content: MessageContent::text("  hello there  \nsecond line"),
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: derived.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("evt-derived-2".into()),
            content: MessageContent::text("later user message must not replace the title"),
        })
        .await
        .unwrap();

    // Thread #2: title-less and has no messages at all → must stay None
    // (the derive helper has nothing to extract from).
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("t-empty").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    // Thread #3: caller supplied an explicit title → list MUST preserve
    // it untouched. This is the "creator-supplied wins over derivation"
    // invariant.
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("t-explicit").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: Some("Caller-supplied title".into()),
            metadata_json: None,
        })
        .await
        .unwrap();
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: ThreadId::new("t-explicit").unwrap(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("evt-explicit-1".into()),
            content: MessageContent::text("user message that must NOT replace the title"),
        })
        .await
        .unwrap();

    let listed = service
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope,
            limit: None,
            cursor: None,
        })
        .await
        .unwrap();

    let by_id: std::collections::HashMap<&str, Option<&str>> = listed
        .threads
        .iter()
        .map(|record| (record.thread_id.as_str(), record.title.as_deref()))
        .collect();

    assert_eq!(
        by_id.get("t-derived").copied().flatten(),
        Some("hello there"),
        "first user message should seed a trimmed first-line title",
    );
    assert!(
        by_id.get("t-empty").copied().flatten().is_none(),
        "thread with no user messages must stay title: None",
    );
    assert_eq!(
        by_id.get("t-explicit").copied().flatten(),
        Some("Caller-supplied title"),
        "explicit EnsureThreadRequest.title must not be overwritten by derivation",
    );
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

fn assert_unknown_thread(error: SessionThreadError, thread_id: &ThreadId) {
    match error {
        SessionThreadError::UnknownThread { thread_id: actual } => assert_eq!(actual, *thread_id),
        other => panic!("expected UnknownThread for {thread_id}, got {other:?}"),
    }
}

fn preview_envelope(invocation_id: InvocationId) -> CapabilityDisplayPreviewEnvelope {
    CapabilityDisplayPreviewEnvelope::new(CapabilityDisplayPreviewEnvelopeInput {
        invocation_id,
        capability_id: CapabilityId::new("demo.echo").unwrap(),
        status: CapabilityDisplayPreviewStatus::Completed,
        title: "echo".to_string(),
        subtitle: None,
        input_summary: Some("{\"message\":\"hello\"}".to_string()),
        output_summary: Some("text output".to_string()),
        output_preview: Some("hello".to_string()),
        output_kind: Some("text".to_string()),
        output_bytes: Some(5),
        result_ref: Some("result:demo-preview".to_string()),
        truncated: false,
        updated_at: Utc::now(),
    })
    .unwrap()
}

/// Wrap a [`RootFilesystem`] in a [`ScopedFilesystem`] that exposes the
/// `/threads` alias rooted under a single tenant/user subtree of the
/// underlying backend. The `tenant`/`user` arguments map to the
/// production composition's `invocation_mount_view`-style rewriting:
/// `/threads → /tenants/<tenant>/users/<user>/threads`. Two
/// `ScopedFilesystem`s built with different `tenant` arguments over the
/// same `RootFilesystem` cannot see each other's data.
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

#[tokio::test]
async fn filesystem_persists_attachment_refs_and_clears_them_on_redaction() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-attachments", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("attachments");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-attachments").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let attachment = AttachmentRef {
        id: "att-1".into(),
        kind: AttachmentKind::Image,
        mime_type: "image/png".into(),
        filename: Some("diagram.png".into()),
        size_bytes: Some(4096),
        storage_key: Some("attachments/2026-06-09/m1-diagram.png".into()),
        extracted_text: None,
    };
    let accepted = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("event-att".into()),
            content: MessageContent::with_attachments("look at this", vec![attachment.clone()]),
        })
        .await
        .unwrap();

    // Re-open the store over the same backend to prove the refs survive a
    // serialize → store → deserialize round trip, not just an in-process cache.
    let reopened = FilesystemSessionThreadService::new(scoped);
    let history = reopened
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].attachments, vec![attachment]);

    reopened
        .redact_message(RedactMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            message_id: accepted.message_id,
            redaction_ref: "redaction:test".into(),
        })
        .await
        .unwrap();

    let after = reopened
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(after.messages[0].status, MessageStatus::Redacted);
    assert!(after.messages[0].content.is_none());
    assert!(after.messages[0].attachments.is_empty());
}

#[tokio::test]
async fn filesystem_persists_multiple_attachment_refs_in_order() {
    // The single-ref test can't catch an ordering or per-element bug in the
    // JSON array round trip. Drive a multi-ref message — distinct kinds, one
    // with `extracted_text: Some(..)` (which the single-ref test never sets) —
    // through the real serialize → store → deserialize path and assert the full
    // vec survives in order.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-multi-att", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("multi-attachments");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-multi-att").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let attachments = vec![
        AttachmentRef {
            id: "att-1".into(),
            kind: AttachmentKind::Image,
            mime_type: "image/png".into(),
            filename: Some("diagram.png".into()),
            size_bytes: Some(4096),
            storage_key: Some("attachments/2026-06-09/m1-0-diagram.png".into()),
            extracted_text: None,
        },
        AttachmentRef {
            id: "att-2".into(),
            kind: AttachmentKind::Document,
            mime_type: "application/pdf".into(),
            filename: Some("report.pdf".into()),
            size_bytes: Some(20_480),
            storage_key: Some("attachments/2026-06-09/m1-1-report.pdf".into()),
            extracted_text: Some("Quarterly revenue up 12%".into()),
        },
        AttachmentRef {
            id: "att-3".into(),
            kind: AttachmentKind::Audio,
            mime_type: "audio/mpeg".into(),
            filename: Some("note.mp3".into()),
            size_bytes: Some(8192),
            storage_key: Some("attachments/2026-06-09/m1-2-note.mp3".into()),
            extracted_text: None,
        },
    ];
    service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("event-multi-att".into()),
            content: MessageContent::with_attachments("three files", attachments.clone()),
        })
        .await
        .unwrap();

    // Re-open over the same backend so the assertion crosses the real JSON
    // serialize → store → deserialize boundary.
    let reopened = FilesystemSessionThreadService::new(scoped);
    let history = reopened
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].attachments, attachments);
}

#[tokio::test]
async fn filesystem_accept_rejects_duplicate_attachment_ids() {
    // The accept path validates attachment refs before persisting. Drive the
    // real caller (not just the helper) so a regression that drops the check
    // would fail here, and assert nothing was written on rejection.
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(backend, "tenant-dup-att", "alice");
    let service = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    let scope = scope("dup-attachments");
    let thread = service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(ThreadId::new("thread-dup-att").unwrap()),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        })
        .await
        .unwrap();

    let dup = AttachmentRef {
        id: "att-dup".into(),
        kind: AttachmentKind::Image,
        mime_type: "image/png".into(),
        filename: Some("diagram.png".into()),
        size_bytes: Some(4096),
        storage_key: Some("attachments/2026-06-09/m1-0-diagram.png".into()),
        extracted_text: None,
    };
    let err = service
        .accept_inbound_message(AcceptInboundMessageRequest {
            scope: scope.clone(),
            thread_id: thread.thread_id.clone(),
            actor_id: "actor-a".into(),
            source_binding_id: None,
            reply_target_binding_id: None,
            external_event_id: Some("event-dup-att".into()),
            content: MessageContent::with_attachments("two refs, one id", vec![dup.clone(), dup]),
        })
        .await
        .expect_err("duplicate attachment ids must be rejected at accept");
    assert!(matches!(err, SessionThreadError::InvalidAttachment(_)));

    // Rejection must not leave a half-written message behind.
    let history = service
        .list_thread_history(ThreadHistoryRequest {
            scope,
            thread_id: thread.thread_id,
        })
        .await
        .unwrap();
    assert!(history.messages.is_empty());
}
