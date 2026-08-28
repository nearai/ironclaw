use super::*;

#[tokio::test(start_paused = true)]
async fn drains_never_overlap_and_coverage_counters_carry_forward() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config().with_queue_capacity(4),
        repository.clone(),
        clock,
    );
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    let (started, release) = repository.block_next_write();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    started.await.expect("write started");
    assert_eq!(recorder.try_record(model_call(2)), RecordOutcome::Accepted);
    assert_eq!(recorder.try_record(automation(3)), RecordOutcome::Accepted);
    let _ = release.send(());
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_batches(&repository, 2).await;
    assert_eq!(repository.max_active_writes(), 1);
    assert!(lifecycle.diagnostics().accepted_observation_count() >= 3);
    assert_eq!(
        repository.batches()[0].collector_coverage()[0].accepted_observation_count(),
        1
    );
    assert_eq!(
        repository.batches()[1].collector_coverage()[0].accepted_observation_count(),
        2
    );
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn shutdown_closes_intake_and_flushes_tail_within_budget() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    lifecycle.shutdown().await;
    assert_eq!(
        recorder.try_record(completed_run(1)),
        RecordOutcome::DroppedClosed
    );
    assert_eq!(repository.batches().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn shutdown_aborts_a_stalled_write_after_the_five_second_budget() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config().with_shutdown_timeout(Duration::from_secs(5)),
        repository.clone(),
        clock,
    );
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    let (started, _release) = repository.block_next_write();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    started.await.expect("write started");
    let shutdown = lifecycle.shutdown();
    tokio::pin!(shutdown);
    tokio::select! {
        biased;
        _ = &mut shutdown => panic!("stalled shutdown completed before its timeout"),
        _ = tokio::time::sleep(Duration::ZERO) => {}
    }
    let shutdown_started = tokio::time::Instant::now();
    tokio::time::advance(Duration::from_secs(5)).await;
    shutdown.await;
    assert!(
        tokio::time::Instant::now().duration_since(shutdown_started) <= Duration::from_secs(5),
        "shutdown elapsed {:?}",
        tokio::time::Instant::now().duration_since(shutdown_started)
    );
    assert_eq!(
        recorder.try_record(completed_run(1)),
        RecordOutcome::DroppedClosed
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_timeout_accounts_for_queued_and_in_flight_observations() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config()
            .with_queue_capacity(4)
            .with_shutdown_timeout(Duration::from_secs(5)),
        repository.clone(),
        clock,
    );
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    let (started, _release) = repository.block_next_write();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    started.await.expect("write started");
    assert_eq!(
        recorder.try_record(completed_run(2)),
        RecordOutcome::Accepted
    );
    let shutdown = lifecycle.shutdown();
    tokio::pin!(shutdown);
    tokio::select! {
        biased;
        _ = &mut shutdown => panic!("stalled shutdown completed before its timeout"),
        _ = tokio::time::sleep(Duration::ZERO) => {}
    }
    let shutdown_started = tokio::time::Instant::now();
    tokio::time::advance(Duration::from_secs(5)).await;
    let diagnostics = shutdown.await;
    assert!(
        tokio::time::Instant::now().duration_since(shutdown_started) <= Duration::from_secs(5),
        "shutdown elapsed {:?}",
        tokio::time::Instant::now().duration_since(shutdown_started)
    );
    assert_eq!(repository.batches().len(), 0);
    assert_eq!(diagnostics.shutdown_timeout_count(), 1);
    assert_eq!(diagnostics.shutdown_write_loss_count(), 2);
    assert_eq!(diagnostics.shutdown_abandoned_observation_count(), 2);
    assert_eq!(diagnostics.write_failed_observation_count(), 2);
    assert_eq!(
        diagnostics.last_failure_class(),
        Some(TelemetryWriteFailureClass::ShutdownTimeout)
    );
}

#[tokio::test(start_paused = true)]
async fn coverage_attribution_overflow_does_not_hide_global_shutdown_loss_count() {
    const OBSERVATIONS: usize = 8_193;
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config().with_queue_capacity(OBSERVATIONS + 1),
        repository.clone(),
        clock,
    );
    let (started, _release) = repository.block_next_write();
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
    assert_eq!(
        lifecycle.diagnostics().accepted_observation_count(),
        OBSERVATIONS as u64
    );
    assert!(lifecycle.diagnostics().coverage_key_overflow_count() > 0);
    let shutdown = lifecycle.shutdown();
    tokio::pin!(shutdown);
    tokio::select! {
        biased;
        _ = &mut shutdown => panic!("stalled shutdown completed before its timeout"),
        _ = tokio::time::sleep(Duration::ZERO) => {}
    }
    let shutdown_started = tokio::time::Instant::now();
    tokio::time::advance(Duration::from_secs(5)).await;
    let diagnostics = shutdown.await;
    assert!(
        tokio::time::Instant::now().duration_since(shutdown_started) <= Duration::from_secs(5),
        "shutdown elapsed {:?}",
        tokio::time::Instant::now().duration_since(shutdown_started)
    );
    assert_eq!(
        diagnostics.shutdown_abandoned_observation_count(),
        OBSERVATIONS as u64
    );
    assert_eq!(diagnostics.shutdown_write_loss_count(), OBSERVATIONS as u64);
    assert!(diagnostics.coverage_key_overflow_count() > 0);
}

#[tokio::test(start_paused = true)]
async fn invalid_timestamp_is_rejected_synchronously_and_covered() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    let future = Utc
        .with_ymd_and_hms(10_000, 1, 1, 0, 0, 0)
        .single()
        .expect("chrono supports this bounded test timestamp");
    let observation = TelemetryObservation::RunSettled(
        RunSettledObservation::new(
            ObservationContext::new(future),
            OriginKind::Human,
            RunOutcome::Completed,
            1,
            None,
            None,
        )
        .expect("valid typed observation"),
    );
    assert_eq!(
        recorder.try_record(observation),
        RecordOutcome::DroppedInvalid
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_batches(&repository, 1).await;
    let diagnostics = lifecycle.diagnostics();
    assert_eq!(diagnostics.invalid_drop_count(), 1); // safety: test-only assertion.
    assert_eq!(
        repository.batches()[0].collector_coverage()[0].invalid_drop_count(),
        1
    );
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn close_and_send_are_linearized_without_persisting_closed_observations() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    let recorder_for_thread = Arc::clone(&recorder);
    let sender = std::thread::spawn(move || recorder_for_thread.try_record(completed_run(2)));
    lifecycle.close_intake();
    let outcome = sender.join().expect("send thread");
    assert!(matches!(
        outcome,
        RecordOutcome::Accepted | RecordOutcome::DroppedClosed
    ));
    lifecycle.shutdown().await;
    let persisted_runs: u64 = repository
        .batches()
        .iter()
        .flat_map(|batch| batch.activity())
        .map(|row| row.run_count())
        .sum();
    assert_eq!(
        persisted_runs,
        u64::from(outcome == RecordOutcome::Accepted)
    );
}

#[tokio::test(start_paused = true)]
async fn a_new_recorder_after_shutdown_has_an_independent_worker() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (old_recorder, old_lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock.clone());
    drop(old_recorder);
    drop(old_lifecycle);

    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    assert_eq!(recorder.try_record(model_call(0)), RecordOutcome::Accepted);
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_batches(&repository, 1).await;
    assert_eq!(
        repository.batches()[0].model_usage()[0].inference_count(),
        1
    );
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn default_collector_ids_are_unique_per_recorder_instance() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));

    let (first_recorder, first_lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock.clone());
    first_lifecycle.close_intake();
    assert_eq!(
        first_recorder.try_record(completed_run(3599)),
        RecordOutcome::DroppedClosed
    );
    first_lifecycle.shutdown().await;

    let (second_recorder, second_lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    second_lifecycle.close_intake();
    assert_eq!(
        second_recorder.try_record(completed_run(3599)),
        RecordOutcome::DroppedClosed
    );
    second_lifecycle.shutdown().await;

    let batches = repository.batches();
    assert_eq!(batches.len(), 2);
    let first_id = batches[0].collector_coverage()[0]
        .collector_instance_id()
        .as_str();
    let second_id = batches[1].collector_coverage()[0]
        .collector_instance_id()
        .as_str();
    assert_ne!(first_id, second_id);
    assert!(first_id.len() <= 128);
    assert!(second_id.len() <= 128);
}
