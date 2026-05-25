//! Backend-agnostic contract tests for [`FilesystemIdempotencyLedger`].
//!
//! Drives the ledger against an [`InMemoryBackend`] so the CAS semantics are
//! deterministic and the test crate does not need a libSQL or Postgres
//! dependency. Postgres- and libSQL-specific quirks (TIMESTAMPTZ mapping,
//! schema typos, ON CONFLICT semantics) are now the concern of
//! `ironclaw_filesystem`'s `PostgresRootFilesystem` /
//! `LibSqlRootFilesystem` test suites — the ledger does not see SQL.
//!
//! The pre-existing line-level review findings (zmanian's review on
//! PR #3590 item #1 — concurrent begin under ON CONFLICT — and serrrfirat's
//! review item — stale owner clobbering a reclaimed row on settle/release)
//! are preserved as regression tests against the new implementation. The
//! action_id ownership check that those reviews exposed is now a property
//! of the type system (the FS layer's `RecordVersion` is the ownership
//! token) — the regression tests prove the behaviour stays correct
//! regardless of backend.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalEventId, ProductAdapterId, ProductInboundAck,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger,
    ProductInboundAction, ProductWorkflowError, SourceBindingKey,
};
use ironclaw_product_workflow_storage::FilesystemIdempotencyLedger;
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

fn fixed_mount_view() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/ledger").expect("alias"),
        VirtualPath::new("/ledger").expect("vpath"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view")
}

fn build_ledger(recovery_lease: Duration) -> FilesystemIdempotencyLedger<InMemoryBackend> {
    let (ledger, _scoped) = build_ledger_and_filesystem(recovery_lease);
    ledger
}

/// Build a ledger alongside the underlying `ScopedFilesystem` so tests
/// that need to assert on raw ledger directory state (e.g. exact row
/// counts after a duplicate begin) can list `/ledger/inbound` directly.
fn build_ledger_and_filesystem(
    recovery_lease: Duration,
) -> (
    FilesystemIdempotencyLedger<InMemoryBackend>,
    Arc<ScopedFilesystem<InMemoryBackend>>,
) {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        backend,
        fixed_mount_view(),
    ));
    let ledger =
        FilesystemIdempotencyLedger::with_recovery_lease(Arc::clone(&scoped), recovery_lease);
    (ledger, scoped)
}

fn fingerprint(event_id: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("telegram_v2").expect("adapter id"),
        AdapterInstallationId::new("install_default").expect("installation id"),
        ExternalActorRef::new("user", "12345", None::<String>).expect("actor ref"),
        SourceBindingKey::new("chat:12345").expect("binding key"),
        ExternalEventId::new(event_id).expect("event id"),
    )
}

fn sample_ack() -> ProductInboundAck {
    ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new("msg-test-1").expect("ref"),
        submitted_run_id: TurnRunId::new(),
    }
}

#[tokio::test]
async fn new_action_inserts_and_returns_new() {
    let ledger = build_ledger(Duration::from_secs(300));
    let decision = ledger
        .begin_or_replay(fingerprint("evt_1"), Utc::now())
        .await
        .expect("begin");
    assert!(matches!(decision, IdempotencyDecision::New(_)));
}

#[tokio::test]
async fn second_begin_while_in_flight_is_transient() {
    let ledger = build_ledger(Duration::from_secs(300));
    let fp = fingerprint("evt_inflight");
    ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("first");
    let err = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect_err("second in-flight must be transient");
    assert!(matches!(err, ProductWorkflowError::Transient { .. }));
}

#[tokio::test]
async fn settle_then_begin_returns_replay() {
    let ledger = build_ledger(Duration::from_secs(300));
    let fp = fingerprint("evt_settle");
    let mut action = match ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };
    action.settle(sample_ack());
    ledger.settle(action.clone()).await.expect("settle");

    let replay = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect("replay");
    match replay {
        IdempotencyDecision::Replay(prior) => {
            assert_eq!(prior.action_id, action.action_id);
            assert_eq!(prior.phase, ActionPhase::Settled);
            assert!(prior.outcome.is_some());
        }
        other => panic!("expected Replay, got {other:?}"),
    }
}

/// Direct row-count check for the duplicate-begin path. The webhook
/// E2E test in `ironclaw_reborn_telegram_v2_host::webhook_e2e` proves
/// `begin_or_replay` returns `Replay` on a duplicate, but it cannot
/// list `/ledger/inbound` through the `IdempotencyLedger` trait — so a
/// regression that wrote two rows at two distinct paths for one
/// fingerprint would still pass there. This test asserts the
/// underlying directory state directly: one row in, one row out,
/// regardless of how many times the same fingerprint is replayed.
#[tokio::test]
async fn duplicate_begin_yields_exactly_one_ledger_row() {
    let (ledger, scoped) = build_ledger_and_filesystem(Duration::from_secs(300));
    let fp = fingerprint("evt_one_row");

    // First begin — fresh fingerprint, expect `New`.
    let mut action = match ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("first begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New on fresh fingerprint, got {other:?}"),
    };
    action.settle(sample_ack());
    ledger.settle(action.clone()).await.expect("settle");

    // Replay the same fingerprint several times. Each call hits the
    // settled-row branch and must not write.
    for _ in 0..3 {
        let replay = ledger
            .begin_or_replay(fp.clone(), Utc::now())
            .await
            .expect("replay");
        assert!(
            matches!(replay, IdempotencyDecision::Replay(_)),
            "settled row must replay, got {replay:?}",
        );
    }

    // List the ledger inbound directory. Exactly one entry must exist
    // for the one fingerprint that has been begun + settled, regardless
    // of how many replays happened.
    let entries = scoped
        .list_dir(
            &ResourceScope::system(),
            &ScopedPath::new("/ledger/inbound").expect("scoped path"),
        )
        .await
        .expect("list ledger inbound");
    let row_count = entries
        .iter()
        .filter(|entry| entry.name.ends_with(".json"))
        .count();
    assert_eq!(
        row_count, 1,
        "exactly one ledger row must exist per fingerprint after begin + settle + replays; \
         found {row_count} entries in /ledger/inbound: {entries:?}",
    );
}

#[tokio::test]
async fn release_drops_inflight_row_and_allows_new_begin() {
    let ledger = build_ledger(Duration::from_secs(300));
    let fp = fingerprint("evt_release");
    let action = match ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };
    ledger.release(action).await.expect("release");

    let next = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect("next begin");
    assert!(matches!(next, IdempotencyDecision::New(_)));
}

#[tokio::test]
async fn release_does_not_drop_settled_row() {
    let ledger = build_ledger(Duration::from_secs(300));
    let fp = fingerprint("evt_settled_release");
    let mut action = match ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };
    action.settle(sample_ack());
    ledger.settle(action.clone()).await.expect("settle");

    // Release on a settled action is a no-op; future begin still replays.
    ledger.release(action).await.expect("release");
    let after = ledger.begin_or_replay(fp, Utc::now()).await.expect("after");
    assert!(matches!(after, IdempotencyDecision::Replay(_)));
}

/// Regression for zmanian's PR #3590 review item #1 — concurrent
/// `begin_or_replay` calls with the same fingerprint must never surface a
/// raw conflict error. The previous SQL impls handled this via
/// `ON CONFLICT DO NOTHING` + a follow-up SELECT; the FS impl handles it
/// via `CasExpectation::Absent` + a bounded retry loop. Either way the
/// trait contract requires exactly one `New` and the rest as `Transient`
/// (or `Replay` if a winner settles first).
#[tokio::test]
async fn concurrent_begin_funnels_through_cas() {
    let ledger = Arc::new(build_ledger(Duration::from_secs(300)));
    let fp = fingerprint("evt_concurrent");

    let mut handles = Vec::new();
    for _ in 0..8 {
        let l = Arc::clone(&ledger);
        let f = fp.clone();
        handles.push(tokio::spawn(async move {
            l.begin_or_replay(f, Utc::now()).await
        }));
    }

    let mut new_count = 0;
    let mut transient_count = 0;
    for h in handles {
        match h.await.expect("join") {
            Ok(IdempotencyDecision::New(_)) => new_count += 1,
            Ok(IdempotencyDecision::Replay(_)) => {
                // Possible if a winner settles in time; not expected here.
            }
            Err(ProductWorkflowError::Transient { .. }) => transient_count += 1,
            Err(other) => panic!("concurrent begin must surface Transient, not {other:?}"),
        }
    }
    assert_eq!(new_count, 1, "exactly one task should win the claim");
    assert!(transient_count >= 1, "losers must surface as Transient");
}

/// Regression for Henry's PR #3590 review item — the recovery-lease
/// contract. A non-terminal row whose `received_at` is older than the
/// configured lease must be atomically reclaimed by the next caller's
/// `begin_or_replay`, returning `New`.
#[tokio::test]
async fn stale_inflight_row_is_reclaimed_on_next_begin() {
    let ledger = build_ledger(Duration::from_secs(1));
    let fp = fingerprint("evt_stale_reclaim");

    let first = match ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("first begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };

    // Drive received_at into the past by passing a future "now" to the
    // second begin, instead of hand-aging the row body. The lease check
    // is `received_at >= now - lease`, so a now() 10s in the future
    // simulates a 10s gap with a 1s lease — past the threshold.
    let future_now = Utc::now() + chrono::Duration::seconds(10);
    let second = ledger
        .begin_or_replay(fp, future_now)
        .await
        .expect("second begin");
    match second {
        IdempotencyDecision::New(reclaimed) => {
            assert_ne!(
                reclaimed.action_id, first.action_id,
                "reclaim must mint a fresh action_id"
            );
        }
        other => panic!("stale row must reclaim as New, got {other:?}"),
    }
}

/// Counter-test: a *fresh* non-terminal row within the lease window must
/// continue to surface as Transient. Prevents the reclaim path from
/// over-firing.
#[tokio::test]
async fn fresh_inflight_row_stays_transient_within_lease() {
    let ledger = build_ledger(Duration::from_secs(3600));
    let fp = fingerprint("evt_fresh_inflight");
    let _first = ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("first begin");
    let err = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect_err("fresh in-flight must be Transient");
    assert!(matches!(err, ProductWorkflowError::Transient { .. }));
}

/// Regression for serrrfirat's PR #3590 review — after the recovery-lease
/// path mints a fresh `action_id`, the original (stale) owner's settle
/// must surface `Transient { reason: superseded }` instead of clobbering
/// the new owner's outcome. With CAS, this is enforced by the action_id
/// mismatch check in `settle` (and structurally by the version bump on
/// reclaim, which the in-memory backend exposes).
#[tokio::test]
async fn stale_settle_after_reclaim_is_superseded() {
    let ledger = build_ledger(Duration::from_secs(1));
    let fp = fingerprint("evt_stale_settle");

    let stale = match ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("first begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };

    let future_now = Utc::now() + chrono::Duration::seconds(10);
    let reclaimed = match ledger
        .begin_or_replay(fp.clone(), future_now)
        .await
        .expect("reclaim")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New on reclaim, got {other:?}"),
    };
    assert_ne!(reclaimed.action_id, stale.action_id);

    let mut stale = stale;
    stale.settle(sample_ack());
    let err = ledger
        .settle(stale)
        .await
        .expect_err("stale settle must be rejected");
    assert!(
        matches!(err, ProductWorkflowError::Transient { ref reason } if reason.contains("superseded")),
        "expected Transient/superseded, got {err:?}"
    );

    // Current owner can still settle normally.
    let mut current = reclaimed;
    current.settle(sample_ack());
    ledger.settle(current).await.expect("current owner settle");
}

/// Regression — stale release must not delete a freshly reclaimed
/// in-flight row owned by a new caller. Long recovery lease + a future
/// `received_at` on the reclaim keeps the new row fresh through the
/// follow-up begin.
#[tokio::test]
async fn stale_release_does_not_drop_reclaimed_row() {
    let ledger = build_ledger(Duration::from_secs(3600));
    let fp = fingerprint("evt_stale_release");

    let stale = match ledger
        .begin_or_replay(fp.clone(), Utc::now() - chrono::Duration::seconds(7200))
        .await
        .expect("first begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };

    let _reclaimed = ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("reclaim");

    // Stale release — silent no-op, not a delete of the fresh row.
    ledger.release(stale).await.expect("stale release ok");

    // Fresh row is still in flight: another begin within lease must be Transient.
    let err = ledger
        .begin_or_replay(fp, Utc::now())
        .await
        .expect_err("fresh row must still be in flight");
    assert!(matches!(err, ProductWorkflowError::Transient { .. }));
}

/// Idempotent re-settle: settling the same action_id twice succeeds the
/// second time (the workflow layer may retry settle on transient
/// downstream failures).
#[tokio::test]
async fn idempotent_resettle_succeeds() {
    let ledger = build_ledger(Duration::from_secs(300));
    let fp = fingerprint("evt_resettle");
    let mut action = match ledger.begin_or_replay(fp, Utc::now()).await.expect("begin") {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };
    action.settle(sample_ack());
    ledger.settle(action.clone()).await.expect("first settle");
    ledger
        .settle(action)
        .await
        .expect("second settle is idempotent");
}

/// Defence-in-depth: action_id mismatch on settle without any reclaim
/// (a wholly fabricated action_id) surfaces superseded, not a silent
/// success. Documents the contract that even without CAS races, only
/// the row's current owner may settle.
#[tokio::test]
async fn fabricated_action_id_settle_is_rejected() {
    let ledger = build_ledger(Duration::from_secs(300));
    let fp = fingerprint("evt_fabricated");
    let action = match ledger
        .begin_or_replay(fp.clone(), Utc::now())
        .await
        .expect("begin")
    {
        IdempotencyDecision::New(a) => a,
        other => panic!("expected New, got {other:?}"),
    };

    // Build a fake settle attempt for the same fingerprint with a
    // different action_id (a wholly fabricated one).
    let mut imposter = ProductInboundAction::begin(action.fingerprint.clone(), Utc::now());
    imposter.settle(sample_ack());
    let err = ledger
        .settle(imposter)
        .await
        .expect_err("fabricated action_id must be rejected");
    assert!(
        matches!(err, ProductWorkflowError::Transient { ref reason } if reason.contains("superseded")),
        "expected superseded transient, got {err:?}"
    );
}
