//! Regression contract for the durable notification inbox's defining promise:
//! an inbox belongs to one recipient.
//!
//! Isolation here is not a check the store performs; it falls out of deriving
//! the filesystem scope from the recipient, so the production scope resolver
//! (`ironclaw_composition::wrap_scoped`) is the thing under test. The store's
//! own contract suite cannot reach this: its `with_fixed_view` harness pins the
//! mount to a single tenant/user path, so every foreign scope is refused before
//! scoping is consulted and the assertion passes no matter what the resolver
//! does. Drop `user_id` from the scope and only this test notices.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_notifications::{
    ListNotificationsRequest, MarkAllNotificationsReadRequest, NOTIFICATION_INBOX_MAX_RECORDS,
    NotificationAction, NotificationId, NotificationInboxStore, NotificationInboxStorePort,
    NotificationKind, NotificationMutationRequest, NotificationRecipient, NotificationSeverity,
    NotificationSource, PublishNotificationRequest,
};

fn tenant() -> TenantId {
    TenantId::new("notification-isolation-tenant").expect("tenant id")
}

fn recipient(user: &str) -> NotificationRecipient {
    NotificationRecipient {
        tenant_id: tenant(),
        user_id: UserId::new(user).expect("user id"),
    }
}

fn seed(user: &str, id: &str) -> PublishNotificationRequest {
    let thread_id = ThreadId::new(format!("thread-{id}")).expect("thread id");
    PublishNotificationRequest {
        id: NotificationId::new(id).expect("notification id"),
        recipient: recipient(user),
        kind: NotificationKind::ApprovalRequired,
        severity: NotificationSeverity::Warning,
        source: NotificationSource {
            thread_id: thread_id.clone(),
            turn_run_id: None,
            lifecycle_ref: None,
        },
        action: NotificationAction::OpenThread { thread_id },
        occurred_at: Utc.timestamp_opt(1_700_000_001, 0).single().expect("time"),
    }
}

fn production_store() -> NotificationInboxStore<InMemoryBackend> {
    // The same resolver composition hands the real inbox, so the mount view is
    // a function of the caller's scope rather than a fixture constant.
    NotificationInboxStore::new(
        ironclaw_composition::wrap_scoped(Arc::new(InMemoryBackend::new())),
        NOTIFICATION_INBOX_MAX_RECORDS,
    )
}

fn ids(view: &ironclaw_notifications::NotificationPage) -> Vec<String> {
    view.notifications
        .iter()
        .map(|record| record.id.as_str().to_string())
        .collect()
}

async fn list(store: &NotificationInboxStore<InMemoryBackend>, user: &str) -> Vec<String> {
    let view = store
        .list(ListNotificationsRequest {
            recipient: recipient(user),
            limit: 50,
            cursor: None,
            include_archived: true,
        })
        .await
        .unwrap_or_else(|error| panic!("{user} lists their own inbox: {error:?}"));
    ids(&view)
}

#[tokio::test]
async fn a_recipient_sees_only_their_own_notifications() {
    let store = production_store();
    store
        .publish(seed("alice", "alice-1"))
        .await
        .expect("seed alice");
    store.publish(seed("bob", "bob-1")).await.expect("seed bob");

    assert_eq!(list(&store, "alice").await, vec!["alice-1".to_string()]);
    assert_eq!(list(&store, "bob").await, vec!["bob-1".to_string()]);
}

#[tokio::test]
async fn knowing_a_notification_id_is_not_authority_over_it() {
    let store = production_store();
    store
        .publish(seed("alice", "alice-1"))
        .await
        .expect("seed alice");
    store.publish(seed("bob", "bob-1")).await.expect("seed bob");

    // Bob names Alice's record in every mutation the surface exposes.
    let intrusion = NotificationMutationRequest {
        recipient: recipient("bob"),
        notification_id: NotificationId::new("alice-1").expect("notification id"),
        occurred_at: Utc.timestamp_opt(1_700_000_010, 0).single().expect("time"),
    };
    for outcome in [
        store.mark_read(intrusion.clone()).await,
        store.archive(intrusion.clone()).await,
        store.resolve(intrusion.clone()).await,
    ] {
        assert!(
            matches!(
                outcome,
                Err(ironclaw_notifications::NotificationInboxError::NotificationNotFound)
            ),
            "a foreign notification id must be an exact not-found denial: {outcome:?}"
        );
    }
    store
        .mark_all_read(MarkAllNotificationsReadRequest {
            recipient: recipient("bob"),
            occurred_at: Utc.timestamp_opt(1_700_000_011, 0).single().expect("time"),
        })
        .await
        .expect("bob settles his own inbox");

    let alice = store
        .list(ListNotificationsRequest {
            recipient: recipient("alice"),
            limit: 50,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("alice lists her inbox");
    assert_eq!(ids(&alice), vec!["alice-1".to_string()]);
    assert_eq!(
        alice.unread_count, 1,
        "none of bob's calls reached alice's record"
    );
    let record = &alice.notifications[0];
    assert!(record.read_at.is_none());
    assert!(record.archived_at.is_none());
    assert!(record.resolved_at.is_none());
}
