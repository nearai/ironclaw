use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, Filter, InMemoryBackend, LibSqlRootFilesystem, Page,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{InvocationId, TenantId, UserId},
    path::{ScopedPath, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_host_api::{
    mount::{MountGrant, MountPermissions, MountView},
    path::MountAlias,
};
use ironclaw_libsql_runtime::LibSqlRuntime;
use ironclaw_telemetry::{
    CollectorCoverage, FilesystemTelemetryRepository, HourlyAutomationUsage, HourlyModelUsage,
    HourlyRunFailure, HourlyUserActivity, LifecycleEvent, ScopedTelemetryBatch, TelemetryBatch,
    TelemetryPageRequest,
};
use ironclaw_telemetry_contracts::observation::{
    AutomationKind, CollectorInstanceId, EffectiveModelId, FailureCategory, LifecycleEventId,
    LifecycleEventKind, LifecycleSubjectKind, OriginKind, ProviderId, SubjectId,
};

fn scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).expect("test tenant"),
        user_id: UserId::new(user).expect("test user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn scoped_filesystem<F>(root: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem + 'static,
{
    Arc::new(ScopedFilesystem::new(root, |scope| {
        let target = VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id.as_str()))
            .expect("test target");
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared").expect("test alias"),
            target,
            MountPermissions::read_write_list_delete(),
        )])
    }))
}

fn in_memory_filesystem() -> (Arc<InMemoryBackend>, Arc<ScopedFilesystem<InMemoryBackend>>) {
    let root = Arc::new(InMemoryBackend::new());
    let filesystem = scoped_filesystem(Arc::clone(&root));
    (root, filesystem)
}

fn activity(tenant: &str, user: &str, hour: chrono::DateTime<Utc>) -> HourlyUserActivity {
    activity_with_counts(tenant, user, hour, 1, 1)
}

fn activity_with_counts(
    tenant: &str,
    user: &str,
    hour: chrono::DateTime<Utc>,
    run_count: u64,
    completed_count: u64,
) -> HourlyUserActivity {
    HourlyUserActivity::new(
        TenantId::new(tenant).expect("test tenant"),
        hour,
        UserId::new(user).expect("test user"),
        OriginKind::Human,
        run_count,
        0,
        0,
        0,
        completed_count,
        0,
        0,
        0,
        10,
        hour,
        hour,
    )
    .expect("test activity")
}

fn model(
    tenant: &str,
    user: &str,
    hour: chrono::DateTime<Utc>,
    provider: &str,
    effective_model: &str,
) -> HourlyModelUsage {
    HourlyModelUsage::new(
        TenantId::new(tenant).expect("test tenant"),
        UserId::new(user).expect("test user"),
        hour,
        ProviderId::new(provider).expect("test provider"),
        EffectiveModelId::new(effective_model).expect("test model"),
        1,
        1,
        10,
        20,
        0,
        0,
        hour,
        hour,
    )
    .expect("test model usage")
}

fn request(
    from: chrono::DateTime<Utc>,
    to: chrono::DateTime<Utc>,
    now: chrono::DateTime<Utc>,
    page_size: usize,
    after: Option<String>,
) -> TelemetryPageRequest {
    TelemetryPageRequest::new(from, to, now, page_size, after).expect("test request")
}

#[test]
fn page_requests_reject_empty_oversized_and_unbounded_inputs() {
    let from = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test timestamp");
    let now = from + Duration::days(366) + Duration::hours(1);

    let empty_range = TelemetryPageRequest::new(from, from, now, 1, None)
        .expect_err("empty range must be rejected");
    assert!(matches!(
        empty_range,
        ironclaw_telemetry::TelemetryRepositoryError::InvalidScanRequest { .. }
    ));

    let oversized_range = TelemetryPageRequest::new(
        from,
        from + Duration::days(366) + Duration::microseconds(1),
        now,
        1,
        None,
    )
    .expect_err("ranges over 366 days must be rejected");
    assert!(matches!(
        oversized_range,
        ironclaw_telemetry::TelemetryRepositoryError::InvalidScanRequest { .. }
    ));

    let zero_page_size = TelemetryPageRequest::new(from, from + Duration::hours(1), now, 0, None)
        .expect_err("zero page size must be rejected");
    assert!(matches!(
        zero_page_size,
        ironclaw_telemetry::TelemetryRepositoryError::InvalidPageRequest { .. }
    ));

    let oversized_page_size =
        TelemetryPageRequest::new(from, from + Duration::hours(1), now, 2_001, None)
            .expect_err("page sizes over 2000 must be rejected");
    assert!(matches!(
        oversized_page_size,
        ironclaw_telemetry::TelemetryRepositoryError::InvalidPageRequest { .. }
    ));

    let oversized_cursor = TelemetryPageRequest::new(
        from,
        from + Duration::hours(1),
        now,
        1,
        Some("x".repeat(4_097)),
    )
    .expect_err("cursors over 4096 bytes must be rejected");
    assert!(matches!(
        oversized_cursor,
        ironclaw_telemetry::TelemetryRepositoryError::InvalidPageRequest { .. }
    ));
}

async fn assert_shared_repository_contract<F>(filesystem: Arc<ScopedFilesystem<F>>)
where
    F: RootFilesystem + 'static,
{
    let repository = FilesystemTelemetryRepository::new(filesystem);
    let tenant = scope("tenant-contract", "user-contract");
    let other_tenant = scope("tenant-other", "user-other");
    let hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    let user_id = UserId::new("user-contract").expect("test user");
    let tenant_id = TenantId::new("tenant-contract").expect("test tenant");

    repository.ensure_indexes(&tenant).await.expect("indexes");
    repository
        .ensure_indexes(&tenant)
        .await
        .expect("duplicate index initialization");

    let failure = HourlyRunFailure::new(
        tenant_id.clone(),
        hour,
        user_id.clone(),
        FailureCategory::new("provider_error").expect("failure category"),
        1,
        hour,
        hour,
    )
    .expect("failure row");
    let automation = HourlyAutomationUsage::new(
        tenant_id.clone(),
        hour,
        user_id.clone(),
        AutomationKind::Cron,
        1,
        1,
        0,
        0,
        0,
        hour,
        hour,
    )
    .expect("automation row");
    let lifecycle = LifecycleEvent::new(
        tenant_id.clone(),
        LifecycleEventId::new("contract-event").expect("event id"),
        Some(user_id),
        LifecycleEventKind::RoutineCreated,
        LifecycleSubjectKind::Routine,
        SubjectId::new("contract-routine").expect("subject id"),
        hour,
    )
    .expect("lifecycle row");
    let coverage = CollectorCoverage::new(
        tenant_id,
        hour,
        CollectorInstanceId::new("contract-collector").expect("collector id"),
        1,
        0,
        0,
        0,
        0,
        hour,
        hour,
    )
    .expect("coverage row");
    let batch = TelemetryBatch::new(
        vec![
            activity("tenant-contract", "user-contract", hour),
            activity("tenant-contract", "user-contract-2", hour),
        ],
        vec![model(
            "tenant-contract",
            "user-contract",
            hour,
            "provider-contract",
            "model-contract",
        )],
        vec![failure],
        vec![automation],
        vec![lifecycle],
        vec![coverage],
    )
    .expect("shared contract batch");
    let report = repository
        .apply_batch(ScopedTelemetryBatch::new(tenant.clone(), batch))
        .await
        .expect("shared contract write");
    assert_eq!(report.applied_record_count(), 7);
    assert_eq!(report.failed_record_count(), 0);

    let page_request = request(
        hour,
        hour + Duration::hours(1),
        hour + Duration::hours(1),
        100,
        None,
    );
    assert_eq!(
        repository
            .read_activity_page(&tenant, &page_request)
            .await
            .expect("activity read")
            .rows()
            .len(),
        2
    );
    let first_activity_page = repository
        .read_activity_page(
            &tenant,
            &request(
                hour,
                hour + Duration::hours(1),
                hour + Duration::hours(1),
                1,
                None,
            ),
        )
        .await
        .expect("first activity page");
    assert_eq!(first_activity_page.rows().len(), 1);
    let second_activity_page = repository
        .read_activity_page(
            &tenant,
            &request(
                hour,
                hour + Duration::hours(1),
                hour + Duration::hours(1),
                1,
                first_activity_page.next_cursor().map(str::to_owned),
            ),
        )
        .await
        .expect("second activity page");
    assert_eq!(second_activity_page.rows().len(), 1);
    assert!(second_activity_page.next_cursor().is_none());
    assert_eq!(
        repository
            .read_model_page(&tenant, &page_request)
            .await
            .expect("model read")
            .rows()
            .len(),
        1
    );
    assert_eq!(
        repository
            .read_failure_page(&tenant, &page_request)
            .await
            .expect("failure read")
            .rows()
            .len(),
        1
    );
    assert_eq!(
        repository
            .read_automation_page(&tenant, &page_request)
            .await
            .expect("automation read")
            .rows()
            .len(),
        1
    );
    assert_eq!(
        repository
            .read_lifecycle_page(&tenant, &page_request)
            .await
            .expect("lifecycle read")
            .rows()
            .len(),
        1
    );
    assert_eq!(
        repository
            .read_coverage_page(&tenant, &page_request)
            .await
            .expect("coverage read")
            .rows()
            .len(),
        1
    );
    assert!(
        repository
            .read_activity_page(&other_tenant, &page_request)
            .await
            .expect("other tenant activity read")
            .rows()
            .is_empty()
    );
    assert!(
        repository
            .read_model_page(&other_tenant, &page_request)
            .await
            .expect("other tenant model read")
            .rows()
            .is_empty()
    );
    assert!(
        repository
            .read_failure_page(&other_tenant, &page_request)
            .await
            .expect("other tenant failure read")
            .rows()
            .is_empty()
    );
    assert!(
        repository
            .read_automation_page(&other_tenant, &page_request)
            .await
            .expect("other tenant automation read")
            .rows()
            .is_empty()
    );
    assert!(
        repository
            .read_lifecycle_page(&other_tenant, &page_request)
            .await
            .expect("other tenant lifecycle read")
            .rows()
            .is_empty()
    );
    assert!(
        repository
            .read_coverage_page(&other_tenant, &page_request)
            .await
            .expect("other tenant coverage read")
            .rows()
            .is_empty()
    );
}

#[tokio::test]
async fn in_memory_repository_satisfies_shared_contract() {
    let (_, filesystem) = in_memory_filesystem();
    assert_shared_repository_contract(filesystem).await;
}

#[tokio::test]
async fn filesystem_repository_is_tenant_scoped_and_additive() {
    let (_, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant_a = scope("tenant-a", "user-a");
    let tenant_b = scope("tenant-b", "user-b");
    let hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");

    repository.ensure_indexes(&tenant_a).await.expect("indexes");
    repository
        .apply_batch(ScopedTelemetryBatch::new(
            tenant_a.clone(),
            ironclaw_telemetry::TelemetryBatch::new(
                vec![activity("tenant-a", "user-a", hour)],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .expect("batch"),
        ))
        .await
        .expect("first write");
    repository
        .apply_batch(ScopedTelemetryBatch::new(
            tenant_a.clone(),
            ironclaw_telemetry::TelemetryBatch::new(
                vec![activity("tenant-a", "user-a", hour)],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .expect("batch"),
        ))
        .await
        .expect("additive write");

    let request = request(
        hour - Duration::hours(1),
        hour + Duration::hours(1),
        hour + Duration::hours(1),
        100,
        None,
    );
    let page = repository
        .read_activity_page(&tenant_a, &request)
        .await
        .expect("tenant read");
    assert_eq!(page.rows().len(), 1);
    assert_eq!(page.rows()[0].run_count(), 2);
    assert!(
        repository
            .read_activity_page(&tenant_b, &request)
            .await
            .expect("other tenant read")
            .rows()
            .is_empty()
    );
}

#[tokio::test]
async fn mismatched_scoped_batch_is_rejected_before_either_tenant_is_written() {
    let (_, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant_a = scope("tenant-a", "user-a");
    let tenant_b = scope("tenant-b", "user-b");
    let hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    let batch = TelemetryBatch::new(
        vec![
            activity("tenant-a", "user-a", hour),
            activity("tenant-b", "user-b", hour),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
    )
    .expect("mismatched batch");

    let error = repository
        .apply_batch(ScopedTelemetryBatch::new(tenant_a.clone(), batch))
        .await
        .expect_err("batch scope mismatch must be rejected");
    assert!(matches!(
        error,
        ironclaw_telemetry::TelemetryRepositoryError::ScopeMismatch
    ));

    let page_request = request(
        hour,
        hour + Duration::hours(1),
        hour + Duration::hours(1),
        100,
        None,
    );
    assert!(
        repository
            .read_activity_page(&tenant_a, &page_request)
            .await
            .expect("tenant A read")
            .rows()
            .is_empty()
    );
    assert!(
        repository
            .read_activity_page(&tenant_b, &page_request)
            .await
            .expect("tenant B read")
            .rows()
            .is_empty()
    );
}

#[tokio::test]
async fn repository_pages_half_open_ranges_and_model_filters() {
    let (_, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant = scope("tenant-a", "user-a");
    let first_hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    let second_hour = first_hour + Duration::hours(1);
    let third_hour = second_hour + Duration::hours(1);
    let batch = TelemetryBatch::new(
        vec![],
        vec![
            model("tenant-a", "user-a", first_hour, "provider-a", "model-a"),
            model("tenant-a", "user-a", first_hour, "provider-b", "model-b"),
            model("tenant-a", "user-a", second_hour, "provider-a", "model-a"),
            model("tenant-a", "user-a", third_hour, "provider-a", "model-c"),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
    )
    .expect("model batch");
    repository
        .apply_batch(ScopedTelemetryBatch::new(tenant.clone(), batch))
        .await
        .expect("model write");

    let first_page = repository
        .read_model_page(
            &tenant,
            &request(
                first_hour,
                third_hour,
                third_hour + Duration::hours(1),
                1,
                None,
            ),
        )
        .await
        .expect("first model page");
    assert_eq!(first_page.rows().len(), 1);
    assert_eq!(first_page.rows()[0].window_start(), first_hour);
    let second_page = repository
        .read_model_page(
            &tenant,
            &request(
                first_hour,
                third_hour,
                third_hour + Duration::hours(1),
                1,
                first_page.next_cursor().map(str::to_owned),
            ),
        )
        .await
        .expect("second model page");
    assert_eq!(second_page.rows().len(), 1);
    assert_eq!(second_page.rows()[0].window_start(), first_hour);
    assert_ne!(
        second_page.rows()[0].provider_id(),
        first_page.rows()[0].provider_id()
    );
    let provider_model_page = repository
        .read_model_page(
            &tenant,
            &request(
                first_hour,
                third_hour,
                third_hour + Duration::hours(1),
                100,
                None,
            )
            .with_provider_id(Some(ProviderId::new("provider-a").expect("provider")))
            .with_effective_model_id(Some(EffectiveModelId::new("model-a").expect("model"))),
        )
        .await
        .expect("provider/model filtered page");
    assert_eq!(provider_model_page.rows().len(), 2);
    assert!(
        provider_model_page
            .rows()
            .iter()
            .all(|row| row.provider_id().as_str() == "provider-a"
                && row.effective_model_id().as_str() == "model-a")
    );
    let provider_page = repository
        .read_model_page(
            &tenant,
            &request(
                first_hour,
                third_hour,
                third_hour + Duration::hours(1),
                100,
                None,
            )
            .with_provider_id(Some(ProviderId::new("provider-b").expect("provider"))),
        )
        .await
        .expect("provider filtered page");
    assert_eq!(provider_page.rows().len(), 1);
    assert_eq!(
        provider_page.rows()[0].effective_model_id().as_str(),
        "model-b"
    );
    let model_page = repository
        .read_model_page(
            &tenant,
            &request(
                first_hour,
                third_hour,
                third_hour + Duration::hours(1),
                100,
                None,
            )
            .with_effective_model_id(Some(EffectiveModelId::new("model-a").expect("model"))),
        )
        .await
        .expect("model filtered page");
    assert_eq!(model_page.rows().len(), 2);
    assert!(
        model_page
            .rows()
            .iter()
            .all(|row| row.effective_model_id().as_str() == "model-a")
    );
}

#[tokio::test]
async fn repository_ceil_lower_bound_excludes_prior_microsecond_record() {
    let (_, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant = scope("tenant-a", "user-a");
    let first_hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    let second_hour = first_hour + Duration::hours(1);
    let batch = TelemetryBatch::new(
        vec![
            activity("tenant-a", "user-a", first_hour),
            activity("tenant-a", "user-b", second_hour),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
    )
    .expect("activity batch");
    repository
        .apply_batch(ScopedTelemetryBatch::new(tenant.clone(), batch))
        .await
        .expect("activity write");

    let page = repository
        .read_activity_page(
            &tenant,
            &request(
                first_hour + Duration::nanoseconds(1),
                second_hour + Duration::nanoseconds(1),
                second_hour + Duration::hours(1),
                100,
                None,
            ),
        )
        .await
        .expect("activity read");
    assert_eq!(page.rows().len(), 1);
    assert_eq!(page.rows()[0].window_start(), second_hour);
}

#[tokio::test]
async fn model_pagination_accepts_a_maximum_length_valid_model_path() {
    let (_, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant = scope("tenant-a", "user-a");
    let hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    let long_user = "u".repeat(128);
    let long_provider = "p".repeat(128);
    let long_model = "m".repeat(128);
    let next_model = "n".repeat(128);
    let batch = TelemetryBatch::new(
        vec![],
        vec![
            model("tenant-a", &long_user, hour, &long_provider, &long_model),
            model("tenant-a", &long_user, hour, &long_provider, &next_model),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
    )
    .expect("maximum length model batch");
    repository
        .apply_batch(ScopedTelemetryBatch::new(tenant.clone(), batch))
        .await
        .expect("maximum length model write");

    let first_page = repository
        .read_model_page(
            &tenant,
            &request(
                hour,
                hour + Duration::hours(1),
                hour + Duration::hours(1),
                1,
                None,
            ),
        )
        .await
        .expect("first long model page");
    assert_eq!(first_page.rows().len(), 1);
    let second_page = repository
        .read_model_page(
            &tenant,
            &request(
                hour,
                hour + Duration::hours(1),
                hour + Duration::hours(1),
                1,
                first_page.next_cursor().map(str::to_owned),
            ),
        )
        .await
        .expect("second long model page");
    assert_eq!(second_page.rows().len(), 1);
    assert!(second_page.next_cursor().is_none());
}

#[tokio::test]
async fn lifecycle_replay_is_idempotent_and_conflicts_fail_closed() {
    let (_, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant = scope("tenant-a", "user-a");
    let occurred_at = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 30, 0)
        .single()
        .expect("test timestamp");
    let event = LifecycleEvent::new(
        TenantId::new("tenant-a").expect("tenant"),
        LifecycleEventId::new("event-1").expect("event"),
        Some(UserId::new("user-a").expect("user")),
        LifecycleEventKind::RoutineCreated,
        LifecycleSubjectKind::Routine,
        SubjectId::new("routine-1").expect("subject"),
        occurred_at,
    )
    .expect("event");
    let batch = TelemetryBatch::new(vec![], vec![], vec![], vec![], vec![event.clone()], vec![])
        .expect("lifecycle batch");
    repository
        .apply_batch(ScopedTelemetryBatch::new(tenant.clone(), batch.clone()))
        .await
        .expect("first lifecycle write");
    repository
        .apply_batch(ScopedTelemetryBatch::new(tenant.clone(), batch))
        .await
        .expect("replayed lifecycle write");
    let page = repository
        .read_lifecycle_page(
            &tenant,
            &request(
                occurred_at - Duration::hours(1),
                occurred_at + Duration::hours(1),
                occurred_at + Duration::hours(1),
                100,
                None,
            ),
        )
        .await
        .expect("lifecycle read");
    assert_eq!(page.rows(), std::slice::from_ref(&event));

    let conflicting_event = LifecycleEvent::new(
        event.tenant_id().clone(),
        event.event_id().clone(),
        event.user_id().cloned(),
        LifecycleEventKind::RoutineEnabled,
        event.subject_kind(),
        event.subject_id().clone(),
        event.occurred_at(),
    )
    .expect("conflicting event");
    let conflict_batch = TelemetryBatch::new(
        vec![],
        vec![],
        vec![],
        vec![],
        vec![conflicting_event],
        vec![],
    )
    .expect("conflict batch");
    let error = repository
        .apply_batch(ScopedTelemetryBatch::new(tenant, conflict_batch))
        .await
        .expect_err("conflicting replay must fail");
    assert!(matches!(
        error,
        ironclaw_telemetry::TelemetryRepositoryError::InvalidProjection
    ));
}

#[tokio::test]
async fn additive_counter_overflow_leaves_existing_record_unchanged() {
    let (_, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant = scope("tenant-a", "user-a");
    let hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    let max = i64::MAX as u64;
    repository
        .apply_batch(ScopedTelemetryBatch::new(
            tenant.clone(),
            TelemetryBatch::new(
                vec![activity_with_counts("tenant-a", "user-a", hour, max, max)],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .expect("maximum batch"),
        ))
        .await
        .expect("maximum write");
    let error = repository
        .apply_batch(ScopedTelemetryBatch::new(
            tenant.clone(),
            TelemetryBatch::new(
                vec![activity("tenant-a", "user-a", hour)],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .expect("increment batch"),
        ))
        .await
        .expect_err("overflow must fail");
    assert!(matches!(
        error,
        ironclaw_telemetry::TelemetryRepositoryError::CounterOverflow { family: "activity" }
    ));
    let page = repository
        .read_activity_page(
            &tenant,
            &request(
                hour - Duration::hours(1),
                hour + Duration::hours(1),
                hour + Duration::hours(1),
                100,
                None,
            ),
        )
        .await
        .expect("read maximum row");
    assert_eq!(page.rows()[0].run_count(), max);
}

#[tokio::test]
async fn typed_reads_reject_malformed_json_even_with_valid_projection() {
    let (root, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant = scope("tenant-a", "user-a");
    let hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    repository
        .apply_batch(ScopedTelemetryBatch::new(
            tenant.clone(),
            TelemetryBatch::new(
                vec![activity("tenant-a", "user-a", hour)],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .expect("batch"),
        ))
        .await
        .expect("write");
    let prefix = ScopedPath::new("/tenant-shared/telemetry/v0/hourly/activity").expect("prefix");
    let stored = filesystem
        .query(&tenant, &prefix, &Filter::All, Page::first(1))
        .await
        .expect("query stored record")
        .into_iter()
        .next()
        .expect("stored record");
    let malformed = Entry {
        body: b"{}".to_vec(),
        content_type: ContentType::json(),
        kind: stored.entry.kind.clone(),
        indexed: stored.entry.indexed.clone(),
    };
    root.put(
        &stored.path,
        malformed,
        CasExpectation::Version(stored.version),
    )
    .await
    .expect("corrupt record");
    let error = repository
        .read_activity_page(
            &tenant,
            &request(
                hour - Duration::hours(1),
                hour + Duration::hours(1),
                hour + Duration::hours(1),
                100,
                None,
            ),
        )
        .await
        .expect_err("malformed record must fail closed");
    assert!(matches!(
        error,
        ironclaw_telemetry::TelemetryRepositoryError::Serialization { .. }
    ));
}

#[tokio::test]
async fn simple_reads_reject_a_persisted_tenant_mismatch() {
    let (root, filesystem) = in_memory_filesystem();
    let repository = FilesystemTelemetryRepository::new(Arc::clone(&filesystem));
    let tenant = scope("tenant-a", "user-a");
    let hour = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("test hour");
    repository
        .apply_batch(ScopedTelemetryBatch::new(
            tenant.clone(),
            TelemetryBatch::new(
                vec![],
                vec![],
                vec![
                    HourlyRunFailure::new(
                        TenantId::new("tenant-a").expect("tenant"),
                        hour,
                        UserId::new("user-a").expect("user"),
                        FailureCategory::new("provider_error").expect("category"),
                        1,
                        hour,
                        hour,
                    )
                    .expect("failure"),
                ],
                vec![],
                vec![],
                vec![],
            )
            .expect("batch"),
        ))
        .await
        .expect("write");

    let prefix =
        ScopedPath::new("/tenant-shared/telemetry/v0/hourly/failure").expect("failure prefix");
    let stored = filesystem
        .query(&tenant, &prefix, &Filter::All, Page::first(1))
        .await
        .expect("query stored failure")
        .into_iter()
        .next()
        .expect("stored failure");
    let mut body: serde_json::Value = serde_json::from_slice(&stored.entry.body).expect("json");
    body["tenant_id"] = serde_json::Value::String("tenant-b".to_owned());
    let entry = Entry {
        body: serde_json::to_vec(&body).expect("encoded json"),
        content_type: ContentType::json(),
        kind: stored.entry.kind.clone(),
        indexed: stored.entry.indexed.clone(),
    };
    root.put(&stored.path, entry, CasExpectation::Version(stored.version))
        .await
        .expect("corrupt tenant field");

    let error = repository
        .read_failure_page(
            &tenant,
            &request(
                hour - Duration::hours(1),
                hour + Duration::hours(1),
                hour + Duration::hours(1),
                100,
                None,
            ),
        )
        .await
        .expect_err("tenant mismatch must fail closed");
    assert!(
        matches!(
            &error,
            ironclaw_telemetry::TelemetryRepositoryError::ScopeMismatch
        ),
        "unexpected tenant-mismatch error: {error:?}"
    );
}

#[tokio::test]
async fn libsql_repository_satisfies_shared_contract_through_scoped_filesystem() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let database_path = directory.path().join("telemetry.db");
    let runtime = Arc::new(
        LibSqlRuntime::open(database_path.display().to_string(), None)
            .await
            .expect("libsql runtime"),
    );
    let root = Arc::new(LibSqlRootFilesystem::from_runtime(runtime));
    root.run_migrations().await.expect("libsql migrations");
    let filesystem = scoped_filesystem(Arc::clone(&root));
    assert_shared_repository_contract(filesystem).await;
}
