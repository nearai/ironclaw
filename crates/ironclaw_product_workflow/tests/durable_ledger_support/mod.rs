#![allow(dead_code)]

use chrono::{Duration, Utc};
use ironclaw_host_api::VirtualPath;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalEventId, ProductAdapterId, ProductInboundAck,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, IdempotencyDecision, IdempotencyLedger, ProductInboundAction,
    ProductWorkflowError, SourceBindingKey,
};

pub fn fingerprint(suffix: &str) -> ActionFingerprintKey {
    fingerprint_for_actor(suffix, "user1")
}

pub fn fingerprint_for_actor(suffix: &str, actor_id: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid adapter"),
        AdapterInstallationId::new("install_alpha").expect("valid installation"),
        ExternalActorRef::new("test", actor_id, Option::<String>::None).expect("valid actor"),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new(format!("evt:{suffix}")).expect("valid event"),
    )
}

pub fn custom_root(suffix: &str) -> VirtualPath {
    VirtualPath::new(format!(
        "/engine/product_workflow/idempotency/test_roots/{suffix}"
    ))
    .expect("valid custom ledger root")
}

pub async fn assert_settled_action_survives_reopen_and_replays(
    ledger: &dyn IdempotencyLedger,
    reopened: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let decision = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin");
    let IdempotencyDecision::New(mut action) = decision else {
        panic!("expected new action");
    };
    action.settle(ProductInboundAck::NoOp);
    ledger.settle(action).await.expect("settle");

    let replay = reopened
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("replay");

    let IdempotencyDecision::Replay(action) = replay else {
        panic!("expected replay");
    };
    assert_eq!(action.outcome, Some(ProductInboundAck::NoOp));
}

pub async fn assert_in_flight_action_blocks_until_lease_expires(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("begin"),
        IdempotencyDecision::New(_)
    ));
    let blocked = ledger
        .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(5))
        .await
        .expect_err("fresh reservation should block");
    assert!(matches!(blocked, ProductWorkflowError::Transient { .. }));

    let reclaimed = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
        .await
        .expect("expired reservation should be reclaimed");
    assert!(matches!(reclaimed, IdempotencyDecision::New(_)));
}

pub async fn assert_release_allows_retry_without_waiting_for_lease(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let decision = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin");
    let IdempotencyDecision::New(action) = decision else {
        panic!("expected new action");
    };
    ledger.release(action).await.expect("release");

    let retry = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("retry after release");
    assert!(matches!(retry, IdempotencyDecision::New(_)));
}

pub async fn assert_duplicate_reservation_contention_serializes(
    first: &dyn IdempotencyLedger,
    second: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let (left, right) = tokio::join!(
        first.begin_or_replay(fingerprint.clone(), received_at),
        second.begin_or_replay(fingerprint, received_at),
    );
    let results = [left, right];
    let new_count = results
        .iter()
        .filter(|result| matches!(result, Ok(IdempotencyDecision::New(_))))
        .count();
    let blocked_count = results
        .iter()
        .filter(|result| matches!(result, Err(ProductWorkflowError::Transient { .. })))
        .count();

    assert_eq!(new_count, 1);
    assert_eq!(blocked_count, 1);
}

pub async fn assert_superseded_reservation_cannot_settle(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let IdempotencyDecision::New(mut stale_action) = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin")
    else {
        panic!("expected new action");
    };

    let IdempotencyDecision::New(mut replacement) = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
        .await
        .expect("expired reservation should be reclaimed")
    else {
        panic!("expected reclaimed action");
    };

    stale_action.settle(ProductInboundAck::NoOp);
    let stale_error = ledger
        .settle(stale_action)
        .await
        .expect_err("superseded action must not settle");
    assert!(matches!(
        stale_error,
        ProductWorkflowError::Transient { .. }
    ));

    replacement.settle(ProductInboundAck::NoOp);
    ledger
        .settle(replacement)
        .await
        .expect("replacement settle");
}

pub async fn assert_settle_missing_reservation_returns_transient(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let mut action = ProductInboundAction::begin(fingerprint(suffix), received_at);
    action.settle(ProductInboundAck::NoOp);

    let error = ledger
        .settle(action)
        .await
        .expect_err("missing reservation must not settle");
    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
}

pub async fn assert_custom_root_isolated_from_default_root(
    custom: &dyn IdempotencyLedger,
    default: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);
    let IdempotencyDecision::New(mut action) = custom
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin in custom root")
    else {
        panic!("expected new custom-root action");
    };
    action.settle(ProductInboundAck::NoOp);
    custom.settle(action).await.expect("settle custom root");

    let default_decision = default
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("begin in default root");
    assert!(matches!(default_decision, IdempotencyDecision::New(_)));
}

pub async fn assert_actor_identity_is_part_of_fingerprint_path(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let first_actor = fingerprint_for_actor(suffix, "user1");
    let second_actor = fingerprint_for_actor(suffix, "user2");

    assert!(matches!(
        ledger
            .begin_or_replay(first_actor, received_at)
            .await
            .expect("begin first actor"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(second_actor, received_at)
            .await
            .expect("begin second actor"),
        IdempotencyDecision::New(_)
    ));
}

pub async fn assert_settled_entry_limit_prunes_oldest(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let oldest = fingerprint(&format!("{suffix}-oldest"));
    let newest = fingerprint(&format!("{suffix}-newest"));

    let IdempotencyDecision::New(mut oldest_action) = ledger
        .begin_or_replay(oldest.clone(), received_at)
        .await
        .expect("begin oldest")
    else {
        panic!("expected new oldest action");
    };
    oldest_action.settle(ProductInboundAck::NoOp);
    ledger.settle(oldest_action).await.expect("settle oldest");

    let IdempotencyDecision::New(mut newest_action) = ledger
        .begin_or_replay(newest.clone(), received_at + Duration::seconds(1))
        .await
        .expect("begin newest")
    else {
        panic!("expected new newest action");
    };
    newest_action.settle(ProductInboundAck::NoOp);
    ledger.settle(newest_action).await.expect("settle newest");

    assert!(matches!(
        ledger
            .begin_or_replay(oldest, received_at + Duration::seconds(2))
            .await
            .expect("oldest was pruned and can reserve again"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(newest, received_at + Duration::seconds(2))
            .await
            .expect("newest remains available for replay"),
        IdempotencyDecision::Replay(_)
    ));
}

pub async fn assert_settled_prune_interval_defers_until_interval(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let first = fingerprint(&format!("{suffix}-first"));
    let second = fingerprint(&format!("{suffix}-second"));
    let third = fingerprint(&format!("{suffix}-third"));

    for (fingerprint, received_at) in [
        (first.clone(), received_at),
        (second.clone(), received_at + Duration::seconds(1)),
    ] {
        let IdempotencyDecision::New(mut action) = ledger
            .begin_or_replay(fingerprint, received_at)
            .await
            .expect("begin settled action")
        else {
            panic!("expected new action before prune interval");
        };
        action.settle(ProductInboundAck::NoOp);
        ledger.settle(action).await.expect("settle action");
    }

    assert!(matches!(
        ledger
            .begin_or_replay(first.clone(), received_at + Duration::seconds(2))
            .await
            .expect("first action is retained before prune interval"),
        IdempotencyDecision::Replay(_)
    ));

    let IdempotencyDecision::New(mut third_action) = ledger
        .begin_or_replay(third.clone(), received_at + Duration::seconds(3))
        .await
        .expect("begin third action")
    else {
        panic!("expected third action");
    };
    third_action.settle(ProductInboundAck::NoOp);
    ledger
        .settle(third_action)
        .await
        .expect("settle third action");

    assert!(matches!(
        ledger
            .begin_or_replay(first, received_at + Duration::seconds(4))
            .await
            .expect("first action is pruned on interval"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(third, received_at + Duration::seconds(4))
            .await
            .expect("newest action remains after interval prune"),
        IdempotencyDecision::Replay(_)
    ));
}

pub async fn settle_noop(
    ledger: &dyn IdempotencyLedger,
    fingerprint: ActionFingerprintKey,
    received_at: chrono::DateTime<Utc>,
) {
    let IdempotencyDecision::New(mut action) = ledger
        .begin_or_replay(fingerprint, received_at)
        .await
        .expect("begin action")
    else {
        panic!("expected new action");
    };
    action.settle(ProductInboundAck::NoOp);
    ledger.settle(action).await.expect("settle action");
}
