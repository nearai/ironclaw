use super::*;

#[tokio::test]
async fn push_candidates_are_separate_from_subscriptions_and_policy_gated() {
    let scope = projection_scope("thread-a");
    let turn_scope = turn_scope("thread-a");
    let outbound = Arc::new(InMemoryOutboundStateStore::default());
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope.clone())),
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
async fn push_candidates_require_projection_access() {
    let scope = projection_scope("thread-a");
    let turn_scope = turn_scope("thread-a");
    let access = Arc::new(DenyingAccessPolicy::default());
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope)),
        Arc::clone(&access),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .push_candidates_for_update(push_request(&turn_scope, OutboundPushKind::Progress))
        .await
        .expect_err("push candidates require projection access");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
    assert_eq!(access.calls(), 1);
}

#[tokio::test]
async fn push_candidates_reject_mismatched_projection_scope() {
    let scope = projection_scope("thread-a");
    let turn_scope = turn_scope("thread-a");
    let mut request = push_request(&turn_scope, OutboundPushKind::Progress);
    request.projection_scope = projection_scope("thread-b");
    let manager = manager(scope);

    let error = manager
        .push_candidates_for_update(request)
        .await
        .expect_err("projection scope must match push turn scope");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
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
