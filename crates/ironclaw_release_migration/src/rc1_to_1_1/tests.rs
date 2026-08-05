use super::*;
use crate::lifecycle::{
    DomainCompletionRecord, MigrationRecord, MigrationStatus, RollbackDisposition, migration_entry,
    migration_path,
};
use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend};
use ironclaw_host_api::ids::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};

#[test]
fn absent_extension_domains_are_not_reported_as_completed() {
    let report = redacted_core_report(
        &ironclaw_processes::LegacyProcessMigrationReport::default(),
        &ironclaw_threads::ThreadStartupMigrationReport::default(),
        &ChannelRootMigrationReport::default(),
        &ironclaw_auth::OAuthProviderAliasMigrationReport::default(),
        None,
        None,
        None,
    );
    assert!(report.get("extension_installations").is_none());
    assert!(report.get("channel_extension_state").is_none());
    assert!(report.get("workspace_artifacts").is_none());
}

#[tokio::test]
async fn release_lifecycle_accepts_another_pair_without_rc1_branching() {
    const TEST_PAIR: ReleasePair = ReleasePair {
        schema: "release-pair-migration-v1",
        source_release: "1.1.0-rc.1",
        target_release: "1.2.0-rc.1",
        migration_path: "/tenants/__system__/shared/startup-migrations/test-pair.json",
        domain_migration_root: "/tenants/__system__/shared/startup-migrations/test-pair/domains",
        old_authorities_retained: true,
        in_place_rows_backward_readable: false,
    };
    let backend = Arc::new(InMemoryBackend::new());
    let lease = ReleasePairMigrationLease::acquire(
        Arc::clone(&backend),
        TEST_PAIR,
        std::future::ready(Ok(json!({"examined_rows": 0}))),
    )
    .await
    .expect("release-neutral lifecycle acquires another pair");
    lease
        .complete(json!({"threads": {"migrated": 0}}))
        .await
        .expect("release-neutral lifecycle completes another pair");

    let stored = backend
        .get(&migration_path(TEST_PAIR).expect("test marker path"))
        .await
        .expect("read marker")
        .expect("marker exists");
    let record: MigrationRecord =
        serde_json::from_slice(&stored.entry.body).expect("decode marker");
    assert_eq!(record.source_release, TEST_PAIR.source_release);
    assert_eq!(record.target_release, TEST_PAIR.target_release);
    assert!(!record.rollback.in_place_rows_backward_readable);
}

#[tokio::test]
async fn legacy_workspace_is_copied_to_scoped_target_and_reverified() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("snapshot");
    let workspace_root = temp.path().join("live-workspace");
    std::fs::create_dir_all(source.join("nested")).expect("source directories");
    std::fs::write(source.join("root.txt"), b"root artifact\n").expect("root artifact");
    std::fs::write(source.join("nested/data.bin"), [0_u8, 1, 2, 3]).expect("nested artifact");

    let input = LegacyWorkspaceMigrationInput {
        source: source.clone(),
        workspace_root: workspace_root.clone(),
        tenant_id: TenantId::new("tenant-a").expect("tenant id"),
        user_id: UserId::new("user-a").expect("user id"),
    };
    let first = migrate_legacy_workspace_snapshot(input.clone())
        .await
        .expect("first migration");
    assert_eq!(first.files_migrated, 2);
    assert_eq!(first.files_unchanged, 0);
    let target = workspace_root.join("tenants/tenant-a/users/user-a");
    assert_eq!(
        std::fs::read(target.join("root.txt")).expect("migrated root artifact"),
        b"root artifact\n"
    );
    assert_eq!(
        std::fs::read(target.join("nested/data.bin")).expect("migrated nested artifact"),
        [0_u8, 1, 2, 3]
    );
    assert!(source.join("root.txt").exists(), "source must be retained");

    let second = migrate_legacy_workspace_snapshot(input)
        .await
        .expect("repeat migration");
    assert_eq!(second.files_migrated, 0);
    assert_eq!(second.files_unchanged, 2);
}

#[tokio::test]
async fn legacy_workspace_conflict_fails_without_overwrite() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("snapshot");
    let workspace_root = temp.path().join("live-workspace");
    let target = workspace_root.join("tenants/tenant-a/users/user-a");
    std::fs::create_dir_all(&source).expect("source directory");
    std::fs::create_dir_all(&target).expect("target directory");
    std::fs::write(source.join("artifact.txt"), b"rc1").expect("source artifact");
    std::fs::write(target.join("artifact.txt"), b"1.1").expect("target artifact");

    let error = migrate_legacy_workspace_snapshot(LegacyWorkspaceMigrationInput {
        source,
        workspace_root,
        tenant_id: TenantId::new("tenant-a").expect("tenant id"),
        user_id: UserId::new("user-a").expect("user id"),
    })
    .await
    .expect_err("divergent target must fail");
    assert!(error.to_string().contains("divergent content"));
    assert_eq!(
        std::fs::read(target.join("artifact.txt")).expect("target remains readable"),
        b"1.1"
    );
}

#[tokio::test]
async fn database_wide_lease_fails_concurrent_startup_and_allows_failed_retry() {
    let backend = Arc::new(InMemoryBackend::new());
    let first = acquire_release_pair_lease(Arc::clone(&backend))
        .await
        .expect("first startup acquires lease");
    let second = match acquire_release_pair_lease(Arc::clone(&backend)).await {
        Ok(_) => panic!("concurrent startup must fail closed"),
        Err(error) => error,
    };
    assert!(matches!(
        second,
        ReleasePairMigrationError::ConcurrentStartup
    ));

    first.fail().await.expect("failed attempt releases lease");
    let retry = acquire_release_pair_lease(Arc::clone(&backend))
        .await
        .expect("failed startup is immediately retryable");
    retry
        .complete(json!({"threads": {"migrated": 1}}))
        .await
        .expect("retry publishes completion");

    let verify = acquire_release_pair_lease(Arc::clone(&backend))
        .await
        .expect("completed migration can be reverified on restart");
    verify
        .complete(json!({"threads": {"migrated": 0, "unchanged": 1}}))
        .await
        .expect("reverification publishes zero-change report");
    let stored = backend
        .get(&migration_path(RC1_TO_1_1).expect("migration path"))
        .await
        .expect("read completion")
        .expect("completion exists");
    let record: MigrationRecord =
        serde_json::from_slice(&stored.entry.body).expect("decode completion");
    assert_eq!(record.status, MigrationStatus::Complete);
    assert_eq!(
        record.report,
        Some(json!({"threads": {"migrated": 0, "unchanged": 1}}))
    );
    assert!(record.rollback.old_authorities_retained);

    let domain_path = virtual_path(&format!(
        "{}/threads-v1.complete.json",
        RC1_TO_1_1.domain_migration_root
    ))
    .expect("domain completion path");
    let domain = backend
        .get(&domain_path)
        .await
        .expect("read domain completion")
        .expect("thread domain completion exists");
    let domain: DomainCompletionRecord =
        serde_json::from_slice(&domain.entry.body).expect("decode domain completion");
    assert_eq!(domain.domain, "threads");
    assert_eq!(domain.status, MigrationStatus::Complete);
    assert_eq!(domain.report, json!({"migrated": 0, "unchanged": 1}));
}

#[tokio::test]
async fn unsupported_release_pair_fails_before_replacing_record() {
    let backend = Arc::new(InMemoryBackend::new());
    let now = Utc::now();
    let incompatible = MigrationRecord {
        schema: RC1_TO_1_1.schema.to_string(),
        source_release: "0.29.0".to_string(),
        target_release: RC1_TO_1_1.target_release.to_string(),
        status: MigrationStatus::Complete,
        attempt_id: "incompatible".to_string(),
        started_at: now,
        lease_expires_at: now,
        finished_at: Some(now),
        source_fingerprint: json!({}),
        report: None,
        rollback: RollbackDisposition {
            old_authorities_retained: true,
            in_place_rows_backward_readable: true,
        },
    };
    backend
        .put(
            &migration_path(RC1_TO_1_1).expect("migration path"),
            migration_entry(&incompatible).expect("migration entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed incompatible record");

    let error = match acquire_release_pair_lease(backend).await {
        Ok(_) => panic!("unsupported release pair must fail closed"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ReleasePairMigrationError::UnsupportedReleasePair
    ));
}

#[tokio::test]
async fn dropped_startup_lease_is_failed_for_immediate_retry() {
    let backend = Arc::new(InMemoryBackend::new());
    let lease = acquire_release_pair_lease(Arc::clone(&backend))
        .await
        .expect("startup acquires lease");
    drop(lease);

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(row) = backend
                .get(&migration_path(RC1_TO_1_1).expect("migration path"))
                .await
                .expect("read lease")
            {
                let record: MigrationRecord =
                    serde_json::from_slice(&row.entry.body).expect("decode lease");
                if record.status == MigrationStatus::Failed {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("drop guard publishes failure");

    let retry = acquire_release_pair_lease(backend)
        .await
        .expect("retry does not wait for the prior lease timeout");
    retry.fail().await.expect("release retry lease");
}

#[tokio::test]
async fn failed_domain_attempt_never_publishes_false_completion_records() {
    let backend = Arc::new(InMemoryBackend::new());
    let lease = acquire_release_pair_lease(Arc::clone(&backend))
        .await
        .expect("startup acquires lease before domain validation");

    // Production calls this path after any malformed/conflicting domain
    // reader fails. It must leave an auditable failed attempt, not either
    // the release-pair completion or a per-domain completion marker.
    lease.fail().await.expect("record failed attempt");
    let stored = backend
        .get(&migration_path(RC1_TO_1_1).expect("migration path"))
        .await
        .expect("read failed attempt")
        .expect("attempt record exists");
    let record: MigrationRecord =
        serde_json::from_slice(&stored.entry.body).expect("decode attempt");
    assert_eq!(record.status, MigrationStatus::Failed);
    assert!(record.report.is_none());
    let domain_rows = backend
        .query(
            &virtual_path(RC1_TO_1_1.domain_migration_root).expect("domain root"),
            &Filter::All,
            Page::new(0, Page::MAX_LIMIT),
        )
        .await
        .expect("query domain completions");
    assert!(domain_rows.is_empty());
}

#[tokio::test]
async fn channel_reference_barrier_requires_the_same_canonical_thread() {
    let backend = InMemoryBackend::new();
    let tenant = TenantId::new("tenant-a").expect("tenant");
    let agent = AgentId::new("agent-a").expect("agent");
    let thread = ThreadId::new("thread-a").expect("thread");
    let report = ChannelRootMigrationReport {
        referenced_threads: vec![ironclaw_conversations::ConversationThreadReference {
            tenant_id: tenant.clone(),
            thread_id: thread.clone(),
            agent_id: Some(agent.clone()),
            project_id: None,
        }],
        ..ChannelRootMigrationReport::default()
    };

    let missing = validate_channel_thread_references(&backend, &report)
        .await
        .expect_err("missing canonical thread must fail the startup barrier");
    assert!(matches!(missing, ReleasePairMigrationError::Domain { .. }));

    let header = SessionThreadRecord {
        scope: ThreadScope {
            tenant_id: tenant,
            agent_id: agent,
            project_id: None,
            owner_user_id: None,
            mission_id: None,
        },
        thread_id: thread,
        created_by_actor_id: "actor-a".to_string(),
        title: None,
        metadata_json: None,
        goal: None,
        created_at: None,
        updated_at: None,
    };
    let path = virtual_path(
        "/tenants/tenant-a/users/__system__/threads/agents/agent-a/owners/__system__/thread-a/thread.json",
    )
    .expect("header path");
    let kind = RecordKind::new("session_thread").expect("thread kind");
    let header = serde_json::to_value(header).expect("serialize header");
    backend
        .put(
            &path,
            Entry::record(kind, &header).expect("header entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed header");

    validate_channel_thread_references(&backend, &report)
        .await
        .expect("matching canonical thread opens barrier");
}
