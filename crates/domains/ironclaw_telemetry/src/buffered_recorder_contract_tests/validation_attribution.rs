use super::*;

#[tokio::test(start_paused = true)]
async fn direct_oversized_queue_configuration_is_clamped_at_spawn() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let direct_config = BufferedRecorderConfig {
        queue_capacity: 8_193,
        ..BufferedRecorderConfig::default()
    };
    assert_eq!(direct_config.effective_queue_capacity(), 8_192);
    let zero_config = BufferedRecorderConfig {
        queue_capacity: 0,
        ..BufferedRecorderConfig::default()
    };
    assert_eq!(zero_config.effective_queue_capacity(), 1);
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(direct_config, repository, clock);

    for offset in 0..8_192 {
        assert_eq!(
            recorder.try_record(completed_run(offset)),
            RecordOutcome::Accepted
        );
    }
    assert_eq!(
        recorder.try_record(completed_run(8_192)),
        RecordOutcome::DroppedQueueFull
    );
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn drop_only_coverage_uses_the_drop_timestamp_for_its_span() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    lifecycle.close_intake();
    let dropped_at = timestamp(3_599);
    assert_eq!(
        recorder.try_record(completed_run(3_599)),
        RecordOutcome::DroppedClosed
    );
    lifecycle.shutdown().await;
    let batches = repository.batches();
    let coverage = &batches[0].collector_coverage()[0];
    assert_eq!(coverage.first_observed_at(), dropped_at);
    assert_eq!(coverage.last_observed_at(), dropped_at);
}

#[tokio::test(start_paused = true)]
async fn invalid_aggregate_is_counted_without_a_repository_write() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    let huge = RunSettledObservation::new(
        context(0),
        OriginKind::Human,
        RunOutcome::Completed,
        i64::MAX as u64,
        None,
        None,
    )
    .expect("maximum duration");
    assert_eq!(
        recorder.try_record(TelemetryObservation::RunSettled(huge.clone())),
        RecordOutcome::Accepted
    );
    assert_eq!(
        recorder.try_record(TelemetryObservation::RunSettled(huge)),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..100 {
        if lifecycle.diagnostics().invalid_drop_count() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(lifecycle.diagnostics().invalid_drop_count(), 2); // safety: test-only assertion.
    assert_eq!(
        lifecycle.diagnostics().last_failure_class(),
        Some(TelemetryWriteFailureClass::CounterOverflow)
    );
    assert_eq!(
        lifecycle
            .diagnostics()
            .failure_class_count(TelemetryWriteFailureClass::CounterOverflow),
        1
    );
    assert!(repository.batches().is_empty());
    lifecycle.shutdown().await;
    let batches = repository.batches();
    let coverage = batches[0].collector_coverage();
    assert_eq!(coverage[0].accepted_observation_count(), 2);
    assert_eq!(coverage[0].invalid_drop_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn repository_record_failures_preserve_typed_diagnostics() {
    let repository = Arc::new(FakeRepository::default());
    repository.fail_next_with(TelemetryRepositoryError::Record(
        RecordError::InvalidWindowStart,
    ));
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..100 {
        if lifecycle.diagnostics().repository_failure_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let diagnostics = lifecycle.diagnostics();
    assert_eq!(
        diagnostics.last_failure_class(),
        Some(TelemetryWriteFailureClass::InvalidRecord)
    );
    assert_eq!(
        diagnostics.failure_class_count(TelemetryWriteFailureClass::InvalidRecord),
        1
    );
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn system_scope_is_rejected_without_entering_a_global_bucket() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);

    assert_eq!(
        recorder.try_record_scoped(ResourceScope::system(), completed_run(0)),
        RecordOutcome::DroppedInvalid
    );
    assert_eq!(lifecycle.diagnostics().invalid_drop_count(), 1); // safety: test-only assertion.
    lifecycle.shutdown().await;
    assert!(repository.batches().is_empty());
}

#[tokio::test(start_paused = true)]
async fn queued_scope_is_the_only_usage_attribution_source() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    let mut trusted_scope = scope();
    trusted_scope.tenant_id = TenantId::new("tenant-b").expect("tenant");
    trusted_scope.user_id = UserId::new("user-b").expect("user");

    assert_eq!(
        recorder.try_record_scoped(trusted_scope, completed_run(0)),
        RecordOutcome::Accepted
    );
    lifecycle.shutdown().await;

    let batches = repository.batches();
    let activity = &batches[0].activity()[0];
    assert_eq!(activity.tenant_id().as_str(), "tenant-b");
    assert_eq!(activity.user_id().as_str(), "user-b");
}

#[tokio::test(start_paused = true)]
async fn lifecycle_subject_user_can_differ_from_scope_user() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    let observation = TelemetryObservation::LifecycleTransition(
        LifecycleTransitionObservation::new(
            Some(UserId::new("subject-user").expect("subject user")),
            LifecycleEventId::new("event-a").expect("event"),
            LifecycleEventKind::MemberAdded,
            LifecycleSubjectKind::User,
            "subject-user",
            timestamp(0),
        )
        .expect("lifecycle observation"),
    );

    assert_eq!(
        recorder.try_record_scoped(scope(), observation),
        RecordOutcome::Accepted
    );
    lifecycle.shutdown().await;

    let batches = repository.batches();
    let event = &batches[0].lifecycle_events()[0];
    assert_eq!(event.tenant_id().as_str(), "tenant-a");
    assert_eq!(event.user_id().map(UserId::as_str), Some("subject-user"));
}

#[tokio::test(start_paused = true)]
async fn malformed_lifecycle_observation_does_not_poison_valid_usage() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    let malformed = TelemetryObservation::LifecycleTransition(
        LifecycleTransitionObservation::new(
            None,
            LifecycleEventId::new("event-without-owner").expect("event"),
            LifecycleEventKind::RoutineCreated,
            LifecycleSubjectKind::Routine,
            "routine-a",
            timestamp(0),
        )
        .expect("structurally valid lifecycle observation"),
    );

    assert_eq!(
        recorder.try_record_scoped(scope(), malformed),
        RecordOutcome::DroppedInvalid
    );
    assert_eq!(
        recorder.try_record_scoped(scope(), completed_run(0)),
        RecordOutcome::Accepted
    );
    lifecycle.shutdown().await;

    let batches = repository.batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].activity().len(), 1);
    assert!(batches[0].lifecycle_events().is_empty());
}
