use chrono::{DateTime, TimeZone, Utc};
use ironclaw_host_api::{
    ids::{InvocationId, TenantId, UserId},
    resource::ResourceScope,
    turn::SanitizedFailure,
};
use ironclaw_telemetry::records::{
    CollectorCoverage, HourlyAutomationUsage, HourlyModelUsage, HourlyRunFailure,
    HourlyUserActivity, LifecycleEvent, RecordError, TelemetryBatch, TelemetryBatchRowFamily,
};
use ironclaw_telemetry::{AggregationError, aggregate_batch, floor_utc_hour};
use ironclaw_telemetry_contracts::observation::{
    AutomationId, AutomationKind, AutomationSettledObservation, EffectiveModelId, LifecycleEventId,
    LifecycleEventKind, LifecycleSubjectKind, LifecycleTransitionObservation,
    ModelCallCompletedObservation, ObservationContext, OriginKind, ProviderId, RunOutcome,
    RunSettledObservation, ScopedTelemetryObservation, TelemetryObservation,
};

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("valid tenant")
}

fn user() -> UserId {
    UserId::new("user-a").expect("valid user")
}

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("valid timestamp")
}

fn context(timestamp: DateTime<Utc>) -> ObservationContext {
    ObservationContext::new(timestamp)
}

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: tenant(),
        user_id: user(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn scoped(observation: TelemetryObservation) -> ScopedTelemetryObservation {
    ScopedTelemetryObservation::new(scope(), observation)
}

fn window() -> DateTime<Utc> {
    at(2026, 8, 26, 10, 0, 0)
}

fn activity_row(user_id: &str, origin: OriginKind) -> HourlyUserActivity {
    HourlyUserActivity::new(
        tenant(),
        window(),
        UserId::new(user_id).expect("user"),
        origin,
        1,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
        window(),
        window(),
    )
    .expect("activity row")
}

fn model_row(model_id: &str) -> HourlyModelUsage {
    HourlyModelUsage::new(
        tenant(),
        user(),
        window(),
        ProviderId::new("provider-a").expect("provider"),
        EffectiveModelId::new(model_id).expect("model"),
        1,
        1,
        1,
        0,
        0,
        0,
        window(),
        window(),
    )
    .expect("model row")
}

fn failure_row(category: &str) -> HourlyRunFailure {
    HourlyRunFailure::new(
        tenant(),
        window(),
        user(),
        ironclaw_telemetry_contracts::observation::FailureCategory::new(category)
            .expect("failure category"),
        1,
        window(),
        window(),
    )
    .expect("failure row")
}

fn automation_row(user_id: &str, kind: AutomationKind) -> HourlyAutomationUsage {
    HourlyAutomationUsage::new(
        tenant(),
        window(),
        UserId::new(user_id).expect("user"),
        kind,
        1,
        1,
        0,
        0,
        0,
        window(),
        window(),
    )
    .expect("automation row")
}

fn lifecycle_row(event_id: &str, subject_id: &str) -> LifecycleEvent {
    LifecycleEvent::new(
        tenant(),
        LifecycleEventId::new(event_id).expect("event"),
        Some(user()),
        LifecycleEventKind::RoutineCreated,
        LifecycleSubjectKind::Routine,
        ironclaw_telemetry_contracts::observation::SubjectId::new(subject_id).expect("subject"),
        window(),
    )
    .expect("lifecycle row")
}

fn coverage_row(collector_id: &str) -> CollectorCoverage {
    CollectorCoverage::new(
        tenant(),
        window(),
        ironclaw_telemetry_contracts::observation::CollectorInstanceId::new(collector_id)
            .expect("collector"),
        1,
        0,
        0,
        0,
        0,
        window(),
        window(),
    )
    .expect("coverage row")
}

fn completed_run(timestamp: DateTime<Utc>, duration_ms: u64) -> ScopedTelemetryObservation {
    scoped(TelemetryObservation::RunSettled(
        RunSettledObservation::new(
            context(timestamp),
            OriginKind::Human,
            RunOutcome::Completed,
            duration_ms,
            Some(0),
            None,
        )
        .expect("valid run observation"),
    ))
}

#[test]
fn utc_floor_is_exact_at_hour_boundaries() {
    let timestamp = at(2026, 8, 26, 10, 23, 45);

    assert_eq!(floor_utc_hour(timestamp), at(2026, 8, 26, 10, 0, 0));
    assert_eq!(
        floor_utc_hour(at(2026, 8, 26, 10, 0, 0)),
        at(2026, 8, 26, 10, 0, 0)
    );
}

#[test]
fn utc_floor_does_not_reinterpret_dst_transitions() {
    let spring_transition = at(2026, 3, 8, 10, 30, 0);
    let fall_transition = at(2026, 11, 1, 8, 30, 0);

    assert_eq!(floor_utc_hour(spring_transition), at(2026, 3, 8, 10, 0, 0));
    assert_eq!(floor_utc_hour(fall_transition), at(2026, 11, 1, 8, 0, 0));
}

#[test]
fn aggregate_is_order_independent_and_reconciles_terminal_counts() {
    let failure = SanitizedFailure::new("model_unavailable").expect("sanitized category");
    let failed = scoped(TelemetryObservation::RunSettled(
        RunSettledObservation::new(
            context(at(2026, 8, 26, 10, 24, 0)),
            OriginKind::Human,
            RunOutcome::Failed,
            20,
            Some(3),
            Some(failure),
        )
        .expect("valid failed run"),
    ));
    let model = scoped(TelemetryObservation::ModelCallCompleted(
        ModelCallCompletedObservation::new(
            context(at(2026, 8, 26, 10, 25, 0)),
            ProviderId::new("provider-a").expect("provider"),
            EffectiveModelId::new("model-a").expect("model"),
            None,
        )
        .expect("valid model call"),
    ));
    let automation = scoped(TelemetryObservation::AutomationSettled(
        AutomationSettledObservation::new(
            context(at(2026, 8, 26, 10, 26, 0)),
            AutomationId::new("automation-a").expect("automation"),
            AutomationKind::Cron,
            RunOutcome::Cancelled,
        )
        .expect("valid automation"),
    ));
    let lifecycle = scoped(TelemetryObservation::LifecycleTransition(
        LifecycleTransitionObservation::new(
            Some(user()),
            LifecycleEventId::new("event-a").expect("event"),
            LifecycleEventKind::RoutineCreated,
            LifecycleSubjectKind::Routine,
            "routine-a".to_owned(),
            at(2026, 8, 26, 10, 27, 0),
        )
        .expect("valid lifecycle"),
    ));
    let duplicate_lifecycle = lifecycle.clone();
    let conflicting_lifecycle = scoped(TelemetryObservation::LifecycleTransition(
        LifecycleTransitionObservation::new(
            Some(user()),
            LifecycleEventId::new("event-a").expect("event"),
            LifecycleEventKind::RoutineDeleted,
            LifecycleSubjectKind::Routine,
            "routine-a".to_owned(),
            at(2026, 8, 26, 10, 27, 1),
        )
        .expect("valid conflicting lifecycle"),
    ));

    let ordered = vec![
        completed_run(at(2026, 8, 26, 10, 23, 0), 10),
        failed.clone(),
        model.clone(),
        automation.clone(),
        lifecycle.clone(),
        duplicate_lifecycle.clone(),
        conflicting_lifecycle.clone(),
    ];
    let reversed = vec![
        duplicate_lifecycle,
        automation,
        model,
        lifecycle,
        failed,
        completed_run(at(2026, 8, 26, 10, 23, 0), 10),
        conflicting_lifecycle,
    ];

    let first = aggregate_batch(&ordered).expect("ordered aggregate");
    let second = aggregate_batch(&reversed).expect("reversed aggregate");

    assert_eq!(first, second);
    assert_eq!(first.activity().len(), 1);
    let activity = &first.activity()[0];
    assert_eq!(activity.run_count(), 2);
    assert_eq!(activity.completed_count(), 1);
    assert_eq!(activity.failed_count(), 1);
    assert_eq!(activity.cancelled_count(), 0);
    assert_eq!(activity.recovery_required_count(), 0);
    assert_eq!(
        activity.completed_count()
            + activity.failed_count()
            + activity.cancelled_count()
            + activity.recovery_required_count(),
        activity.run_count()
    );
    assert_eq!(first.model_usage()[0].inference_count(), 1);
    assert_eq!(first.model_usage()[0].usage_reported_count(), 0);
    assert_eq!(first.lifecycle_events().len(), 1);
    assert_eq!(
        first.lifecycle_events()[0].event_kind(),
        LifecycleEventKind::RoutineCreated
    );
}

#[test]
fn aggregate_accumulates_failure_categories_across_origins() {
    let failure = SanitizedFailure::new("model_unavailable").expect("sanitized category");
    let human = scoped(TelemetryObservation::RunSettled(
        RunSettledObservation::new(
            context(at(2026, 8, 26, 10, 24, 0)),
            OriginKind::Human,
            RunOutcome::Failed,
            20,
            Some(0),
            Some(failure.clone()),
        )
        .expect("valid human failure"),
    ));
    let automation = scoped(TelemetryObservation::RunSettled(
        RunSettledObservation::new(
            context(at(2026, 8, 26, 10, 25, 0)),
            OriginKind::Automation,
            RunOutcome::Failed,
            30,
            Some(0),
            Some(failure),
        )
        .expect("valid automation failure"),
    ));

    let batch = aggregate_batch([human, automation]).expect("aggregate failures");

    assert_eq!(batch.run_failures().len(), 1);
    assert_eq!(batch.run_failures()[0].failure_count(), 2);
}

#[test]
fn aggregate_rejects_sum_above_signed_bigint_maximum() {
    let observations = [
        completed_run(at(2026, 8, 26, 10, 0, 0), i64::MAX as u64),
        completed_run(at(2026, 8, 26, 10, 1, 0), 1),
    ];

    assert!(matches!(
        aggregate_batch(&observations),
        Err(AggregationError::CounterOutOfRange { .. })
    ));
}

#[test]
fn aggregate_accepts_signed_bigint_maximum_on_first_observation() {
    let observation = completed_run(at(2026, 8, 26, 10, 0, 0), i64::MAX as u64);

    let batch = aggregate_batch([observation]).expect("signed BIGINT maximum is valid");

    assert_eq!(batch.activity()[0].total_run_latency_ms(), i64::MAX as u64);
}

#[test]
fn model_usage_sum_above_signed_bigint_maximum_is_rejected() {
    let make_model = |timestamp, input_tokens| {
        scoped(TelemetryObservation::ModelCallCompleted(
            ModelCallCompletedObservation::new(
                context(timestamp),
                ProviderId::new("provider-a").expect("provider"),
                EffectiveModelId::new("model-a").expect("model"),
                Some(ironclaw_telemetry_contracts::observation::ModelUsage::new(
                    input_tokens,
                    0,
                    0,
                    0,
                )),
            )
            .expect("valid model observation"),
        ))
    };
    let observations = [
        make_model(at(2026, 8, 26, 10, 0, 0), i64::MAX as u64),
        make_model(at(2026, 8, 26, 10, 1, 0), 1),
    ];

    assert!(matches!(
        aggregate_batch(&observations),
        Err(AggregationError::CounterOutOfRange { .. })
    ));
}

#[test]
fn hourly_activity_constructor_rejects_unreconciled_terminal_counts() {
    let timestamp = at(2026, 8, 26, 10, 0, 0);
    let result = HourlyUserActivity::new(
        tenant(),
        timestamp,
        user(),
        OriginKind::Human,
        2,
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        10,
        timestamp,
        timestamp,
    );

    assert!(matches!(result, Err(RecordError::TerminalCountMismatch)));
}

#[test]
fn lifecycle_event_constructor_rejects_non_tenant_without_user() {
    let timestamp = at(2026, 8, 26, 10, 0, 0);
    let result = LifecycleEvent::new(
        tenant(),
        LifecycleEventId::new("event-user").expect("event"),
        None,
        LifecycleEventKind::RoutineCreated,
        LifecycleSubjectKind::Routine,
        ironclaw_telemetry_contracts::observation::SubjectId::new("routine-a").expect("subject"),
        timestamp,
    );

    assert!(matches!(result, Err(RecordError::MissingUserAttribution)));
}

#[test]
fn telemetry_batch_constructor_rejects_duplicate_lifecycle_keys() {
    let timestamp = at(2026, 8, 26, 10, 0, 0);
    let event = LifecycleEvent::new(
        tenant(),
        LifecycleEventId::new("event-a").expect("event"),
        Some(user()),
        LifecycleEventKind::RoutineCreated,
        LifecycleSubjectKind::Routine,
        ironclaw_telemetry_contracts::observation::SubjectId::new("routine-a").expect("subject"),
        timestamp,
    )
    .expect("valid lifecycle event");

    let result = TelemetryBatch::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![event.clone(), event],
        Vec::new(),
    );

    assert!(matches!(
        result,
        Err(RecordError::DuplicateRow {
            family: TelemetryBatchRowFamily::LifecycleEvent
        })
    ));
}

#[test]
fn telemetry_batch_constructor_rejects_duplicate_activity_keys() {
    let result = TelemetryBatch::new(
        vec![
            activity_row("user-a", OriginKind::Human),
            activity_row("user-a", OriginKind::Human),
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert!(matches!(
        result,
        Err(RecordError::DuplicateRow {
            family: TelemetryBatchRowFamily::Activity
        })
    ));
}

#[test]
fn telemetry_batch_constructor_rejects_duplicate_model_keys() {
    let result = TelemetryBatch::new(
        Vec::new(),
        vec![model_row("model-a"), model_row("model-a")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert!(matches!(
        result,
        Err(RecordError::DuplicateRow {
            family: TelemetryBatchRowFamily::ModelUsage
        })
    ));
}

#[test]
fn telemetry_batch_constructor_rejects_duplicate_failure_keys() {
    let result = TelemetryBatch::new(
        Vec::new(),
        Vec::new(),
        vec![
            failure_row("model_unavailable"),
            failure_row("model_unavailable"),
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert!(matches!(
        result,
        Err(RecordError::DuplicateRow {
            family: TelemetryBatchRowFamily::RunFailure
        })
    ));
}

#[test]
fn telemetry_batch_constructor_rejects_duplicate_automation_keys() {
    let result = TelemetryBatch::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![
            automation_row("user-a", AutomationKind::Cron),
            automation_row("user-a", AutomationKind::Cron),
        ],
        Vec::new(),
        Vec::new(),
    );

    assert!(matches!(
        result,
        Err(RecordError::DuplicateRow {
            family: TelemetryBatchRowFamily::AutomationUsage
        })
    ));
}

#[test]
fn telemetry_batch_constructor_rejects_duplicate_coverage_keys() {
    let result = TelemetryBatch::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![coverage_row("collector-a"), coverage_row("collector-a")],
    );

    assert!(matches!(
        result,
        Err(RecordError::DuplicateRow {
            family: TelemetryBatchRowFamily::CollectorCoverage
        })
    ));
}

#[test]
fn telemetry_batch_constructor_accepts_distinct_rows_in_each_family() {
    let result = TelemetryBatch::new(
        vec![
            activity_row("user-a", OriginKind::Human),
            activity_row("user-b", OriginKind::Human),
        ],
        vec![model_row("model-a"), model_row("model-b")],
        vec![failure_row("model_unavailable"), failure_row("cancelled")],
        vec![
            automation_row("user-a", AutomationKind::Cron),
            automation_row("user-a", AutomationKind::Once),
        ],
        vec![
            lifecycle_row("event-a", "routine-a"),
            lifecycle_row("event-b", "routine-b"),
        ],
        vec![coverage_row("collector-a"), coverage_row("collector-b")],
    );

    let batch = result.expect("all distinct durable row keys are valid");
    assert_eq!(batch.activity().len(), 2);
    assert_eq!(batch.model_usage().len(), 2);
    assert_eq!(batch.run_failures().len(), 2);
    assert_eq!(batch.automation_usage().len(), 2);
    assert_eq!(batch.lifecycle_events().len(), 2);
    assert_eq!(batch.collector_coverage().len(), 2);
}
