use super::*;

#[tokio::test(start_paused = true)]
async fn try_record_is_synchronous_and_queue_pressure_is_typed() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        BufferedRecorderConfig::default().with_queue_capacity(1),
        repository.clone(),
        clock,
    );
    let first = recorder.try_record(completed_run(0));
    assert_eq!(first, RecordOutcome::Accepted);
    let second = recorder.try_record(completed_run(1));
    assert_eq!(second, RecordOutcome::DroppedQueueFull);
    assert_eq!(lifecycle.diagnostics().queue_full_drop_count(), 1);
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn queue_full_drop_is_written_to_tenant_hour_coverage() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config().with_queue_capacity(1),
        repository.clone(),
        clock,
    );
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    assert_eq!(
        recorder.try_record(completed_run(1)),
        RecordOutcome::DroppedQueueFull
    );
    lifecycle.shutdown().await;
    let batches = repository.batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(
        batches[0].collector_coverage()[0].queue_full_drop_count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn coverage_only_commit_then_error_retains_a_loss_marker() {
    let repository = Arc::new(FakeRepository::default());
    repository.fail_next_after_commit();
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);

    lifecycle.close_intake();
    assert_eq!(
        recorder.try_record(completed_run(3599)),
        RecordOutcome::DroppedClosed
    );
    for _ in 0..100 {
        if lifecycle.diagnostics().repository_failure_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let attempted = repository.batches();
    assert_eq!(attempted.len(), 1);
    assert_eq!(attempted[0].collector_coverage()[0].closed_drop_count(), 1);
    assert_eq!(
        attempted[0].collector_coverage()[0].write_failed_observation_count(),
        0
    );

    lifecycle.shutdown().await;
    let batches = repository.batches();
    assert_eq!(batches.len(), 2);
    let marker = &batches[1].collector_coverage()[0];
    assert_eq!(marker.accepted_observation_count(), 0);
    assert_eq!(marker.queue_full_drop_count(), 0);
    assert_eq!(marker.closed_drop_count(), 0);
    assert_eq!(marker.invalid_drop_count(), 0);
    assert_eq!(marker.write_failed_observation_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn repeated_marker_failure_retains_a_fresh_marker_for_retry() {
    let repository = Arc::new(FakeRepository::default());
    repository.fail_next_after_commit();
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);

    lifecycle.close_intake();
    assert_eq!(
        recorder.try_record(completed_run(3599)),
        RecordOutcome::DroppedClosed
    );
    for _ in 0..100 {
        if lifecycle.diagnostics().repository_failure_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }

    repository.fail_next_after_commit();
    lifecycle.close_intake();
    for _ in 0..100 {
        if lifecycle.diagnostics().repository_failure_count() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let failed_markers = repository.batches();
    assert_eq!(failed_markers.len(), 2);
    let first_attempt = &failed_markers[0].collector_coverage()[0];
    assert_eq!(first_attempt.accepted_observation_count(), 0);
    assert_eq!(first_attempt.queue_full_drop_count(), 0);
    assert_eq!(first_attempt.closed_drop_count(), 1);
    assert_eq!(first_attempt.invalid_drop_count(), 0);
    assert_eq!(first_attempt.write_failed_observation_count(), 0);
    let second_attempt = &failed_markers[1].collector_coverage()[0];
    assert_eq!(second_attempt.accepted_observation_count(), 0);
    assert_eq!(second_attempt.queue_full_drop_count(), 0);
    assert_eq!(second_attempt.closed_drop_count(), 0);
    assert_eq!(second_attempt.invalid_drop_count(), 0);
    assert_eq!(second_attempt.write_failed_observation_count(), 0);

    lifecycle.shutdown().await;
    assert_eq!(repository.batches().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn closed_drop_is_written_to_tenant_hour_coverage() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    lifecycle.close_intake();
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::DroppedClosed
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    wait_for_batches(&repository, 1).await;
    assert_eq!(
        repository.batches()[0].collector_coverage()[0].closed_drop_count(),
        1
    );
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn five_hundred_twelve_items_trigger_one_aggregate_drain() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config().with_queue_capacity(600),
        repository.clone(),
        clock,
    );
    for offset in 0..512 {
        assert_eq!(
            recorder.try_record(completed_run(offset)),
            RecordOutcome::Accepted
        );
    }
    wait_for_batches(&repository, 1).await;
    assert_eq!(repository.batches()[0].activity()[0].run_count(), 512);
    assert_eq!(lifecycle.diagnostics().flushed_batch_count(), 1);
    lifecycle.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn one_second_of_paused_time_triggers_a_nonempty_drain() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) =
        BufferedTelemetryRecorder::spawn_with_sink(config(), repository.clone(), clock);
    assert_eq!(recorder.try_record(model_call(0)), RecordOutcome::Accepted);
    assert!(repository.batches().is_empty());
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
async fn continuous_queue_drop_notifications_do_not_starve_the_batch_deadline() {
    let repository = Arc::new(FakeRepository::default());
    let clock = Arc::new(FixedClock::new(timestamp(0)));
    let (recorder, lifecycle) = BufferedTelemetryRecorder::spawn_with_sink(
        config().with_queue_capacity(1),
        repository.clone(),
        clock,
    );
    assert_eq!(
        recorder.try_record(completed_run(0)),
        RecordOutcome::Accepted
    );
    let started = Arc::new(std::sync::Barrier::new(2));
    let stop = Arc::new(AtomicBool::new(false));
    let flood_recorder = Arc::clone(&recorder);
    let flood_started = Arc::clone(&started);
    let flood_stop = Arc::clone(&stop);
    let flood = std::thread::spawn(move || {
        flood_started.wait();
        while !flood_stop.load(Ordering::Acquire) {
            let _ = flood_recorder.try_record(completed_run(1));
            std::thread::yield_now();
        }
    });
    started.wait();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let flushed_at_deadline = !repository.batches().is_empty();
    stop.store(true, Ordering::Release);
    flood.join().expect("flood thread");
    assert!(flushed_at_deadline);
    lifecycle.shutdown().await;
}
