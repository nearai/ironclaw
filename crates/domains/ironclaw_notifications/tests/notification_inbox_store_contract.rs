#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError, Filter,
    InMemoryBackend, IndexSpec, LibSqlRootFilesystem, Page, RecordVersion, RootFilesystem,
    ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    ids::{TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    turn::TurnRunId,
};
use ironclaw_notifications::{
    LifecycleRef, ListNotificationsRequest, MarkAllNotificationsReadRequest,
    NOTIFICATION_INBOX_MAX_RECORDS, NOTIFICATION_PAGE_LIMIT_MAX, NoopNotificationInboxStore,
    NotificationAction, NotificationId, NotificationInboxError, NotificationInboxStore,
    NotificationInboxStorePort, NotificationInitialState, NotificationKind,
    NotificationMutationOutcome, NotificationMutationRequest, NotificationRecipient,
    NotificationSeverity, NotificationSource, PublishNotificationRequest,
};
use tokio::sync::Mutex;

const TEST_ROOT: &str = "/engine/tenants/test/users/test/notifications";

/// The bound the capacity tests configure. Publishing to the ceiling is O(n^2)
/// in the snapshot — every CAS round-trip re-encodes every record — so proving
/// the contract at the production ceiling costs minutes without saying anything
/// the bound below does not.
const TEST_CAPACITY: usize = 8;

fn scoped<F: RootFilesystem>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/notifications").expect("alias"),
        VirtualPath::new(TEST_ROOT).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn recipient() -> NotificationRecipient {
    NotificationRecipient {
        tenant_id: TenantId::new("test").expect("tenant"),
        user_id: UserId::new("test").expect("user"),
    }
}

fn request(id: &str, timestamp: i64) -> PublishNotificationRequest {
    let thread_id = ThreadId::new(format!("thread-{id}")).expect("thread");
    PublishNotificationRequest {
        id: NotificationId::new(id).expect("id"),
        recipient: recipient(),
        kind: NotificationKind::ApprovalRequired,
        severity: NotificationSeverity::Warning,
        source: NotificationSource {
            thread_id: thread_id.clone(),
            turn_run_id: Some(TurnRunId::new()),
            lifecycle_ref: Some(LifecycleRef::new(format!("gate-{id}")).expect("lifecycle ref")),
        },
        action: NotificationAction::OpenThread { thread_id },
        initial_state: NotificationInitialState::Open,
        occurred_at: Utc.timestamp_opt(timestamp, 0).single().expect("time"),
    }
}

#[tokio::test]
async fn notification_inbox_is_durable_paginated_and_idempotent() {
    let backend = Arc::new(InMemoryBackend::new());
    let first =
        NotificationInboxStore::new(scoped(Arc::clone(&backend)), NOTIFICATION_INBOX_MAX_RECORDS);
    let first_request = request("notification-1", 1_700_000_001);
    first
        .publish(first_request.clone())
        .await
        .expect("publish first");

    let mut actionable_state_change = first_request.clone();
    actionable_state_change.initial_state = NotificationInitialState::Resolved;
    let actionable_retry = first
        .publish(actionable_state_change)
        .await
        .expect("actionable retry remains idempotent");
    assert!(
        actionable_retry.resolved_at.is_none(),
        "an actionable producer retry cannot bypass workflow resolution",
    );

    let mut severity_conflict = first_request.clone();
    severity_conflict.severity = NotificationSeverity::Error;
    assert!(matches!(
        first.publish(severity_conflict).await,
        Err(NotificationInboxError::InvalidRequest { .. })
    ));
    let unchanged = first
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("read after conflicting publish");
    assert_eq!(unchanged.notifications.len(), 1);
    assert_eq!(
        unchanged.notifications[0].severity,
        NotificationSeverity::Warning
    );
    assert_eq!(
        unchanged.notifications[0].updated_at,
        first_request.occurred_at
    );

    let mut kind_conflict = first_request.clone();
    kind_conflict.kind = NotificationKind::RunBlocked;
    assert!(matches!(
        first.publish(kind_conflict).await,
        Err(NotificationInboxError::InvalidRequest { .. })
    ));
    first
        .publish(first_request.clone())
        .await
        .expect("idempotent retry");

    let mut delayed_retry = first_request;
    delayed_retry.occurred_at = Utc.timestamp_opt(1_700_000_100, 0).single().expect("time");
    first
        .publish(delayed_retry)
        .await
        .expect("delayed idempotent retry");
    first
        .publish(request("notification-2", 1_700_000_002))
        .await
        .expect("publish second");

    let reopened = NotificationInboxStore::new(scoped(backend), NOTIFICATION_INBOX_MAX_RECORDS);
    let page = reopened
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 1,
            cursor: None,
            include_archived: false,
        })
        .await
        .expect("list first page");
    assert_eq!(page.notifications.len(), 1);
    assert_eq!(page.notifications[0].id.as_str(), "notification-2");
    assert_eq!(page.unread_count, 2);
    let cursor = page.next_cursor.expect("second page cursor");
    reopened
        .archive(NotificationMutationRequest {
            recipient: recipient(),
            notification_id: NotificationId::new("notification-2").expect("id"),
            occurred_at: Utc.timestamp_opt(1_700_000_010, 0).single().expect("time"),
        })
        .await
        .expect("archive page anchor");
    let second_page = reopened
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 1,
            cursor: Some(cursor),
            include_archived: false,
        })
        .await
        .expect("resume after archived anchor");
    assert_eq!(second_page.notifications[0].id.as_str(), "notification-1");
}

#[tokio::test]
async fn terminal_retry_repairs_legacy_open_state_without_reopening_lifecycle() {
    let backend = Arc::new(InMemoryBackend::new());
    let first = NotificationInboxStore::new(scoped(Arc::clone(&backend)), 1);
    let mut terminal = request("legacy-terminal", 1_700_000_001);
    terminal.kind = NotificationKind::RunCompleted;
    terminal.severity = NotificationSeverity::Success;
    // This is the pre-initial-state persisted shape: terminal producers used
    // to create records without stamping `resolved_at`.
    terminal.initial_state = NotificationInitialState::Open;
    first
        .publish(terminal.clone())
        .await
        .expect("publish legacy terminal record");

    let read_at = Utc.timestamp_opt(1_700_000_010, 0).single().expect("time");
    let archived_at = Utc.timestamp_opt(1_700_000_020, 0).single().expect("time");
    let notification_id = terminal.id.clone();
    first
        .mark_read(NotificationMutationRequest {
            recipient: recipient(),
            notification_id: notification_id.clone(),
            occurred_at: read_at,
        })
        .await
        .expect("mark legacy record read");
    first
        .archive(NotificationMutationRequest {
            recipient: recipient(),
            notification_id: notification_id.clone(),
            occurred_at: archived_at,
        })
        .await
        .expect("archive legacy record");

    let reopened = NotificationInboxStore::new(scoped(Arc::clone(&backend)), 1);
    let before = reopened
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("read legacy record before reconciliation")
        .notifications
        .into_iter()
        .next()
        .expect("legacy record");
    assert!(before.resolved_at.is_none());

    terminal.initial_state = NotificationInitialState::Resolved;
    terminal.occurred_at = Utc.timestamp_opt(1_700_000_030, 0).single().expect("time");
    let reconciled = reopened
        .publish(terminal)
        .await
        .expect("terminal retry reconciles legacy state");
    assert_eq!(reconciled.resolved_at, Some(before.created_at));
    assert_eq!(reconciled.read_at, before.read_at);
    assert_eq!(reconciled.archived_at, before.archived_at);
    assert_eq!(reconciled.updated_at, before.updated_at);
    assert_eq!(
        reopened
            .reopen(NotificationMutationRequest {
                recipient: recipient(),
                notification_id,
                occurred_at: Utc.timestamp_opt(1_700_000_031, 0).single().expect("time"),
            })
            .await
            .expect("terminal notifications cannot be reopened"),
        NotificationMutationOutcome::AlreadySettled,
    );

    reopened
        .publish(request("gate-after-legacy-terminal", 1_700_000_040))
        .await
        .expect("reconciled terminal record is reclaimable");
}

#[tokio::test]
async fn notification_lifecycle_is_scoped_archivable_and_idempotent() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = NotificationInboxStore::new(scoped(backend), NOTIFICATION_INBOX_MAX_RECORDS);
    store
        .publish(request("notification-lifecycle", 1_700_000_001))
        .await
        .expect("publish notification");
    store
        .publish(request("notification-unread", 1_700_000_002))
        .await
        .expect("publish unread notification");
    store
        .publish(request("notification-archived", 1_700_000_003))
        .await
        .expect("publish archived notification");

    let read_at = Utc.timestamp_opt(1_700_000_010, 0).single().expect("time");
    let lifecycle = NotificationMutationRequest {
        recipient: recipient(),
        notification_id: NotificationId::new("notification-lifecycle").expect("id"),
        occurred_at: read_at,
    };
    store.mark_read(lifecycle.clone()).await.expect("mark read");
    store
        .mark_read(lifecycle.clone())
        .await
        .expect("idempotent mark read");
    store
        .resolve(lifecycle.clone())
        .await
        .expect("resolve notification");
    assert_eq!(
        store
            .reopen(lifecycle.clone())
            .await
            .expect("reopen authoritative actionable notification"),
        NotificationMutationOutcome::Applied,
    );
    assert_eq!(
        store
            .reopen(lifecycle.clone())
            .await
            .expect("idempotent reopen"),
        NotificationMutationOutcome::AlreadySettled,
    );
    let reopened_page = store
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("list reopened notification");
    let reopened_lifecycle = reopened_page
        .notifications
        .iter()
        .find(|record| record.id.as_str() == "notification-lifecycle")
        .expect("reopened lifecycle notification");
    assert_eq!(reopened_lifecycle.resolved_at, None);
    assert_eq!(
        reopened_lifecycle.read_at,
        Some(read_at),
        "reopen must not clear read state"
    );
    store
        .resolve(lifecycle)
        .await
        .expect("resolve reopened notification");

    store
        .resolve(NotificationMutationRequest {
            recipient: recipient(),
            notification_id: NotificationId::new("notification-unread").expect("id"),
            occurred_at: read_at,
        })
        .await
        .expect("resolve unread notification");
    let resolved_page = store
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: false,
        })
        .await
        .expect("list resolved notification");
    let resolved_unread = resolved_page
        .notifications
        .iter()
        .find(|record| record.id.as_str() == "notification-unread")
        .expect("resolved unread notification");
    assert_eq!(resolved_unread.read_at, None, "resolve must not imply read");
    assert_eq!(resolved_unread.resolved_at, Some(read_at));

    let archived_at = Utc.timestamp_opt(1_700_000_020, 0).single().expect("time");
    let archived = NotificationMutationRequest {
        recipient: recipient(),
        notification_id: NotificationId::new("notification-archived").expect("id"),
        occurred_at: archived_at,
    };
    store
        .resolve(archived.clone())
        .await
        .expect("resolve notification before archive");
    store
        .archive(archived.clone())
        .await
        .expect("archive notification");
    store
        .reopen(archived)
        .await
        .expect("reopen archived notification");
    store
        .mark_all_read(MarkAllNotificationsReadRequest {
            recipient: recipient(),
            occurred_at: Utc.timestamp_opt(1_700_000_030, 0).single().expect("time"),
        })
        .await
        .expect("mark visible notifications read");

    let visible = store
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: false,
        })
        .await
        .expect("list visible");
    assert_eq!(visible.notifications.len(), 2);
    assert_eq!(visible.unread_count, 0);
    let lifecycle = visible
        .notifications
        .iter()
        .find(|record| record.id.as_str() == "notification-lifecycle")
        .expect("lifecycle notification");
    assert_eq!(lifecycle.read_at, Some(read_at));
    assert!(lifecycle.resolved_at.is_some());

    let all = store
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("list archived");
    let archived = all
        .notifications
        .iter()
        .find(|record| record.id.as_str() == "notification-archived")
        .expect("archived notification");
    assert_eq!(archived.archived_at, Some(archived_at));
    assert_eq!(
        archived.resolved_at, None,
        "reopen must preserve archive state while restoring actionability"
    );
    assert_eq!(archived.read_at, None, "archive must not imply read");

    let foreign = NotificationRecipient {
        tenant_id: recipient().tenant_id,
        user_id: UserId::new("foreign").expect("user"),
    };
    assert!(matches!(
        store
            .list(ListNotificationsRequest {
                recipient: foreign,
                limit: 10,
                cursor: None,
                include_archived: false,
            })
            .await,
        Err(NotificationInboxError::AccessDenied)
    ));
    assert!(matches!(
        store
            .mark_read(NotificationMutationRequest {
                recipient: recipient(),
                notification_id: NotificationId::new("missing").expect("id"),
                occurred_at: read_at,
            })
            .await,
        Err(NotificationInboxError::NotificationNotFound)
    ));
}

#[tokio::test]
async fn unwired_notification_store_fails_closed_for_reads_and_writes() {
    let store = NoopNotificationInboxStore;
    let list_error = store
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: false,
        })
        .await
        .expect_err("an unwired inbox must not look empty");
    assert!(matches!(list_error, NotificationInboxError::Backend { .. }));

    let publish_error = store
        .publish(request("notification-unwired", 1_700_000_001))
        .await
        .expect_err("an unwired inbox rejects writes");
    assert!(matches!(
        publish_error,
        NotificationInboxError::Backend { .. }
    ));
}

#[tokio::test]
async fn a_lowered_bound_drains_the_snapshot_instead_of_locking_the_recipient_out() {
    let backend = Arc::new(InMemoryBackend::new());
    let roomy = NotificationInboxStore::new(scoped(Arc::clone(&backend)), TEST_CAPACITY * 3);

    // Fill and close well past the smaller bound this recipient will be reopened
    // with, the way lowering the configured capacity would leave a live inbox.
    for index in 0..TEST_CAPACITY * 3 {
        let id = format!("closed-{index}");
        roomy
            .publish(request(&id, 1_700_000_000 + index as i64))
            .await
            .expect("publish under the roomy bound");
        let mutation = NotificationMutationRequest {
            recipient: recipient(),
            notification_id: NotificationId::new(id.as_str()).expect("id"),
            occurred_at: Utc
                .timestamp_opt(1_700_500_000 + index as i64, 0)
                .single()
                .expect("time"),
        };
        roomy.resolve(mutation.clone()).await.expect("resolve");
        roomy.archive(mutation).await.expect("archive");
    }

    let tightened = NotificationInboxStore::new(scoped(Arc::clone(&backend)), TEST_CAPACITY);

    // Reading never rejects an over-bound snapshot: a lowered bound must not turn
    // a configuration change into a locked-out inbox.
    let before = tightened
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: NOTIFICATION_PAGE_LIMIT_MAX,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("an over-bound snapshot stays readable");
    assert_eq!(before.notifications.len(), TEST_CAPACITY * 3);

    // Publishing drains to the active bound rather than shedding one record per
    // call, so the tightened bound actually takes effect.
    let arrival = request("gate-after-tightening", 1_800_000_000);
    let arrival_id = arrival.id.clone();
    tightened
        .publish(arrival)
        .await
        .expect("the new gate is admitted");

    let after = tightened
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: NOTIFICATION_PAGE_LIMIT_MAX,
            cursor: None,
            include_archived: true,
        })
        .await
        .expect("list after draining");
    assert_eq!(
        after.notifications.len(),
        TEST_CAPACITY,
        "the snapshot converges on the active bound instead of staying over it"
    );
    assert!(
        after
            .notifications
            .iter()
            .any(|record| record.id == arrival_id),
        "the arrival that triggered the drain is one of the survivors"
    );
    assert!(
        after
            .notifications
            .iter()
            .all(|record| record.id.as_str() != "closed-0"),
        "draining sheds the oldest closed records first"
    );
}

#[tokio::test]
async fn notification_inbox_enforces_limits_and_bounds_cas_retries() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = NotificationInboxStore::new(scoped(Arc::clone(&backend)), TEST_CAPACITY);
    for limit in [0, NOTIFICATION_PAGE_LIMIT_MAX + 1] {
        assert!(matches!(
            store
                .list(ListNotificationsRequest {
                    recipient: recipient(),
                    limit,
                    cursor: None,
                    include_archived: false,
                })
                .await,
            Err(NotificationInboxError::InvalidRequest { .. })
        ));
    }

    for index in 0..TEST_CAPACITY {
        store
            .publish(request(
                &format!("notification-capacity-{index}"),
                1_700_001_000 + index as i64,
            ))
            .await
            .expect("publish within capacity");
    }
    store
        .archive(NotificationMutationRequest {
            recipient: recipient(),
            notification_id: NotificationId::new("notification-capacity-0").expect("id"),
            occurred_at: Utc.timestamp_opt(1_700_009_000, 0).single().expect("time"),
        })
        .await
        .expect("archive one capacity record");
    assert!(matches!(
        store
            .publish(request("notification-capacity-overflow", 1_700_009_001))
            .await,
        Err(NotificationInboxError::InvalidRequest { .. })
    ));

    let racing = Arc::new(VersionRacingBackend::new(Arc::new(InMemoryBackend::new())));
    let racing_store = NotificationInboxStore::new(scoped(Arc::clone(&racing)), TEST_CAPACITY);
    racing.arm(TEST_ROOT, 1).await;
    racing_store
        .publish(request("notification-cas-retry", 1_700_010_000))
        .await
        .expect("retry transient CAS conflict");
    assert_eq!(racing.injected_count().await, 1);

    racing.arm(TEST_ROOT, u32::MAX).await;
    assert!(matches!(
        racing_store
            .publish(request("notification-cas-exhausted", 1_700_010_001))
            .await,
        Err(NotificationInboxError::Backend { .. })
    ));
    let surviving = racing_store
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: false,
        })
        .await
        .expect("surviving page");
    assert_eq!(surviving.notifications.len(), 1);
}

#[tokio::test]
async fn a_repeated_mutation_reports_that_nothing_changed() {
    let backend = Arc::new(InMemoryBackend::new());
    let store =
        NotificationInboxStore::new(scoped(Arc::clone(&backend)), NOTIFICATION_INBOX_MAX_RECORDS);
    store
        .publish(request("settled", 1_700_000_000))
        .await
        .expect("publish");
    let mutation = NotificationMutationRequest {
        recipient: recipient(),
        notification_id: NotificationId::new("settled").expect("id"),
        occurred_at: Utc.timestamp_opt(1_700_000_500, 0).single().expect("time"),
    };

    // The first call changes durable state; the repeat is a no-op inside CAS.
    // Both succeed, so success alone cannot stand in for evidence of a write.
    for (label, first, again) in [
        (
            "mark_read",
            store.mark_read(mutation.clone()),
            store.mark_read(mutation.clone()),
        ),
        (
            "resolve",
            store.resolve(mutation.clone()),
            store.resolve(mutation.clone()),
        ),
        (
            "archive",
            store.archive(mutation.clone()),
            store.archive(mutation.clone()),
        ),
    ] {
        assert_eq!(
            first.await.expect(label),
            NotificationMutationOutcome::Applied,
            "{label} changed durable state"
        );
        assert_eq!(
            again.await.expect(label),
            NotificationMutationOutcome::AlreadySettled,
            "{label} repeated is reported as unchanged"
        );
    }

    assert_eq!(
        store
            .mark_all_read(MarkAllNotificationsReadRequest {
                recipient: recipient(),
                occurred_at: Utc.timestamp_opt(1_700_001_000, 0).single().expect("time"),
            })
            .await
            .expect("mark all read"),
        NotificationMutationOutcome::AlreadySettled,
        "nothing is unread, so mark-all-read reports no change"
    );
}

#[tokio::test]
async fn a_full_inbox_evicts_closed_records_so_a_new_gate_still_arrives() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = NotificationInboxStore::new(scoped(Arc::clone(&backend)), TEST_CAPACITY);

    // Terminal outcomes are resolved when published. Archiving them must make
    // them reclaimable without requiring a second producer lifecycle event.
    for index in 0..TEST_CAPACITY {
        let id = format!("closed-{index}");
        let mut terminal = request(&id, 1_700_000_000 + index as i64);
        terminal.kind = NotificationKind::RunCompleted;
        terminal.severity = NotificationSeverity::Success;
        terminal.initial_state = NotificationInitialState::Resolved;
        store
            .publish(terminal)
            .await
            .expect("publish within capacity");
        let mutation = NotificationMutationRequest {
            recipient: recipient(),
            notification_id: NotificationId::new(id.as_str()).expect("id"),
            occurred_at: Utc
                .timestamp_opt(1_700_500_000 + index as i64, 0)
                .single()
                .expect("time"),
        };
        store.archive(mutation).await.expect("archive");
    }

    let arrival = request("gate-after-capacity", 1_800_000_000);
    let arrival_id = arrival.id.clone();
    store
        .publish(arrival)
        .await
        .expect("a closed record is evicted so the newest gate still arrives");

    let page = store
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 1,
            cursor: None,
            include_archived: false,
        })
        .await
        .expect("list");
    assert_eq!(
        page.notifications.first().map(|record| record.id.clone()),
        Some(arrival_id),
        "the newest gate notification is the visible one"
    );
    assert_eq!(page.unread_count, 1, "only the new arrival is unread");

    // Which record was reclaimed matters: dropping the newest closed record
    // instead of the oldest would satisfy the capacity check just as well.
    // The list is newest-first, so the oldest ids sit on the last page.
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = store
            .list(ListNotificationsRequest {
                recipient: recipient(),
                limit: NOTIFICATION_PAGE_LIMIT_MAX,
                cursor,
                include_archived: true,
            })
            .await
            .expect("list including archived");
        ids.extend(
            page.notifications
                .iter()
                .map(|record| record.id.as_str().to_string()),
        );
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    assert!(
        !ids.iter().any(|id| id == "closed-0"),
        "the oldest closed record is the one reclaimed"
    );
    assert!(
        ids.iter().any(|id| id == "closed-1"),
        "the next-oldest closed record survives"
    );
    assert_eq!(
        ids.len(),
        TEST_CAPACITY,
        "reclaiming keeps the snapshot at its bound rather than growing past it"
    );
}

#[tokio::test]
async fn a_retry_for_a_reclaimed_id_is_a_new_record_not_a_duplicate() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = NotificationInboxStore::new(scoped(Arc::clone(&backend)), TEST_CAPACITY);

    for index in 0..TEST_CAPACITY {
        let id = format!("closed-{index}");
        store
            .publish(request(&id, 1_700_000_000 + index as i64))
            .await
            .expect("publish within capacity");
        let mutation = NotificationMutationRequest {
            recipient: recipient(),
            notification_id: NotificationId::new(id.as_str()).expect("id"),
            occurred_at: Utc
                .timestamp_opt(1_700_500_000 + index as i64, 0)
                .single()
                .expect("time"),
        };
        store.resolve(mutation.clone()).await.expect("resolve");
        store.archive(mutation).await.expect("archive");
    }
    store
        .publish(request("gate-after-capacity", 1_800_000_000))
        .await
        .expect("reclaim");

    // This is the acknowledged edge of the idempotency window, pinned so the
    // behaviour is a decision rather than a surprise: `closed-0` left the
    // snapshot, so a producer retry for it can no longer be recognised as a
    // duplicate and lands as a fresh unread record.
    let republished = store
        .publish(request("closed-0", 1_800_000_001))
        .await
        .expect("a reclaimed id is publishable again");
    assert!(
        republished.read_at.is_none() && republished.archived_at.is_none(),
        "the retry arrives as a new record, not the reclaimed one's lifecycle"
    );
}

#[tokio::test]
async fn a_full_inbox_of_open_records_still_refuses_rather_than_dropping_one() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = NotificationInboxStore::new(scoped(Arc::clone(&backend)), TEST_CAPACITY);

    // Nothing is archived, so there is no closed record to reclaim. Refusing is
    // correct here: evicting an open record would lose an actionable gate.
    for index in 0..TEST_CAPACITY {
        store
            .publish(request(
                &format!("open-{index}"),
                1_700_000_000 + index as i64,
            ))
            .await
            .expect("publish within capacity");
    }

    assert!(matches!(
        store.publish(request("overflow", 1_800_000_000)).await,
        Err(NotificationInboxError::InvalidRequest { .. })
    ));
}

#[tokio::test]
async fn a_notification_cannot_point_its_action_at_another_thread() {
    let backend = Arc::new(InMemoryBackend::new());
    let store =
        NotificationInboxStore::new(scoped(Arc::clone(&backend)), NOTIFICATION_INBOX_MAX_RECORDS);

    let mut mismatched = request("action-mismatch", 1_700_000_000);
    mismatched.action = NotificationAction::OpenThread {
        thread_id: ThreadId::new("thread-somebody-else").expect("thread"),
    };
    assert!(
        matches!(
            store.publish(mismatched).await,
            Err(NotificationInboxError::InvalidRequest { .. })
        ),
        "an action thread that differs from the source thread is rejected"
    );
}

#[tokio::test]
async fn a_lifecycle_reference_is_bounded_and_free_of_control_characters() {
    assert!(LifecycleRef::new("gate-1").is_ok());
    assert!(LifecycleRef::new(String::new()).is_err(), "empty");
    assert!(LifecycleRef::new("gate\n1").is_err(), "control character");
    assert!(LifecycleRef::new("x".repeat(512)).is_ok(), "at the bound");
    assert!(
        LifecycleRef::new("x".repeat(513)).is_err(),
        "past the bound"
    );
}

#[tokio::test]
async fn notification_inbox_persists_across_libsql_reopen() {
    let directory = tempfile::tempdir().expect("temporary libSQL directory");
    let database_path = directory.path().join("notification-inbox.db");
    {
        let database = Arc::new(
            libsql::Builder::new_local(&database_path)
                .build()
                .await
                .expect("build database"),
        );
        let root = Arc::new(LibSqlRootFilesystem::new(database).expect("build root filesystem"));
        root.run_migrations().await.expect("run migrations");
        let store = NotificationInboxStore::new(scoped(root), NOTIFICATION_INBOX_MAX_RECORDS);
        store
            .publish(request("notification-libsql", 1_700_000_001))
            .await
            .expect("persist notification");
    }

    let database = Arc::new(
        libsql::Builder::new_local(&database_path)
            .build()
            .await
            .expect("reopen database"),
    );
    let root = Arc::new(LibSqlRootFilesystem::new(database).expect("reopen root filesystem"));
    root.run_migrations().await.expect("rerun migrations");
    let reopened = NotificationInboxStore::new(scoped(root), NOTIFICATION_INBOX_MAX_RECORDS);
    let page = reopened
        .list(ListNotificationsRequest {
            recipient: recipient(),
            limit: 10,
            cursor: None,
            include_archived: false,
        })
        .await
        .expect("read reopened notification");
    assert_eq!(page.unread_count, 1);
    assert_eq!(page.notifications[0].id.as_str(), "notification-libsql");
}

struct VersionRacingBackend {
    inner: Arc<InMemoryBackend>,
    state: Mutex<RacingState>,
}

struct RacingState {
    target_prefix: Option<String>,
    injected: u32,
    remaining: u32,
}

impl VersionRacingBackend {
    fn new(inner: Arc<InMemoryBackend>) -> Self {
        Self {
            inner,
            state: Mutex::new(RacingState {
                target_prefix: None,
                injected: 0,
                remaining: 0,
            }),
        }
    }

    async fn arm(&self, prefix: &str, count: u32) {
        let mut state = self.state.lock().await;
        state.target_prefix = Some(prefix.to_string());
        state.injected = 0;
        state.remaining = count;
    }

    async fn injected_count(&self) -> u32 {
        self.state.lock().await.injected
    }
}

#[async_trait]
impl RootFilesystem for VersionRacingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        {
            let mut state = self.state.lock().await;
            if state.remaining > 0
                && state
                    .target_prefix
                    .as_deref()
                    .is_some_and(|prefix| path.as_str().starts_with(prefix))
            {
                state.remaining -= 1;
                state.injected += 1;
                return Err(FilesystemError::VersionMismatch {
                    path: path.clone(),
                    expected: None,
                    found: None,
                });
            }
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}
