// Contract tests construct the store directly under test.
#![allow(clippy::disallowed_methods)]
// arch-exempt: large_file, cross-instance claim regressions share the existing outbound persistence harness, plan #6175

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use async_trait::async_trait;
use ironclaw_event_log::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, ContentType, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, Filter, InMemoryBackend, IndexKind, IndexName, IndexSpec,
    LibSqlRootFilesystem, Page, RecordVersion, RootFilesystem, ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::turn::{
    ReplyTargetBindingRef, RunOriginAdapter, TurnActor, TurnRunId, TurnScope,
};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, RunId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
};
use ironclaw_outbound::*;
use tokio::sync::{Mutex, Notify};

const TEST_OUTBOUND_ROOT: &str = "/engine/tenants/test/users/test/outbound";

/// Build a `ScopedFilesystem<F>` with full read/write/list/delete permissions
/// on the `/outbound` alias, mapped to a distinct tenant-scoped
/// [`VirtualPath`] subtree. Tests can pass in a different `target_root` to
/// simulate multiple tenants sharing one underlying backend
/// (`filesystem_outbound_store_isolates_two_tenants_*` below).
fn build_scoped_fs<F: RootFilesystem>(
    backend: Arc<F>,
    target_root: &str,
) -> Arc<ScopedFilesystem<F>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/outbound").expect("alias"),
        VirtualPath::new(target_root).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn build_outbound_store_for_backend(
    backend: Arc<InMemoryBackend>,
) -> OutboundStateStore<InMemoryBackend> {
    OutboundStateStore::new(build_scoped_fs(backend, TEST_OUTBOUND_ROOT))
}

fn build_outbound_store_with_permissions<F: RootFilesystem>(
    backend: Arc<F>,
    permissions: MountPermissions,
) -> OutboundStateStore<F> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/outbound").expect("alias"),
        VirtualPath::new(TEST_OUTBOUND_ROOT).expect("target"),
        permissions,
    )])
    .expect("mount view");
    OutboundStateStore::new(Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)))
}

fn reply_attachment_scope() -> ironclaw_host_api::resource::ResourceScope {
    turn_scope().to_resource_scope()
}

fn reply_attachment_intent(path: &str, size_bytes: u64) -> ReplyAttachmentIntent {
    ReplyAttachmentIntent {
        path: ScopedPath::new(path).expect("scoped attachment path"),
        filename: path
            .rsplit('/')
            .next()
            .expect("attachment path has a filename")
            .to_string(),
        mime_type: "text/plain".to_string(),
        size_bytes,
    }
}

#[tokio::test]
async fn reply_attachment_intents_preserve_order_and_seal_idempotently() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(backend);
    let scope = reply_attachment_scope();
    let run_id = RunId::new();
    let first = reply_attachment_intent("/workspace/reports/first.txt", 5);
    let second = reply_attachment_intent("/workspace/reports/second.txt", 7);

    store
        .register(&scope, &run_id, first.clone())
        .await
        .expect("register first attachment");
    store
        .register(&scope, &run_id, first.clone())
        .await
        .expect("identical retry is idempotent");
    store
        .register(&scope, &run_id, second.clone())
        .await
        .expect("register second attachment");

    assert_eq!(
        store
            .seal(&scope, &run_id)
            .await
            .expect("seal attachment intents"),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        store
            .seal(&scope, &run_id)
            .await
            .expect("repeated sealing is idempotent"),
        vec![first, second]
    );
}

#[tokio::test]
async fn reply_attachment_intents_fail_closed_on_conflict_or_post_seal_registration() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(backend);
    let scope = reply_attachment_scope();
    let run_id = RunId::new();
    let original = reply_attachment_intent("/workspace/report.txt", 5);
    store
        .register(&scope, &run_id, original.clone())
        .await
        .expect("register original attachment");

    let mut conflicting = original.clone();
    conflicting.mime_type = "application/json".to_string();
    assert!(matches!(
        store.register(&scope, &run_id, conflicting).await,
        Err(OutboundError::ReplyAttachmentIntentConflict)
    ));

    store
        .seal(&scope, &run_id)
        .await
        .expect("seal attachment intents");
    assert!(matches!(
        store
            .register(
                &scope,
                &run_id,
                reply_attachment_intent("/workspace/late.txt", 1),
            )
            .await,
        Err(OutboundError::ReplyAttachmentIntentsSealed)
    ));
}

#[tokio::test]
async fn reply_attachment_intents_enforce_shared_count_and_byte_budgets() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(backend);
    let scope = reply_attachment_scope();
    let count_run_id = RunId::new();

    for index in 0..ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_count {
        store
            .register(
                &scope,
                &count_run_id,
                reply_attachment_intent(&format!("/workspace/count/{index}.txt"), 1),
            )
            .await
            .expect("register attachment within count budget");
    }
    assert!(matches!(
        store
            .register(
                &scope,
                &count_run_id,
                reply_attachment_intent("/workspace/count/overflow.txt", 1),
            )
            .await,
        Err(OutboundError::ReplyAttachmentIntentLimitExceeded)
    ));

    let size_run_id = RunId::new();
    assert!(matches!(
        store
            .register(
                &scope,
                &size_run_id,
                reply_attachment_intent(
                    "/workspace/too-large.txt",
                    ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64 + 1,
                ),
            )
            .await,
        Err(OutboundError::ReplyAttachmentIntentLimitExceeded)
    ));

    let total_run_id = RunId::new();
    let half = ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes as u64 / 2;
    store
        .register(
            &scope,
            &total_run_id,
            reply_attachment_intent("/workspace/total/first.txt", half + 1),
        )
        .await
        .expect("register first attachment within total budget");
    assert!(matches!(
        store
            .register(
                &scope,
                &total_run_id,
                reply_attachment_intent("/workspace/total/second.txt", half + 1),
            )
            .await,
        Err(OutboundError::ReplyAttachmentIntentLimitExceeded)
    ));
}

#[tokio::test]
async fn reply_attachment_intents_reject_unstable_paths_and_unsafe_metadata() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(backend);
    let scope = reply_attachment_scope();

    let outside_workspace = reply_attachment_intent("/artifacts/report.txt", 1);
    assert!(matches!(
        store
            .register(&scope, &RunId::new(), outside_workspace)
            .await,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let mut unsafe_filename = reply_attachment_intent("/workspace/report.txt", 1);
    unsafe_filename.filename = "../report.txt".to_string();
    assert!(matches!(
        store.register(&scope, &RunId::new(), unsafe_filename).await,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let mut invalid_mime = reply_attachment_intent("/workspace/report.txt", 1);
    invalid_mime.mime_type = "text/plain; charset=utf-8".to_string();
    assert!(matches!(
        store.register(&scope, &RunId::new(), invalid_mime).await,
        Err(OutboundError::InvalidRequest { .. })
    ));
}

#[tokio::test]
async fn reply_attachment_intents_merge_concurrent_writers_across_store_instances() {
    let backend = Arc::new(InMemoryBackend::new());
    let first_store = Arc::new(build_outbound_store_for_backend(Arc::clone(&backend)));
    let second_store = Arc::new(build_outbound_store_for_backend(backend));
    let scope = reply_attachment_scope();
    let run_id = RunId::new();
    let first = reply_attachment_intent("/workspace/concurrent/first.txt", 5);
    let second = reply_attachment_intent("/workspace/concurrent/second.txt", 7);

    let (first_result, second_result) = tokio::join!(
        first_store.register(&scope, &run_id, first.clone()),
        second_store.register(&scope, &run_id, second.clone()),
    );
    first_result.expect("first concurrent registration");
    second_result.expect("second concurrent registration");

    let sealed = first_store
        .seal(&scope, &run_id)
        .await
        .expect("seal merged attachment intents");
    assert_eq!(sealed.len(), 2);
    assert!(sealed.contains(&first));
    assert!(sealed.contains(&second));
}

#[tokio::test]
async fn reply_attachment_intents_survive_store_reopen_and_isolate_scope() {
    let backend = Arc::new(InMemoryBackend::new());
    let first_store = build_outbound_store_for_backend(Arc::clone(&backend));
    let scope = reply_attachment_scope();
    let run_id = RunId::new();
    let intent = reply_attachment_intent("/workspace/durable.txt", 9);
    first_store
        .register(&scope, &run_id, intent.clone())
        .await
        .expect("register durable attachment");
    drop(first_store);

    let reopened_store = build_outbound_store_for_backend(backend);
    assert_eq!(
        reopened_store
            .seal(&scope, &run_id)
            .await
            .expect("seal attachment after reopening store"),
        vec![intent]
    );

    let mut other_scope = scope;
    other_scope.user_id = UserId::new("different-outbound-user").expect("user id");
    assert!(
        reopened_store
            .seal(&other_scope, &run_id)
            .await
            .expect("seal isolated empty scope")
            .is_empty()
    );
}

#[tokio::test]
async fn reply_attachment_intents_persist_across_libsql_reopen() {
    let directory = tempfile::tempdir().expect("temporary libSQL directory");
    let database_path = directory.path().join("outbound-reply-attachments.db");
    let scope = reply_attachment_scope();
    let run_id = RunId::new();
    let intent = reply_attachment_intent("/workspace/libsql-durable.txt", 11);

    {
        let database = Arc::new(
            libsql::Builder::new_local(&database_path)
                .build()
                .await
                .expect("build first libSQL database"),
        );
        let root = Arc::new(
            LibSqlRootFilesystem::new(database).expect("build first libSQL root filesystem"),
        );
        root.run_migrations()
            .await
            .expect("migrate first libSQL filesystem");
        let store = OutboundStateStore::new(build_scoped_fs(root, TEST_OUTBOUND_ROOT));
        store
            .register(&scope, &run_id, intent.clone())
            .await
            .expect("persist attachment intent");
    }

    let reopened_database = Arc::new(
        libsql::Builder::new_local(&database_path)
            .build()
            .await
            .expect("reopen libSQL database"),
    );
    let reopened_root = Arc::new(
        LibSqlRootFilesystem::new(reopened_database).expect("reopen libSQL root filesystem"),
    );
    reopened_root
        .run_migrations()
        .await
        .expect("migrate reopened libSQL filesystem");
    let reopened_store =
        OutboundStateStore::new(build_scoped_fs(reopened_root, TEST_OUTBOUND_ROOT));

    assert_eq!(
        reopened_store
            .seal(&scope, &run_id)
            .await
            .expect("seal intent from reopened durable store"),
        vec![intent]
    );
}

#[tokio::test]
async fn outbound_state_store_satisfies_outbound_contract_on_in_memory_backend() {
    // The new OutboundStateStore runs the same contract suite as
    // the in-memory and SQL backends, demonstrating that it satisfies the
    // OutboundStateStore trait identically. The InMemoryBackend from
    // ironclaw_filesystem stands in as the underlying mount; in production
    // this would be a libSQL- or Postgres-backed RootFilesystem, or an
    // HSM-decorated mount, with no consumer-side code change.
    let backend = std::sync::Arc::new(ironclaw_filesystem::InMemoryBackend::new());
    let store = build_outbound_store_for_backend(Arc::clone(&backend));
    communication_preferences_are_tenant_user_scoped(&store).await;
    communication_preferences_are_shared_agent_scoped(&store).await;
    communication_preferences_reject_empty_updated_by(&store).await;
    communication_preferences_reject_empty_shared_agent_scope(&store).await;
    communication_preference_put_existing_conflicts_without_writing(&store).await;
    communication_preference_atomic_update_preserves_untouched_fields(&store).await;
    communication_preference_update_inserts_absent_record(&store).await;
    communication_preference_stale_version_conflicts_without_writing(&store).await;
    communication_preference_update_rejects_invalid_or_mismatched_record(&store).await;
    communication_preference_write_rejects_oversized_notification_set(&store).await;
    notification_targets_round_trip_and_default_empty(&backend).await;
    outbound_state_store_rejects_communication_preference_put_cas_conflict(&backend).await;
    outbound_state_store_rejects_communication_preference_update_cas_conflict(&backend).await;
    outbound_state_store_rejects_mismatched_communication_preference_identity(&backend, &store)
        .await;
    durable_policy_subscription_delivery_flow(&store).await;
    subscription_cursor_rejects_mismatched_scope(&store).await;
    subscription_ids_are_scoped_not_global(&store).await;
    subscription_upsert_rejects_backward_cursor(&store).await;
    delivery_status_rejects_inconsistent_failure_kind(&store).await;
    coordinator_delivery_lifecycle_round_trips(&store).await;
    recovery_transition_never_clobbers_delivered(&store).await;
    notification_policy_rejects_excessive_targets(&store).await;
}

async fn communication_preference_write_rejects_oversized_notification_set<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound-notification-cap").unwrap();
    let user_id = UserId::new("user-outbound-notification-cap").unwrap();
    let key = CommunicationPreferenceKey::personal(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id.clone()),
        legacy_notification_target: None,
        default_modality: Some(CommunicationModality::Text),
        notification_targets: (0..=NOTIFICATION_TARGETS_CAP)
            .map(|index| {
                OutboundDeliveryTargetId::new(format!("target:notification-cap-{index}")).unwrap()
            })
            .collect(),
        updated_at: now(),
        updated_by: user_id,
    };

    assert!(matches!(
        store.put_communication_preference(record).await,
        Err(OutboundError::InvalidRequest { .. })
    ));
    assert!(
        store
            .load_communication_preference(key)
            .await
            .unwrap()
            .is_none(),
        "an over-cap record must not be partially persisted"
    );
}

#[tokio::test]
async fn delivery_attempt_point_read_returns_only_the_exact_scoped_row() {
    let store = build_outbound_store_for_backend(Arc::new(InMemoryBackend::new()));
    let scope = turn_scope();
    let delivery_id = OutboundDeliveryId::new();
    let attempt = OutboundDeliveryAttempt {
        delivery_id,
        scope: scope.clone(),
        candidate: OutboundPushCandidate {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
            turn_run_id: Some(TurnRunId::new()),
            target: reply_ref("reply-point-read"),
            kind: OutboundPushKind::ModelDelivery,
            projection_ref: ProjectionUpdateRef::new("projection:point-read").unwrap(),
            requires_reply_target_revalidation: true,
        },
        status: OutboundDeliveryStatus::Delivered,
        attempted_at: now(),
        failure_kind: None,
    };
    store
        .record_delivery_attempt(attempt.clone())
        .await
        .expect("persist point-read attempt");

    assert_eq!(
        store
            .load_delivery_attempt(scope, delivery_id)
            .await
            .expect("point read"),
        Some(attempt)
    );
    assert!(
        store
            .load_delivery_attempt(sibling_turn_scope(), delivery_id)
            .await
            .expect("scope mismatch remains non-enumerating")
            .is_none()
    );
}

#[tokio::test]
async fn delivery_send_claim_is_atomic_across_store_instances() {
    let backend = Arc::new(InMemoryBackend::new());
    let first = Arc::new(build_outbound_store_for_backend(Arc::clone(&backend)));
    let second = Arc::new(build_outbound_store_for_backend(backend));
    let scope = turn_scope();
    let delivery_id = OutboundDeliveryId::new();
    first
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id,
            scope: scope.clone(),
            candidate: OutboundPushCandidate {
                tenant_id: scope.tenant_id.clone(),
                agent_id: scope.agent_id.clone(),
                project_id: scope.project_id.clone(),
                thread_id: scope.thread_id.clone(),
                turn_run_id: Some(TurnRunId::new()),
                target: reply_ref("reply-cross-instance-claim"),
                kind: OutboundPushKind::FinalReply,
                projection_ref: ProjectionUpdateRef::new("projection:cross-instance-claim")
                    .unwrap(),
                requires_reply_target_revalidation: true,
            },
            status: OutboundDeliveryStatus::Prepared,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    let first_request = ClaimDeliveryAttemptForSendRequest {
        delivery_id,
        scope: scope.clone(),
    };
    let second_request = first_request.clone();
    let (first_claim, second_claim) = tokio::join!(
        first.claim_delivery_attempt_for_send(first_request),
        second.claim_delivery_attempt_for_send(second_request),
    );
    let claims = [first_claim.unwrap(), second_claim.unwrap()];
    assert_eq!(claims.into_iter().filter(|claimed| *claimed).count(), 1);

    let attempts = first.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, OutboundDeliveryStatus::Sending);
}

// Legacy LibSqlOutboundStateStore / PostgresOutboundStateStore have been
// deleted. The OutboundStateStore over LibSqlRootFilesystem /
// PostgresRootFilesystem (driven by the production `MountView`) replaces
// them; durability across reopen is now a property of the
// `RootFilesystem` backend, not of an outbound-specific persistence
// implementation.

async fn load_preference_record<S>(
    store: &S,
    key: CommunicationPreferenceKey,
) -> Option<CommunicationPreferenceRecord>
where
    S: CommunicationPreferenceRepository,
{
    store
        .load_communication_preference(key)
        .await
        .unwrap()
        .map(|versioned| versioned.record)
}

async fn write_preference_record<S>(
    store: &S,
    record: CommunicationPreferenceRecord,
    expected_version: Option<CommunicationPreferenceVersion>,
) -> VersionedCommunicationPreferenceRecord
where
    S: CommunicationPreferenceRepository,
{
    store
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record,
            expected_version,
        })
        .await
        .unwrap()
}

async fn communication_preferences_are_tenant_user_scoped<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound").unwrap();
    let user_id = UserId::new("user-outbound").unwrap();
    let updated_by = UserId::new("tenant-admin-outbound").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id.clone(), user_id.clone()),
        legacy_notification_target: Some(reply_ref("reply-pref-legacy")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: updated_by.clone(),
    };
    assert_eq!(record.key(), key);

    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();
    let inserted = store
        .load_communication_preference(key.clone())
        .await
        .unwrap()
        .expect("inserted preference record");
    assert_eq!(
        load_preference_record(store, key.clone()).await,
        Some(record.clone())
    );

    let sibling_user_key = CommunicationPreferenceKey::new(
        tenant_id.clone(),
        UserId::new("user-outbound-sibling").unwrap(),
    );
    assert!(
        store
            .load_communication_preference(sibling_user_key)
            .await
            .unwrap()
            .is_none()
    );

    let sibling_tenant_key =
        CommunicationPreferenceKey::new(TenantId::new("tenant-outbound-sibling").unwrap(), user_id);
    assert!(
        store
            .load_communication_preference(sibling_tenant_key)
            .await
            .unwrap()
            .is_none()
    );

    let updated = CommunicationPreferenceRecord {
        legacy_notification_target: Some(reply_ref("reply-pref-legacy-updated")),
        default_modality: Some(CommunicationModality::Voice),
        updated_at: now(),
        updated_by,
        ..record
    };
    write_preference_record(store, updated.clone(), Some(inserted.version)).await;
    assert_eq!(load_preference_record(store, key).await, Some(updated));

    let thread_policy = store
        .load_thread_notification_policy(turn_scope())
        .await
        .unwrap();
    assert!(
        thread_policy.targets.is_empty(),
        "user communication preferences must not mutate thread notification policy"
    );
}

async fn communication_preferences_are_shared_agent_scoped<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound-shared").unwrap();
    let agent_id = AgentId::new("agent-outbound-shared").unwrap();
    let project_id = ProjectId::new("project-outbound-shared").unwrap();
    let updated_by = UserId::new("tenant-admin-outbound-shared").unwrap();
    let project_key = CommunicationPreferenceKey::shared_agent(
        tenant_id.clone(),
        agent_id.clone(),
        Some(project_id.clone()),
    );
    let project_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::shared_agent(
            tenant_id.clone(),
            agent_id.clone(),
            Some(project_id.clone()),
        ),
        legacy_notification_target: Some(reply_ref("reply-pref-shared-project")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: updated_by.clone(),
    };
    store
        .put_communication_preference(project_record.clone())
        .await
        .unwrap();
    assert_eq!(project_record.key(), project_key);
    assert_eq!(
        load_preference_record(store, project_key.clone()).await,
        Some(project_record)
    );

    let projectless_key =
        CommunicationPreferenceKey::shared_agent(tenant_id.clone(), agent_id.clone(), None);
    let projectless_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::shared_agent(tenant_id.clone(), agent_id.clone(), None),
        legacy_notification_target: Some(reply_ref("reply-pref-shared-projectless")),
        default_modality: Some(CommunicationModality::Voice),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by,
    };
    store
        .put_communication_preference(projectless_record.clone())
        .await
        .unwrap();
    assert_eq!(
        load_preference_record(store, projectless_key).await,
        Some(projectless_record)
    );

    let personal_key = CommunicationPreferenceKey::personal(
        tenant_id,
        UserId::new("user-outbound-shared").unwrap(),
    );
    assert!(
        store
            .load_communication_preference(personal_key)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .load_communication_preference(CommunicationPreferenceKey::shared_agent(
                TenantId::new("tenant-outbound-shared-other").unwrap(),
                agent_id,
                Some(project_id),
            ))
            .await
            .unwrap()
            .is_none()
    );
}

async fn communication_preferences_reject_empty_updated_by<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let valid_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(
            TenantId::new("tenant-outbound-validation").unwrap(),
            UserId::new("user-outbound-validation").unwrap(),
        ),
        legacy_notification_target: Some(reply_ref("reply-pref-validation")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("user-outbound-validation-updater").unwrap(),
    };

    let mut missing_updater = valid_record.clone();
    missing_updater.updated_by = UserId::from_trusted(String::new());
    let result = store.put_communication_preference(missing_updater).await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));

    let mut missing_tenant = valid_record.clone();
    missing_tenant.scope = DeliveryDefaultScope::personal(
        TenantId::from_trusted(String::new()),
        UserId::new("user-outbound-validation").unwrap(),
    );
    let result = store.put_communication_preference(missing_tenant).await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));

    let mut missing_user = valid_record;
    missing_user.scope = DeliveryDefaultScope::personal(
        TenantId::new("tenant-outbound-validation").unwrap(),
        UserId::from_trusted(String::new()),
    );
    let result = store.put_communication_preference(missing_user).await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));
}

async fn communication_preferences_reject_empty_shared_agent_scope<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let valid_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::shared_agent(
            TenantId::new("tenant-outbound-shared-validation").unwrap(),
            AgentId::new("agent-outbound-shared-validation").unwrap(),
            None,
        ),
        legacy_notification_target: Some(reply_ref("reply-pref-shared-validation")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-shared-validation").unwrap(),
    };

    let mut missing_tenant = valid_record.clone();
    missing_tenant.scope = DeliveryDefaultScope::shared_agent(
        TenantId::from_trusted(String::new()),
        AgentId::new("agent-outbound-shared-validation").unwrap(),
        None,
    );
    let result = store.put_communication_preference(missing_tenant).await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));

    let mut missing_agent = valid_record.clone();
    missing_agent.scope = DeliveryDefaultScope::shared_agent(
        TenantId::new("tenant-outbound-shared-validation").unwrap(),
        AgentId::from_trusted(String::new()),
        None,
    );
    let result = store.put_communication_preference(missing_agent).await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));

    let mut missing_project = valid_record;
    missing_project.scope = DeliveryDefaultScope::shared_agent(
        TenantId::new("tenant-outbound-shared-validation").unwrap(),
        AgentId::new("agent-outbound-shared-validation").unwrap(),
        Some(ProjectId::from_trusted(String::new())),
    );
    let result = store.put_communication_preference(missing_project).await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));
}

async fn communication_preference_put_existing_conflicts_without_writing<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound-duplicate").unwrap();
    let user_id = UserId::new("user-outbound-duplicate").unwrap();
    let key = CommunicationPreferenceKey::personal(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-duplicate")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-duplicate").unwrap(),
    };
    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();

    let duplicate = CommunicationPreferenceRecord {
        legacy_notification_target: Some(reply_ref("reply-pref-duplicate-replacement")),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-duplicate-2").unwrap(),
        ..record.clone()
    };
    let result = store.put_communication_preference(duplicate).await;
    assert!(matches!(result, Err(OutboundError::CasConflict)));
    assert_eq!(load_preference_record(store, key).await, Some(record));
}

async fn communication_preference_atomic_update_preserves_untouched_fields<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound-atomic").unwrap();
    let user_id = UserId::new("user-outbound-atomic").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-atomic-final")),
        default_modality: Some(CommunicationModality::Voice),
        // Non-empty so the assertions below actually prove preservation
        // through the atomic update path rather than comparing empty == empty.
        notification_targets: vec![
            OutboundDeliveryTargetId::new("target:atomic-untouched-a").unwrap(),
            OutboundDeliveryTargetId::new("target:atomic-untouched-b").unwrap(),
        ],
        updated_at: now(),
        updated_by: UserId::new("user-outbound-atomic-updater").unwrap(),
    };
    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();

    let existing = store
        .load_communication_preference(key.clone())
        .await
        .unwrap()
        .expect("existing communication preference");
    let updated = write_preference_record(
        store,
        CommunicationPreferenceRecord {
            legacy_notification_target: Some(reply_ref("reply-pref-atomic-final-updated")),
            updated_at: now(),
            updated_by: UserId::new("user-outbound-atomic-updater-2").unwrap(),
            ..existing.record
        },
        Some(existing.version),
    )
    .await
    .record;

    assert_eq!(
        updated.legacy_notification_target,
        Some(reply_ref("reply-pref-atomic-final-updated"))
    );
    assert_eq!(updated.notification_targets, record.notification_targets);
    assert_eq!(updated.default_modality, record.default_modality);
    assert_eq!(load_preference_record(store, key).await, Some(updated));
}

async fn communication_preference_update_inserts_absent_record<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound-update-absent").unwrap();
    let user_id = UserId::new("user-outbound-update-absent").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-update-absent-final")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-update-absent").unwrap(),
    };
    let updated = write_preference_record(store, record.clone(), None)
        .await
        .record;

    assert_eq!(updated, record);
    assert_eq!(load_preference_record(store, key).await, Some(record));
}

async fn communication_preference_stale_version_conflicts_without_writing<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound-update-error").unwrap();
    let user_id = UserId::new("user-outbound-update-error").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-update-error-final")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("user-outbound-update-error-updater").unwrap(),
    };
    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();

    let existing = store
        .load_communication_preference(key.clone())
        .await
        .unwrap()
        .expect("existing communication preference");
    let first_update = CommunicationPreferenceRecord {
        legacy_notification_target: Some(reply_ref("reply-pref-update-error-race")),
        updated_at: now(),
        updated_by: UserId::new("user-outbound-update-error-racer").unwrap(),
        ..existing.record.clone()
    };
    write_preference_record(store, first_update, Some(existing.version)).await;
    let stale_update = CommunicationPreferenceRecord {
        legacy_notification_target: Some(reply_ref("reply-pref-update-error-stale")),
        updated_at: now(),
        updated_by: UserId::new("user-outbound-update-error-stale").unwrap(),
        ..existing.record
    };
    let result = store
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: stale_update,
            expected_version: Some(existing.version),
        })
        .await;

    assert!(matches!(result, Err(OutboundError::CasConflict)));
}

async fn communication_preference_update_rejects_invalid_or_mismatched_record<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStorePort,
{
    let tenant_id = TenantId::new("tenant-outbound-update-invalid").unwrap();
    let user_id = UserId::new("user-outbound-update-invalid").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-update-invalid-final")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("user-outbound-update-invalid-updater").unwrap(),
    };
    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();

    let existing = store
        .load_communication_preference(key.clone())
        .await
        .unwrap()
        .expect("existing communication preference");
    let mut invalid_record = existing.record.clone();
    invalid_record.updated_by = UserId::from_trusted(String::new());
    let invalid_result = store
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: invalid_record,
            expected_version: Some(existing.version),
        })
        .await;
    assert!(matches!(
        invalid_result,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let mut mismatched_record = existing.record;
    mismatched_record.scope = DeliveryDefaultScope::personal(
        TenantId::new("tenant-outbound-update-invalid").unwrap(),
        UserId::new("user-outbound-update-invalid-other").unwrap(),
    );
    let mismatch_result = store
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: mismatched_record,
            expected_version: Some(existing.version),
        })
        .await;
    assert!(matches!(mismatch_result, Err(OutboundError::CasConflict)));
    assert_eq!(load_preference_record(store, key).await, Some(record));
}

/// Write a record with a 2-element `notification_targets` set, reopen the
/// store over the same backend, and read it back unchanged. Then prove a
/// legacy row persisted before this field existed (no `notification_targets`
/// key at all) still loads, defaulting to an empty vec.
async fn notification_targets_round_trip_and_default_empty(backend: &Arc<InMemoryBackend>) {
    let store = build_outbound_store_for_backend(Arc::clone(backend));
    let tenant_id = TenantId::new("tenant-outbound-notification-targets").unwrap();
    let user_id = UserId::new("user-outbound-notification-targets").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let targets = vec![
        OutboundDeliveryTargetId::new("target:notification-a").unwrap(),
        OutboundDeliveryTargetId::new("target:notification-b").unwrap(),
    ];
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: None,
        default_modality: Some(CommunicationModality::Text),
        notification_targets: targets.clone(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-notification-targets").unwrap(),
    };
    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();

    // Reopen: a fresh store instance over the same backend reads back the
    // identical 2-element notification target set.
    let reopened = build_outbound_store_for_backend(Arc::clone(backend));
    assert_eq!(
        load_preference_record(&reopened, key.clone()).await,
        Some(record)
    );

    // A legacy row written before this field existed omits the key
    // entirely. Simulate that by serializing a real record, then stripping
    // `notification_targets` out of the JSON before writing it directly to
    // the backend (bypassing the record's own Serialize impl, which always
    // emits the field).
    let legacy_tenant_id = TenantId::new("tenant-outbound-notification-targets-legacy").unwrap();
    let legacy_user_id = UserId::new("user-outbound-notification-targets-legacy").unwrap();
    let legacy_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(legacy_tenant_id, legacy_user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-notification-targets-legacy")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-notification-targets-legacy").unwrap(),
    };
    let (legacy_key, legacy_path) =
        put_preference_and_find_virtual_path(backend, &store, legacy_record.clone()).await;

    let mut legacy_json = serde_json::to_value(&legacy_record).unwrap();
    legacy_json
        .as_object_mut()
        .expect("record serializes to a JSON object")
        .remove("notification_targets");
    let entry = Entry::bytes(serde_json::to_vec(&legacy_json).unwrap())
        .with_content_type(ContentType::json());
    backend
        .put(&legacy_path, entry, CasExpectation::Any)
        .await
        .unwrap();

    let reloaded_legacy = store
        .load_communication_preference(legacy_key)
        .await
        .unwrap()
        .expect("legacy preference row without notification_targets");
    assert!(reloaded_legacy.record.notification_targets.is_empty());
}

async fn outbound_state_store_rejects_mismatched_communication_preference_identity(
    backend: &Arc<InMemoryBackend>,
    store: &OutboundStateStore<InMemoryBackend>,
) {
    let tenant_id = TenantId::new("tenant-outbound-corrupt").unwrap();
    let user_id = UserId::new("user-outbound-corrupt").unwrap();
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id.clone(), user_id.clone()),
        legacy_notification_target: Some(reply_ref("reply-pref-corrupt")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-corrupt").unwrap(),
    };
    let (key, path) = put_preference_and_find_virtual_path(backend, store, record.clone()).await;

    let mut user_mismatch_record = record;
    user_mismatch_record.scope = DeliveryDefaultScope::personal(
        tenant_id.clone(),
        UserId::new("user-outbound-corrupt-other").unwrap(),
    );
    let entry = Entry::bytes(serde_json::to_vec(&user_mismatch_record).unwrap())
        .with_content_type(ContentType::json());
    backend
        .put(&path, entry, CasExpectation::Any)
        .await
        .unwrap();

    let result = store.load_communication_preference(key.clone()).await;
    assert!(matches!(result, Err(OutboundError::Backend)));

    let tenant_mismatch_tenant_id = TenantId::new("tenant-outbound-corrupt-tenant").unwrap();
    let tenant_mismatch_user_id = UserId::new("user-outbound-corrupt-tenant").unwrap();
    let tenant_mismatch_seed = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(
            tenant_mismatch_tenant_id,
            tenant_mismatch_user_id.clone(),
        ),
        legacy_notification_target: Some(reply_ref("reply-pref-corrupt-tenant-seed")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-corrupt-tenant-seed").unwrap(),
    };
    let (tenant_mismatch_key, tenant_mismatch_path) =
        put_preference_and_find_virtual_path(backend, store, tenant_mismatch_seed).await;
    let tenant_mismatch_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(
            TenantId::new("tenant-outbound-corrupt-other").unwrap(),
            tenant_mismatch_user_id,
        ),
        legacy_notification_target: Some(reply_ref("reply-pref-corrupt-tenant")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-corrupt-tenant").unwrap(),
    };
    let tenant_mismatch_entry = Entry::bytes(serde_json::to_vec(&tenant_mismatch_record).unwrap())
        .with_content_type(ContentType::json());
    backend
        .put(
            &tenant_mismatch_path,
            tenant_mismatch_entry,
            CasExpectation::Any,
        )
        .await
        .unwrap();

    let result = store
        .load_communication_preference(tenant_mismatch_key)
        .await;
    assert!(matches!(result, Err(OutboundError::Backend)));
}

#[tokio::test]
async fn outbound_state_store_personal_and_shared_agent_hashes_are_always_distinct() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(Arc::clone(&backend));
    let tenant_id = TenantId::new("tenant-outbound-hash-distinct").unwrap();
    let shared_id = "same-principal-id";
    let personal_key =
        CommunicationPreferenceKey::personal(tenant_id.clone(), UserId::new(shared_id).unwrap());
    let personal_record = CommunicationPreferenceRecord {
        scope: personal_key.scope.clone(),
        legacy_notification_target: Some(reply_ref("reply-pref-hash-personal")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-hash-personal").unwrap(),
    };
    let (_, personal_path) =
        put_preference_and_find_virtual_path(&backend, &store, personal_record.clone()).await;

    let shared_key =
        CommunicationPreferenceKey::shared_agent(tenant_id, AgentId::new(shared_id).unwrap(), None);
    let shared_record = CommunicationPreferenceRecord {
        scope: shared_key.scope.clone(),
        legacy_notification_target: Some(reply_ref("reply-pref-hash-shared")),
        default_modality: Some(CommunicationModality::Voice),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-hash-shared").unwrap(),
    };
    let (_, shared_path) =
        put_preference_and_find_virtual_path(&backend, &store, shared_record.clone()).await;

    assert_ne!(
        personal_path, shared_path,
        "personal and shared-agent preference scopes with the same id text must not share a v2 hash path",
    );
    assert_eq!(
        communication_preference_virtual_paths(&backend).await.len(),
        2
    );
    assert_eq!(
        load_preference_record(&store, personal_key).await,
        Some(personal_record)
    );
    assert_eq!(
        load_preference_record(&store, shared_key).await,
        Some(shared_record)
    );
}

async fn outbound_state_store_rejects_communication_preference_put_cas_conflict(
    backend: &Arc<InMemoryBackend>,
) {
    let racing = Arc::new(VersionRacingBackend::new(Arc::clone(backend)));
    let store = OutboundStateStore::new(build_scoped_fs(Arc::clone(&racing), TEST_OUTBOUND_ROOT));
    let tenant_id = TenantId::new("tenant-outbound-cas").unwrap();
    let user_id = UserId::new("user-outbound-cas").unwrap();
    racing
        .arm(
            &format!("{TEST_OUTBOUND_ROOT}/communication-preferences/"),
            1,
        )
        .await;

    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id.clone(), user_id.clone()),
        legacy_notification_target: Some(reply_ref("reply-pref-cas")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-cas").unwrap(),
    };
    let result = store.put_communication_preference(record).await;
    assert!(matches!(result, Err(OutboundError::CasConflict)));
    assert_eq!(
        load_preference_record(&store, CommunicationPreferenceKey::new(tenant_id, user_id),).await,
        None
    );
    assert_eq!(racing.injected_count().await, 1);
}

async fn outbound_state_store_rejects_communication_preference_update_cas_conflict(
    backend: &Arc<InMemoryBackend>,
) {
    let racing = Arc::new(VersionRacingBackend::new(Arc::clone(backend)));
    let store = OutboundStateStore::new(build_scoped_fs(Arc::clone(&racing), TEST_OUTBOUND_ROOT));
    let tenant_id = TenantId::new("tenant-outbound-update-cas").unwrap();
    let user_id = UserId::new("user-outbound-update-cas").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-update-cas")),
        default_modality: Some(CommunicationModality::Voice),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-update-cas").unwrap(),
    };
    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();
    racing
        .arm(
            &format!("{TEST_OUTBOUND_ROOT}/communication-preferences/"),
            1,
        )
        .await;

    let existing = store
        .load_communication_preference(key.clone())
        .await
        .unwrap()
        .expect("existing communication preference");
    let updated = CommunicationPreferenceRecord {
        legacy_notification_target: Some(reply_ref("reply-pref-update-cas-final-updated")),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-update-cas-2").unwrap(),
        ..existing.record
    };
    let result = store
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: updated,
            expected_version: Some(existing.version),
        })
        .await;

    assert!(matches!(result, Err(OutboundError::CasConflict)));
    assert_eq!(racing.injected_count().await, 1);
    assert_eq!(load_preference_record(&store, key).await, Some(record));
}

#[tokio::test]
async fn outbound_state_store_rejects_communication_preference_write_on_unsupported_cas_mount() {
    let inner = Arc::new(InMemoryBackend::new());
    let backend = Arc::new(UnsupportedCriticalCasBackend::new(Arc::clone(&inner)));
    let store = OutboundStateStore::new(build_scoped_fs(Arc::clone(&backend), TEST_OUTBOUND_ROOT));
    let tenant_id = TenantId::new("tenant-outbound-unsupported-cas").unwrap();
    let user_id = UserId::new("user-outbound-unsupported-cas").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        legacy_notification_target: Some(reply_ref("reply-pref-unsupported-cas")),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-unsupported-cas").unwrap(),
    };

    let result = store
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record,
            expected_version: None,
        })
        .await;

    assert!(matches!(result, Err(OutboundError::Backend)));
    assert_eq!(backend.unsupported_count().await, 1);
    assert_eq!(load_preference_record(&store, key).await, None);
}

#[tokio::test]
async fn delivery_send_claim_fails_closed_on_unsupported_cas_mount() {
    let inner = Arc::new(InMemoryBackend::new());
    let backend = Arc::new(UnsupportedCriticalCasBackend::new(Arc::clone(&inner)));
    let store = OutboundStateStore::new(build_scoped_fs(Arc::clone(&backend), TEST_OUTBOUND_ROOT));
    let scope = turn_scope();
    let delivery_id = OutboundDeliveryId::new();
    store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id,
            scope: scope.clone(),
            candidate: OutboundPushCandidate {
                tenant_id: scope.tenant_id.clone(),
                agent_id: scope.agent_id.clone(),
                project_id: scope.project_id.clone(),
                thread_id: scope.thread_id.clone(),
                turn_run_id: Some(TurnRunId::new()),
                target: reply_ref("reply-unsupported-claim"),
                kind: OutboundPushKind::FinalReply,
                projection_ref: ProjectionUpdateRef::new("projection:unsupported-claim").unwrap(),
                requires_reply_target_revalidation: true,
            },
            status: OutboundDeliveryStatus::Prepared,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    let claim = store
        .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
            delivery_id,
            scope: scope.clone(),
        })
        .await;
    assert!(matches!(claim, Err(OutboundError::Backend)));
    assert_eq!(backend.unsupported_count().await, 1);
    let attempt = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempt[0].status, OutboundDeliveryStatus::Prepared);
}

async fn durable_policy_subscription_delivery_flow(store: &impl OutboundStateStorePort) {
    let scope = turn_scope();
    let default_reply = reply_ref("reply-default");
    let extra_final = reply_ref("reply-extra-final");
    let progress_target = reply_ref("reply-progress");

    let default_final = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:final-1").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(targets(&default_final), vec![default_reply.clone()]);

    let default_progress = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: None,
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::Progress,
            projection_ref: ProjectionUpdateRef::new("projection:progress-1").unwrap(),
        })
        .await
        .unwrap();
    assert!(default_progress.candidates.is_empty());

    // ModelDelivery behaves like FinalReply: it must reach the ordinary
    // reply_target even with no thread notification policy configured yet
    // (no opted-in extra targets to fall back on).
    let default_model_delivery = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::ModelDelivery,
            projection_ref: ProjectionUpdateRef::new("projection:model-delivery-1").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&default_model_delivery),
        vec![default_reply.clone()]
    );

    store
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: scope.clone(),
            targets: vec![
                ThreadNotificationTarget {
                    target: extra_final.clone(),
                    final_replies: true,
                    progress: false,
                },
                ThreadNotificationTarget {
                    target: progress_target.clone(),
                    final_replies: false,
                    progress: true,
                },
                ThreadNotificationTarget {
                    target: default_reply.clone(),
                    final_replies: true,
                    progress: true,
                },
            ],
        })
        .await
        .unwrap();

    let final_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:final-2").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&final_plan),
        vec![default_reply.clone(), extra_final]
    );
    assert!(
        final_plan
            .candidates
            .iter()
            .all(|candidate| candidate.requires_reply_target_revalidation)
    );

    let progress_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: None,
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::Progress,
            projection_ref: ProjectionUpdateRef::new("projection:progress-2").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&progress_plan),
        vec![progress_target.clone(), default_reply.clone()]
    );

    let auth_prompt_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: None,
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::AuthPrompt,
            projection_ref: ProjectionUpdateRef::new("projection:auth-prompt-1").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&auth_prompt_plan),
        vec![progress_target, default_reply.clone()]
    );

    seed_subscription(store).await;
    let initial_cursor = ProjectionCursor::origin_for_scope(projection_scope());
    let loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded, initial_cursor,
        "subscription creation must retain the caller's initial after_cursor"
    );

    let delivery_id = OutboundDeliveryId::new();
    let initial_attempt = OutboundDeliveryAttempt {
        delivery_id,
        scope: scope.clone(),
        candidate: final_plan.candidates[0].clone(),
        status: OutboundDeliveryStatus::Pending,
        attempted_at: now(),
        failure_kind: None,
    };
    store
        .record_delivery_attempt(initial_attempt.clone())
        .await
        .unwrap();
    let wrong_scope_update = store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: sibling_turn_scope(),
            status: OutboundDeliveryStatus::Failed,
            updated_at: now(),
            failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
        })
        .await;
    assert!(matches!(
        wrong_scope_update,
        Err(OutboundError::SubscriptionScopeMismatch)
    ));

    store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Failed,
            updated_at: now(),
            failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
        })
        .await
        .unwrap();

    store
        .record_delivery_attempt(initial_attempt)
        .await
        .unwrap();
    let after_duplicate_retry = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(after_duplicate_retry.len(), 1);
    assert_eq!(
        after_duplicate_retry[0].status,
        OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        after_duplicate_retry[0].failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );

    let duplicate_different_candidate = store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id,
            scope: scope.clone(),
            candidate: progress_plan.candidates[0].clone(),
            status: OutboundDeliveryStatus::Pending,
            attempted_at: now(),
            failure_kind: None,
        })
        .await;
    assert!(matches!(
        duplicate_different_candidate,
        Err(OutboundError::Backend)
    ));

    let deliveries = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        deliveries[0].failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );

    let policy_after_failure = store
        .load_thread_notification_policy(scope.clone())
        .await
        .unwrap();
    assert_eq!(policy_after_failure.targets.len(), 3);

    full_turn_scope_isolation(store, scope).await;
}

async fn seed_subscription(store: &impl OutboundStateStorePort) {
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::origin_for_scope(projection_scope())),
        })
        .await
        .unwrap();
}

async fn subscription_cursor_rejects_mismatched_scope(store: &impl OutboundStateStorePort) {
    let wrong_actor = TurnActor::new(UserId::new("user-other").unwrap());
    let result = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: wrong_actor,
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await;
    // Anti-enumeration: wrong actor/scope reads look identical to missing
    // subscription ids, so callers cannot distinguish an existing foreign row
    // from absence.
    assert!(matches!(result, Ok(None)));

    let rebind = store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id(),
            actor: TurnActor::new(UserId::new("user-other").unwrap()),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::for_scope(
                projection_scope(),
                EventCursor::new(99),
            )),
        })
        .await;
    assert!(matches!(
        rebind,
        Err(OutboundError::SubscriptionScopeMismatch)
    ));
}

async fn subscription_ids_are_scoped_not_global(store: &impl OutboundStateStorePort) {
    let shared_subscription_id =
        ProjectionSubscriptionId::new(format!("webui-scoped-subscription-{}", TurnRunId::new()))
            .unwrap();
    let base_cursor = ProjectionCursor::for_scope(projection_scope(), EventCursor::new(10));
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: shared_subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(base_cursor.clone()),
        })
        .await
        .unwrap();

    let sibling_actor = TurnActor::new(UserId::new("user-outbound-sibling").unwrap());
    let sibling_scope = projection_scope_for_user("user-outbound-sibling");
    let sibling_cursor = ProjectionCursor::for_scope(sibling_scope.clone(), EventCursor::new(3));
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: shared_subscription_id.clone(),
            actor: sibling_actor.clone(),
            scope: sibling_scope.clone(),
            thread_id: thread_id(),
            cursor: Some(sibling_cursor.clone()),
        })
        .await
        .unwrap();

    let base_loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: shared_subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(base_loaded, base_cursor);

    let sibling_loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: shared_subscription_id.clone(),
            actor: sibling_actor.clone(),
            scope: sibling_scope.clone(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sibling_loaded, sibling_cursor);

    let unrelated_actor = TurnActor::new(UserId::new("user-outbound-unrelated").unwrap());
    let unrelated_scope = projection_scope_for_user("user-outbound-unrelated");
    let unrelated_lookup = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: shared_subscription_id.clone(),
            actor: unrelated_actor,
            scope: unrelated_scope,
            thread_id: thread_id(),
        })
        .await;
    // Anti-enumeration: even when the id exists for sibling tuples, an
    // unrelated tuple receives the same `None` result as a missing id.
    assert!(matches!(unrelated_lookup, Ok(None)));
}

async fn subscription_upsert_rejects_backward_cursor(store: &impl OutboundStateStorePort) {
    let subscription_id =
        ProjectionSubscriptionId::new(format!("webui-subscription-backward-{}", TurnRunId::new()))
            .unwrap();
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::for_scope(
                projection_scope(),
                EventCursor::new(42),
            )),
        })
        .await
        .unwrap();

    let stale_upsert = store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::for_scope(
                projection_scope(),
                EventCursor::new(6),
            )),
        })
        .await;
    assert!(matches!(
        stale_upsert,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id,
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.runtime, EventCursor::new(42));
}

/// The coordinator's crash-visible lifecycle (`Prepared` → `Sending` →
/// terminal) persists and reloads on every backend: a crash between vendor
/// egress and the result write leaves `Sending`, which recovery marks
/// `Unknown` — never a blind resend (OUT-3/OUT-6/OUT-9).
async fn coordinator_delivery_lifecycle_round_trips(store: &impl OutboundStateStorePort) {
    let scope = turn_scope();
    let delivery_id = OutboundDeliveryId::new();
    let candidate = OutboundPushCandidate {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        thread_id: scope.thread_id.clone(),
        turn_run_id: Some(TurnRunId::new()),
        target: reply_ref("reply-coordinator-lifecycle"),
        kind: OutboundPushKind::FinalReply,
        projection_ref: ProjectionUpdateRef::new(format!(
            "projection:coordinator-lifecycle:{}",
            TurnRunId::new()
        ))
        .unwrap(),
        requires_reply_target_revalidation: true,
    };
    store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id,
            scope: scope.clone(),
            candidate,
            status: OutboundDeliveryStatus::Prepared,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    assert!(
        store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap(),
        "the first caller atomically owns vendor egress"
    );
    assert!(
        !store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap(),
        "a replay cannot claim the same durable attempt"
    );
    let wrong_scope_claim = store
        .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
            delivery_id,
            scope: sibling_turn_scope(),
        })
        .await;
    assert!(matches!(
        wrong_scope_claim,
        Err(OutboundError::DeliveryNotFound | OutboundError::SubscriptionScopeMismatch)
    ));
    let in_flight = store.list_delivery_attempts(scope.clone()).await.unwrap();
    let attempt = in_flight
        .iter()
        .find(|attempt| attempt.delivery_id == delivery_id)
        .expect("attempt persisted");
    assert_eq!(attempt.status, OutboundDeliveryStatus::Sending);

    // `Unknown` never carries a failure kind (the outcome is ambiguous, not
    // a known failure) — and a kind-carrying Unknown is rejected.
    let unknown_with_kind = store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Unknown,
            updated_at: now(),
            failure_kind: Some(DeliveryFailureKind::TransportUnavailable),
        })
        .await;
    assert!(matches!(
        unknown_with_kind,
        Err(OutboundError::InvalidRequest { .. })
    ));

    store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Unknown,
            updated_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();
    let settled = store.list_delivery_attempts(scope.clone()).await.unwrap();
    let attempt = settled
        .iter()
        .find(|attempt| attempt.delivery_id == delivery_id)
        .expect("attempt persisted");
    assert_eq!(attempt.status, OutboundDeliveryStatus::Unknown);
}

async fn recovery_transition_never_clobbers_delivered(store: &impl OutboundStateStorePort) {
    let scope = turn_scope();
    let candidate = |marker: &str| OutboundPushCandidate {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        thread_id: scope.thread_id.clone(),
        turn_run_id: Some(TurnRunId::new()),
        target: reply_ref(marker),
        kind: OutboundPushKind::FinalReply,
        projection_ref: ProjectionUpdateRef::new(format!(
            "projection:{marker}:{}",
            TurnRunId::new()
        ))
        .unwrap(),
        requires_reply_target_revalidation: true,
    };

    // A genuinely-interrupted send is still `Sending`: recovery re-verifies
    // that under CAS and transitions it to `Unknown`, reporting the conversion.
    let interrupted = OutboundDeliveryId::new();
    store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id: interrupted,
            scope: scope.clone(),
            candidate: candidate("reply-recovery-interrupted"),
            status: OutboundDeliveryStatus::Prepared,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();
    assert!(
        store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id: interrupted,
                scope: scope.clone(),
            })
            .await
            .unwrap()
    );
    assert!(
        store
            .recover_interrupted_delivery_attempt(RecoverInterruptedDeliveryRequest {
                delivery_id: interrupted,
                scope: scope.clone(),
            })
            .await
            .unwrap(),
        "a still-Sending attempt is recovered to Unknown"
    );
    let attempts = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(
        attempts
            .iter()
            .find(|attempt| attempt.delivery_id == interrupted)
            .expect("interrupted attempt persisted")
            .status,
        OutboundDeliveryStatus::Unknown
    );

    // The crash-recovery race: another worker completed egress and durably
    // wrote `Delivered`, while a stale recovery list snapshot still believes
    // the attempt is `Sending`. Re-verifying `Sending` inside the same CAS read
    // must no-op instead of clobbering the successful delivery to `Unknown`.
    let delivered = OutboundDeliveryId::new();
    store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id: delivered,
            scope: scope.clone(),
            candidate: candidate("reply-recovery-delivered"),
            status: OutboundDeliveryStatus::Prepared,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();
    assert!(
        store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id: delivered,
                scope: scope.clone(),
            })
            .await
            .unwrap()
    );
    store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id: delivered,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Delivered,
            updated_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    assert!(
        !store
            .recover_interrupted_delivery_attempt(RecoverInterruptedDeliveryRequest {
                delivery_id: delivered,
                scope: scope.clone(),
            })
            .await
            .unwrap(),
        "recovery must not claim an attempt that already advanced past Sending"
    );
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts
            .iter()
            .find(|attempt| attempt.delivery_id == delivered)
            .expect("delivered attempt persisted")
            .status,
        OutboundDeliveryStatus::Delivered,
        "a successful delivery must never be clobbered back to Unknown by stale recovery"
    );
}

async fn delivery_status_rejects_inconsistent_failure_kind(store: &impl OutboundStateStorePort) {
    let scope = turn_scope();
    let delivery_id = OutboundDeliveryId::new();
    let attempt = OutboundDeliveryAttempt {
        delivery_id,
        scope: scope.clone(),
        candidate: OutboundPushCandidate {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
            turn_run_id: Some(TurnRunId::new()),
            target: reply_ref("reply-status-validation"),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new(format!(
                "projection:status-validation:{}",
                TurnRunId::new()
            ))
            .unwrap(),
            requires_reply_target_revalidation: true,
        },
        status: OutboundDeliveryStatus::Pending,
        attempted_at: now(),
        failure_kind: None,
    };
    store.record_delivery_attempt(attempt).await.unwrap();

    let delivered_with_failure = store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Delivered,
            updated_at: now(),
            failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
        })
        .await;
    assert!(matches!(
        delivered_with_failure,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let failed_without_failure = store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Failed,
            updated_at: now(),
            failure_kind: None,
        })
        .await;
    assert!(matches!(
        failed_without_failure,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let deliveries = store.list_delivery_attempts(scope).await.unwrap();
    let stored = deliveries
        .iter()
        .find(|attempt| attempt.delivery_id == delivery_id)
        .unwrap();
    assert_eq!(stored.status, OutboundDeliveryStatus::Pending);
    assert_eq!(stored.failure_kind, None);
}

async fn notification_policy_rejects_excessive_targets(store: &impl OutboundStateStorePort) {
    let targets = (0..33)
        .map(|i| ThreadNotificationTarget {
            target: reply_ref(&format!("reply-too-many-{i}")),
            final_replies: true,
            progress: false,
        })
        .collect();
    let result = store
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: turn_scope(),
            targets,
        })
        .await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));
}

async fn full_turn_scope_isolation(store: &impl OutboundStateStorePort, original_scope: TurnScope) {
    let sibling_scope = sibling_turn_scope();
    let sibling_target = reply_ref("reply-sibling");
    store
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: sibling_scope.clone(),
            targets: vec![ThreadNotificationTarget {
                target: sibling_target.clone(),
                final_replies: true,
                progress: true,
            }],
        })
        .await
        .unwrap();

    let original_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: original_scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: reply_ref("reply-default"),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:isolated-original").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&original_plan),
        vec![reply_ref("reply-default"), reply_ref("reply-extra-final")]
    );

    let sibling_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: sibling_scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: reply_ref("reply-sibling-default"),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:isolated-sibling").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&sibling_plan),
        vec![reply_ref("reply-sibling-default"), sibling_target]
    );

    let sibling_delivery_id = OutboundDeliveryId::new();
    store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id: sibling_delivery_id,
            scope: sibling_scope.clone(),
            candidate: sibling_plan.candidates[0].clone(),
            status: OutboundDeliveryStatus::Pending,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    let original_deliveries = store.list_delivery_attempts(original_scope).await.unwrap();
    assert_eq!(original_deliveries.len(), 1);
    let sibling_deliveries = store.list_delivery_attempts(sibling_scope).await.unwrap();
    assert_eq!(sibling_deliveries.len(), 1);
    assert_eq!(sibling_deliveries[0].delivery_id, sibling_delivery_id);
}

fn targets(plan: &OutboundPushPlan) -> Vec<ReplyTargetBindingRef> {
    plan.candidates
        .iter()
        .map(|candidate| candidate.target.clone())
        .collect()
}

fn subscription_id() -> ProjectionSubscriptionId {
    ProjectionSubscriptionId::new("webui-subscription-1").unwrap()
}

fn turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-outbound").unwrap(),
        Some(AgentId::new("agent-outbound").unwrap()),
        Some(ProjectId::new("project-outbound").unwrap()),
        thread_id(),
    )
}

fn sibling_turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-outbound").unwrap(),
        Some(AgentId::new("agent-outbound-other").unwrap()),
        Some(ProjectId::new("project-outbound-other").unwrap()),
        thread_id(),
    )
}

fn projection_scope() -> ProjectionScope {
    projection_scope_for_user("user-outbound")
}

fn projection_scope_for_user(user_id: &str) -> ProjectionScope {
    ProjectionScope {
        stream: EventStreamKey::new(
            TenantId::new("tenant-outbound").unwrap(),
            UserId::new(user_id).unwrap(),
            Some(AgentId::new("agent-outbound").unwrap()),
        ),
        read_scope: ReadScope {
            project_id: Some(ProjectId::new("project-outbound").unwrap()),
            mission_id: None,
            thread_id: Some(thread_id()),
            process_id: None,
        },
    }
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user-outbound").unwrap())
}

fn thread_id() -> ThreadId {
    ThreadId::new("thread-outbound").unwrap()
}

fn reply_ref(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).unwrap()
}

fn now() -> ironclaw_host_api::Timestamp {
    chrono::Utc::now()
}

async fn put_preference_and_find_virtual_path(
    backend: &Arc<InMemoryBackend>,
    store: &OutboundStateStore<InMemoryBackend>,
    record: CommunicationPreferenceRecord,
) -> (CommunicationPreferenceKey, VirtualPath) {
    let before = communication_preference_virtual_paths(backend).await;
    let key = record.key();
    store.put_communication_preference(record).await.unwrap();
    let mut added = communication_preference_virtual_paths(backend)
        .await
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect::<Vec<_>>();
    assert_eq!(added.len(), 1);
    (key, added.remove(0))
}

async fn communication_preference_virtual_paths(
    backend: &Arc<InMemoryBackend>,
) -> Vec<VirtualPath> {
    let root = VirtualPath::new(format!("{TEST_OUTBOUND_ROOT}/communication-preferences")).unwrap();
    let mut paths = backend
        .list_dir(&root)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    paths
}

// ── F4 — CAS retry / drain / backwards-race regression tests ─────────────

/// Test backend that wraps an inner [`RootFilesystem`] and injects a single
/// [`FilesystemError::VersionMismatch`] on the next `put` to any path matching
/// the configured prefix. The injection auto-disarms after firing once so the
/// retry pass forwards to the inner backend and converges.
///
/// Audit finding F4: the existing contract suite never exercised the CAS
/// retry loop introduced for F1. This mock proves the retry budget actually
/// converges on a transient race rather than failing the first attempt.
struct VersionRacingBackend {
    inner: Arc<InMemoryBackend>,
    state: Mutex<RacingState>,
}

struct RacingState {
    /// Path prefix to inject conflicts on. `None` = no injection scheduled.
    target_prefix: Option<String>,
    /// Total number of injected conflicts produced so far.
    injected: u32,
    /// Remaining injections; decrements per fired conflict.
    remaining: u32,
}

impl VersionRacingBackend {
    fn new(inner: Arc<InMemoryBackend>) -> Self {
        Self {
            inner,
            state: Mutex::new(RacingState {
                target_prefix: None,
                injected: 0,
                remaining: 0,
            }),
        }
    }

    /// Arm the backend to inject `count` `VersionMismatch` errors on the next
    /// `count` `put` calls whose path starts with `prefix`. Tests use this to
    /// simulate a single racing writer landing between our read and put.
    async fn arm(&self, prefix: &str, count: u32) {
        let mut state = self.state.lock().await;
        state.target_prefix = Some(prefix.to_string());
        state.injected = 0;
        state.remaining = count;
    }

    async fn injected_count(&self) -> u32 {
        self.state.lock().await.injected
    }
}

#[async_trait]
impl RootFilesystem for VersionRacingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        {
            let mut state = self.state.lock().await;
            if state.remaining > 0
                && state
                    .target_prefix
                    .as_deref()
                    .is_some_and(|prefix| path.as_str().starts_with(prefix))
            {
                state.remaining -= 1;
                state.injected += 1;
                // Surface as if the path's version had advanced under us.
                return Err(FilesystemError::VersionMismatch {
                    path: path.clone(),
                    expected: None,
                    found: None,
                });
            }
        }
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

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

/// Synchronization decorator for deterministic two-store conditional-delete
/// races. This is an interleaving barrier, not an I/O fault fake: every
/// operation delegates to the real [`InMemoryBackend`].
struct DeletePauseBackend {
    inner: Arc<InMemoryBackend>,
    pause_point: AtomicU8,
    delete_entered: Notify,
    release_delete: Notify,
}

impl DeletePauseBackend {
    const NONE: u8 = 0;
    const BEFORE_DELETE: u8 = 1;
    const AFTER_DELETE: u8 = u8::MAX;

    fn new(inner: Arc<InMemoryBackend>) -> Self {
        Self {
            inner,
            pause_point: AtomicU8::new(Self::NONE),
            delete_entered: Notify::new(),
            release_delete: Notify::new(),
        }
    }

    fn arm_before_delete(&self) {
        self.pause_point
            .store(Self::BEFORE_DELETE, Ordering::SeqCst);
    }

    fn arm_before_deletes(&self, count: u8) {
        assert!(count < Self::AFTER_DELETE);
        self.pause_point.store(count, Ordering::SeqCst);
    }

    fn arm_after_delete(&self) {
        self.pause_point.store(Self::AFTER_DELETE, Ordering::SeqCst);
    }

    async fn wait_for_delete(&self) {
        self.delete_entered.notified().await;
    }

    fn release_delete(&self) {
        self.release_delete.notify_one();
    }

    async fn pause(&self) {
        self.delete_entered.notify_one();
        self.release_delete.notified().await;
    }
}

#[async_trait]
impl RootFilesystem for DeletePauseBackend {
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

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn delete_if_version(
        &self,
        path: &VirtualPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        let pause_point = self
            .pause_point
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current == Self::AFTER_DELETE {
                    Some(Self::NONE)
                } else if current > Self::NONE {
                    Some(current - 1)
                } else {
                    None
                }
            })
            .unwrap_or(Self::NONE);
        match pause_point {
            Self::NONE => self.inner.delete_if_version(path, expected_version).await,
            Self::AFTER_DELETE => {
                let result = self.inner.delete_if_version(path, expected_version).await;
                self.pause().await;
                result
            }
            _ => {
                self.pause().await;
                self.inner.delete_if_version(path, expected_version).await
            }
        }
    }
}

/// Test backend that mimics a mount that cannot honor CAS writes for critical
/// preference updates or delivery ownership claims. An accidental byte
/// fallback would retry as `CasExpectation::Any` and succeed through the inner
/// backend, so the tests above prove both operations fail closed instead.
struct UnsupportedCriticalCasBackend {
    inner: Arc<InMemoryBackend>,
    unsupported: Mutex<u32>,
}

impl UnsupportedCriticalCasBackend {
    fn new(inner: Arc<InMemoryBackend>) -> Self {
        Self {
            inner,
            unsupported: Mutex::new(0),
        }
    }

    async fn unsupported_count(&self) -> u32 {
        *self.unsupported.lock().await
    }
}

#[async_trait]
impl RootFilesystem for UnsupportedCriticalCasBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let preference_requires_cas = path
            .as_str()
            .starts_with(&format!("{TEST_OUTBOUND_ROOT}/communication-preferences/"))
            && !matches!(cas, CasExpectation::Any);
        let delivery_claim_requires_cas = path
            .as_str()
            .starts_with(&format!("{TEST_OUTBOUND_ROOT}/deliveries/"))
            && matches!(cas, CasExpectation::Version(_));
        if preference_requires_cas || delivery_claim_requires_cas {
            *self.unsupported.lock().await += 1;
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            });
        }
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

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

/// Audit finding F4 + F3: write more than `Page::MAX_LIMIT` (1024) delivery
/// attempts for the same scope and assert `list_delivery_attempts` returns
/// every one. Before F3 the unpaginated `list_dir` would silently truncate
/// past 1024 rows; with the drain loop, the consumer sees the full set.
#[tokio::test]
async fn list_delivery_attempts_drains_more_than_page_max_limit() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(backend);

    let scope = turn_scope();
    let candidate_template = || OutboundPushCandidate {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        thread_id: scope.thread_id.clone(),
        turn_run_id: Some(TurnRunId::new()),
        target: reply_ref("reply-drain"),
        kind: OutboundPushKind::FinalReply,
        projection_ref: ProjectionUpdateRef::new(format!("projection:drain:{}", TurnRunId::new()))
            .unwrap(),
        requires_reply_target_revalidation: true,
    };

    // One past the page limit so the drain loop has to execute at least two
    // iterations to surface the tail. 1025 keeps the test fast in CI.
    let total: usize = (Page::MAX_LIMIT as usize) + 1;
    for _ in 0..total {
        store
            .record_delivery_attempt(OutboundDeliveryAttempt {
                delivery_id: OutboundDeliveryId::new(),
                scope: scope.clone(),
                candidate: candidate_template(),
                status: OutboundDeliveryStatus::Pending,
                attempted_at: now(),
                failure_kind: None,
            })
            .await
            .unwrap();
    }

    let drained = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        drained.len(),
        total,
        "drain loop must return every delivery, including rows past Page::MAX_LIMIT",
    );
}

/// Regression test mirroring the engine-store
/// `outbound_state_store_isolates_two_tenants_with_same_user_project_ids`
/// shape: the outbound store must enforce tenant isolation through the
/// [`ScopedFilesystem`] mount permission boundary, not assume path strings
/// inside outbound code already encode tenant identity.
///
/// Two stores share one [`InMemoryBackend`] but are constructed with
/// different [`MountView`]s — each one resolves the `/outbound` alias to a
/// distinct tenant-scoped [`VirtualPath`] subtree. Writing the same
/// `(user_id, project_id, thread_id)` tuple on store A must NOT make the
/// delivery / policy visible from store B. Before the migration to
/// `Arc<ScopedFilesystem<F>>`, the outbound store spoke raw `VirtualPath`s
/// directly to a `RootFilesystem` and threaded tenant identity into the
/// hash key only — any composition layer that forgot to also discriminate
/// by tenant in the path would leak across tenants; this test fails closed
/// if that ever regresses.
#[tokio::test]
async fn filesystem_outbound_store_isolates_two_tenants_with_same_user_project_ids() {
    let backend = Arc::new(InMemoryBackend::new());
    let store_a = OutboundStateStore::new(build_scoped_fs(
        Arc::clone(&backend),
        "/engine/tenants/a/users/alice/outbound",
    ));
    let store_b = OutboundStateStore::new(build_scoped_fs(
        Arc::clone(&backend),
        "/engine/tenants/b/users/alice/outbound",
    ));

    // Identical `(agent_id, project_id, thread_id)` for both stores — the
    // only thing that should keep them apart is the mount-time tenant
    // prefix. The TurnScope still carries each store's own tenant_id so
    // policy/cursor lookups validate end-to-end.
    let shared_agent = AgentId::new("agent-shared").unwrap();
    let shared_project = ProjectId::new("project-shared").unwrap();
    let shared_thread = ThreadId::new("thread-shared").unwrap();
    let scope_a = TurnScope::new(
        TenantId::new("tenant-a").unwrap(),
        Some(shared_agent.clone()),
        Some(shared_project.clone()),
        shared_thread.clone(),
    );
    let scope_b = TurnScope::new(
        TenantId::new("tenant-b").unwrap(),
        Some(shared_agent),
        Some(shared_project),
        shared_thread,
    );

    let target = reply_ref("reply-tenant-isolation");
    store_a
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: scope_a.clone(),
            targets: vec![ThreadNotificationTarget {
                target: target.clone(),
                final_replies: true,
                progress: true,
            }],
        })
        .await
        .unwrap();

    // Tenant A sees its own policy.
    let policy_a = store_a
        .load_thread_notification_policy(scope_a.clone())
        .await
        .unwrap();
    assert_eq!(
        policy_a.targets.len(),
        1,
        "tenant A must see the policy it just wrote",
    );

    // Tenant B does NOT see tenant A's policy and falls back to the
    // default-for-scope, despite sharing (agent_id, project_id, thread_id).
    let policy_b = store_b
        .load_thread_notification_policy(scope_b.clone())
        .await
        .unwrap();
    assert!(
        policy_b.targets.is_empty(),
        "tenant B must NOT see tenant A's policy (cross-tenant leak)",
    );

    // Delivery attempts also isolate by mount prefix: record an attempt on
    // tenant A and verify tenant B's `list_delivery_attempts` for the
    // matching scope is empty even though the backend is shared.
    let delivery_id = OutboundDeliveryId::new();
    store_a
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id,
            scope: scope_a.clone(),
            candidate: OutboundPushCandidate {
                tenant_id: scope_a.tenant_id.clone(),
                agent_id: scope_a.agent_id.clone(),
                project_id: scope_a.project_id.clone(),
                thread_id: scope_a.thread_id.clone(),
                turn_run_id: Some(TurnRunId::new()),
                target,
                kind: OutboundPushKind::FinalReply,
                projection_ref: ProjectionUpdateRef::new("projection:tenant-isolation").unwrap(),
                requires_reply_target_revalidation: true,
            },
            status: OutboundDeliveryStatus::Pending,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    let a_deliveries = store_a.list_delivery_attempts(scope_a).await.unwrap();
    assert_eq!(
        a_deliveries.len(),
        1,
        "tenant A must see the delivery it just recorded",
    );
    let b_deliveries = store_b.list_delivery_attempts(scope_b).await.unwrap();
    assert!(
        b_deliveries.is_empty(),
        "tenant B list_delivery_attempts must be empty under shared (agent, project, thread) — got {} rows",
        b_deliveries.len(),
    );
}

/// Defense-in-depth regression for the tenant-isolation indexed
/// projection (see
/// `docs/internal/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`):
/// every `OutboundStateStore` write decorates its `Entry`
/// with a `tenant_id` projection so an admin-tier query can filter
/// explicitly by tenant and a path-rewriting bug surfaces as a
/// query-time mismatch.
///
/// Records a delivery attempt and a run-delivery-cleanup snapshot under tenant
/// A's scope, then issues raw `RootFilesystem::query` calls against both roots
/// with `Filter::Eq { key: "tenant_id", value: <tenant-a> }`. Each record must
/// be returned for tenant A and hidden from a different tenant.
#[tokio::test]
async fn filesystem_outbound_store_writes_tenant_id_indexed_projection() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = build_scoped_fs(
        Arc::clone(&backend),
        "/engine/tenants/tenant-outbound/users/user-outbound/outbound",
    );
    let store = OutboundStateStore::new(Arc::clone(&scoped));
    let scope = turn_scope();
    let delivery_id = OutboundDeliveryId::new();
    store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id,
            scope: scope.clone(),
            candidate: OutboundPushCandidate {
                tenant_id: scope.tenant_id.clone(),
                agent_id: scope.agent_id.clone(),
                project_id: scope.project_id.clone(),
                thread_id: scope.thread_id.clone(),
                turn_run_id: Some(TurnRunId::new()),
                target: reply_ref("reply-projection-test"),
                kind: OutboundPushKind::FinalReply,
                projection_ref: ProjectionUpdateRef::new("projection:tenant-index").unwrap(),
                requires_reply_target_revalidation: true,
            },
            status: OutboundDeliveryStatus::Pending,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    // Resolve the alias-relative deliveries prefix to the backing
    // VirtualPath through the same MountView the store uses, so the raw
    // query targets exactly the bytes the backend stored.
    let deliveries_prefix =
        ironclaw_host_api::path::ScopedPath::new("/outbound/deliveries".to_string()).unwrap();
    let virtual_prefix = scoped
        .resolve(&scope.to_resource_scope(), &deliveries_prefix)
        .unwrap();
    let tenant_key = ironclaw_filesystem::IndexKey::new("tenant_id").unwrap();

    let hit = backend
        .query(
            &virtual_prefix,
            &Filter::Eq {
                key: tenant_key.clone(),
                value: ironclaw_filesystem::IndexValue::Text(scope.tenant_id.as_str().to_string()),
            },
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .unwrap();
    assert_eq!(
        hit.len(),
        1,
        "tenant_id projection must surface the delivery via Filter::Eq",
    );

    let miss = backend
        .query(
            &virtual_prefix,
            &Filter::Eq {
                key: tenant_key.clone(),
                value: ironclaw_filesystem::IndexValue::Text("tenant-b".to_string()),
            },
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .unwrap();
    assert!(
        miss.is_empty(),
        "tenant_id projection must NOT surface tenant-outbound's delivery under tenant-b query; got {} rows",
        miss.len(),
    );

    let cleanup = cleanup_record(TurnRunId::new(), "tenant-index");
    store
        .put_run_delivery_cleanup(cleanup)
        .await
        .expect("persist cleanup snapshot");
    let cleanup_prefix =
        ironclaw_host_api::path::ScopedPath::new("/outbound/run-delivery-cleanup".to_string())
            .unwrap();
    let cleanup_virtual_prefix = scoped
        .resolve(&scope.to_resource_scope(), &cleanup_prefix)
        .unwrap();
    let conflicting_cleanup_index = IndexSpec::new(
        IndexName::new("outbound_by_tenant").unwrap(),
        vec![ironclaw_filesystem::IndexKey::new("wrong_tenant_key").unwrap()],
        IndexKind::Exact,
    );
    assert!(
        matches!(
            backend
                .ensure_index(&cleanup_virtual_prefix, &conflicting_cleanup_index)
                .await,
            Err(FilesystemError::IndexConflict { .. })
        ),
        "cleanup mutation must declare the canonical tenant index before writing"
    );
    let cleanup_hit = backend
        .query(
            &cleanup_virtual_prefix,
            &Filter::Eq {
                key: tenant_key.clone(),
                value: ironclaw_filesystem::IndexValue::Text(scope.tenant_id.as_str().to_string()),
            },
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .unwrap();
    assert_eq!(
        cleanup_hit.len(),
        1,
        "tenant_id projection must surface the cleanup snapshot via Filter::Eq",
    );

    let cleanup_miss = backend
        .query(
            &cleanup_virtual_prefix,
            &Filter::Eq {
                key: tenant_key,
                value: ironclaw_filesystem::IndexValue::Text("tenant-b".to_string()),
            },
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .unwrap();
    assert!(
        cleanup_miss.is_empty(),
        "tenant_id projection must NOT surface tenant-outbound's cleanup snapshot under tenant-b query; got {} rows",
        cleanup_miss.len(),
    );
}

#[tokio::test]
async fn completing_last_run_delivery_cleanup_record_deletes_snapshot() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(Arc::clone(&backend));
    let record = RunDeliveryCleanupRecord::new(
        turn_scope(),
        TurnRunId::new(),
        RunOriginAdapter::new("test-adapter").expect("adapter"),
        reply_ref("reply-cleanup-compaction"),
        "conversation-cleanup-compaction".to_string(),
        "vendor-message-cleanup-compaction".to_string(),
    )
    .expect("cleanup record");
    let cleanup_root = VirtualPath::new(format!("{TEST_OUTBOUND_ROOT}/run-delivery-cleanup"))
        .expect("cleanup root");

    store
        .put_run_delivery_cleanup(record.clone())
        .await
        .expect("persist cleanup record");
    assert_eq!(
        backend
            .query(&cleanup_root, &Filter::All, Page::default())
            .await
            .expect("query cleanup snapshots")
            .len(),
        1
    );

    store
        .complete_run_delivery_cleanup(&record)
        .await
        .expect("complete cleanup record");
    assert!(
        backend
            .query(&cleanup_root, &Filter::All, Page::default())
            .await
            .expect("query compacted cleanup snapshots")
            .is_empty(),
        "an empty cleanup snapshot must be removed instead of retained forever"
    );
}

fn cleanup_record(run_id: TurnRunId, suffix: &str) -> RunDeliveryCleanupRecord {
    RunDeliveryCleanupRecord::new(
        turn_scope(),
        run_id,
        RunOriginAdapter::new("test-adapter").expect("adapter"),
        reply_ref(&format!("reply-cleanup-{suffix}")),
        format!("conversation-cleanup-{suffix}"),
        format!("vendor-message-cleanup-{suffix}"),
    )
    .expect("cleanup record")
}

#[tokio::test]
async fn cleanup_put_rejects_invalid_existing_snapshot_without_writing() {
    for (case, mismatch_identity) in [("mismatched-identity", true), ("malformed-record", false)] {
        let backend = Arc::new(InMemoryBackend::new());
        let store = build_outbound_store_for_backend(Arc::clone(&backend));
        let run_id = TurnRunId::new();
        let existing = cleanup_record(run_id, &format!("{case}-existing"));
        let incoming = cleanup_record(run_id, &format!("{case}-incoming"));
        store
            .put_run_delivery_cleanup(existing)
            .await
            .expect("seed cleanup snapshot");

        let cleanup_root = VirtualPath::new(format!("{TEST_OUTBOUND_ROOT}/run-delivery-cleanup"))
            .expect("cleanup root");
        let mut rows = backend
            .query(&cleanup_root, &Filter::All, Page::default())
            .await
            .expect("query seeded cleanup snapshot");
        assert_eq!(rows.len(), 1, "{case}: exactly one snapshot must exist");
        let stored = rows.remove(0);
        let path = stored.path.clone();
        let mut snapshot_json: serde_json::Value =
            serde_json::from_slice(&stored.entry.body).expect("decode raw cleanup snapshot");
        let record_json = snapshot_json
            .get_mut("records")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|records| records.first_mut())
            .expect("cleanup snapshot has one record");
        if mismatch_identity {
            record_json["run_id"] =
                serde_json::to_value(TurnRunId::new()).expect("serialize mismatched run id");
        } else {
            record_json["vendor_message_ref"] = serde_json::Value::String(String::new());
        }
        let corrupted_body =
            serde_json::to_vec(&snapshot_json).expect("encode corrupted cleanup snapshot");
        let mut corrupted_entry = stored.entry;
        corrupted_entry.body = corrupted_body.clone();
        let corrupted_version = backend
            .put(
                &path,
                corrupted_entry,
                CasExpectation::Version(stored.version),
            )
            .await
            .expect("write corrupted cleanup snapshot");

        assert!(
            matches!(
                store.put_run_delivery_cleanup(incoming).await,
                Err(OutboundError::Serialization)
            ),
            "{case}: put must reject invalid authoritative snapshot data"
        );
        let after = backend
            .get(&path)
            .await
            .expect("read snapshot after rejected put")
            .expect("corrupted snapshot remains");
        assert_eq!(
            after.version, corrupted_version,
            "{case}: rejected put must not write"
        );
        assert_eq!(
            after.entry.body, corrupted_body,
            "{case}: rejected put must preserve authoritative bytes"
        );
    }
}

#[tokio::test]
async fn cleanup_completion_retries_delete_when_second_store_adds_sibling_record() {
    let inner = Arc::new(InMemoryBackend::new());
    let backend = Arc::new(DeletePauseBackend::new(inner));
    let first = Arc::new(OutboundStateStore::new(build_scoped_fs(
        Arc::clone(&backend),
        TEST_OUTBOUND_ROOT,
    )));
    let second = OutboundStateStore::new(build_scoped_fs(Arc::clone(&backend), TEST_OUTBOUND_ROOT));
    let run_id = TurnRunId::new();
    let completed = cleanup_record(run_id, "completed");
    let sibling = cleanup_record(run_id, "sibling");

    first
        .put_run_delivery_cleanup(completed.clone())
        .await
        .expect("seed cleanup record");
    backend.arm_before_delete();
    let completed_for_task = completed.clone();
    let completion = {
        let first = Arc::clone(&first);
        tokio::spawn(async move {
            first
                .complete_run_delivery_cleanup(&completed_for_task)
                .await
        })
    };
    backend.wait_for_delete().await;

    second
        .put_run_delivery_cleanup(sibling.clone())
        .await
        .expect("second store adds sibling during delete race");
    backend.release_delete();
    completion
        .await
        .expect("completion task joins")
        .expect("completion retries version mismatch");

    assert_eq!(
        second
            .load_run_delivery_cleanup(completed.request())
            .await
            .expect("load cleanup snapshot"),
        vec![sibling],
        "the second store's sibling record must survive delete-vs-write contention"
    );
}

#[tokio::test]
async fn cleanup_completion_uses_shared_retry_budget_under_two_store_contention() {
    let inner = Arc::new(InMemoryBackend::new());
    let backend = Arc::new(DeletePauseBackend::new(inner));
    let first = Arc::new(OutboundStateStore::new(build_scoped_fs(
        Arc::clone(&backend),
        TEST_OUTBOUND_ROOT,
    )));
    let second = OutboundStateStore::new(build_scoped_fs(Arc::clone(&backend), TEST_OUTBOUND_ROOT));
    let run_id = TurnRunId::new();
    let completed = cleanup_record(run_id, "retry-budget-completed");
    let racing = cleanup_record(run_id, "retry-budget-racing");

    first
        .put_run_delivery_cleanup(completed.clone())
        .await
        .expect("seed cleanup record");
    backend.arm_before_deletes(5);
    let completed_for_task = completed.clone();
    let completion = {
        let first = Arc::clone(&first);
        tokio::spawn(async move {
            first
                .complete_run_delivery_cleanup(&completed_for_task)
                .await
        })
    };

    for _ in 0..5 {
        backend.wait_for_delete().await;
        second
            .put_run_delivery_cleanup(racing.clone())
            .await
            .expect("racing store bumps the snapshot version");
        second
            .complete_run_delivery_cleanup(&racing)
            .await
            .expect("racing store restores the one-record snapshot");
        backend.release_delete();
    }

    completion
        .await
        .expect("completion task joins")
        .expect("shared retry budget exceeds the legacy fixed five attempts");
    assert!(
        second
            .load_run_delivery_cleanup(completed.request())
            .await
            .expect("load cleanup snapshot")
            .is_empty()
    );
}

#[tokio::test]
async fn cleanup_completion_rechecks_aba_recreation_by_second_store() {
    let inner = Arc::new(InMemoryBackend::new());
    let backend = Arc::new(DeletePauseBackend::new(inner));
    let first = Arc::new(OutboundStateStore::new(build_scoped_fs(
        Arc::clone(&backend),
        TEST_OUTBOUND_ROOT,
    )));
    let second = OutboundStateStore::new(build_scoped_fs(Arc::clone(&backend), TEST_OUTBOUND_ROOT));
    let record = cleanup_record(TurnRunId::new(), "aba");

    first
        .put_run_delivery_cleanup(record.clone())
        .await
        .expect("seed cleanup record");
    backend.arm_after_delete();
    let record_for_task = record.clone();
    let completion = {
        let first = Arc::clone(&first);
        tokio::spawn(async move { first.complete_run_delivery_cleanup(&record_for_task).await })
    };
    backend.wait_for_delete().await;

    second
        .put_run_delivery_cleanup(record.clone())
        .await
        .expect("second store recreates same record after delete commits");
    backend.release_delete();
    completion
        .await
        .expect("completion task joins")
        .expect("completion rechecks recreated path");

    assert!(
        second
            .load_run_delivery_cleanup(record.request())
            .await
            .expect("load cleanup snapshot")
            .is_empty(),
        "completion must not return while an ABA recreation still contains the record"
    );
}

#[tokio::test]
async fn cleanup_completion_delete_permission_fault_preserves_snapshot() {
    let backend = Arc::new(InMemoryBackend::new());
    let writer = build_outbound_store_with_permissions(
        Arc::clone(&backend),
        MountPermissions::read_write_list_delete(),
    );
    let no_delete = build_outbound_store_with_permissions(backend, MountPermissions::read_write());
    let record = cleanup_record(TurnRunId::new(), "permission-fault");

    writer
        .put_run_delivery_cleanup(record.clone())
        .await
        .expect("seed cleanup record");
    assert!(matches!(
        no_delete.complete_run_delivery_cleanup(&record).await,
        Err(OutboundError::Backend)
    ));
    assert_eq!(
        writer
            .load_run_delivery_cleanup(record.request())
            .await
            .expect("load preserved cleanup snapshot"),
        vec![record],
        "a conditional-delete fault must not drop or rewrite the cleanup snapshot"
    );
}

// ─── Progressive reply publication on the attempt aggregate ─────────────────
//
// `docs/internal/design/2026-08-31-progressive-reply-publication.md` §5: the
// persisted attempt row carries a serde-defaulted `publication` substate with
// its own lease/fence ownership, monotonic revisions, and one-way settlement.
// Every scenario below runs against the in-memory backend and the libSQL
// backend through the same `OutboundStateStorePort` seam.

fn prepared_attempt(scope: &TurnScope, marker: &str, run_id: TurnRunId) -> OutboundDeliveryAttempt {
    OutboundDeliveryAttempt {
        delivery_id: OutboundDeliveryId::new(),
        scope: scope.clone(),
        candidate: OutboundPushCandidate {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
            turn_run_id: Some(run_id),
            target: reply_ref(marker),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new(format!("projection:{marker}:{run_id}"))
                .unwrap(),
            requires_reply_target_revalidation: true,
        },
        status: OutboundDeliveryStatus::Prepared,
        attempted_at: now(),
        failure_kind: None,
    }
}

fn publication_target(run_id: TurnRunId, key: &str) -> ReplyPublicationTarget {
    ReplyPublicationTarget {
        run_id,
        key: ReplyPublicationTargetKey::new(key).unwrap(),
    }
}

fn publisher(name: &str) -> PublisherId {
    PublisherId::new(name).unwrap()
}

fn seconds(count: i64) -> chrono::Duration {
    chrono::Duration::seconds(count)
}

const LEASE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn claim_request(
    delivery_id: OutboundDeliveryId,
    scope: &TurnScope,
    owner: &str,
    now: ironclaw_host_api::Timestamp,
) -> ClaimReplyPublicationLeaseRequest {
    ClaimReplyPublicationLeaseRequest {
        delivery_id,
        scope: scope.clone(),
        owner: publisher(owner),
        ttl: LEASE_TTL,
        now,
    }
}

#[allow(clippy::too_many_arguments)]
fn advance_request(
    delivery_id: OutboundDeliveryId,
    scope: &TurnScope,
    fence: u64,
    desired_revision: u64,
    published_revision: u64,
    terminal_revision: Option<u64>,
    generation: Option<u64>,
    checkpoint: Option<ironclaw_extension_contracts::reply::ReplySinkCheckpoint>,
    evidence: ReplyPublicationEvidence,
    now: ironclaw_host_api::Timestamp,
) -> AdvanceReplyPublicationRequest {
    AdvanceReplyPublicationRequest {
        delivery_id,
        scope: scope.clone(),
        fence,
        desired_revision,
        published_revision,
        terminal_revision,
        generation,
        checkpoint,
        evidence,
        now,
    }
}

fn settle_request(
    delivery_id: OutboundDeliveryId,
    scope: &TurnScope,
    fence: u64,
    settlement: ReplyPublicationSettlement,
    now: ironclaw_host_api::Timestamp,
) -> SettleReplyPublicationRequest {
    SettleReplyPublicationRequest {
        delivery_id,
        scope: scope.clone(),
        fence,
        settlement,
        now,
    }
}

fn checkpoint(payload: &str) -> ironclaw_extension_contracts::reply::ReplySinkCheckpoint {
    ironclaw_extension_contracts::reply::ReplySinkCheckpoint::new(1, payload).unwrap()
}

fn evidence(provider_ref: &str, read_back_verified: bool) -> ReplyPublicationEvidence {
    ReplyPublicationEvidence {
        provider_refs: ironclaw_extension_contracts::reply::ReplyProviderRefs::new(vec![
            ironclaw_extension_contracts::reply::ReplyProviderRef::new(provider_ref).unwrap(),
        ])
        .unwrap(),
        read_back_verified,
        last_outcome: None,
        generation_changed: false,
    }
}

fn acquired(claim: ReplyPublicationClaim) -> ReplyPublicationRecord {
    match claim {
        ReplyPublicationClaim::Acquired(record) => record,
        other => panic!("expected an acquired lease, got {other:?}"),
    }
}

async fn open_and_claim(
    store: &impl OutboundStateStorePort,
    scope: &TurnScope,
    marker: &str,
    run_id: TurnRunId,
    owner: &str,
    now: ironclaw_host_api::Timestamp,
) -> ReplyPublicationRecord {
    let attempt = prepared_attempt(scope, marker, run_id);
    store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: attempt.clone(),
            target: publication_target(run_id, marker),
            descriptor: None,
            now,
        })
        .await
        .unwrap();
    acquired(
        store
            .claim_reply_publication_lease(claim_request(attempt.delivery_id, scope, owner, now))
            .await
            .unwrap(),
    )
}

async fn run_reply_publication_contract(store: &impl OutboundStateStorePort) {
    reply_publication_open_is_idempotent_and_rejects_a_different_target(store).await;
    reply_publication_lease_claim_holds_reenters_and_takes_over_after_expiry(store).await;
    reply_publication_advance_is_monotonic_fenced_and_lease_bound(store).await;
    reply_publication_settlement_is_guarded_and_one_way(store).await;
    reply_publication_rows_are_left_alone_by_crash_recovery(store).await;
    pre_publication_attempt_rows_keep_the_one_shot_lifecycle(store).await;
    open_reply_publications_list_spans_the_tenant_and_excludes_settled_and_plain_rows(store).await;
}

/// The boot-time crash-recovery read: every still-`Active` publication in the
/// caller's tenant is returned — across thread scopes — while settled
/// publications and plain one-shot attempt rows are not.
async fn open_reply_publications_list_spans_the_tenant_and_excludes_settled_and_plain_rows(
    store: &impl OutboundStateStorePort,
) {
    let now = now();
    let scope_one = TurnScope::new(
        TenantId::new("sweep-tenant").unwrap(),
        Some(AgentId::new("sweep-agent").unwrap()),
        None,
        ThreadId::new("sweep-thread-one").unwrap(),
    );
    let scope_two = TurnScope::new(
        TenantId::new("sweep-tenant").unwrap(),
        Some(AgentId::new("sweep-agent").unwrap()),
        None,
        ThreadId::new("sweep-thread-two").unwrap(),
    );
    // Active publication in thread one.
    let active_run = TurnRunId::new();
    let active = prepared_attempt(&scope_one, "sweep-active", active_run);
    store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: active.clone(),
            target: publication_target(active_run, "sweep-active"),
            descriptor: None,
            now,
        })
        .await
        .unwrap();
    // Active publication in thread two — a different scope of the same
    // tenant, which the per-scope listing cannot see but the sweep must.
    let sibling_run = TurnRunId::new();
    let sibling = prepared_attempt(&scope_two, "sweep-sibling", sibling_run);
    store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: sibling.clone(),
            target: publication_target(sibling_run, "sweep-sibling"),
            descriptor: None,
            now,
        })
        .await
        .unwrap();
    // Settled publication in thread one: claimed, advanced to its terminal
    // revision, settled Delivered.
    let settled_run = TurnRunId::new();
    let settled = open_and_claim(
        store,
        &scope_one,
        "sweep-settled",
        settled_run,
        "sweeper",
        now,
    )
    .await;
    store
        .advance_reply_publication(advance_request(
            settled.attempt.delivery_id,
            &scope_one,
            settled.publication.fence,
            1,
            1,
            Some(1),
            Some(1),
            None,
            evidence("sweep-ref", false),
            now,
        ))
        .await
        .unwrap();
    store
        .settle_reply_publication(settle_request(
            settled.attempt.delivery_id,
            &scope_one,
            settled.publication.fence,
            ReplyPublicationSettlement::Delivered,
            now,
        ))
        .await
        .unwrap();
    // A plain one-shot attempt row carries no publication substate.
    let plain = prepared_attempt(&scope_one, "sweep-plain", TurnRunId::new());
    store.record_delivery_attempt(plain.clone()).await.unwrap();

    let open = store
        .list_open_reply_publications(scope_one.clone())
        .await
        .unwrap();
    let ids: Vec<_> = open
        .iter()
        .map(|record| record.attempt.delivery_id)
        .collect();
    assert!(
        ids.contains(&active.delivery_id),
        "the active publication in the query scope's thread is listed"
    );
    assert!(
        ids.contains(&sibling.delivery_id),
        "an active publication in ANOTHER thread of the same tenant is listed"
    );
    assert!(
        !ids.contains(&settled.attempt.delivery_id),
        "a settled publication is not resumed"
    );
    assert!(
        !ids.contains(&plain.delivery_id),
        "a plain one-shot attempt row is not a publication"
    );
    for record in &open {
        assert!(record.publication.status.is_active());
    }
}

#[tokio::test]
async fn reply_publication_contract_holds_on_the_in_memory_backend() {
    let store = build_outbound_store_for_backend(Arc::new(InMemoryBackend::new()));
    run_reply_publication_contract(&store).await;
}

#[tokio::test]
async fn reply_publication_contract_holds_on_libsql_and_survives_reopen() {
    let directory = tempfile::tempdir().expect("temporary libSQL directory");
    let database_path = directory.path().join("outbound-reply-publication.db");
    let scope = turn_scope();
    let run_id = TurnRunId::new();
    let t0 = now();

    let survivor = {
        let database = Arc::new(
            libsql::Builder::new_local(&database_path)
                .build()
                .await
                .expect("build first libSQL database"),
        );
        let root = Arc::new(
            LibSqlRootFilesystem::new(database).expect("build first libSQL root filesystem"),
        );
        root.run_migrations()
            .await
            .expect("migrate first libSQL filesystem");
        let store = OutboundStateStore::new(build_scoped_fs(root, TEST_OUTBOUND_ROOT));
        run_reply_publication_contract(&store).await;

        let claimed =
            open_and_claim(&store, &scope, "libsql-survivor", run_id, "worker-a", t0).await;
        store
            .advance_reply_publication(advance_request(
                claimed.attempt.delivery_id,
                &scope,
                claimed.publication.fence,
                2,
                1,
                Some(2),
                Some(3),
                Some(checkpoint("libsql-cp-1")),
                evidence("libsql-msg-1", false),
                t0 + seconds(1),
            ))
            .await
            .unwrap()
    };

    let reopened_database = Arc::new(
        libsql::Builder::new_local(&database_path)
            .build()
            .await
            .expect("reopen libSQL database"),
    );
    let reopened_root = Arc::new(
        LibSqlRootFilesystem::new(reopened_database).expect("reopen libSQL root filesystem"),
    );
    reopened_root
        .run_migrations()
        .await
        .expect("migrate reopened libSQL filesystem");
    let reopened = OutboundStateStore::new(build_scoped_fs(reopened_root, TEST_OUTBOUND_ROOT));

    assert_eq!(
        reopened
            .load_reply_publication(scope.clone(), survivor.attempt.delivery_id)
            .await
            .unwrap(),
        Some(survivor.clone()),
        "publication state (lease, fence, revisions, checkpoint, evidence) survives a reopen"
    );
    assert_eq!(
        reopened
            .list_reply_publications(scope.clone(), run_id)
            .await
            .unwrap(),
        vec![survivor.clone()]
    );
    // After a restart the old lease expires and a new publisher takes over
    // with a bumped fence — the crash-recovery path for publication rows.
    let taken = acquired(
        reopened
            .claim_reply_publication_lease(claim_request(
                survivor.attempt.delivery_id,
                &scope,
                "worker-b",
                t0 + seconds(120),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(taken.publication.fence, survivor.publication.fence + 1);
    assert_eq!(
        taken.publication.checkpoint,
        survivor.publication.checkpoint
    );
    assert_eq!(taken.publication.published_revision, 1);
}

async fn reply_publication_open_is_idempotent_and_rejects_a_different_target(
    store: &impl OutboundStateStorePort,
) {
    let scope = turn_scope();
    let run_id = TurnRunId::new();
    let attempt = prepared_attempt(&scope, "reply-publication-open", run_id);
    let delivery_id = attempt.delivery_id;
    let target = publication_target(run_id, "slack:C1:171.1");
    let t0 = now();

    let opened = store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: attempt.clone(),
            target: target.clone(),
            descriptor: None,
            now: t0,
        })
        .await
        .unwrap();
    assert_eq!(opened.attempt, attempt);
    assert_eq!(opened.publication.target, target);
    assert_eq!(opened.publication.fence, 0);
    assert_eq!(opened.publication.lease, None);
    assert_eq!(opened.publication.desired_revision, 0);
    assert_eq!(opened.publication.published_revision, 0);
    assert_eq!(opened.publication.terminal_revision, None);
    assert_eq!(opened.publication.generation, None);
    assert_eq!(opened.publication.checkpoint, None);
    assert_eq!(
        opened.publication.evidence,
        ReplyPublicationEvidence::default()
    );
    assert_eq!(opened.publication.status, ReplyPublicationStatus::Active);
    assert_eq!(opened.publication.updated_at, t0);

    // A replay with the same target (even at a later clock) returns the
    // existing record unchanged.
    let replayed = store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: attempt.clone(),
            target: target.clone(),
            descriptor: None,
            now: t0 + seconds(5),
        })
        .await
        .unwrap();
    assert_eq!(replayed, opened);

    // A different key or a different run under the same delivery id conflicts.
    let other_key = store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: attempt.clone(),
            target: publication_target(run_id, "slack:C1:171.2"),
            descriptor: None,
            now: t0,
        })
        .await;
    assert!(matches!(
        other_key,
        Err(OutboundError::ReplyPublicationTargetMismatch)
    ));
    let other_run = store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: attempt.clone(),
            target: publication_target(TurnRunId::new(), "slack:C1:171.1"),
            descriptor: None,
            now: t0,
        })
        .await;
    assert!(matches!(
        other_run,
        Err(OutboundError::ReplyPublicationTargetMismatch)
    ));
    assert_eq!(
        store
            .load_reply_publication(scope.clone(), delivery_id)
            .await
            .unwrap(),
        Some(opened.clone()),
        "a rejected open leaves the existing publication untouched"
    );

    // A plain attempt row recorded through the one-shot path is never adopted.
    let plain = prepared_attempt(&scope, "reply-publication-open-plain", run_id);
    store.record_delivery_attempt(plain.clone()).await.unwrap();
    let adopt = store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: plain.clone(),
            target: publication_target(run_id, "plain"),
            descriptor: None,
            now: t0,
        })
        .await;
    assert!(matches!(
        adopt,
        Err(OutboundError::ReplyPublicationTargetMismatch)
    ));
    assert_eq!(
        store
            .load_reply_publication(scope.clone(), plain.delivery_id)
            .await
            .unwrap(),
        None
    );

    // Only a `Prepared` attempt can open a publication; nothing is written
    // for a rejected open.
    let mut sending = prepared_attempt(&scope, "reply-publication-open-sending", run_id);
    sending.status = OutboundDeliveryStatus::Sending;
    let rejected = store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: sending.clone(),
            target: publication_target(run_id, "sending"),
            descriptor: None,
            now: t0,
        })
        .await;
    assert!(matches!(
        rejected,
        Err(OutboundError::InvalidRequest { .. })
    ));
    assert_eq!(
        store
            .load_delivery_attempt(scope.clone(), sending.delivery_id)
            .await
            .unwrap(),
        None
    );

    // Point reads and run listings are scope-exact; the plain row is not a
    // publication; the attempt row itself stays an ordinary delivery attempt.
    assert_eq!(
        store
            .load_reply_publication(sibling_turn_scope(), delivery_id)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .list_reply_publications(scope.clone(), run_id)
            .await
            .unwrap(),
        vec![opened.clone()]
    );
    assert!(
        store
            .list_reply_publications(scope.clone(), TurnRunId::new())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_reply_publications(sibling_turn_scope(), run_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .load_delivery_attempt(scope.clone(), delivery_id)
            .await
            .unwrap(),
        Some(attempt.clone())
    );
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert!(attempts.iter().any(|listed| listed == &attempt));
    assert!(attempts.iter().any(|listed| listed == &plain));
}

async fn reply_publication_lease_claim_holds_reenters_and_takes_over_after_expiry(
    store: &impl OutboundStateStorePort,
) {
    let scope = turn_scope();
    let run_id = TurnRunId::new();
    let attempt = prepared_attempt(&scope, "reply-publication-claim", run_id);
    let delivery_id = attempt.delivery_id;
    let t0 = now();
    store
        .open_reply_publication(OpenReplyPublicationRequest {
            attempt: attempt.clone(),
            target: publication_target(run_id, "claim"),
            descriptor: None,
            now: t0,
        })
        .await
        .unwrap();

    // A lease needs a positive ttl.
    let zero_ttl = store
        .claim_reply_publication_lease(ClaimReplyPublicationLeaseRequest {
            delivery_id,
            scope: scope.clone(),
            owner: publisher("worker-a"),
            ttl: std::time::Duration::ZERO,
            now: t0,
        })
        .await;
    assert!(matches!(
        zero_ttl,
        Err(OutboundError::InvalidRequest { .. })
    ));

    // First claim: fence bumps to 1, the attempt moves Prepared -> Sending.
    let first = acquired(
        store
            .claim_reply_publication_lease(claim_request(delivery_id, &scope, "worker-a", t0))
            .await
            .unwrap(),
    );
    assert_eq!(first.publication.fence, 1);
    assert_eq!(
        first.publication.lease,
        Some(ReplyPublicationLease {
            owner: publisher("worker-a"),
            expires_at: t0 + seconds(30),
        })
    );
    assert_eq!(first.attempt.status, OutboundDeliveryStatus::Sending);
    assert_eq!(first.attempt.failure_kind, None);
    assert_eq!(first.publication.updated_at, t0);
    assert_eq!(first.publication.status, ReplyPublicationStatus::Active);

    // Another publisher is held off while the lease is live.
    let held = store
        .claim_reply_publication_lease(claim_request(
            delivery_id,
            &scope,
            "worker-b",
            t0 + seconds(1),
        ))
        .await
        .unwrap();
    assert_eq!(
        held,
        ReplyPublicationClaim::Held {
            owner: publisher("worker-a"),
            expires_at: t0 + seconds(30),
        }
    );

    // The owner re-enters with the same fence and an extended expiry.
    let reentered = acquired(
        store
            .claim_reply_publication_lease(claim_request(
                delivery_id,
                &scope,
                "worker-a",
                t0 + seconds(10),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(reentered.publication.fence, 1);
    assert_eq!(
        reentered.publication.lease,
        Some(ReplyPublicationLease {
            owner: publisher("worker-a"),
            expires_at: t0 + seconds(40),
        })
    );
    assert_eq!(reentered.attempt.status, OutboundDeliveryStatus::Sending);

    // Once the lease expires a new owner takes over with a bumped fence; the
    // attempt stays `Sending` (the one-shot claim is not re-run).
    let taken = acquired(
        store
            .claim_reply_publication_lease(claim_request(
                delivery_id,
                &scope,
                "worker-b",
                t0 + seconds(41),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(taken.publication.fence, 2);
    assert_eq!(
        taken.publication.lease,
        Some(ReplyPublicationLease {
            owner: publisher("worker-b"),
            expires_at: t0 + seconds(71),
        })
    );
    assert_eq!(taken.attempt.status, OutboundDeliveryStatus::Sending);
    let stale_owner = store
        .claim_reply_publication_lease(claim_request(
            delivery_id,
            &scope,
            "worker-a",
            t0 + seconds(42),
        ))
        .await
        .unwrap();
    assert_eq!(
        stale_owner,
        ReplyPublicationClaim::Held {
            owner: publisher("worker-b"),
            expires_at: t0 + seconds(71),
        }
    );
    assert_eq!(
        store
            .load_reply_publication(scope.clone(), delivery_id)
            .await
            .unwrap(),
        Some(taken)
    );

    // Unknown ids and plain attempt rows carry no publication; a foreign
    // scope never sees the row as a publication.
    let missing = store
        .claim_reply_publication_lease(claim_request(
            OutboundDeliveryId::new(),
            &scope,
            "worker-a",
            t0,
        ))
        .await;
    assert!(matches!(
        missing,
        Err(OutboundError::ReplyPublicationNotFound)
    ));
    let plain = prepared_attempt(&scope, "reply-publication-claim-plain", run_id);
    store.record_delivery_attempt(plain.clone()).await.unwrap();
    let plain_claim = store
        .claim_reply_publication_lease(claim_request(plain.delivery_id, &scope, "worker-a", t0))
        .await;
    assert!(matches!(
        plain_claim,
        Err(OutboundError::ReplyPublicationNotFound)
    ));
    let wrong_scope = store
        .claim_reply_publication_lease(claim_request(
            delivery_id,
            &sibling_turn_scope(),
            "worker-c",
            t0 + seconds(100),
        ))
        .await;
    assert!(matches!(
        wrong_scope,
        Err(OutboundError::SubscriptionScopeMismatch | OutboundError::ReplyPublicationNotFound)
    ));
}
async fn reply_publication_advance_is_monotonic_fenced_and_lease_bound(
    store: &impl OutboundStateStorePort,
) {
    let scope = turn_scope();
    let run_id = TurnRunId::new();
    let t0 = now();
    let held = open_and_claim(
        store,
        &scope,
        "reply-publication-advance",
        run_id,
        "worker-a",
        t0,
    )
    .await;
    let delivery_id = held.attempt.delivery_id;

    // A stale fence is rejected before anything else is inspected.
    let stale = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            0,
            1,
            0,
            None,
            None,
            None,
            ReplyPublicationEvidence::default(),
            t0,
        ))
        .await;
    assert!(
        matches!(
            stale,
            Err(OutboundError::StaleReplyPublisher {
                expected_fence: 0,
                actual_fence: 1,
            })
        ),
        "got {stale:?}"
    );

    // Forward progress records revisions, generation, checkpoint, evidence.
    let first = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            1,
            3,
            1,
            None,
            Some(7),
            Some(checkpoint("cp-1")),
            evidence("msg-1", false),
            t0 + seconds(1),
        ))
        .await
        .unwrap();
    assert_eq!(first.publication.desired_revision, 3);
    assert_eq!(first.publication.published_revision, 1);
    assert_eq!(first.publication.terminal_revision, None);
    assert_eq!(first.publication.generation, Some(7));
    assert_eq!(first.publication.checkpoint, Some(checkpoint("cp-1")));
    assert_eq!(first.publication.evidence, evidence("msg-1", false));
    assert_eq!(first.publication.updated_at, t0 + seconds(1));
    assert_eq!(first.publication.fence, 1);
    assert_eq!(first.publication.lease, held.publication.lease);
    assert_eq!(first.publication.status, ReplyPublicationStatus::Active);
    assert_eq!(first.attempt.status, OutboundDeliveryStatus::Sending);

    // Regressions: desired backwards, published backwards, published past
    // desired.
    for (desired, published) in [(2, 1), (3, 0), (4, 5)] {
        let regressed = store
            .advance_reply_publication(advance_request(
                delivery_id,
                &scope,
                1,
                desired,
                published,
                None,
                Some(7),
                None,
                evidence("msg-1", false),
                t0 + seconds(2),
            ))
            .await;
        assert!(
            matches!(
                regressed,
                Err(OutboundError::ReplyPublicationRevisionRegressed { .. })
            ),
            "desired {desired} / published {published} must be refused, got {regressed:?}"
        );
    }

    // The terminal revision is set once, cannot exceed the desired revision,
    // and every later request must carry the same value. A `None` checkpoint
    // keeps the previous one.
    let beyond_desired = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            1,
            4,
            2,
            Some(5),
            Some(7),
            None,
            evidence("msg-1", false),
            t0 + seconds(2),
        ))
        .await;
    assert!(matches!(
        beyond_desired,
        Err(OutboundError::InvalidRequest { .. })
    ));
    let terminal = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            1,
            4,
            2,
            Some(4),
            Some(7),
            None,
            evidence("msg-1", false),
            t0 + seconds(2),
        ))
        .await
        .unwrap();
    assert_eq!(terminal.publication.terminal_revision, Some(4));
    assert_eq!(terminal.publication.checkpoint, Some(checkpoint("cp-1")));
    for changed_terminal in [Some(5), None] {
        let refused = store
            .advance_reply_publication(advance_request(
                delivery_id,
                &scope,
                1,
                4,
                3,
                changed_terminal,
                Some(7),
                None,
                evidence("msg-1", false),
                t0 + seconds(3),
            ))
            .await;
        assert!(
            matches!(refused, Err(OutboundError::InvalidRequest { .. })),
            "terminal revision {changed_terminal:?} must be refused once set, got {refused:?}"
        );
    }
    let mut richer = evidence("msg-2", true);
    richer.generation_changed = true;
    richer.last_outcome =
        Some(ironclaw_extension_contracts::reply::ReplyOutcomeReason::new("rate limited once"));
    let progressed = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            1,
            4,
            3,
            Some(4),
            Some(8),
            Some(checkpoint("cp-2")),
            richer.clone(),
            t0 + seconds(3),
        ))
        .await
        .unwrap();
    assert_eq!(progressed.publication.published_revision, 3);
    assert_eq!(progressed.publication.generation, Some(8));
    assert_eq!(progressed.publication.checkpoint, Some(checkpoint("cp-2")));
    assert_eq!(progressed.publication.evidence, richer);

    // Re-applying the same revisions is accepted (idempotent re-apply).
    let replayed = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            1,
            4,
            3,
            Some(4),
            Some(8),
            None,
            richer.clone(),
            t0 + seconds(4),
        ))
        .await
        .unwrap();
    assert_eq!(replayed.publication.published_revision, 3);
    assert_eq!(replayed.publication.updated_at, t0 + seconds(4));

    // Releasing the lease keeps the fence; guarded writes then need a lease.
    let stale_release = store
        .release_reply_publication_lease(ReleaseReplyPublicationLeaseRequest {
            delivery_id,
            scope: scope.clone(),
            fence: 0,
        })
        .await;
    assert!(matches!(
        stale_release,
        Err(OutboundError::StaleReplyPublisher { .. })
    ));
    store
        .release_reply_publication_lease(ReleaseReplyPublicationLeaseRequest {
            delivery_id,
            scope: scope.clone(),
            fence: 1,
        })
        .await
        .unwrap();
    let released = store
        .load_reply_publication(scope.clone(), delivery_id)
        .await
        .unwrap()
        .expect("publication persisted");
    assert_eq!(released.publication.lease, None);
    assert_eq!(released.publication.fence, 1);
    assert_eq!(released.publication.published_revision, 3);
    let unleased = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            1,
            4,
            4,
            Some(4),
            Some(8),
            None,
            richer.clone(),
            t0 + seconds(5),
        ))
        .await;
    assert!(matches!(
        unleased,
        Err(OutboundError::ReplyPublicationLeaseRequired)
    ));

    // Re-claiming bumps the fence: the old fence is stale, the new one writes.
    let reclaimed = acquired(
        store
            .claim_reply_publication_lease(claim_request(
                delivery_id,
                &scope,
                "worker-a",
                t0 + seconds(6),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(reclaimed.publication.fence, 2);
    let old_fence = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            1,
            4,
            4,
            Some(4),
            Some(8),
            None,
            richer.clone(),
            t0 + seconds(7),
        ))
        .await;
    assert!(matches!(
        old_fence,
        Err(OutboundError::StaleReplyPublisher {
            expected_fence: 1,
            actual_fence: 2,
        })
    ));
    let done = store
        .advance_reply_publication(advance_request(
            delivery_id,
            &scope,
            2,
            4,
            4,
            Some(4),
            Some(8),
            None,
            richer,
            t0 + seconds(7),
        ))
        .await
        .unwrap();
    assert_eq!(done.publication.published_revision, 4);
    assert_eq!(done.publication.terminal_revision, Some(4));

    // A plain attempt row has nothing to advance.
    let plain = prepared_attempt(&scope, "reply-publication-advance-plain", run_id);
    store.record_delivery_attempt(plain.clone()).await.unwrap();
    let plain_advance = store
        .advance_reply_publication(advance_request(
            plain.delivery_id,
            &scope,
            0,
            1,
            0,
            None,
            None,
            None,
            ReplyPublicationEvidence::default(),
            t0,
        ))
        .await;
    assert!(matches!(
        plain_advance,
        Err(OutboundError::ReplyPublicationNotFound)
    ));
}

async fn reply_publication_settlement_is_guarded_and_one_way(store: &impl OutboundStateStorePort) {
    let scope = turn_scope();
    let run_id = TurnRunId::new();
    let t0 = now();

    // `Delivered` requires the terminal revision to be known and applied.
    let delivered_publication = open_and_claim(
        store,
        &scope,
        "reply-publication-settle-delivered",
        run_id,
        "worker-a",
        t0,
    )
    .await;
    let delivered_id = delivered_publication.attempt.delivery_id;
    let no_terminal = store
        .settle_reply_publication(settle_request(
            delivered_id,
            &scope,
            1,
            ReplyPublicationSettlement::Delivered,
            t0 + seconds(1),
        ))
        .await;
    assert!(matches!(
        no_terminal,
        Err(OutboundError::ReplyPublicationNotTerminal)
    ));
    store
        .advance_reply_publication(advance_request(
            delivered_id,
            &scope,
            1,
            2,
            1,
            Some(2),
            Some(1),
            Some(checkpoint("settle-cp")),
            evidence("settle-msg", true),
            t0 + seconds(1),
        ))
        .await
        .unwrap();
    let terminal_pending = store
        .settle_reply_publication(settle_request(
            delivered_id,
            &scope,
            1,
            ReplyPublicationSettlement::Delivered,
            t0 + seconds(2),
        ))
        .await;
    assert!(matches!(
        terminal_pending,
        Err(OutboundError::ReplyPublicationNotTerminal)
    ));
    store
        .advance_reply_publication(advance_request(
            delivered_id,
            &scope,
            1,
            2,
            2,
            Some(2),
            Some(1),
            None,
            evidence("settle-msg", true),
            t0 + seconds(2),
        ))
        .await
        .unwrap();
    let stale = store
        .settle_reply_publication(settle_request(
            delivered_id,
            &scope,
            0,
            ReplyPublicationSettlement::Delivered,
            t0 + seconds(3),
        ))
        .await;
    assert!(matches!(
        stale,
        Err(OutboundError::StaleReplyPublisher { .. })
    ));
    let delivered = store
        .settle_reply_publication(settle_request(
            delivered_id,
            &scope,
            1,
            ReplyPublicationSettlement::Delivered,
            t0 + seconds(3),
        ))
        .await
        .unwrap();
    assert_eq!(
        delivered.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    assert_eq!(delivered.publication.lease, None);
    assert_eq!(delivered.publication.fence, 1);
    assert_eq!(delivered.publication.updated_at, t0 + seconds(3));
    assert_eq!(
        delivered.publication.checkpoint,
        Some(checkpoint("settle-cp"))
    );
    assert_eq!(delivered.attempt.status, OutboundDeliveryStatus::Delivered);
    assert_eq!(delivered.attempt.failure_kind, None);
    assert_eq!(
        store
            .load_delivery_attempt(scope.clone(), delivered_id)
            .await
            .unwrap()
            .map(|attempt| attempt.status),
        Some(OutboundDeliveryStatus::Delivered)
    );

    // Settlement is one-way: no second settle, no advance, no new lease.
    let again = store
        .settle_reply_publication(settle_request(
            delivered_id,
            &scope,
            1,
            ReplyPublicationSettlement::Unknown,
            t0 + seconds(4),
        ))
        .await;
    assert!(matches!(again, Err(OutboundError::ReplyPublicationSettled)));
    let advance_after = store
        .advance_reply_publication(advance_request(
            delivered_id,
            &scope,
            1,
            3,
            2,
            Some(2),
            Some(1),
            None,
            evidence("settle-msg", true),
            t0 + seconds(4),
        ))
        .await;
    assert!(matches!(
        advance_after,
        Err(OutboundError::ReplyPublicationSettled)
    ));
    for owner in ["worker-a", "worker-b"] {
        let claim = store
            .claim_reply_publication_lease(claim_request(
                delivered_id,
                &scope,
                owner,
                t0 + seconds(100),
            ))
            .await
            .unwrap();
        assert_eq!(claim, ReplyPublicationClaim::Settled(delivered.clone()));
    }
    assert_eq!(
        store
            .load_reply_publication(scope.clone(), delivered_id)
            .await
            .unwrap(),
        Some(delivered.clone()),
        "nothing after settlement mutates the record"
    );

    // `Unknown` and `Failed(kind)` settle any active publication, terminal
    // known or not, and need only the fence — not a live lease.
    let unknown_publication = open_and_claim(
        store,
        &scope,
        "reply-publication-settle-unknown",
        run_id,
        "worker-a",
        t0,
    )
    .await;
    let unknown_id = unknown_publication.attempt.delivery_id;
    store
        .release_reply_publication_lease(ReleaseReplyPublicationLeaseRequest {
            delivery_id: unknown_id,
            scope: scope.clone(),
            fence: 1,
        })
        .await
        .unwrap();
    let unknown = store
        .settle_reply_publication(settle_request(
            unknown_id,
            &scope,
            1,
            ReplyPublicationSettlement::Unknown,
            t0 + seconds(5),
        ))
        .await
        .unwrap();
    assert_eq!(
        unknown.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Unknown)
    );
    assert_eq!(unknown.attempt.status, OutboundDeliveryStatus::Unknown);
    assert_eq!(unknown.attempt.failure_kind, None);

    let failed_publication = open_and_claim(
        store,
        &scope,
        "reply-publication-settle-failed",
        run_id,
        "worker-a",
        t0,
    )
    .await;
    let failed_id = failed_publication.attempt.delivery_id;
    let failed = store
        .settle_reply_publication(settle_request(
            failed_id,
            &scope,
            1,
            ReplyPublicationSettlement::Failed(DeliveryFailureKind::Rejected),
            t0 + seconds(6),
        ))
        .await
        .unwrap();
    assert_eq!(
        failed.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            DeliveryFailureKind::Rejected
        ))
    );
    assert_eq!(failed.publication.lease, None);
    assert_eq!(failed.attempt.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        failed.attempt.failure_kind,
        Some(DeliveryFailureKind::Rejected)
    );

    // The run listing returns every publication regardless of status, and
    // the attempt rows keep reporting the settled delivery statuses.
    let listed = store
        .list_reply_publications(scope.clone(), run_id)
        .await
        .unwrap();
    let mut listed_ids: Vec<_> = listed
        .iter()
        .map(|record| record.attempt.delivery_id)
        .collect();
    listed_ids.sort_by_key(|id| id.to_string());
    let mut expected_ids = vec![delivered_id, unknown_id, failed_id];
    expected_ids.sort_by_key(|id| id.to_string());
    assert_eq!(listed_ids, expected_ids);
    assert!(listed.contains(&delivered));
    assert!(listed.contains(&unknown));
    assert!(listed.contains(&failed));
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    let status_of = |id: OutboundDeliveryId| {
        attempts
            .iter()
            .find(|attempt| attempt.delivery_id == id)
            .map(|attempt| (attempt.status, attempt.failure_kind))
    };
    assert_eq!(
        status_of(delivered_id),
        Some((OutboundDeliveryStatus::Delivered, None))
    );
    assert_eq!(
        status_of(unknown_id),
        Some((OutboundDeliveryStatus::Unknown, None))
    );
    assert_eq!(
        status_of(failed_id),
        Some((
            OutboundDeliveryStatus::Failed,
            Some(DeliveryFailureKind::Rejected)
        ))
    );
}

async fn reply_publication_rows_are_left_alone_by_crash_recovery(
    store: &impl OutboundStateStorePort,
) {
    let scope = turn_scope();
    let run_id = TurnRunId::new();
    let t0 = now();
    let held = open_and_claim(
        store,
        &scope,
        "reply-publication-recovery",
        run_id,
        "worker-a",
        t0,
    )
    .await;
    let delivery_id = held.attempt.delivery_id;
    assert_eq!(held.attempt.status, OutboundDeliveryStatus::Sending);

    // A `Sending` attempt that carries a publication is not an interrupted
    // one-shot send: recovery reports `false` and leaves the row untouched.
    assert!(
        !store
            .recover_interrupted_delivery_attempt(RecoverInterruptedDeliveryRequest {
                delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap(),
        "publication rows recover through lease expiry, never by being marked Unknown"
    );
    assert_eq!(
        store
            .load_reply_publication(scope.clone(), delivery_id)
            .await
            .unwrap(),
        Some(held.clone())
    );
    let listed = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|attempt| attempt.delivery_id == delivery_id),
        Some(&held.attempt),
        "publication rows still appear in the attempt listing"
    );

    // The one-shot contract is unchanged for a plain interrupted send.
    let plain = prepared_attempt(&scope, "reply-publication-recovery-plain", run_id);
    store.record_delivery_attempt(plain.clone()).await.unwrap();
    assert!(
        store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id: plain.delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap()
    );
    assert!(
        store
            .recover_interrupted_delivery_attempt(RecoverInterruptedDeliveryRequest {
                delivery_id: plain.delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .load_delivery_attempt(scope, plain.delivery_id)
            .await
            .unwrap()
            .map(|attempt| attempt.status),
        Some(OutboundDeliveryStatus::Unknown)
    );
}

async fn pre_publication_attempt_rows_keep_the_one_shot_lifecycle(
    store: &impl OutboundStateStorePort,
) {
    let scope = turn_scope();
    let run_id = TurnRunId::new();
    let t0 = now();
    let attempt = prepared_attempt(&scope, "reply-publication-legacy", run_id);
    let delivery_id = attempt.delivery_id;
    store
        .record_delivery_attempt(attempt.clone())
        .await
        .unwrap();

    assert_eq!(
        store
            .load_reply_publication(scope.clone(), delivery_id)
            .await
            .unwrap(),
        None
    );
    assert!(
        store
            .list_reply_publications(scope.clone(), run_id)
            .await
            .unwrap()
            .is_empty()
    );
    let settle = store
        .settle_reply_publication(settle_request(
            delivery_id,
            &scope,
            0,
            ReplyPublicationSettlement::Unknown,
            t0,
        ))
        .await;
    assert!(matches!(
        settle,
        Err(OutboundError::ReplyPublicationNotFound)
    ));
    let release = store
        .release_reply_publication_lease(ReleaseReplyPublicationLeaseRequest {
            delivery_id,
            scope: scope.clone(),
            fence: 0,
        })
        .await;
    assert!(matches!(
        release,
        Err(OutboundError::ReplyPublicationNotFound)
    ));

    // The one-shot lifecycle is exactly as before, and never grows a
    // publication.
    assert!(
        store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap()
    );
    store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Delivered,
            updated_at: t0 + seconds(1),
            failure_kind: None,
        })
        .await
        .unwrap();
    let delivered = store
        .load_delivery_attempt(scope.clone(), delivery_id)
        .await
        .unwrap()
        .expect("attempt persisted");
    assert_eq!(delivered.status, OutboundDeliveryStatus::Delivered);
    assert_eq!(
        store
            .load_reply_publication(scope.clone(), delivery_id)
            .await
            .unwrap(),
        None
    );
    assert!(
        store
            .list_reply_publications(scope, run_id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The exact bytes a pre-change store wrote — the attempt alone, no
/// `publication` key — must load, run the one-shot lifecycle unchanged, and
/// keep serializing without a `publication` key when rewritten.
#[tokio::test]
async fn legacy_delivery_attempt_rows_without_a_publication_substate_behave_unchanged() {
    let backend = Arc::new(InMemoryBackend::new());
    let filesystem = build_scoped_fs(Arc::clone(&backend), TEST_OUTBOUND_ROOT);
    let store = OutboundStateStore::new(Arc::clone(&filesystem));
    let scope = turn_scope();
    let resource_scope = scope.to_resource_scope();
    let attempt = prepared_attempt(&scope, "legacy-row", TurnRunId::new());
    let delivery_id = attempt.delivery_id;
    let path = ScopedPath::new(format!("/outbound/deliveries/{delivery_id}.json")).unwrap();
    let legacy_bytes = serde_json::to_vec(&attempt).unwrap();
    filesystem
        .put(
            &resource_scope,
            &path,
            Entry::bytes(legacy_bytes).with_content_type(ContentType::json()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .load_delivery_attempt(scope.clone(), delivery_id)
            .await
            .unwrap(),
        Some(attempt.clone())
    );
    assert_eq!(
        store
            .load_reply_publication(scope.clone(), delivery_id)
            .await
            .unwrap(),
        None
    );
    let claim = store
        .claim_reply_publication_lease(claim_request(delivery_id, &scope, "worker-a", now()))
        .await;
    assert!(matches!(
        claim,
        Err(OutboundError::ReplyPublicationNotFound)
    ));

    assert!(
        store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap()
    );
    assert!(
        store
            .recover_interrupted_delivery_attempt(RecoverInterruptedDeliveryRequest {
                delivery_id,
                scope: scope.clone(),
            })
            .await
            .unwrap()
    );
    let recovered = store
        .load_delivery_attempt(scope.clone(), delivery_id)
        .await
        .unwrap()
        .expect("legacy row persisted");
    assert_eq!(recovered.status, OutboundDeliveryStatus::Unknown);

    let rewritten = filesystem
        .get(&resource_scope, &path)
        .await
        .unwrap()
        .expect("legacy row persisted");
    let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten.entry.body).unwrap();
    assert!(rewritten_json.get("publication").is_none());
    assert_eq!(rewritten_json, serde_json::to_value(&recovered).unwrap());

    // A freshly recorded attempt serializes byte-for-byte as the attempt
    // alone, so older readers keep decoding new rows.
    let fresh = prepared_attempt(&scope, "fresh-row", TurnRunId::new());
    store.record_delivery_attempt(fresh.clone()).await.unwrap();
    let fresh_path =
        ScopedPath::new(format!("/outbound/deliveries/{}.json", fresh.delivery_id)).unwrap();
    let stored = filesystem
        .get(&resource_scope, &fresh_path)
        .await
        .unwrap()
        .expect("fresh row persisted");
    assert_eq!(stored.entry.body, serde_json::to_vec(&fresh).unwrap());
}
