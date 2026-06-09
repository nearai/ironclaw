use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, ContentType, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, Filter, InMemoryBackend, IndexSpec, Page, RecordVersion, RootFilesystem,
    ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, ThreadId,
    UserId, VirtualPath,
};
use ironclaw_outbound::*;
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use tokio::sync::Mutex;

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
) -> FilesystemOutboundStateStore<InMemoryBackend> {
    FilesystemOutboundStateStore::new(build_scoped_fs(backend, TEST_OUTBOUND_ROOT))
}

#[tokio::test]
async fn in_memory_defaults_policy_progress_opt_in_and_subscription_scope() {
    let store = InMemoryOutboundStateStore::default();
    communication_preferences_are_tenant_user_scoped(&store).await;
    communication_preferences_are_shared_agent_scoped(&store).await;
    communication_preferences_reject_empty_updated_by(&store).await;
    communication_preferences_reject_empty_shared_agent_scope(&store).await;
    communication_preference_put_existing_conflicts_without_writing(&store).await;
    communication_preference_atomic_update_preserves_existing_slots(&store).await;
    communication_preference_update_inserts_absent_record(&store).await;
    communication_preference_stale_version_conflicts_without_writing(&store).await;
    communication_preference_update_rejects_invalid_or_mismatched_record(&store).await;
    durable_policy_subscription_delivery_flow(&store).await;
    subscription_cursor_rejects_mismatched_scope(&store).await;
    subscription_ids_are_scoped_not_global(&store).await;
    subscription_cursor_rejects_backward_advancement(&store).await;
    delivery_status_rejects_inconsistent_failure_kind(&store).await;
    notification_policy_rejects_excessive_targets(&store).await;
}

#[tokio::test]
async fn filesystem_store_satisfies_outbound_contract_on_in_memory_backend() {
    // The new FilesystemOutboundStateStore runs the same contract suite as
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
    communication_preference_atomic_update_preserves_existing_slots(&store).await;
    communication_preference_update_inserts_absent_record(&store).await;
    communication_preference_stale_version_conflicts_without_writing(&store).await;
    communication_preference_update_rejects_invalid_or_mismatched_record(&store).await;
    filesystem_store_rejects_communication_preference_put_cas_conflict(&backend).await;
    filesystem_store_rejects_communication_preference_update_cas_conflict(&backend).await;
    filesystem_store_rejects_mismatched_communication_preference_identity(&backend, &store).await;
    durable_policy_subscription_delivery_flow(&store).await;
    subscription_cursor_rejects_mismatched_scope(&store).await;
    subscription_ids_are_scoped_not_global(&store).await;
    subscription_cursor_rejects_backward_advancement(&store).await;
    delivery_status_rejects_inconsistent_failure_kind(&store).await;
    notification_policy_rejects_excessive_targets(&store).await;
}

// Legacy LibSqlOutboundStateStore / PostgresOutboundStateStore have been
// deleted. The FilesystemOutboundStateStore over LibSqlRootFilesystem /
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
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let tenant_id = TenantId::new("tenant-outbound").unwrap();
    let user_id = UserId::new("user-outbound").unwrap();
    let updated_by = UserId::new("tenant-admin-outbound").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id.clone(), user_id.clone()),
        final_reply_target: Some(reply_ref("reply-pref-final")),
        progress_target: Some(reply_ref("reply-pref-progress")),
        approval_prompt_target: Some(reply_ref("reply-pref-approval")),
        auth_prompt_target: Some(reply_ref("reply-pref-auth")),
        default_modality: Some(CommunicationModality::Text),
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
        final_reply_target: Some(reply_ref("reply-pref-final-updated")),
        progress_target: None,
        approval_prompt_target: Some(reply_ref("reply-pref-approval")),
        auth_prompt_target: None,
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
    S: CommunicationPreferenceRepository + OutboundStateStore,
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
        final_reply_target: Some(reply_ref("reply-pref-shared-project")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
        final_reply_target: Some(reply_ref("reply-pref-shared-projectless")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Voice),
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
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let valid_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(
            TenantId::new("tenant-outbound-validation").unwrap(),
            UserId::new("user-outbound-validation").unwrap(),
        ),
        final_reply_target: Some(reply_ref("reply-pref-validation")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let valid_record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::shared_agent(
            TenantId::new("tenant-outbound-shared-validation").unwrap(),
            AgentId::new("agent-outbound-shared-validation").unwrap(),
            None,
        ),
        final_reply_target: Some(reply_ref("reply-pref-shared-validation")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let tenant_id = TenantId::new("tenant-outbound-duplicate").unwrap();
    let user_id = UserId::new("user-outbound-duplicate").unwrap();
    let key = CommunicationPreferenceKey::personal(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        final_reply_target: Some(reply_ref("reply-pref-duplicate")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-duplicate").unwrap(),
    };
    store
        .put_communication_preference(record.clone())
        .await
        .unwrap();

    let duplicate = CommunicationPreferenceRecord {
        final_reply_target: Some(reply_ref("reply-pref-duplicate-replacement")),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-duplicate-2").unwrap(),
        ..record.clone()
    };
    let result = store.put_communication_preference(duplicate).await;
    assert!(matches!(result, Err(OutboundError::CasConflict)));
    assert_eq!(load_preference_record(store, key).await, Some(record));
}

async fn communication_preference_atomic_update_preserves_existing_slots<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let tenant_id = TenantId::new("tenant-outbound-atomic").unwrap();
    let user_id = UserId::new("user-outbound-atomic").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        final_reply_target: Some(reply_ref("reply-pref-atomic-final")),
        progress_target: Some(reply_ref("reply-pref-atomic-progress")),
        approval_prompt_target: Some(reply_ref("reply-pref-atomic-approval")),
        auth_prompt_target: Some(reply_ref("reply-pref-atomic-auth")),
        default_modality: Some(CommunicationModality::Voice),
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
            final_reply_target: Some(reply_ref("reply-pref-atomic-final-updated")),
            updated_at: now(),
            updated_by: UserId::new("user-outbound-atomic-updater-2").unwrap(),
            ..existing.record
        },
        Some(existing.version),
    )
    .await
    .record;

    assert_eq!(
        updated.final_reply_target,
        Some(reply_ref("reply-pref-atomic-final-updated"))
    );
    assert_eq!(updated.progress_target, record.progress_target);
    assert_eq!(
        updated.approval_prompt_target,
        record.approval_prompt_target
    );
    assert_eq!(updated.auth_prompt_target, record.auth_prompt_target);
    assert_eq!(updated.default_modality, record.default_modality);
    assert_eq!(load_preference_record(store, key).await, Some(updated));
}

async fn communication_preference_update_inserts_absent_record<S>(store: &S)
where
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let tenant_id = TenantId::new("tenant-outbound-update-absent").unwrap();
    let user_id = UserId::new("user-outbound-update-absent").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        final_reply_target: Some(reply_ref("reply-pref-update-absent-final")),
        progress_target: Some(reply_ref("reply-pref-update-absent-progress")),
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let tenant_id = TenantId::new("tenant-outbound-update-error").unwrap();
    let user_id = UserId::new("user-outbound-update-error").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        final_reply_target: Some(reply_ref("reply-pref-update-error-final")),
        progress_target: Some(reply_ref("reply-pref-update-error-progress")),
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
        final_reply_target: Some(reply_ref("reply-pref-update-error-race")),
        updated_at: now(),
        updated_by: UserId::new("user-outbound-update-error-racer").unwrap(),
        ..existing.record.clone()
    };
    write_preference_record(store, first_update, Some(existing.version)).await;
    let stale_update = CommunicationPreferenceRecord {
        final_reply_target: Some(reply_ref("reply-pref-update-error-stale")),
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
    S: CommunicationPreferenceRepository + OutboundStateStore,
{
    let tenant_id = TenantId::new("tenant-outbound-update-invalid").unwrap();
    let user_id = UserId::new("user-outbound-update-invalid").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        final_reply_target: Some(reply_ref("reply-pref-update-invalid-final")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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

async fn filesystem_store_rejects_mismatched_communication_preference_identity(
    backend: &Arc<InMemoryBackend>,
    store: &FilesystemOutboundStateStore<InMemoryBackend>,
) {
    let tenant_id = TenantId::new("tenant-outbound-corrupt").unwrap();
    let user_id = UserId::new("user-outbound-corrupt").unwrap();
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id.clone(), user_id.clone()),
        final_reply_target: Some(reply_ref("reply-pref-corrupt")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
        final_reply_target: Some(reply_ref("reply-pref-corrupt-tenant-seed")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
        final_reply_target: Some(reply_ref("reply-pref-corrupt-tenant")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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
async fn filesystem_store_personal_and_shared_agent_hashes_are_always_distinct() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(Arc::clone(&backend));
    let tenant_id = TenantId::new("tenant-outbound-hash-distinct").unwrap();
    let shared_id = "same-principal-id";
    let personal_key =
        CommunicationPreferenceKey::personal(tenant_id.clone(), UserId::new(shared_id).unwrap());
    let personal_record = CommunicationPreferenceRecord {
        scope: personal_key.scope.clone(),
        final_reply_target: Some(reply_ref("reply-pref-hash-personal")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin-outbound-hash-personal").unwrap(),
    };
    let (_, personal_path) =
        put_preference_and_find_virtual_path(&backend, &store, personal_record.clone()).await;

    let shared_key =
        CommunicationPreferenceKey::shared_agent(tenant_id, AgentId::new(shared_id).unwrap(), None);
    let shared_record = CommunicationPreferenceRecord {
        scope: shared_key.scope.clone(),
        final_reply_target: Some(reply_ref("reply-pref-hash-shared")),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Voice),
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

async fn filesystem_store_rejects_communication_preference_put_cas_conflict(
    backend: &Arc<InMemoryBackend>,
) {
    let racing = Arc::new(VersionRacingBackend::new(Arc::clone(backend)));
    let store =
        FilesystemOutboundStateStore::new(build_scoped_fs(Arc::clone(&racing), TEST_OUTBOUND_ROOT));
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
        final_reply_target: Some(reply_ref("reply-pref-cas")),
        progress_target: Some(reply_ref("reply-pref-cas-progress")),
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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

async fn filesystem_store_rejects_communication_preference_update_cas_conflict(
    backend: &Arc<InMemoryBackend>,
) {
    let racing = Arc::new(VersionRacingBackend::new(Arc::clone(backend)));
    let store =
        FilesystemOutboundStateStore::new(build_scoped_fs(Arc::clone(&racing), TEST_OUTBOUND_ROOT));
    let tenant_id = TenantId::new("tenant-outbound-update-cas").unwrap();
    let user_id = UserId::new("user-outbound-update-cas").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        final_reply_target: Some(reply_ref("reply-pref-update-cas")),
        progress_target: Some(reply_ref("reply-pref-update-cas-progress")),
        approval_prompt_target: Some(reply_ref("reply-pref-update-cas-approval")),
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Voice),
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
        final_reply_target: Some(reply_ref("reply-pref-update-cas-final-updated")),
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
async fn filesystem_store_rejects_communication_preference_write_on_unsupported_cas_mount() {
    let inner = Arc::new(InMemoryBackend::new());
    let backend = Arc::new(UnsupportedPreferenceCasBackend::new(Arc::clone(&inner)));
    let store = FilesystemOutboundStateStore::new(build_scoped_fs(
        Arc::clone(&backend),
        TEST_OUTBOUND_ROOT,
    ));
    let tenant_id = TenantId::new("tenant-outbound-unsupported-cas").unwrap();
    let user_id = UserId::new("user-outbound-unsupported-cas").unwrap();
    let key = CommunicationPreferenceKey::new(tenant_id.clone(), user_id.clone());
    let record = CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(tenant_id, user_id),
        final_reply_target: Some(reply_ref("reply-pref-unsupported-cas")),
        progress_target: Some(reply_ref("reply-pref-unsupported-cas-progress")),
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
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

async fn durable_policy_subscription_delivery_flow(store: &impl OutboundStateStore) {
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
    let cursor = ProjectionCursor::for_scope(projection_scope(), EventCursor::new(42));
    store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: cursor.clone(),
        })
        .await
        .unwrap();
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
    assert_eq!(loaded, cursor);

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

async fn seed_subscription(store: &impl OutboundStateStore) {
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

async fn subscription_cursor_rejects_mismatched_scope(store: &impl OutboundStateStore) {
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

    let mut wrong_scope = projection_scope();
    wrong_scope.read_scope.thread_id = Some(ThreadId::new("thread-other").unwrap());
    let result = store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: ProjectionCursor::for_scope(wrong_scope, EventCursor::new(7)),
        })
        .await;
    assert!(matches!(
        result,
        Err(OutboundError::SubscriptionScopeMismatch)
    ));

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

async fn subscription_ids_are_scoped_not_global(store: &impl OutboundStateStore) {
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

async fn subscription_cursor_rejects_backward_advancement(store: &impl OutboundStateStore) {
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

    let regression = store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id.clone(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: ProjectionCursor::for_scope(projection_scope(), EventCursor::new(7)),
        })
        .await;
    assert!(matches!(
        regression,
        Err(OutboundError::InvalidRequest { .. })
    ));

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

async fn delivery_status_rejects_inconsistent_failure_kind(store: &impl OutboundStateStore) {
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

async fn notification_policy_rejects_excessive_targets(store: &impl OutboundStateStore) {
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

async fn full_turn_scope_isolation(store: &impl OutboundStateStore, original_scope: TurnScope) {
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
    store: &FilesystemOutboundStateStore<InMemoryBackend>,
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

/// Test backend that mimics a mount that cannot honor CAS writes for
/// communication-preference records. An accidental byte fallback would retry
/// as `CasExpectation::Any` and succeed through the inner backend, so the
/// test above proves preference writes fail closed instead.
struct UnsupportedPreferenceCasBackend {
    inner: Arc<InMemoryBackend>,
    unsupported: Mutex<u32>,
}

impl UnsupportedPreferenceCasBackend {
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
impl RootFilesystem for UnsupportedPreferenceCasBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if path
            .as_str()
            .starts_with(&format!("{TEST_OUTBOUND_ROOT}/communication-preferences/"))
            && !matches!(cas, CasExpectation::Any)
        {
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

/// Audit finding F4: prove the CAS retry loop on
/// `advance_subscription_cursor` converges when a racing writer bumps the
/// version exactly once between the store's read and put. Before F1 this
/// would silently lose the forward progression because the put used
/// `CasExpectation::Any`; before F5 the retry loop couldn't distinguish a
/// transient race from a permanent backend error.
#[tokio::test]
async fn advance_subscription_cursor_retries_through_cas_conflict() {
    let inner = Arc::new(InMemoryBackend::new());
    let racing = Arc::new(VersionRacingBackend::new(Arc::clone(&inner)));
    let store = FilesystemOutboundStateStore::new(build_scoped_fs(
        Arc::clone(&racing),
        "/engine/tenants/test/users/test/outbound",
    ));
    seed_subscription(&store).await;

    // Arm one injected conflict on the next put to any subscription path.
    // The store's read returns version v1; we inject `VersionMismatch` on
    // the first put, forcing the retry loop to re-read, re-validate
    // progression, and put again with the new version — which succeeds.
    // The injected prefix matches the resolved VirtualPath the
    // ScopedFilesystem produces for the `/outbound/subscriptions/...` alias.
    racing
        .arm("/engine/tenants/test/users/test/outbound/subscriptions/", 1)
        .await;

    let cursor = ProjectionCursor::for_scope(projection_scope(), EventCursor::new(101));
    store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: cursor.clone(),
        })
        .await
        .expect("retry loop must converge after one transient CAS conflict");

    assert_eq!(
        racing.injected_count().await,
        1,
        "exactly one CAS conflict should have been injected and recovered from",
    );

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
    assert_eq!(loaded, cursor);
}

/// Audit finding F4: with two racing advancers, the loser must NOT silently
/// overwrite the winner's higher cursor. F1's retry loop re-reads and
/// re-validates progression on every attempt, so the loser's request is
/// rejected with `InvalidRequest` because its target cursor is now
/// regressing against the winner's persisted state.
#[tokio::test]
async fn concurrent_backwards_race_rejected_after_winner_advances() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = build_outbound_store_for_backend(Arc::clone(&backend));
    seed_subscription(&store).await;

    // Winner advances first to cursor=100.
    let winner_cursor = ProjectionCursor::for_scope(projection_scope(), EventCursor::new(100));
    store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: winner_cursor.clone(),
        })
        .await
        .unwrap();

    // Loser tries to advance to a strictly lower cursor=50. Even without a
    // racing CAS conflict, the progression re-check inside the retry loop
    // catches the regression on the first iteration.
    let loser_cursor = ProjectionCursor::for_scope(projection_scope(), EventCursor::new(50));
    let regression = store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: loser_cursor,
        })
        .await;
    assert!(
        matches!(regression, Err(OutboundError::InvalidRequest { .. })),
        "regressing cursor must be rejected, got {regression:?}",
    );

    // And the winner's progress is preserved.
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
    assert_eq!(loaded, winner_cursor);
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
/// `filesystem_store_isolates_two_tenants_with_same_user_project_ids`
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
    let store_a = FilesystemOutboundStateStore::new(build_scoped_fs(
        Arc::clone(&backend),
        "/engine/tenants/a/users/alice/outbound",
    ));
    let store_b = FilesystemOutboundStateStore::new(build_scoped_fs(
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
/// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`):
/// every `FilesystemOutboundStateStore` write decorates its `Entry`
/// with a `tenant_id` projection so an admin-tier query can filter
/// explicitly by tenant and a path-rewriting bug surfaces as a
/// query-time mismatch.
///
/// Records a delivery attempt under tenant A's scope, then issues a
/// raw `RootFilesystem::query` against `/outbound/deliveries` with
/// `Filter::Eq { key: "tenant_id", value: <tenant-a> }` and asserts the
/// record is returned; a query for a different tenant must return zero
/// rows.
#[tokio::test]
async fn filesystem_outbound_store_writes_tenant_id_indexed_projection() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = build_scoped_fs(
        Arc::clone(&backend),
        "/engine/tenants/tenant-outbound/users/user-outbound/outbound",
    );
    let store = FilesystemOutboundStateStore::new(Arc::clone(&scoped));
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
        ironclaw_host_api::ScopedPath::new("/outbound/deliveries".to_string()).unwrap();
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
                key: tenant_key,
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
}
