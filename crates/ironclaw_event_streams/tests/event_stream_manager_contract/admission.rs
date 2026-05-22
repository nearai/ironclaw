use super::support::*;

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
async fn terminal_lag_releases_admission_permit_before_subscription_drop() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(1));
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
        Arc::clone(&source),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let mut first = manager
        .subscribe(ProjectionSubscribeRequest {
            capabilities: SubscriberCapabilities { buffer_capacity: 1 },
            ..subscribe_request(scope.clone(), None)
        })
        .await
        .expect("first subscription admitted");
    assert!(matches!(
        first.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 11, 11,
        )))
        .expect("publish first update");
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 12, 12,
        )))
        .expect("publish second update");
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 13, 13,
        )))
        .expect("publish third update");

    assert!(matches!(
        timeout(Duration::from_secs(1), first.next())
            .await
            .expect("terminal lag")
            .expect("terminal lag"),
        ProjectionStreamItem::Lagged { .. }
    ));

    let _second = manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect("terminal lag released first admission permit");
}

#[tokio::test]
async fn concurrent_stream_admission_enforces_global_limit_once() {
    let admission = Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(
        ProjectionStreamLimits {
            per_tenant: 10,
            per_actor: 10,
            per_scope: 10,
            global: 1,
        },
    ));
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();

    for index in 0..8 {
        let admission = Arc::clone(&admission);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let scope = projection_scope_for("tenant-a", &format!("user-{index}"), "thread-a");
            barrier.wait().await;
            admission
                .admit(ProjectionStreamAdmissionRequest {
                    actor: TurnActor::new(scope.stream.user_id.clone()),
                    tenant_id: scope.stream.tenant_id.clone(),
                    target: ProjectionTarget::Thread {
                        thread_id: scope.read_scope.thread_id.clone().unwrap(),
                    },
                    scope,
                    view: ProjectionViewClass::ProductThread,
                })
                .await
        }));
    }

    let mut admitted = Vec::new();
    let mut denied = 0;
    for handle in handles {
        match handle.await.expect("admission task joined") {
            Ok(permit) => admitted.push(permit),
            Err(ProjectionStreamError::AdmissionDenied) => denied += 1,
            Err(error) => panic!("unexpected admission error: {error:?}"),
        }
    }

    assert_eq!(admitted.len(), 1);
    assert_eq!(denied, 7);
}
