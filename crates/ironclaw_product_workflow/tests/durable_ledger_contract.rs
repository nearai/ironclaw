#![cfg(feature = "libsql")]

use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalEventId, ProductAdapterId, ProductInboundAck,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, IdempotencyDecision, IdempotencyLedger, RebornLibSqlIdempotencyLedger,
    SourceBindingKey,
};

fn fingerprint(suffix: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid adapter"),
        AdapterInstallationId::new("install_alpha").expect("valid installation"),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new(format!("evt:{suffix}")).expect("valid event"),
    )
}

async fn libsql_db(path: &str) -> Arc<libsql::Database> {
    Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build libsql db"),
    )
}

#[tokio::test]
async fn libsql_settled_action_survives_reopen_and_replays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let received_at = Utc::now();
    let fingerprint = fingerprint("settled-replay");

    let ledger = RebornLibSqlIdempotencyLedger::new(libsql_db(&db_path).await);
    let decision = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin");
    let IdempotencyDecision::New(mut action) = decision else {
        panic!("expected new action");
    };
    action.settle(ProductInboundAck::NoOp);
    ledger.settle(action).await.expect("settle");

    let reopened = RebornLibSqlIdempotencyLedger::new(libsql_db(&db_path).await);
    let replay = reopened
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("replay");

    let IdempotencyDecision::Replay(action) = replay else {
        panic!("expected replay");
    };
    assert_eq!(action.outcome, Some(ProductInboundAck::NoOp));
}

#[tokio::test]
async fn libsql_in_flight_action_blocks_until_lease_expires() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_db(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );
    let received_at = Utc::now();
    let fingerprint = fingerprint("lease");

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
    assert!(matches!(
        blocked,
        ironclaw_product_workflow::ProductWorkflowError::Transient { .. }
    ));

    let reclaimed = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
        .await
        .expect("expired reservation should be reclaimed");
    assert!(matches!(reclaimed, IdempotencyDecision::New(_)));
}

#[tokio::test]
async fn libsql_release_allows_retry_without_waiting_for_lease() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_db(&db_path.display().to_string()).await,
        Duration::seconds(60),
    );
    let received_at = Utc::now();
    let fingerprint = fingerprint("release");

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
