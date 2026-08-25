//! Regression contract for the durable notification inbox's defining promise:
//! an inbox belongs to one recipient.
//!
//! Isolation here is not a check the store performs; it falls out of deriving
//! the filesystem scope from the recipient, so the production scope resolver
//! (`ironclaw_composition::wrap_scoped`) is the thing under test. The store's
//! own contract suite cannot reach this: its `with_fixed_view` harness pins the
//! mount to a single tenant/user path, so every foreign scope is refused before
//! scoping is consulted and the assertion passes no matter what the resolver
//! does. Drop either half of the recipient — `tenant_id` or `user_id` — from
//! the scope and only this test notices.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_notifications::{
    ListNotificationsRequest, MarkAllNotificationsReadRequest, NOTIFICATION_INBOX_MAX_RECORDS,
    NotificationAction, NotificationId, NotificationInboxStore, NotificationInboxStorePort,
    NotificationInitialState, NotificationKind, NotificationMutationRequest, NotificationRecipient,
    NotificationSeverity, NotificationSource, PublishNotificationRequest,
};

const TENANT: &str = "notification-isolation-tenant";

fn tenant(tenant: &str) -> TenantId {
    TenantId::new(tenant).expect("tenant id")
}

fn recipient_in(tenant_id: &str, user: &str) -> NotificationRecipient {
    NotificationRecipient {
        tenant_id: tenant(tenant_id),
        user_id: UserId::new(user).expect("user id"),
    }
}

fn recipient(user: &str) -> NotificationRecipient {
    recipient_in(TENANT, user)
}

fn seed_in(tenant_id: &str, user: &str, id: &str) -> PublishNotificationRequest {
    let thread_id = ThreadId::new(format!("thread-{id}")).expect("thread id");
    PublishNotificationRequest {
        id: NotificationId::new(id).expect("notification id"),
        recipient: recipient_in(tenant_id, user),
        kind: NotificationKind::ApprovalRequired,
        severity: NotificationSeverity::Warning,
        source: NotificationSource {
            thread_id: thread_id.clone(),
            turn_run_id: None,
            lifecycle_ref: None,
        },
        action: NotificationAction::OpenThread { thread_id },
        initial_state: NotificationInitialState::Open,
        occurred_at: Utc.timestamp_opt(1_700_000_001, 0).single().expect("time"),
    }
}

fn seed(user: &str, id: &str) -> PublishNotificationRequest {
    seed_in(TENANT, user, id)
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

async fn list_in(
    store: &NotificationInboxStore<InMemoryBackend>,
    tenant_id: &str,
    user: &str,
) -> Vec<String> {
    let view = store
        .list(ListNotificationsRequest {
            recipient: recipient_in(tenant_id, user),
            limit: 50,
            cursor: None,
            include_archived: true,
        })
        .await
        .unwrap_or_else(|error| panic!("{user} lists their own inbox: {error:?}"));
    ids(&view)
}

async fn list(store: &NotificationInboxStore<InMemoryBackend>, user: &str) -> Vec<String> {
    list_in(store, TENANT, user).await
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

#[tokio::test]
async fn the_same_user_id_in_two_tenants_gets_two_inboxes() {
    // A resolver that scopes by user but forgets the tenant passes every other
    // test in this file: they all sit inside one tenant, so `user_id` alone is
    // enough to tell the fixtures apart. Only a shared user id across tenants
    // makes the missing half observable.
    const OTHER_TENANT: &str = "notification-isolation-other-tenant";
    let store = production_store();
    store
        .publish(seed_in(TENANT, "carol", "home-1"))
        .await
        .expect("seed carol in her own tenant");
    store
        .publish(seed_in(OTHER_TENANT, "carol", "guest-1"))
        .await
        .expect("seed the same user id in another tenant");

    assert_eq!(
        list_in(&store, TENANT, "carol").await,
        vec!["home-1".to_string()],
        "a tenant lists only its own record"
    );
    assert_eq!(
        list_in(&store, OTHER_TENANT, "carol").await,
        vec!["guest-1".to_string()],
        "and the other tenant only its own"
    );

    // Naming the other tenant's record must be a denial, not a cross-tenant
    // mutation that happens to match on user id.
    let intrusion = NotificationMutationRequest {
        recipient: recipient_in(OTHER_TENANT, "carol"),
        notification_id: NotificationId::new("home-1").expect("notification id"),
        occurred_at: Utc.timestamp_opt(1_700_000_020, 0).single().expect("time"),
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
            "a record from another tenant must be an exact not-found denial: {outcome:?}"
        );
    }

    store
        .mark_all_read(MarkAllNotificationsReadRequest {
            recipient: recipient_in(OTHER_TENANT, "carol"),
            occurred_at: Utc.timestamp_opt(1_700_000_021, 0).single().expect("time"),
        })
        .await
        .expect("the guest tenant settles its own inbox");

    let home = store
        .list(ListNotificationsRequest {
            recipient: recipient_in(TENANT, "carol"),
            limit: 50,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("the home tenant lists its inbox");
    assert_eq!(ids(&home), vec!["home-1".to_string()]);
    assert_eq!(
        home.unread_count, 1,
        "nothing the other tenant did reached this record"
    );
    let record = &home.notifications[0];
    assert!(record.read_at.is_none());
    assert!(record.archived_at.is_none());
    assert!(record.resolved_at.is_none());
}
