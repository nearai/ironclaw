use super::*;

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

    let replay = expect_thread_update(subscription.next().await.expect("live update"));
    assert_eq!(replay.updates[0].cursor, EventCursor::new(11));
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(11));
}

#[tokio::test]
async fn live_fanout_delivers_shared_projection_update_payload() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(PointerUpdateSource::new(8));
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope.clone())),
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

    let envelope = Arc::new(ProductProjectionEnvelope::ThreadUpdates(replay(
        &scope, 11, 11,
    )));
    source
        .publish_shared(Arc::clone(&envelope))
        .expect("publish shared update");

    match subscription.next().await.expect("shared live update") {
        ProjectionStreamItem::Update(delivered) => {
            assert!(Arc::ptr_eq(&delivered, &envelope));
        }
        other => panic!("expected shared update, got {other:?}"),
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

    let replay = expect_thread_update(subscription.next().await.expect("replayed update"));
    assert_eq!(replay.updates[0].kind, TimelineEntryKind::DispatchSucceeded);
    assert_eq!(replay.next_cursor.runtime, EventCursor::new(3));
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
async fn fetch_snapshot_rejects_unsupported_view_target_before_projection_call() {
    let scope = projection_scope("thread-a");
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
        .fetch_snapshot(ProjectionFetchRequest {
            view: ProjectionViewClass::ProductMission,
            target: ProjectionTarget::Mission {
                mission_id: MissionId::new("mission-a").unwrap(),
            },
            ..fetch_request(scope)
        })
        .await
        .expect_err("unsupported fetch target rejected before projection");

    assert!(matches!(
        error,
        ProjectionStreamError::InvalidRequest { .. }
    ));
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
async fn fetch_snapshot_rejects_capability_activity_thread_mismatch() {
    let requested = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(ActivityThreadMismatchProjectionService),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .fetch_snapshot(fetch_request(requested))
        .await
        .expect_err("foreign activity thread payload rejected");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn fetch_snapshot_rejects_redaction_failure() {
    let scope = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(RejectSnapshotRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .fetch_snapshot(fetch_request(scope))
        .await
        .expect_err("unsafe fetch snapshot rejected");

    assert!(matches!(error, ProjectionStreamError::Redaction));
}

#[tokio::test]
async fn fetch_snapshot_maps_projection_snapshot_errors() {
    let scope = projection_scope("thread-a");
    let rebase_requested = ProjectionCursor::for_scope(scope.clone(), EventCursor::new(99));
    let rebase_earliest = ProjectionCursor::origin_for_scope(scope.clone());

    for (projection_error, expected) in [
        (
            ProjectionError::InvalidRequest {
                reason: "bad snapshot request",
            },
            ProjectionStreamError::InvalidRequest {
                reason: "bad snapshot request",
            },
        ),
        (
            ProjectionError::Source {
                operation: "projection snapshot failed",
            },
            ProjectionStreamError::Source,
        ),
        (
            ProjectionError::RebaseRequired {
                requested: Box::new(rebase_requested),
                earliest: Box::new(rebase_earliest),
            },
            ProjectionStreamError::InvalidRequest {
                reason: "projection rebase required outside subscribe flow",
            },
        ),
    ] {
        let manager = EventStreamManager::new(
            Arc::new(FailingSnapshotProjectionService {
                error: projection_error,
            }),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::new(InMemoryProjectionUpdateSource::new(8)),
            Arc::new(NoExposureProjectionRedactionValidator),
            Arc::new(InMemoryOutboundStateStore::default()),
        );

        let actual = manager
            .fetch_snapshot(fetch_request(scope.clone()))
            .await
            .expect_err("snapshot error is mapped");

        assert_same_error_kind(actual, expected);
    }
}
