use super::*;

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
async fn subscribe_resume_rejects_redaction_failure() {
    let requested = projection_scope("thread-a");
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(requested.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(RejectLiveUpdateRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .subscribe(subscribe_request(
            requested.clone(),
            Some(ProjectionCursor::for_scope(requested, EventCursor::new(1))),
        ))
        .await
        .expect_err("unsafe replay payload rejected");

    assert!(matches!(error, ProjectionStreamError::Redaction));
}

#[tokio::test]
async fn subscribe_resume_maps_projection_update_errors() {
    let scope = projection_scope("thread-a");
    for (projection_error, expected) in [
        (
            ProjectionError::InvalidRequest {
                reason: "bad replay request",
            },
            ProjectionStreamError::InvalidRequest {
                reason: "bad replay request",
            },
        ),
        (
            ProjectionError::Source {
                operation: "projection backend failed",
            },
            ProjectionStreamError::Source,
        ),
    ] {
        let manager = EventStreamManager::new(
            Arc::new(FailingUpdatesProjectionService {
                error: projection_error,
            }),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::new(InMemoryProjectionUpdateSource::new(8)),
            Arc::new(NoExposureProjectionRedactionValidator),
            Arc::new(InMemoryOutboundStateStore::default()),
        );

        let actual = manager
            .subscribe(subscribe_request(
                scope.clone(),
                Some(ProjectionCursor::for_scope(
                    scope.clone(),
                    EventCursor::new(1),
                )),
            ))
            .await
            .expect_err("resume update error is mapped");

        assert_same_error_kind(actual, expected);
    }
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
