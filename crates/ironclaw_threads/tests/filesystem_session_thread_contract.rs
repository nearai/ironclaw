//! Contract tests for [`FilesystemSessionThreadService`].
//!
//! Drives the production filesystem-backed store over an
//! [`InMemoryBackend`] composed under a `/threads` mount alias whose
//! `VirtualPath` target encodes a tenant/user prefix. Mirrors the shape of
//! the run-state and processes filesystem contract suites — see
//! `crates/ironclaw_run_state/tests/run_state_contract.rs` and
//! `crates/ironclaw_processes/tests/process_store_contract.rs`.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, ThreadId,
    UserId, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest, CreateSummaryArtifactRequest,
    EnsureThreadRequest, FilesystemSessionThreadService, LoadContextWindowRequest, MessageContent,
    MessageKind, MessageStatus, RedactMessageRequest, SessionThreadError, SessionThreadService,
    ThreadHistoryRequest, ThreadScope, UpdateAssistantDraftRequest,
};

mod support;
use support::{assert_two_page_thread_listing, seed_listable_threads};

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
async fn filesystem_store_lists_threads_for_exact_scope_after_reopen() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_threads_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let service = FilesystemSessionThreadService::new(scoped);
    let owned_scope = scope("fs-list-owned");
    let thread_ids =
        seed_listable_threads(&service, &owned_scope, scope("fs-list-other"), "fs-list").await;

    let scoped = scoped_threads_fs_at(backend, "tenant-a", "alice");
    let reopened = FilesystemSessionThreadService::new(scoped);
    assert_two_page_thread_listing(&reopened, owned_scope, thread_ids).await;
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
            summary_kind: "model_context".into(),
            content: MessageContent::text("summary that mentions secret token"),
            model_context_policy: Some("replace_range_when_selected".into()),
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

fn scope(label: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new(format!("tenant-{label}")).unwrap(),
        agent_id: AgentId::new(format!("agent-{label}")).unwrap(),
        project_id: Some(ProjectId::new(format!("project-{label}")).unwrap()),
        owner_user_id: Some(UserId::new(format!("user-{label}")).unwrap()),
        mission_id: None,
    }
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
