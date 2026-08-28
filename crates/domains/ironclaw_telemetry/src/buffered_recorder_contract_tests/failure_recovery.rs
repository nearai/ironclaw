use super::*;

#[tokio::test(start_paused = true)]
async fn repository_failure_drops_only_that_drain_and_later_drain_continues() {
    let repository = Arc::new(FakeRepository::default());
    repository.fail_next();
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
    assert_eq!(lifecycle.diagnostics().repository_failure_count(), 1);
    assert_eq!(
        lifecycle.diagnostics().last_failure_class(),
        Some(TelemetryWriteFailureClass::StorageOperation)
    );
    assert_eq!(recorder.try_record(model_call(2)), RecordOutcome::Accepted);
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_batches(&repository, 1).await;
    assert_eq!(
        repository.batches()[0].model_usage()[0].inference_count(),
        1
    );
    assert_eq!(lifecycle.diagnostics().write_failed_observation_count(), 1);
    let batches = repository.batches();
    let coverage = batches[0].collector_coverage();
    assert_eq!(coverage.len(), 1);
    assert_eq!(coverage[0].accepted_observation_count(), 1);
    assert_eq!(coverage[0].write_failed_observation_count(), 1);
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn write_failure_coverage_counts_each_observation_in_a_tenant_hour() {
    let repository = Arc::new(FakeRepository::default());
    repository.fail_next();
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);

    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    assert_eq!(
        recorder.try_record(completed_run(1)),
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

    assert_eq!(
        recorder.try_record(completed_run(2)),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..100 {
        if lifecycle.diagnostics().flushed_batch_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(lifecycle.diagnostics().flushed_batch_count(), 1);

    let batches = repository.batches();
    let coverage = &batches[0].collector_coverage()[0];
    assert_eq!(coverage.accepted_observation_count(), 1);
    assert_eq!(coverage.write_failed_observation_count(), 2);
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn partial_report_is_not_counted_as_a_successful_flush() {
    let repository = Arc::new(FakeRepository::default());
    repository.return_next_report(BatchApplyReport::from_counts(0, 1));
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);

    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    assert_eq!(
        recorder.try_record(completed_run(1)),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..100 {
        if lifecycle.diagnostics().partial_batch_failure_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(lifecycle.diagnostics().partial_batch_failure_count(), 1);
    assert_eq!(lifecycle.diagnostics().flushed_batch_count(), 0);

    assert_eq!(
        recorder.try_record(completed_run(2)),
        RecordOutcome::Accepted
    );
    let diagnostics = lifecycle.shutdown().await;
    assert_eq!(diagnostics.flushed_batch_count(), 1);
    let batches = repository.batches();
    assert_eq!(batches.len(), 2);
    assert_eq!(
        batches[1].collector_coverage()[0].write_failed_observation_count(),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn tenant_fan_out_continues_after_failure_and_preserves_queued_scopes() {
    let repository = Arc::new(FakeRepository::default());
    repository.fail_on_write(2);
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    let (tenant_a_scope, tenant_a_observation) = completed_run_for_tenant_hour(0);
    let (tenant_b_scope, tenant_b_observation) = completed_run_for_tenant_hour(1);
    let (tenant_c_scope, tenant_c_observation) = completed_run_for_tenant_hour(2);

    assert_eq!(
        recorder.try_record_scoped(tenant_a_scope.clone(), tenant_a_observation),
        RecordOutcome::Accepted
    );
    assert_eq!(
        recorder.try_record_scoped(tenant_b_scope.clone(), tenant_b_observation),
        RecordOutcome::Accepted
    );
    assert_eq!(
        recorder.try_record_scoped(tenant_c_scope.clone(), tenant_c_observation),
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
    assert_eq!(lifecycle.diagnostics().repository_failure_count(), 1);
    assert_eq!(repository.scopes().len(), 2);
    assert_eq!(repository.scopes()[0], tenant_a_scope);
    assert_eq!(repository.scopes()[1], tenant_c_scope);
    assert_eq!(
        repository.batches()[0].activity()[0].tenant_id().as_str(),
        "tenant-0"
    );
    assert_eq!(
        repository.batches()[1].activity()[0].tenant_id().as_str(),
        "tenant-2"
    );

    assert_eq!(
        recorder.try_record_scoped(tenant_b_scope.clone(), completed_run(2)),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    lifecycle.close_intake();
    let diagnostics = lifecycle.shutdown().await;
    assert_eq!(
        diagnostics.flushed_batch_count(),
        1,
        "scopes={:?}",
        repository.scopes()
    );
    assert_eq!(
        repository.scopes().len(),
        3,
        "batches={:?}",
        repository.batches()
    );
    assert_eq!(repository.scopes()[2], tenant_b_scope);
    assert!(
        repository.batches()[2]
            .activity()
            .iter()
            .all(|row| row.tenant_id().as_str() == "tenant-1")
    );
    assert_eq!(
        repository.batches()[2]
            .collector_coverage()
            .iter()
            .map(|row| row.write_failed_observation_count())
            .sum::<u64>(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn commit_then_error_does_not_replay_attempted_coverage() {
    let repository = Arc::new(FakeRepository::default());
    repository.fail_next_after_commit();
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
    assert_eq!(repository.batches().len(), 1);
    let attempted_batches = repository.batches();
    assert_eq!(
        attempted_batches[0].collector_coverage()[0].accepted_observation_count(),
        1
    );
    assert_eq!(
        attempted_batches[0].collector_coverage()[0].write_failed_observation_count(),
        0
    );

    assert_eq!(
        recorder.try_record(completed_run(1)),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_batches(&repository, 2).await;

    let batches = repository.batches();
    let retry_coverage = batches[1].collector_coverage();
    assert_eq!(retry_coverage.len(), 1);
    assert_eq!(retry_coverage[0].accepted_observation_count(), 1);
    assert_eq!(retry_coverage[0].queue_full_drop_count(), 0);
    assert_eq!(retry_coverage[0].closed_drop_count(), 0);
    assert_eq!(retry_coverage[0].invalid_drop_count(), 0);
    assert_eq!(retry_coverage[0].write_failed_observation_count(), 1);
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn pending_coverage_is_bounded_during_an_outage_and_later_drains_continue() {
    const OBSERVATIONS: usize = 8_193;
    let repository = Arc::new(FakeRepository::default());
    repository.set_fail_all(true);
    let (started, release) = repository.block_next_write();
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config().with_queue_capacity(OBSERVATIONS),
        repository.clone(),
        clock,
    );
    let (tenant_scope, tenant_observation) = completed_run_for_tenant_hour(0);
    assert_eq!(
        recorder.try_record_scoped(tenant_scope, tenant_observation),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    started.await.expect("write started");
    for index in 1..OBSERVATIONS {
        let (tenant_scope, tenant_observation) = completed_run_for_tenant_hour(index as u64);
        assert_eq!(
            recorder.try_record_scoped(tenant_scope, tenant_observation),
            RecordOutcome::Accepted
        );
    }
    let _ = release.send(());
    for _ in 0..100 {
        if lifecycle.diagnostics().repository_failure_count() >= 17 {
            break;
        }
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
    }
    let diagnostics = lifecycle.diagnostics();
    assert_eq!(
        diagnostics.accepted_observation_count(),
        OBSERVATIONS as u64
    );
    assert!(
        diagnostics.repository_failure_count() >= 17,
        "diagnostics={diagnostics:?}"
    );
    assert!(diagnostics.coverage_key_overflow_count() > 0);

    repository.set_fail_all(false);
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..100 {
        if repository
            .batches()
            .iter()
            .any(|batch| !batch.activity().is_empty())
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        repository
            .batches()
            .iter()
            .any(|batch| !batch.activity().is_empty())
    );
    lifecycle.shutdown().await;
}
