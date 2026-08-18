use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    turn::{TurnId, TurnRunId},
};
use ironclaw_threads::{
    AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, FilesystemSessionThreadService,
    InMemorySessionThreadService, MessageContent, PublishStructuredFinalizationMessageRequest,
    PutStructuredFinalizationRequest, ReadStructuredFinalizationRequest, SessionThreadError,
    SessionThreadService, StructuredFinalizationAccounting, StructuredFinalizationRecord,
    StructuredFinalizationUsage, ThreadMessageId, ThreadScope,
};

fn scope(agent: &str) -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant").expect("tenant"),
        agent_id: AgentId::new(agent).expect("agent"),
        project_id: Some(ProjectId::new("project").expect("project")),
        owner_user_id: Some(UserId::new("user").expect("user")),
        mission_id: None,
    }
}

async fn seeded_service(scope: &ThreadScope, thread_id: &ThreadId) -> InMemorySessionThreadService {
    let service = InMemorySessionThreadService::default();
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    service
}

fn scoped_filesystem()
-> Arc<ironclaw_filesystem::ScopedFilesystem<ironclaw_filesystem::InMemoryBackend>> {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads").expect("alias"),
        VirtualPath::new("/tenants/tenant/users/user/threads").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mounts");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

async fn seeded_filesystem_service(
    scope: &ThreadScope,
    thread_id: &ThreadId,
) -> Arc<dyn SessionThreadService> {
    let filesystem = scoped_filesystem();
    let service: Arc<dyn SessionThreadService> =
        Arc::new(FilesystemSessionThreadService::new(filesystem));
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    service
}

fn record(
    scope: ThreadScope,
    thread_id: ThreadId,
    owner_fence: &str,
) -> StructuredFinalizationRecord {
    StructuredFinalizationRecord {
        scope,
        thread_id,
        turn_id: TurnId::new(),
        turn_run_id: TurnRunId::new(),
        contract_name: "suggestions".to_string(),
        schema_digest: "sha256-schema".to_string(),
        candidate: "candidate retained as nonterminal LLM data".to_string(),
        raw_json: r#"{"items":[{"title":"one"}]}"#.to_string(),
        accounting: StructuredFinalizationAccounting {
            usage: Some(StructuredFinalizationUsage {
                input_tokens: 11,
                output_tokens: 7,
                cache_read_input_tokens: 2,
                cache_creation_input_tokens: 3,
            }),
            elapsed_ms: 42,
            model_profile_id: Some("nearai".to_string()),
            provider_id: Some("near".to_string()),
            model_id: Some("model".to_string()),
        },
        owner_fence: owner_fence.to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn assert_structured_finalization_message_publish(
    service: &dyn SessionThreadService,
    scope: ThreadScope,
    thread_id: ThreadId,
) {
    let finalization = record(scope.clone(), thread_id.clone(), "lease-publish");
    let turn_run_id = finalization.turn_run_id;
    let message = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: turn_run_id.to_string(),
            content: MessageContent::text(finalization.candidate.clone()),
        })
        .await
        .expect("candidate assistant message");
    service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: finalization.clone(),
        })
        .await
        .expect("durable finalization record");
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;

    let published = service
        .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: message.message_id,
            turn_run_id,
            replacement: finalization.raw_json.clone(),
        })
        .await
        .expect("publish should update the exact finalized message");
    assert_eq!(published.message_id, message.message_id);
    assert_eq!(
        published.content.as_deref(),
        Some(finalization.raw_json.as_str())
    );
    assert_eq!(published.status, ironclaw_threads::MessageStatus::Finalized);
    assert!(
        published.updated_at > message.updated_at,
        "publishing replacement content must advance message activity"
    );

    let replay = service
        .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: message.message_id,
            turn_run_id,
            replacement: finalization.raw_json.clone(),
        })
        .await
        .expect("publishing the same durable record must be idempotent");
    assert_eq!(replay, published);

    let missing_run = TurnRunId::new();
    let missing_message = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: missing_run.to_string(),
            content: MessageContent::text("candidate without a durable record"),
        })
        .await
        .expect("message for missing-record rejection");
    let error = service
        .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: missing_message.message_id,
            turn_run_id: missing_run,
            replacement: r#"{"items":[]}"#.to_string(),
        })
        .await
        .expect_err("missing durable record must fail closed");
    assert!(matches!(
        error,
        SessionThreadError::StructuredFinalizationPublishMismatch { .. }
    ));

    let mismatched_record = record(scope.clone(), thread_id.clone(), "lease-content");
    let mismatched_run = mismatched_record.turn_run_id;
    let mismatched_message = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: mismatched_run.to_string(),
            content: MessageContent::text("different current content"),
        })
        .await
        .expect("message for candidate-mismatch rejection");
    service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: mismatched_record.clone(),
        })
        .await
        .expect("mismatched candidate record");
    let error = service
        .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: mismatched_message.message_id,
            turn_run_id: mismatched_run,
            replacement: mismatched_record.raw_json,
        })
        .await
        .expect_err("current content must match the durable candidate");
    assert!(matches!(
        error,
        SessionThreadError::StructuredFinalizationPublishMismatch { .. }
    ));

    let error = service
        .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: message.message_id,
            turn_run_id,
            replacement: r#"{"items":[]}"#.to_string(),
        })
        .await
        .expect_err("a different replacement must fail closed");
    assert!(matches!(
        error,
        SessionThreadError::StructuredFinalizationPublishMismatch { .. }
    ));

    let error = service
        .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: message.message_id,
            turn_run_id: TurnRunId::new(),
            replacement: finalization.raw_json.clone(),
        })
        .await
        .expect_err("a different run must fail closed");
    assert!(matches!(
        error,
        SessionThreadError::StructuredFinalizationPublishMismatch { .. }
    ));

    let error = service
        .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            message_id: ThreadMessageId::new(),
            turn_run_id,
            replacement: finalization.raw_json,
        })
        .await
        .expect_err("an unknown message must fail closed");
    assert!(matches!(error, SessionThreadError::UnknownMessage { .. }));
}

async fn assert_concurrent_publish_is_idempotent(
    service: Arc<dyn SessionThreadService>,
    scope: ThreadScope,
    thread_id: ThreadId,
) {
    let finalization = record(scope.clone(), thread_id.clone(), "lease-race");
    let turn_run_id = finalization.turn_run_id;
    let message = service
        .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: turn_run_id.to_string(),
            content: MessageContent::text(finalization.candidate.clone()),
        })
        .await
        .expect("candidate assistant message");
    service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: finalization.clone(),
        })
        .await
        .expect("durable finalization record");
    let request = PublishStructuredFinalizationMessageRequest {
        scope,
        thread_id,
        message_id: message.message_id,
        turn_run_id,
        replacement: finalization.raw_json,
    };
    let (first, second) = tokio::join!(
        service.publish_structured_finalization_message(request.clone()),
        service.publish_structured_finalization_message(request),
    );
    let first = first.expect("first concurrent publish");
    let second = second.expect("second concurrent publish");
    assert_eq!(first, second);
    assert_eq!(first.message_id, message.message_id);
}

async fn assert_structured_finalization_rejections(
    service: &dyn SessionThreadService,
    scope: ThreadScope,
    thread_id: ThreadId,
) {
    let first = record(scope.clone(), thread_id.clone(), "lease-a");
    let run_id = first.turn_run_id;
    service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: first.clone(),
        })
        .await
        .expect("first CAS write");

    let cross_scope = ThreadScope {
        agent_id: AgentId::new("different-agent").expect("agent"),
        ..scope.clone()
    };
    let error = service
        .read_structured_finalization(ReadStructuredFinalizationRequest {
            scope: cross_scope,
            thread_id: thread_id.clone(),
            turn_run_id: run_id,
        })
        .await
        .expect_err("structured finalization must be scoped by the full thread scope");
    assert!(
        matches!(error, SessionThreadError::UnknownThread { .. }),
        "cross-scope reads must not reveal a run-scoped finalization record: {error:?}"
    );

    let mut stale_owner = first.clone();
    stale_owner.owner_fence = "lease-b".to_string();
    let successor_replay = service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: stale_owner,
        })
        .await
        .expect("successor fence must replay the immutable record");
    assert_eq!(successor_replay, first);
    assert_eq!(successor_replay.owner_fence, "lease-a");

    let mut conflicting_output = first.clone();
    conflicting_output.raw_json = r#"{"items":[]}"#.to_string();
    let error = service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: conflicting_output,
        })
        .await
        .expect_err("different output must not overwrite");
    assert!(matches!(
        error,
        SessionThreadError::StructuredFinalizationConflict { .. }
    ));

    let stored = service
        .read_structured_finalization(ReadStructuredFinalizationRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
            turn_run_id: run_id,
        })
        .await
        .expect("read")
        .expect("record");
    assert_eq!(stored.raw_json, first.raw_json);
    assert_eq!(stored.owner_fence, "lease-a");
}

async fn assert_delete_recreate_does_not_replay_finalization(
    service: &dyn SessionThreadService,
    scope: ThreadScope,
    thread_id: ThreadId,
) {
    let record = record(scope.clone(), thread_id.clone(), "lease-a");
    let run_id = record.turn_run_id;
    service
        .put_structured_finalization(PutStructuredFinalizationRequest { record })
        .await
        .expect("write finalization");

    service
        .delete_thread(&scope, &thread_id)
        .await
        .expect("delete thread");
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("recreate thread");

    let read = service
        .read_structured_finalization(ReadStructuredFinalizationRequest {
            scope,
            thread_id,
            turn_run_id: run_id,
        })
        .await
        .expect("read recreated thread");
    assert!(
        read.is_none(),
        "deleted thread evidence must not be replayed into a new incarnation"
    );
}

#[tokio::test]
async fn run_record_is_durable_readable_and_idempotent_for_same_owner() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("thread").expect("thread");
    let service = seeded_service(&scope, &thread_id).await;
    let record = record(scope.clone(), thread_id.clone(), "lease-a");
    let run_id = record.turn_run_id;

    let stored = service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: record.clone(),
        })
        .await
        .expect("first CAS write");
    assert_eq!(stored, record);

    let replay = service
        .put_structured_finalization(PutStructuredFinalizationRequest { record })
        .await
        .expect("same-owner replay");
    assert_eq!(replay, stored);

    let read = service
        .read_structured_finalization(ReadStructuredFinalizationRequest {
            scope,
            thread_id,
            turn_run_id: run_id,
        })
        .await
        .expect("read")
        .expect("record");
    assert_eq!(read.raw_json, r#"{"items":[{"title":"one"}]}"#);
    assert_eq!(read.accounting.usage.expect("usage").output_tokens, 7);
}

#[tokio::test]
async fn in_memory_publish_preserves_message_identity_and_is_fail_closed() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("publish-memory").expect("thread");
    let service = seeded_service(&scope, &thread_id).await;
    assert_structured_finalization_message_publish(&service, scope, thread_id).await;
}

#[tokio::test]
async fn filesystem_publish_preserves_message_identity_and_is_fail_closed() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("publish-filesystem").expect("thread");
    let service = seeded_filesystem_service(&scope, &thread_id).await;
    assert_structured_finalization_message_publish(service.as_ref(), scope, thread_id).await;
}

#[tokio::test]
async fn in_memory_concurrent_publish_is_idempotent() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("publish-memory-race").expect("thread");
    let service: Arc<dyn SessionThreadService> = Arc::new(seeded_service(&scope, &thread_id).await);
    assert_concurrent_publish_is_idempotent(service, scope, thread_id).await;
}

#[tokio::test]
async fn filesystem_concurrent_publish_is_idempotent() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("publish-filesystem-race").expect("thread");
    let service = seeded_filesystem_service(&scope, &thread_id).await;
    assert_concurrent_publish_is_idempotent(service, scope, thread_id).await;
}

#[tokio::test]
async fn successor_fence_replays_and_conflicting_output_is_rejected() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("thread").expect("thread");
    let service = seeded_service(&scope, &thread_id).await;
    assert_structured_finalization_rejections(&service, scope, thread_id).await;
}

#[tokio::test]
async fn filesystem_structured_finalization_matches_in_memory_rejections() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("filesystem-rejections").expect("thread");
    let service = seeded_filesystem_service(&scope, &thread_id).await;
    assert_structured_finalization_rejections(service.as_ref(), scope, thread_id).await;
}

#[tokio::test]
async fn deleting_and_recreating_a_thread_does_not_replay_finalization() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("reused-thread").expect("thread");
    let service = seeded_service(&scope, &thread_id).await;
    assert_delete_recreate_does_not_replay_finalization(&service, scope, thread_id).await;
}

#[tokio::test]
async fn filesystem_deleting_and_recreating_a_thread_does_not_replay_finalization() {
    let scope = scope("agent");
    let thread_id = ThreadId::new("filesystem-reused-thread").expect("thread");
    let service = seeded_filesystem_service(&scope, &thread_id).await;
    assert_delete_recreate_does_not_replay_finalization(service.as_ref(), scope, thread_id).await;
}

#[tokio::test]
async fn filesystem_delete_retains_finalization_at_the_archive_audit_seam() {
    use ironclaw_host_api::path::ScopedPath;

    let filesystem = scoped_filesystem();
    let service = FilesystemSessionThreadService::new(Arc::clone(&filesystem));
    let scope = scope("agent");
    let thread_id = ThreadId::new("retained-thread").expect("thread");
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let record = record(scope.clone(), thread_id.clone(), "lease-a");
    let run_id = record.turn_run_id;
    service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: record.clone(),
        })
        .await
        .expect("finalization");

    let thread_path = ScopedPath::new(
        "/threads/agents/agent/projects/project/owners/user/threads/retained-thread/thread.json",
    )
    .expect("thread path");
    let thread_entry = filesystem
        .get(&scope.to_resource_scope(), &thread_path)
        .await
        .expect("thread audit read")
        .expect("thread entry");
    let incarnation = serde_json::from_slice::<serde_json::Value>(&thread_entry.entry.body)
        .expect("thread JSON")["incarnation_id"]
        .as_str()
        .expect("incarnation id")
        .to_string();
    let archive_path = ScopedPath::new(format!(
        "/threads/agents/agent/projects/project/owners/user/structured-finalizations/retained-thread/{incarnation}/{run_id}.json"
    ))
    .expect("archive path");

    service
        .delete_thread(&scope, &thread_id)
        .await
        .expect("delete thread");

    let archived = filesystem
        .get(&scope.to_resource_scope(), &archive_path)
        .await
        .expect("finalization archive read")
        .expect("finalization evidence must survive transcript deletion");
    let archived_record =
        serde_json::from_slice::<StructuredFinalizationRecord>(&archived.entry.body)
            .expect("archived finalization JSON");
    assert_eq!(archived_record, record);
}

#[tokio::test]
async fn filesystem_record_survives_service_recreation_and_replays_without_inference() {
    let filesystem = scoped_filesystem();
    let first_service = FilesystemSessionThreadService::new(Arc::clone(&filesystem));
    let scope = scope("agent");
    let thread_id = ThreadId::new("thread").expect("thread");
    first_service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let record = record(scope.clone(), thread_id.clone(), "lease-a");
    let run_id = record.turn_run_id;
    first_service
        .put_structured_finalization(PutStructuredFinalizationRequest {
            record: record.clone(),
        })
        .await
        .expect("first durable write");
    drop(first_service);

    let restarted_service = FilesystemSessionThreadService::new(filesystem);
    let read = restarted_service
        .read_structured_finalization(ReadStructuredFinalizationRequest {
            scope,
            thread_id,
            turn_run_id: run_id,
        })
        .await
        .expect("restart read")
        .expect("durable record");
    assert_eq!(read, record);
}

#[tokio::test]
async fn filesystem_records_are_independent_for_two_runs_on_one_thread() {
    let filesystem = scoped_filesystem();
    let service = FilesystemSessionThreadService::new(filesystem);
    let scope = scope("agent");
    let thread_id = ThreadId::new("thread-two-runs").expect("thread");
    service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "test".to_string(),
            title: None,
            metadata_json: None,
        })
        .await
        .expect("thread");
    let first = record(scope.clone(), thread_id.clone(), "lease-a");
    let mut second = record(scope.clone(), thread_id.clone(), "lease-b");
    second.raw_json = r#"{"items":[{"title":"two"}]}"#.to_string();
    for value in [&first, &second] {
        service
            .put_structured_finalization(PutStructuredFinalizationRequest {
                record: value.clone(),
            })
            .await
            .expect("independent run write");
    }
    for value in [first, second] {
        let read = service
            .read_structured_finalization(ReadStructuredFinalizationRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: value.turn_run_id,
            })
            .await
            .expect("read")
            .expect("record");
        assert_eq!(read, value);
    }
}

#[allow(dead_code)]
fn _arc_service_is_send_sync(
    service: Arc<dyn SessionThreadService>,
) -> Arc<dyn SessionThreadService> {
    service
}
