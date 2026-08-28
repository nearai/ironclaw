use super::*;

#[tokio::test]
async fn oauth_turn_gate_continuation_resolves_only_the_committed_gate_instance() {
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbox = notification_inbox();
    let run_id = TurnRunId::new();
    let gate_ref = TurnGateRef::new("gate:oauth-callback").expect("gate ref");
    coordinator.set_state(run_state(
        run_id,
        TurnStatus::BlockedAuth,
        Some(gate_ref.as_str()),
    ));
    let notification_id = crate::run_delivery::run_notification_inbox_id(
        run_id,
        NotificationKind::AuthenticationRequired,
        Some(gate_ref.as_str()),
    )
    .expect("notification id");
    inbox
        .publish(PublishNotificationRequest {
            id: notification_id,
            recipient: NotificationRecipient {
                tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                user_id: UserId::new("alice").expect("user"),
            },
            kind: NotificationKind::AuthenticationRequired,
            severity: NotificationSeverity::Warning,
            source: NotificationSource {
                thread_id: ThreadId::new("thread-auth").expect("thread"),
                turn_run_id: Some(run_id),
                lifecycle_ref: Some(LifecycleRef::new(gate_ref.as_str()).expect("lifecycle ref")),
                credential_providers: Vec::new(),
            },
            action: NotificationAction::OpenThread {
                thread_id: ThreadId::new("thread-auth").expect("thread"),
            },
            initial_state: NotificationInitialState::Open,
            occurred_at: Utc::now(),
        })
        .await
        .expect("seed auth notification");
    let dispatcher = ProductAuthTurnGateResumeDispatcher::new(coordinator.clone())
        .with_notification_inbox(Arc::clone(&inbox) as Arc<dyn NotificationInboxStorePort>);
    let event = scoped_event(AuthContinuationRef::TurnGateResume {
        turn_run_ref: TurnRunRef::new(run_id.to_string()).expect("run ref"),
        gate_ref: AuthGateRef::new(gate_ref.as_str()).expect("auth gate ref"),
    });

    dispatcher
        .dispatch_auth_continuation(event.clone())
        .await
        .expect("OAuth continuation resumes the gate");

    let page = inbox
        .list(ListNotificationsRequest {
            recipient: NotificationRecipient {
                tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                user_id: UserId::new("alice").expect("user"),
            },
            limit: 10,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("list notifications");
    assert_eq!(page.notifications.len(), 1);
    assert!(page.notifications[0].resolved_at.is_some());

    coordinator.set_state(run_state(
        run_id,
        TurnStatus::BlockedAuth,
        Some(gate_ref.as_str()),
    ));
    inbox
        .reopen(NotificationMutationRequest {
            recipient: NotificationRecipient {
                tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                user_id: UserId::new("alice").expect("user"),
            },
            notification_id: page.notifications[0].id.clone(),
            occurred_at: Utc::now(),
        })
        .await
        .expect("reopen recurring auth notification");

    dispatcher
        .dispatch_auth_continuation(event)
        .await
        .expect("cached resume replay converges");

    let page = inbox
        .list(ListNotificationsRequest {
            recipient: NotificationRecipient {
                tenant_id: TenantId::new("tenant-auth").expect("tenant"),
                user_id: UserId::new("alice").expect("user"),
            },
            limit: 10,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("list recurring auth notification");
    assert!(
        page.notifications[0].resolved_at.is_none(),
        "a cached resume result cannot settle a newly current instance of the same gate"
    );
}
