use super::*;

#[tokio::test]
async fn no_exposure_validator_rejects_sentinel_payloads() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(
            replay_with_error_kind(&scope, 11, 11, "RAW_PROMPT_SENTINEL"),
        ))
        .expect("publish sentinel payload");

    match subscription.next().await.expect("redaction marker") {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::RedactionBlocked);
        }
        other => panic!("expected redaction lag marker, got {other:?}"),
    }
}

#[test]
fn no_exposure_validator_rejects_every_forbidden_sentinel_class() {
    let scope = projection_scope("thread-a");
    let validator = NoExposureProjectionRedactionValidator;

    for sentinel in [
        "RAW_PROMPT_SENTINEL",
        "TOOL_INPUT_SENTINEL",
        "TOOL_OUTPUT_SENTINEL",
        "SECRET_SENTINEL",
        "HOST_PATH_SENTINEL",
        "RAW_RUNTIME_OUTPUT_SENTINEL",
        "BACKEND_DIAGNOSTIC_SENTINEL",
        "RAW_PROVIDER_ERROR_SENTINEL",
        "INVOCATION_FINGERPRINT_SENTINEL",
        "APPROVAL_REASON_SENTINEL",
        "LEASE_MATERIAL_SENTINEL",
    ] {
        let envelope = ProductProjectionEnvelope::ThreadUpdates(replay_with_error_kind(
            &scope, 11, 11, sentinel,
        ));
        let error = validator
            .validate(&envelope)
            .expect_err("forbidden sentinel rejected");

        assert!(
            matches!(error, ProjectionStreamError::Redaction),
            "sentinel {sentinel} was not rejected as redaction"
        );
    }
}

#[test]
fn in_memory_update_source_publish_without_subscribers_returns_source() {
    let scope = projection_scope("thread-a");
    let source = InMemoryProjectionUpdateSource::new(8);
    let error = source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect_err("broadcast without subscribers fails");

    assert!(matches!(error, ProjectionStreamError::Source));
}

#[tokio::test]
async fn in_memory_update_source_publish_after_subscriber_drop_returns_source() {
    let scope = projection_scope("thread-a");
    let source = InMemoryProjectionUpdateSource::new(8);
    let receiver = source
        .subscribe(ProjectionLiveUpdateRequest {
            actor: actor("user-a"),
            scope: scope.clone(),
            view: ProjectionViewClass::ProductThread,
            target: ProjectionTarget::Thread {
                thread_id: scope.read_scope.thread_id.clone().unwrap(),
            },
        })
        .await
        .expect("source subscription");
    drop(receiver);

    let error = source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect_err("retained sender without receivers fails");

    assert!(matches!(error, ProjectionStreamError::Source));
}

#[tokio::test]
async fn redaction_gate_blocks_sentinel_payloads_at_stream_boundary() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = TestManager {
        inner: EventStreamManager::new(
            Arc::new(FakeProjectionService::new(scope.clone())),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::clone(&source),
            Arc::new(RejectLiveUpdateRedactionValidator),
            Arc::new(InMemoryOutboundStateStore::default()),
        ),
        update_source: Arc::clone(&source),
    };
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish unsafe payload");

    match subscription.next().await.expect("redaction marker") {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::RedactionBlocked);
        }
        other => panic!("expected redaction lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn live_validation_source_failure_is_not_reported_as_redaction() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = TestManager {
        inner: EventStreamManager::new(
            Arc::new(FakeProjectionService::new(scope.clone())),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::clone(&source),
            Arc::new(SourceFailingLiveUpdateValidator),
            Arc::new(InMemoryOutboundStateStore::default()),
        ),
        update_source: Arc::clone(&source),
    };
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish source-failing payload");

    match subscription.next().await.expect("source failure marker") {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::SourceFailed);
        }
        other => panic!("expected source-failure lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn repeated_snapshot_subscriptions_reuse_redaction_validation_decision() {
    let scope = projection_scope("thread-a");
    let fixed_snapshot = snapshot(&scope, 10);
    let validator = Arc::new(CountingRedactionValidator::default());
    let manager = EventStreamManager::new(
        Arc::new(StaticSnapshotProjectionService::new(fixed_snapshot)),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::clone(&validator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let mut first = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("first subscription");
    assert!(matches!(
        first.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    drop(first);

    let mut second = manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect("second subscription");
    assert!(matches!(
        second.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    assert_eq!(validator.calls(), 1);
}

#[tokio::test]
async fn validation_cache_revalidates_distinct_payloads_at_same_cursor() {
    let scope = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(ChangingSnapshotProjectionService::new()),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(RejectTruncatedSnapshotValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let mut first = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("first safe subscription");
    assert!(matches!(
        first.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    drop(first);

    let error = manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect_err("second payload with same cursor is revalidated");

    assert!(matches!(error, ProjectionStreamError::Redaction));
}

#[tokio::test]
async fn product_thread_subscription_blocks_debug_live_updates() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::Debug(
            ironclaw_event_streams::DebugProjectionPayload {
                cursor: ProjectionCursor::for_scope(scope, EventCursor::new(11)),
                redacted_summary: "redacted".to_string(),
            },
        ))
        .expect("publish debug payload");

    match subscription.next().await.expect("access marker") {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::AccessBlocked);
        }
        other => panic!("expected access lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn product_thread_subscription_blocks_foreign_thread_live_payload() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay_for_thread(
            &scope, 11, 11, "thread-b",
        )))
        .expect("publish foreign payload with matching cursor scope");

    match subscription.next().await.expect("access marker") {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::AccessBlocked);
        }
        other => panic!("expected access lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn live_forwarding_advances_cursor_and_reports_latest_reconnect_cursor() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish live update");
    let delivered = expect_thread_update(
        timeout(Duration::from_secs(1), subscription.next())
            .await
            .expect("next live item")
            .expect("next live item"),
    );
    assert_eq!(delivered.next_cursor.runtime, EventCursor::new(11));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish duplicate live update");
    source
        .publish(ProductProjectionEnvelope::Debug(
            ironclaw_event_streams::DebugProjectionPayload {
                cursor: ProjectionCursor::for_scope(scope, EventCursor::new(12)),
                redacted_summary: "redacted".to_string(),
            },
        ))
        .expect("publish blocked live update");

    match timeout(Duration::from_secs(1), subscription.next())
        .await
        .expect("lag item")
        .expect("lag item")
    {
        ProjectionStreamItem::Lagged {
            reason,
            snapshot_cursor,
        } => {
            assert_eq!(reason, LagReason::AccessBlocked);
            assert_eq!(snapshot_cursor.runtime, EventCursor::new(11));
        }
        other => panic!("expected access lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn terminal_lag_waits_for_already_buffered_update() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(ProjectionSubscribeRequest {
            capabilities: SubscriberCapabilities { buffer_capacity: 2 },
            ..subscribe_request(scope.clone(), None)
        })
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish valid update");
    source
        .publish(ProductProjectionEnvelope::Debug(
            ironclaw_event_streams::DebugProjectionPayload {
                cursor: ProjectionCursor::for_scope(scope.clone(), EventCursor::new(12)),
                redacted_summary: "redacted".to_string(),
            },
        ))
        .expect("publish blocked update");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let replay = expect_thread_update(
        timeout(Duration::from_secs(1), subscription.next())
            .await
            .expect("buffered update")
            .expect("buffered update"),
    );
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(11));

    match timeout(Duration::from_secs(1), subscription.next())
        .await
        .expect("terminal lag")
        .expect("terminal lag")
    {
        ProjectionStreamItem::Lagged {
            reason,
            snapshot_cursor,
        } => {
            assert_eq!(reason, LagReason::AccessBlocked);
            assert_eq!(snapshot_cursor.runtime, EventCursor::new(11));
        }
        other => panic!("expected access lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn subscription_ignores_live_updates_from_other_scope() {
    let scope = projection_scope("thread-a");
    let foreign_scope = projection_scope("thread-b");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    let mut foreign_subscription = manager
        .subscribe(subscribe_request(foreign_scope.clone(), None))
        .await
        .expect("foreign subscription registers source scope");
    assert!(matches!(
        foreign_subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &foreign_scope,
            11,
            11,
        )))
        .expect("publish foreign update");
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 12, 12,
        )))
        .expect("publish matching update");

    let replay = expect_thread_update(subscription.next().await.expect("matching update"));
    assert_eq!(replay.next_cursor.scope, scope);
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(12));
}

#[tokio::test]
async fn unrelated_scope_live_burst_does_not_lag_subscription() {
    let noisy_scope = projection_scope("thread-a");
    let quiet_scope = projection_scope("thread-b");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(1));
    let noisy_manager = manager_with_source(noisy_scope.clone(), Arc::clone(&source));
    let quiet_manager = manager_with_source(quiet_scope.clone(), Arc::clone(&source));
    let mut noisy_subscription = noisy_manager
        .subscribe(subscribe_request(noisy_scope.clone(), None))
        .await
        .expect("noisy subscription");
    assert!(matches!(
        noisy_subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    let mut quiet_subscription = quiet_manager
        .subscribe(subscribe_request(quiet_scope.clone(), None))
        .await
        .expect("quiet subscription");
    assert!(matches!(
        quiet_subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    for cursor in 11..14 {
        source
            .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
                &noisy_scope,
                cursor,
                cursor,
            )))
            .expect("publish noisy update");
    }
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &quiet_scope,
            20,
            20,
        )))
        .expect("publish quiet update");

    let replay = expect_thread_update(
        timeout(Duration::from_secs(1), quiet_subscription.next())
            .await
            .expect("quiet subscription is not lagged by noisy scope")
            .expect("quiet live update"),
    );
    assert_eq!(replay.next_cursor.scope, quiet_scope);
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(20));
}

#[tokio::test]
async fn slow_subscriber_gets_backpressure_lag_marker() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(ProjectionSubscribeRequest {
            capabilities: SubscriberCapabilities { buffer_capacity: 1 },
            ..subscribe_request(scope.clone(), None)
        })
        .await
        .expect("authorized subscription");

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish update");

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(matches!(
        timeout(Duration::from_secs(1), subscription.next())
            .await
            .expect("buffered snapshot")
            .expect("buffered snapshot"),
        ProjectionStreamItem::Snapshot(_)
    ));

    match timeout(Duration::from_secs(1), subscription.next())
        .await
        .expect("terminal backpressure marker")
        .expect("terminal backpressure marker")
    {
        ProjectionStreamItem::Lagged {
            reason,
            snapshot_cursor,
        } => {
            assert_eq!(reason, LagReason::SubscriberBackpressure);
            assert_eq!(snapshot_cursor.runtime, EventCursor::new(10));
        }
        other => panic!("expected backpressure marker, got {other:?}"),
    }
    assert!(
        subscription.next().await.is_none(),
        "terminal lag should close the observable stream"
    );
}

#[tokio::test]
async fn live_subscription_registered_before_snapshot_prevents_gap() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = EventStreamManager::new(
        Arc::new(SnapshotPublishingProjectionService::new(
            scope.clone(),
            Arc::clone(&source),
        )),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::clone(&source),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");

    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    let replay = expect_thread_update(
        subscription
            .next()
            .await
            .expect("live update from snapshot race"),
    );
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(11));
}

#[tokio::test]
async fn truncated_snapshot_emits_terminal_lag_before_live_tail() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = EventStreamManager::new(
        Arc::new(TruncatedProjectionService::snapshot(scope.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::clone(&source),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");

    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    match subscription.next().await.expect("truncated snapshot lag") {
        ProjectionStreamItem::Lagged {
            reason,
            snapshot_cursor,
        } => {
            assert_eq!(reason, LagReason::SourceLagged);
            assert_eq!(snapshot_cursor.runtime, EventCursor::new(10));
        }
        other => panic!("expected truncated snapshot lag, got {other:?}"),
    }

    assert!(
        subscription.next().await.is_none(),
        "truncated snapshot must terminate instead of tailing live updates"
    );
}

#[tokio::test]
async fn truncated_resume_replay_emits_terminal_lag_before_live_tail() {
    let scope = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(TruncatedProjectionService::replay(scope.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let mut subscription = manager
        .subscribe(subscribe_request(
            scope.clone(),
            Some(ProjectionCursor::for_scope(
                scope.clone(),
                EventCursor::new(1),
            )),
        ))
        .await
        .expect("authorized resume");

    let replay = expect_thread_update(subscription.next().await.expect("truncated replay page"));
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(3));
    match subscription.next().await.expect("truncated replay lag") {
        ProjectionStreamItem::Lagged {
            reason,
            snapshot_cursor,
        } => {
            assert_eq!(reason, LagReason::SourceLagged);
            assert_eq!(snapshot_cursor.runtime, EventCursor::new(3));
        }
        other => panic!("expected truncated replay lag, got {other:?}"),
    }
}
