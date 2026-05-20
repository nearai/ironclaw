use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, ProjectionCursor, ProjectionError, ProjectionReplay, ProjectionRequest,
    ProjectionScope, ProjectionSnapshot, RunProjectionStatus, RunStatusProjection, ThreadTimeline,
    TimelineEntry, TimelineEntryKind,
};
use ironclaw_event_streams::{
    AllowAllProjectionAccessPolicy, EventStreamManager, InMemoryProjectionStreamAdmissionPolicy,
    InMemoryProjectionUpdateSource, LagReason, NoExposureProjectionRedactionValidator,
    ProductProjectionEnvelope, ProjectionAccessPolicy, ProjectionAccessRequest,
    ProjectionFetchRequest, ProjectionLiveUpdateRequest, ProjectionRedactionValidator,
    ProjectionStreamError, ProjectionStreamItem, ProjectionStreamLimits,
    ProjectionSubscribeRequest, ProjectionTarget, ProjectionUpdateSource, ProjectionViewClass,
    PushCandidatesForUpdateRequest, SubscriberCapabilities, keep_alive_item,
};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, MissionId, ProjectId, RuntimeKind, TenantId, ThreadId,
    UserId,
};
use ironclaw_outbound::{
    AdvanceSubscriptionCursorRequest, InMemoryOutboundStateStore, LoadSubscriptionCursorRequest,
    OutboundDeliveryAttempt, OutboundError, OutboundPushKind, OutboundPushPlan,
    OutboundPushTargetRequest, OutboundStateStore, ProjectionSubscriptionRecord,
    ProjectionUpdateRef, ThreadNotificationPolicy, ThreadNotificationTarget,
    UpdateDeliveryStatusRequest,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn authorized_subscription_without_cursor_emits_snapshot_then_ordered_updates() {
    let scope = projection_scope("thread-a");
    let manager = manager_with_source(
        scope.clone(),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
    );

    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");

    let first = subscription.next().await.expect("initial snapshot");
    assert!(matches!(first, ProjectionStreamItem::Snapshot(_)));

    manager
        .update_source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish live update");

    let second = subscription.next().await.expect("live update");
    match second {
        ProjectionStreamItem::Update(ProductProjectionEnvelope::ThreadUpdates(replay)) => {
            assert_eq!(replay.updates[0].cursor, EventCursor::new(11));
            assert_eq!(replay.next_cursor.runtime, EventCursor::new(11));
        }
        other => panic!("expected thread update, got {other:?}"),
    }
}

#[tokio::test]
async fn valid_cursor_emits_only_replayed_updates() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());

    let mut subscription = manager
        .subscribe(subscribe_request(
            scope.clone(),
            Some(ProjectionCursor::for_scope(scope, EventCursor::new(1))),
        ))
        .await
        .expect("authorized resume");

    match subscription.next().await.expect("replayed update") {
        ProjectionStreamItem::Update(ProductProjectionEnvelope::ThreadUpdates(replay)) => {
            assert_eq!(replay.updates[0].kind, TimelineEntryKind::DispatchSucceeded);
            assert_eq!(replay.next_cursor.runtime, EventCursor::new(3));
        }
        other => panic!("expected replayed update, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_snapshot_returns_authorized_product_snapshot() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());

    let response = manager
        .fetch_snapshot(fetch_request(scope))
        .await
        .expect("authorized fetch");

    match response.snapshot {
        ProductProjectionEnvelope::ThreadSnapshot(snapshot) => {
            assert_eq!(snapshot.next_cursor, response.cursor);
            assert_eq!(snapshot.timeline.entries.len(), 1);
        }
        other => panic!("expected thread snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_snapshot_denied_before_projection_call() {
    let scope = projection_scope("thread-a");
    let projection = Arc::new(FakeProjectionService::new(scope.clone()));
    let access = Arc::new(DenyingAccessPolicy::default());
    let manager = EventStreamManager::new(
        Arc::clone(&projection),
        Arc::clone(&access),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .fetch_snapshot(fetch_request(scope))
        .await
        .expect_err("access denial");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
    assert_eq!(access.calls(), 1);
    assert_eq!(projection.calls(), Vec::<&'static str>::new());
}

#[tokio::test]
async fn fetch_snapshot_rejects_actor_stream_user_mismatch_before_projection_call() {
    let scope = projection_scope_for("tenant-a", "user-b", "thread-a");
    let projection = Arc::new(FakeProjectionService::new(scope.clone()));
    let manager = EventStreamManager::new(
        Arc::clone(&projection),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .fetch_snapshot(fetch_request(scope))
        .await
        .expect_err("actor cannot read another user's stream");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
    assert_eq!(projection.calls(), Vec::<&'static str>::new());
}

#[tokio::test]
async fn fetch_snapshot_rejects_projection_scope_mismatch() {
    let requested = projection_scope("thread-a");
    let returned = projection_scope("thread-b");
    let manager = EventStreamManager::new(
        Arc::new(ScopeMismatchProjectionService::new(returned)),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .fetch_snapshot(fetch_request(requested))
        .await
        .expect_err("scope mismatch");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn fetch_snapshot_rejects_projection_payload_thread_mismatch() {
    let requested = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(PayloadThreadMismatchProjectionService),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .fetch_snapshot(fetch_request(requested))
        .await
        .expect_err("foreign thread payload rejected");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn access_policy_runs_before_projection_or_live_subscription() {
    let scope = projection_scope("thread-a");
    let projection = Arc::new(FakeProjectionService::new(scope.clone()));
    let access = Arc::new(DenyingAccessPolicy::default());
    let update_source = Arc::new(CountingUpdateSource::default());
    let manager = EventStreamManager::new(
        Arc::clone(&projection),
        Arc::clone(&access),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::clone(&update_source),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .subscribe(subscribe_request(
            scope.clone(),
            Some(ProjectionCursor::for_scope(scope, EventCursor::new(99))),
        ))
        .await
        .expect_err("access denial");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
    assert_eq!(access.calls(), 1);
    assert_eq!(projection.calls(), Vec::<&'static str>::new());
    assert_eq!(update_source.calls(), 0);
}

#[tokio::test]
async fn subscribe_rejects_actor_stream_user_mismatch_before_projection_or_live_subscription() {
    let scope = projection_scope_for("tenant-a", "user-b", "thread-a");
    let projection = Arc::new(FakeProjectionService::new(scope.clone()));
    let update_source = Arc::new(CountingUpdateSource::default());
    let manager = EventStreamManager::new(
        Arc::clone(&projection),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::clone(&update_source),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect_err("actor cannot subscribe to another user's stream");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
    assert_eq!(projection.calls(), Vec::<&'static str>::new());
    assert_eq!(update_source.calls(), 0);
}

#[tokio::test]
async fn debug_admin_view_is_denied_without_widening_product_thread_streams() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());
    let error = manager
        .subscribe(ProjectionSubscribeRequest {
            view: ProjectionViewClass::DebugSupport,
            ..subscribe_request(scope, None)
        })
        .await
        .expect_err("debug view denied");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn stale_cursor_after_valid_access_rebases_to_fresh_snapshot() {
    let scope = projection_scope("thread-a");
    let stale = ProjectionCursor::for_scope(scope.clone(), EventCursor::new(99));
    let manager = manager(scope);

    let mut subscription = manager
        .subscribe(subscribe_request(stale.scope.clone(), Some(stale.clone())))
        .await
        .expect("authorized subscription");

    match subscription.next().await.expect("rebase item") {
        ProjectionStreamItem::RebaseRequired {
            rebased_from,
            snapshot_cursor,
            ..
        } => {
            assert_eq!(rebased_from, Some(stale));
            assert_eq!(snapshot_cursor.runtime, EventCursor::new(10));
        }
        other => panic!("expected rebase item, got {other:?}"),
    }
}

#[tokio::test]
async fn foreign_cursor_is_rejected_as_authority_failure() {
    let requested_scope = projection_scope("thread-a");
    let foreign_scope = projection_scope("thread-b");
    let manager = manager(requested_scope.clone());

    let error = manager
        .subscribe(subscribe_request(
            requested_scope,
            Some(ProjectionCursor::for_scope(
                foreign_scope,
                EventCursor::new(7),
            )),
        ))
        .await
        .expect_err("foreign cursor rejected");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn valid_resume_rejects_projection_scope_mismatch() {
    let requested = projection_scope("thread-a");
    let returned = projection_scope("thread-b");
    let manager = EventStreamManager::new(
        Arc::new(ScopeMismatchProjectionService::new(returned)),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .subscribe(subscribe_request(
            requested.clone(),
            Some(ProjectionCursor::for_scope(requested, EventCursor::new(1))),
        ))
        .await
        .expect_err("scope mismatch");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn valid_resume_rejects_projection_payload_thread_mismatch() {
    let requested = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(PayloadThreadMismatchProjectionService),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .subscribe(subscribe_request(
            requested.clone(),
            Some(ProjectionCursor::for_scope(requested, EventCursor::new(1))),
        ))
        .await
        .expect_err("foreign thread replay payload rejected");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn broadcast_lag_emits_explicit_lagged_item() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(1));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(ProjectionSubscribeRequest {
            capabilities: SubscriberCapabilities { buffer_capacity: 1 },
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
            &scope, 2, 2,
        )))
        .unwrap();
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 3, 3,
        )))
        .unwrap();
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 4, 4,
        )))
        .unwrap();

    match timeout(Duration::from_secs(1), subscription.next())
        .await
        .expect("terminal lag item")
        .expect("terminal lag item")
    {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::SourceLagged);
        }
        other => panic!("expected lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn stream_admission_denial_is_structured_and_product_safe() {
    let scope = projection_scope("thread-a");
    let projection = Arc::new(FakeProjectionService::new(scope.clone()));
    let manager = EventStreamManager::new(
        projection,
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(
            ProjectionStreamLimits {
                per_tenant: 1,
                per_actor: 1,
                per_scope: 1,
                global: 1,
            },
        )),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let _first = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("first subscription admitted");
    let error = manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect_err("second subscription rejected");

    assert!(matches!(error, ProjectionStreamError::AdmissionDenied));
}

#[tokio::test]
async fn tenant_admission_limit_is_enforced_independently() {
    let first = projection_scope_for("tenant-a", "user-a", "thread-a");
    let second = projection_scope_for("tenant-a", "user-b", "thread-b");

    assert_second_subscription_denied_by_admission(
        ProjectionStreamLimits {
            per_tenant: 1,
            per_actor: 10,
            per_scope: 10,
            global: 10,
        },
        first,
        second,
    )
    .await;
}

#[tokio::test]
async fn actor_admission_limit_is_enforced_independently() {
    let first = projection_scope_for("tenant-a", "user-a", "thread-a");
    let second = projection_scope_for("tenant-a", "user-a", "thread-b");

    assert_second_subscription_denied_by_admission(
        ProjectionStreamLimits {
            per_tenant: 10,
            per_actor: 1,
            per_scope: 10,
            global: 10,
        },
        first,
        second,
    )
    .await;
}

#[tokio::test]
async fn scope_admission_limit_is_enforced_independently() {
    let scope = projection_scope_for("tenant-a", "user-a", "thread-a");

    assert_second_subscription_denied_by_admission(
        ProjectionStreamLimits {
            per_tenant: 10,
            per_actor: 10,
            per_scope: 1,
            global: 10,
        },
        scope.clone(),
        scope,
    )
    .await;
}

#[tokio::test]
async fn global_admission_limit_is_enforced_independently() {
    let first = projection_scope_for("tenant-a", "user-a", "thread-a");
    let second = projection_scope_for("tenant-b", "user-b", "thread-b");

    assert_second_subscription_denied_by_admission(
        ProjectionStreamLimits {
            per_tenant: 10,
            per_actor: 10,
            per_scope: 10,
            global: 1,
        },
        first,
        second,
    )
    .await;
}

#[tokio::test]
async fn per_actor_admission_is_scoped_by_tenant() {
    let scope_a = projection_scope_for("tenant-a", "user-a", "thread-a");
    let scope_b = projection_scope_for("tenant-b", "user-a", "thread-b");
    let admission = Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(
        ProjectionStreamLimits {
            per_tenant: 1,
            per_actor: 1,
            per_scope: 1,
            global: 2,
        },
    ));
    let manager_a = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope_a.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::clone(&admission),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );
    let manager_b = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope_b.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::clone(&admission),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let _tenant_a = manager_a
        .subscribe(subscribe_request(scope_a, None))
        .await
        .expect("tenant A subscription admitted");
    let _tenant_b = manager_b
        .subscribe(subscribe_request(scope_b, None))
        .await
        .expect("same user id in tenant B remains admitted");
}

#[tokio::test]
async fn subscribe_error_after_admission_releases_permit() {
    let scope = projection_scope("thread-a");
    let admission = Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(
        ProjectionStreamLimits {
            per_tenant: 1,
            per_actor: 1,
            per_scope: 1,
            global: 1,
        },
    ));
    let failing_manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::clone(&admission),
        Arc::new(FailingUpdateSource),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = failing_manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect_err("update source failure");
    assert!(matches!(error, ProjectionStreamError::Source));

    let healthy_manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::clone(&admission),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );
    let _subscription = healthy_manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect("admission permit was released after source failure");
}

#[tokio::test]
async fn dropping_subscription_releases_admission_permit() {
    let scope = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(
            ProjectionStreamLimits {
                per_tenant: 1,
                per_actor: 1,
                per_scope: 1,
                global: 1,
            },
        )),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let first = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("first subscription admitted");
    assert!(matches!(
        manager
            .subscribe(subscribe_request(scope.clone(), None))
            .await,
        Err(ProjectionStreamError::AdmissionDenied)
    ));

    drop(first);

    let _replacement = manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect("replacement subscription admitted after drop");
}

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
    match timeout(Duration::from_secs(1), subscription.next())
        .await
        .expect("next live item")
        .expect("next live item")
    {
        ProjectionStreamItem::Update(ProductProjectionEnvelope::ThreadUpdates(replay)) => {
            assert_eq!(replay.next_cursor.runtime, EventCursor::new(11));
        }
        other => panic!("expected delivered update, got {other:?}"),
    }

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

    match subscription.next().await.expect("matching update") {
        ProjectionStreamItem::Update(ProductProjectionEnvelope::ThreadUpdates(replay)) => {
            assert_eq!(replay.next_cursor.scope, scope);
            assert_eq!(replay.next_cursor.runtime, EventCursor::new(12));
        }
        other => panic!("expected matching update, got {other:?}"),
    }
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
    match subscription
        .next()
        .await
        .expect("live update from snapshot race")
    {
        ProjectionStreamItem::Update(ProductProjectionEnvelope::ThreadUpdates(replay)) => {
            assert_eq!(replay.next_cursor.runtime, EventCursor::new(11));
        }
        other => panic!("expected captured live update, got {other:?}"),
    }
}

#[tokio::test]
async fn push_candidates_are_separate_from_subscriptions_and_policy_gated() {
    let scope = projection_scope("thread-a");
    let turn_scope = turn_scope("thread-a");
    let outbound = Arc::new(InMemoryOutboundStateStore::default());
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope)),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::clone(&outbound),
    );

    let final_reply = manager
        .push_candidates_for_update(push_request(&turn_scope, OutboundPushKind::FinalReply))
        .await
        .expect("final reply candidates");
    assert_eq!(final_reply.len(), 1);
    assert_eq!(final_reply[0].target, reply_target("reply-default"));

    let progress = manager
        .push_candidates_for_update(push_request(&turn_scope, OutboundPushKind::Progress))
        .await
        .expect("progress candidates");
    assert!(progress.is_empty());

    outbound
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: turn_scope.clone(),
            targets: vec![ThreadNotificationTarget {
                target: reply_target("reply-progress"),
                final_replies: false,
                progress: true,
            }],
        })
        .await
        .expect("store policy");

    let progress = manager
        .push_candidates_for_update(push_request(&turn_scope, OutboundPushKind::Progress))
        .await
        .expect("progress candidates");
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].target, reply_target("reply-progress"));
}

#[tokio::test]
async fn push_candidates_maps_outbound_store_failures() {
    let scope = turn_scope("thread-a");
    for (kind, expected) in [
        (
            FailingOutboundKind::AccessDenied,
            ProjectionStreamError::AccessDenied,
        ),
        (
            FailingOutboundKind::InvalidRequest,
            ProjectionStreamError::InvalidRequest {
                reason: "bad request",
            },
        ),
        (
            FailingOutboundKind::Backend,
            ProjectionStreamError::Outbound,
        ),
    ] {
        let manager = EventStreamManager::new(
            Arc::new(FakeProjectionService::new(projection_scope("thread-a"))),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::new(InMemoryProjectionUpdateSource::new(8)),
            Arc::new(NoExposureProjectionRedactionValidator),
            Arc::new(FailingOutboundStore { kind }),
        );

        let actual = manager
            .push_candidates_for_update(push_request(&scope, OutboundPushKind::Progress))
            .await
            .expect_err("mapped outbound error");

        assert_same_error_kind(actual, expected);
    }
}

#[tokio::test]
async fn unsupported_view_target_returns_invalid_request() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());
    let error = manager
        .subscribe(ProjectionSubscribeRequest {
            view: ProjectionViewClass::ProductMission,
            target: ProjectionTarget::Mission {
                mission_id: MissionId::new("mission-a").unwrap(),
            },
            ..subscribe_request(scope, None)
        })
        .await
        .expect_err("unsupported first-slice view");

    assert!(matches!(
        error,
        ProjectionStreamError::InvalidRequest { .. }
    ));
}

#[tokio::test]
async fn zero_subscription_buffer_capacity_is_clamped_to_one() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());
    let mut subscription = manager
        .subscribe(ProjectionSubscribeRequest {
            capabilities: SubscriberCapabilities { buffer_capacity: 0 },
            ..subscribe_request(scope, None)
        })
        .await
        .expect("zero capacity clamped");

    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
}

#[tokio::test]
async fn oversized_subscription_buffer_capacity_is_rejected() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());
    let error = manager
        .subscribe(ProjectionSubscribeRequest {
            capabilities: SubscriberCapabilities {
                buffer_capacity: usize::MAX,
            },
            ..subscribe_request(scope, None)
        })
        .await
        .expect_err("oversized buffer rejected");

    assert!(matches!(
        error,
        ProjectionStreamError::InvalidRequest { .. }
    ));
}

#[test]
fn keepalive_items_do_not_advance_projection_cursor_or_carry_payload() {
    assert_eq!(keep_alive_item(), ProjectionStreamItem::KeepAlive);
}

struct TestManager {
    inner: EventStreamManager,
    update_source: Arc<InMemoryProjectionUpdateSource>,
}

impl std::ops::Deref for TestManager {
    type Target = EventStreamManager;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

fn manager(scope: ProjectionScope) -> TestManager {
    manager_with_source(scope, Arc::new(InMemoryProjectionUpdateSource::new(8)))
}

fn manager_with_source(
    scope: ProjectionScope,
    update_source: Arc<InMemoryProjectionUpdateSource>,
) -> TestManager {
    TestManager {
        inner: EventStreamManager::new(
            Arc::new(FakeProjectionService::new(scope)),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::clone(&update_source),
            Arc::new(NoExposureProjectionRedactionValidator),
            Arc::new(InMemoryOutboundStateStore::default()),
        ),
        update_source,
    }
}

async fn assert_second_subscription_denied_by_admission(
    limits: ProjectionStreamLimits,
    first: ProjectionScope,
    second: ProjectionScope,
) {
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(first.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(limits)),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let _first = manager
        .subscribe(subscribe_request_for_stream_user(first, None))
        .await
        .expect("first subscription admitted");
    let error = manager
        .subscribe(subscribe_request_for_stream_user(second, None))
        .await
        .expect_err("second subscription rejected by targeted admission limit");

    assert!(matches!(error, ProjectionStreamError::AdmissionDenied));
}

#[derive(Default)]
struct DenyingAccessPolicy {
    calls: Mutex<usize>,
}

impl DenyingAccessPolicy {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl ProjectionAccessPolicy for DenyingAccessPolicy {
    async fn authorize(
        &self,
        _request: ProjectionAccessRequest,
    ) -> Result<(), ProjectionStreamError> {
        *self.calls.lock().unwrap() += 1;
        Err(ProjectionStreamError::AccessDenied)
    }
}

#[derive(Default)]
struct CountingUpdateSource {
    calls: Mutex<usize>,
}

impl CountingUpdateSource {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl ProjectionUpdateSource for CountingUpdateSource {
    async fn subscribe(
        &self,
        request: ProjectionLiveUpdateRequest,
    ) -> Result<
        tokio::sync::broadcast::Receiver<Arc<ProductProjectionEnvelope>>,
        ProjectionStreamError,
    > {
        *self.calls.lock().unwrap() += 1;
        Ok(InMemoryProjectionUpdateSource::new(1)
            .subscribe(request)
            .await?)
    }
}

struct FailingUpdateSource;

#[async_trait]
impl ProjectionUpdateSource for FailingUpdateSource {
    async fn subscribe(
        &self,
        _request: ProjectionLiveUpdateRequest,
    ) -> Result<
        tokio::sync::broadcast::Receiver<Arc<ProductProjectionEnvelope>>,
        ProjectionStreamError,
    > {
        Err(ProjectionStreamError::Source)
    }
}

struct FakeProjectionService {
    scope: ProjectionScope,
    calls: Mutex<Vec<&'static str>>,
}

impl FakeProjectionService {
    fn new(scope: ProjectionScope) -> Self {
        Self {
            scope,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventProjectionService for FakeProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.calls.lock().unwrap().push("snapshot");
        Ok(snapshot(&request.scope, 10))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        self.calls.lock().unwrap().push("updates");
        let cursor = request.after.expect("test supplies cursor");
        if cursor.runtime == EventCursor::new(99) {
            return Err(ProjectionError::RebaseRequired {
                requested: Box::new(cursor),
                earliest: Box::new(ProjectionCursor::origin_for_scope(self.scope.clone())),
            });
        }
        Ok(replay(&request.scope, 2, 3))
    }
}

struct ScopeMismatchProjectionService {
    scope: ProjectionScope,
}

impl ScopeMismatchProjectionService {
    fn new(scope: ProjectionScope) -> Self {
        Self { scope }
    }
}

#[async_trait]
impl EventProjectionService for ScopeMismatchProjectionService {
    async fn snapshot(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(snapshot(&self.scope, 10))
    }

    async fn updates(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&self.scope, 2, 3))
    }
}

struct PayloadThreadMismatchProjectionService;

#[async_trait]
impl EventProjectionService for PayloadThreadMismatchProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(snapshot_for_thread(&request.scope, 10, "thread-b"))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay_for_thread(&request.scope, 2, 3, "thread-b"))
    }
}

struct SnapshotPublishingProjectionService {
    scope: ProjectionScope,
    source: Arc<InMemoryProjectionUpdateSource>,
}

impl SnapshotPublishingProjectionService {
    fn new(scope: ProjectionScope, source: Arc<InMemoryProjectionUpdateSource>) -> Self {
        Self { scope, source }
    }
}

#[async_trait]
impl EventProjectionService for SnapshotPublishingProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.source
            .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
                &self.scope,
                11,
                11,
            )))
            .expect("publish race update");
        Ok(snapshot(&request.scope, 10))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

struct StaticSnapshotProjectionService {
    snapshot: ProjectionSnapshot,
}

impl StaticSnapshotProjectionService {
    fn new(snapshot: ProjectionSnapshot) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl EventProjectionService for StaticSnapshotProjectionService {
    async fn snapshot(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(self.snapshot.clone())
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

struct ChangingSnapshotProjectionService {
    calls: Mutex<usize>,
}

impl ChangingSnapshotProjectionService {
    fn new() -> Self {
        Self {
            calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl EventProjectionService for ChangingSnapshotProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        let mut calls = self.calls.lock().unwrap();
        let mut snapshot = snapshot(&request.scope, 10);
        if *calls > 0 {
            snapshot.truncated = true;
        }
        *calls += 1;
        Ok(snapshot)
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

struct RejectLiveUpdateRedactionValidator;

impl ProjectionRedactionValidator for RejectLiveUpdateRedactionValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadUpdates(_) => Err(ProjectionStreamError::Redaction),
            _ => Ok(()),
        }
    }
}

struct SourceFailingLiveUpdateValidator;

impl ProjectionRedactionValidator for SourceFailingLiveUpdateValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadUpdates(_) => Err(ProjectionStreamError::Source),
            _ => Ok(()),
        }
    }
}

struct RejectTruncatedSnapshotValidator;

impl ProjectionRedactionValidator for RejectTruncatedSnapshotValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadSnapshot(snapshot) if snapshot.truncated => {
                Err(ProjectionStreamError::Redaction)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Default)]
struct CountingRedactionValidator {
    calls: Mutex<usize>,
}

impl CountingRedactionValidator {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl ProjectionRedactionValidator for CountingRedactionValidator {
    fn validate(&self, _envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        *self.calls.lock().unwrap() += 1;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum FailingOutboundKind {
    AccessDenied,
    InvalidRequest,
    Backend,
}

struct FailingOutboundStore {
    kind: FailingOutboundKind,
}

impl FailingOutboundStore {
    fn error(&self) -> OutboundError {
        match self.kind {
            FailingOutboundKind::AccessDenied => OutboundError::AccessDenied,
            FailingOutboundKind::InvalidRequest => OutboundError::InvalidRequest {
                reason: "bad request",
            },
            FailingOutboundKind::Backend => OutboundError::Backend,
        }
    }
}

#[async_trait]
impl OutboundStateStore for FailingOutboundStore {
    async fn put_thread_notification_policy(
        &self,
        _policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn load_thread_notification_policy(
        &self,
        _scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        Err(self.error())
    }

    async fn plan_push_targets(
        &self,
        _request: OutboundPushTargetRequest,
    ) -> Result<OutboundPushPlan, OutboundError> {
        Err(self.error())
    }

    async fn upsert_subscription(
        &self,
        _record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn load_subscription_cursor(
        &self,
        _request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        Err(self.error())
    }

    async fn advance_subscription_cursor(
        &self,
        _request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn record_delivery_attempt(
        &self,
        _attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn update_delivery_status(
        &self,
        _request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn list_delivery_attempts(
        &self,
        _scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        Err(self.error())
    }
}

fn assert_same_error_kind(actual: ProjectionStreamError, expected: ProjectionStreamError) {
    match (actual, expected) {
        (ProjectionStreamError::AccessDenied, ProjectionStreamError::AccessDenied)
        | (ProjectionStreamError::AdmissionDenied, ProjectionStreamError::AdmissionDenied)
        | (ProjectionStreamError::Source, ProjectionStreamError::Source)
        | (ProjectionStreamError::Redaction, ProjectionStreamError::Redaction)
        | (ProjectionStreamError::Outbound, ProjectionStreamError::Outbound) => {}
        (
            ProjectionStreamError::InvalidRequest { reason: actual },
            ProjectionStreamError::InvalidRequest { reason: expected },
        ) => assert_eq!(actual, expected),
        (actual, expected) => panic!("expected {expected:?}, got {actual:?}"),
    }
}

fn subscribe_request(
    scope: ProjectionScope,
    after_cursor: Option<ProjectionCursor>,
) -> ProjectionSubscribeRequest {
    ProjectionSubscribeRequest {
        actor: actor("user-a"),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        after_cursor,
        limit: 16,
        capabilities: SubscriberCapabilities { buffer_capacity: 2 },
    }
}

fn subscribe_request_for_stream_user(
    scope: ProjectionScope,
    after_cursor: Option<ProjectionCursor>,
) -> ProjectionSubscribeRequest {
    ProjectionSubscribeRequest {
        actor: TurnActor::new(scope.stream.user_id.clone()),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        after_cursor,
        limit: 16,
        capabilities: SubscriberCapabilities { buffer_capacity: 2 },
    }
}

fn fetch_request(scope: ProjectionScope) -> ProjectionFetchRequest {
    ProjectionFetchRequest {
        actor: actor("user-a"),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        limit: 16,
    }
}

fn push_request(scope: &TurnScope, kind: OutboundPushKind) -> PushCandidatesForUpdateRequest {
    PushCandidatesForUpdateRequest {
        scope: scope.clone(),
        turn_run_id: None,
        reply_target: reply_target("reply-default"),
        kind,
        projection_ref: ProjectionUpdateRef::new("projection:update:1").unwrap(),
    }
}

fn snapshot(scope: &ProjectionScope, cursor: u64) -> ProjectionSnapshot {
    ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: vec![timeline_entry(
                scope,
                cursor,
                TimelineEntryKind::DispatchRequested,
            )],
        },
        runs: vec![run_status(scope, cursor)],
        next_cursor: ProjectionCursor::for_scope(scope.clone(), EventCursor::new(cursor)),
        truncated: false,
    }
}

fn snapshot_for_thread(scope: &ProjectionScope, cursor: u64, thread: &str) -> ProjectionSnapshot {
    let mut snapshot = snapshot(scope, cursor);
    let thread_id = Some(ThreadId::new(thread).unwrap());
    for entry in &mut snapshot.timeline.entries {
        entry.thread_id = thread_id.clone();
    }
    for run in &mut snapshot.runs {
        run.thread_id = thread_id.clone();
    }
    snapshot
}

fn replay(scope: &ProjectionScope, cursor: u64, next: u64) -> ProjectionReplay {
    ProjectionReplay {
        updates: vec![timeline_entry(
            scope,
            cursor,
            TimelineEntryKind::DispatchSucceeded,
        )],
        runs: vec![run_status(scope, next)],
        next_cursor: ProjectionCursor::for_scope(scope.clone(), EventCursor::new(next)),
        truncated: false,
    }
}

fn replay_with_error_kind(
    scope: &ProjectionScope,
    cursor: u64,
    next: u64,
    error_kind: &str,
) -> ProjectionReplay {
    let mut replay = replay(scope, cursor, next);
    for entry in &mut replay.updates {
        entry.error_kind = Some(error_kind.to_string());
    }
    for run in &mut replay.runs {
        run.error_kind = Some(error_kind.to_string());
    }
    replay
}

fn replay_for_thread(
    scope: &ProjectionScope,
    cursor: u64,
    next: u64,
    thread: &str,
) -> ProjectionReplay {
    let mut replay = replay(scope, cursor, next);
    let thread_id = Some(ThreadId::new(thread).unwrap());
    for entry in &mut replay.updates {
        entry.thread_id = thread_id.clone();
    }
    for run in &mut replay.runs {
        run.thread_id = thread_id.clone();
    }
    replay
}

fn timeline_entry(scope: &ProjectionScope, cursor: u64, kind: TimelineEntryKind) -> TimelineEntry {
    TimelineEntry {
        cursor: EventCursor::new(cursor),
        event_id: ironclaw_events::RuntimeEventId::new(),
        timestamp: chrono::Utc::now(),
        kind,
        invocation_id: InvocationId::new(),
        thread_id: scope.read_scope.thread_id.clone(),
        capability_id: CapabilityId::new("script.echo").unwrap(),
        provider: Some(ExtensionId::new("script").unwrap()),
        runtime: Some(RuntimeKind::Script),
        process_id: None,
        output_bytes: Some(12),
        error_kind: None,
    }
}

fn run_status(scope: &ProjectionScope, cursor: u64) -> RunStatusProjection {
    RunStatusProjection {
        invocation_id: InvocationId::new(),
        capability_id: CapabilityId::new("script.echo").unwrap(),
        thread_id: scope.read_scope.thread_id.clone(),
        status: RunProjectionStatus::Completed,
        provider: Some(ExtensionId::new("script").unwrap()),
        runtime: Some(RuntimeKind::Script),
        process_id: None,
        error_kind: None,
        last_cursor: EventCursor::new(cursor),
        updated_at: chrono::Utc::now(),
    }
}

fn projection_scope(thread: &str) -> ProjectionScope {
    projection_scope_for("tenant-a", "user-a", thread)
}

fn projection_scope_for(tenant: &str, user: &str, thread: &str) -> ProjectionScope {
    let thread_id = ThreadId::new(thread).unwrap();
    ProjectionScope {
        stream: EventStreamKey::new(
            TenantId::new(tenant).unwrap(),
            UserId::new(user).unwrap(),
            None,
        ),
        read_scope: ReadScope {
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: None,
            thread_id: Some(thread_id),
            process_id: None,
        },
    }
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-a").unwrap(),
        None,
        Some(ProjectId::new("project-a").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn actor(user: &str) -> TurnActor {
    TurnActor::new(UserId::new(user).unwrap())
}

fn reply_target(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).unwrap()
}
