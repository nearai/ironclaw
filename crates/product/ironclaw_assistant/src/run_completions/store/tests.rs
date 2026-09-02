//! Lifecycle tests for the durable notice store, kept in a sibling file so
//! the store module itself stays within the file-size norm
//! (`.claude/rules/architecture.md` §5).

use super::*;
use chrono::Duration as ChronoDuration;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
use ironclaw_host_api::path::{MountAlias, VirtualPath};

fn store() -> RunCompletionNoticeStore<InMemoryBackend> {
    RunCompletionNoticeStore::new(Arc::new(ScopedFilesystem::new(
        Arc::new(InMemoryBackend::new()),
        |scope: &ResourceScope| {
            MountView::new(vec![MountGrant::new(
                MountAlias::new(RUN_NOTICES_MOUNT_ALIAS)?,
                VirtualPath::new(format!(
                    "/tenants/{}/users/{}/run-notices",
                    scope.tenant_id, scope.user_id
                ))?,
                MountPermissions::read_write_list_delete(),
            )])
        },
    )))
}

fn owner(user: &str) -> RunCompletionOwner {
    RunCompletionOwner {
        tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
        user_id: UserId::new(user).expect("user"),
    }
}

fn new_notice(suffix: &str) -> NewRunCompletionNotice {
    NewRunCompletionNotice {
        notice_id: format!("rcn-{suffix}"),
        run_id: format!("run-{suffix}"),
        thread_id: format!("thread-{suffix}"),
        agent_id: Some("agent-alpha".to_string()),
        project_id: None,
        thread_tag: format!("rct-{suffix}"),
        terminal_projection_ref: format!("run-completion/rcn-{suffix}"),
        completed_at: Utc::now(),
        arbitration_closes_at: Utc::now() + ChronoDuration::seconds(1),
    }
}

#[tokio::test]
async fn create_is_idempotent_and_sequences_are_monotonic() {
    let store = store();
    let owner = owner("user-a");

    let first = store
        .create_notice(&owner, new_notice("a"))
        .await
        .expect("first create");
    let NoticeCreateOutcome::Created(first_notice) = first else {
        panic!("first write must create");
    };
    let second = store
        .create_notice(&owner, new_notice("b"))
        .await
        .expect("second create");
    let NoticeCreateOutcome::Created(second_notice) = second else {
        panic!("second write must create");
    };
    assert!(
        second_notice.sequence > first_notice.sequence,
        "the owner sequence is monotonic",
    );

    let replay = store
        .create_notice(&owner, new_notice("a"))
        .await
        .expect("duplicate create");
    let NoticeCreateOutcome::AlreadyRecorded(replayed) = replay else {
        panic!("duplicate journal delivery must rewrite nothing");
    };
    assert_eq!(replayed, first_notice, "the immutable fact is unchanged");
}

#[tokio::test]
async fn read_settles_pending_delivery_but_keeps_the_fact() {
    let store = store();
    let owner = owner("user-a");
    let NoticeCreateOutcome::Created(notice) = store
        .create_notice(&owner, new_notice("a"))
        .await
        .expect("create")
    else {
        panic!("create");
    };

    let read = store
        .mark_read(
            &owner,
            &notice.notice_id,
            CompletionReadEvidence::ReplyRendered {
                browser_instance_id: "browser-1".to_string(),
            },
            Utc::now(),
        )
        .await
        .expect("read transition");
    assert!(read.is_read());
    assert!(matches!(
        read.delivery,
        CompletionDeliveryState::NoExternalTarget { .. }
    ));
    // Read is not deletion: the record remains durable and queryable.
    let reread = store
        .get(&owner, &notice.notice_id)
        .await
        .expect("get")
        .expect("record retained");
    assert!(reread.is_read());
    assert!(
        store
            .unread_snapshot(&owner)
            .await
            .expect("snapshot")
            .is_empty(),
        "read notices leave the unread snapshot",
    );
}

#[tokio::test]
async fn grant_lifecycle_transitions_are_state_checked() {
    let store = store();
    let owner = owner("user-a");
    let NoticeCreateOutcome::Created(notice) = store
        .create_notice(&owner, new_notice("a"))
        .await
        .expect("create")
    else {
        panic!("create");
    };

    let expires = Utc::now() + ChronoDuration::seconds(2);
    store
        .issue_grant(
            &owner,
            &notice.notice_id,
            NewGrant {
                grant_id: "grant-1".to_string(),
                browser_instance_id: "browser-1".to_string(),
                surface: CompletionSurface::InApp,
                state_revision: 41,
                expires_at: expires,
            },
        )
        .await
        .expect("grant issues from pending");
    let double_grant = store
        .issue_grant(
            &owner,
            &notice.notice_id,
            NewGrant {
                grant_id: "grant-2".to_string(),
                browser_instance_id: "browser-2".to_string(),
                surface: CompletionSurface::InApp,
                state_revision: 42,
                expires_at: expires,
            },
        )
        .await;
    assert!(
        matches!(double_grant, Err(RunCompletionStoreError::Conflict { .. })),
        "a second concurrent grant must lose the CAS: {double_grant:?}",
    );

    let mismatched_ack = store
        .acknowledge_presented(&owner, &notice.notice_id, "grant-2", Utc::now())
        .await;
    assert!(matches!(
        mismatched_ack,
        Err(RunCompletionStoreError::Conflict { .. })
    ));
    let presented = store
        .acknowledge_presented(&owner, &notice.notice_id, "grant-1", Utc::now())
        .await
        .expect("matching acknowledgement");
    assert!(matches!(
        presented.delivery,
        CompletionDeliveryState::Presented {
            surface: CompletionSurface::InApp,
            ..
        }
    ));
}

#[tokio::test]
async fn push_ownership_has_exactly_one_winner() {
    let store = Arc::new(store());
    let owner = owner("user-a");
    let NoticeCreateOutcome::Created(notice) = store
        .create_notice(&owner, new_notice("a"))
        .await
        .expect("create")
    else {
        panic!("create");
    };

    let mut winners = 0;
    let mut tasks = tokio::task::JoinSet::new();
    for attempt in 0..8 {
        let store = Arc::clone(&store);
        let owner = owner.clone();
        let notice_id = notice.notice_id.clone();
        tasks.spawn(async move {
            store
                .claim_push(
                    &owner,
                    &notice_id,
                    &format!("delivery-{attempt}"),
                    Utc::now(),
                )
                .await
                .is_ok()
        });
    }
    while let Some(result) = tasks.join_next().await {
        if result.expect("task joins") {
            winners += 1;
        }
    }
    assert_eq!(winners, 1, "only one replica may own the push CAS");
}

#[tokio::test]
async fn listing_queries_are_owner_partitioned_and_ordered() {
    let store = store();
    let owner_a = owner("user-a");
    let owner_b = owner("user-b");
    for suffix in ["a1", "a2", "a3"] {
        store
            .create_notice(&owner_a, new_notice(suffix))
            .await
            .expect("create");
    }
    store
        .create_notice(&owner_b, new_notice("b1"))
        .await
        .expect("create");

    let all_a = store.list_after(&owner_a, None, 250).await.expect("list");
    assert_eq!(all_a.len(), 3, "owner A sees exactly their notices");
    assert!(
        all_a.windows(2).all(|w| w[0].sequence < w[1].sequence),
        "replay is oldest-first",
    );
    let after = store
        .list_after(&owner_a, Some(all_a[0].sequence), 250)
        .await
        .expect("list after");
    assert_eq!(after.len(), 2, "resume excludes the cursor position");

    let unread_b = store.unread_snapshot(&owner_b).await.expect("snapshot");
    assert_eq!(unread_b.len(), 1);
    assert_eq!(unread_b[0].run_id, "run-b1");

    let thread_unread = store
        .unread_for_thread(&owner_a, "thread-a2", 99)
        .await
        .expect("thread unread");
    assert_eq!(thread_unread.len(), 1);

    let pending = store
        .in_delivery_state(
            &owner_a,
            CompletionDeliveryStateKind::PendingArbitration,
            250,
        )
        .await
        .expect("state scan");
    assert_eq!(pending.len(), 3, "boot reconciliation sees pending work");
}

#[tokio::test]
async fn intents_replace_per_browser_and_stay_bounded() {
    let store = store();
    let owner = owner("user-a");
    let NoticeCreateOutcome::Created(notice) = store
        .create_notice(&owner, new_notice("a"))
        .await
        .expect("create")
    else {
        panic!("create");
    };
    let intent = |browser: &str, revision: u64| CompletionIntentRecord {
        browser_instance_id: browser.to_string(),
        tab_id: "tab-1".to_string(),
        state_revision: revision,
        focus_epoch: 1,
        intent: ironclaw_product_contracts::run_completions::RunCompletionIntentKind::InApp,
        offered_at: Utc::now(),
    };

    store
        .record_intent(&owner, &notice.notice_id, intent("browser-1", 1))
        .await
        .expect("first intent");
    let updated = store
        .record_intent(&owner, &notice.notice_id, intent("browser-1", 2))
        .await
        .expect("replacement intent");
    assert_eq!(
        updated.intents.len(),
        1,
        "a newer revision replaces the same profile's intent",
    );
    assert_eq!(updated.intents[0].state_revision, 2);
    // A delayed older report (revision 1 arriving after revision 2) must
    // not roll the profile's intent back to stale focus state.
    let stale = store
        .record_intent(&owner, &notice.notice_id, intent("browser-1", 1))
        .await
        .expect("stale intent is an idempotent no-op");
    assert_eq!(stale.intents.len(), 1);
    assert_eq!(
        stale.intents[0].state_revision, 2,
        "an older revision never replaces a newer one"
    );

    for extra in 0..(RUN_COMPLETION_MAX_INTENTS_PER_NOTICE - 1) {
        store
            .record_intent(
                &owner,
                &notice.notice_id,
                intent(&format!("browser-extra-{extra}"), 1),
            )
            .await
            .expect("intent within budget");
    }
    let overflow = store
        .record_intent(&owner, &notice.notice_id, intent("browser-overflow", 1))
        .await;
    assert!(
        matches!(overflow, Err(RunCompletionStoreError::Invalid { .. })),
        "the per-notice intent budget is enforced: {overflow:?}",
    );
}
