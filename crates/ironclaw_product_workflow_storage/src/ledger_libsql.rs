//! libSQL-backed [`IdempotencyLedger`] implementation.
//!
//! Schema lives in the host crate (`src/db/libsql_migrations.rs` migration V26).
//! This file only deals with row layout and state transitions.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_product_adapters::ProductInboundAck;
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger,
    ProductActionId, ProductInboundAction, ProductWorkflowError,
};
use uuid::Uuid;

use crate::error::{libsql_error, transient};
use crate::phase::{parse_phase, phase_to_str};
use crate::recovery::DEFAULT_RECOVERY_LEASE;

/// libSQL-backed durable idempotency ledger.
///
/// Webhook retries for the same external event return a `Replay` of the prior
/// outcome instead of double-dispatching downstream. Required-but-in-flight
/// duplicates surface as transient errors so the protocol layer can retry
/// after the in-flight action settles, **or** after the recovery lease TTL
/// elapses and `begin_or_replay` reclaims the stale reservation.
#[derive(Clone)]
pub struct LibSqlProductIdempotencyLedger {
    db: Arc<::libsql::Database>,
    recovery_lease: Duration,
}

impl LibSqlProductIdempotencyLedger {
    /// Construct with the [`DEFAULT_RECOVERY_LEASE`].
    pub fn new(db: Arc<::libsql::Database>) -> Self {
        Self::with_recovery_lease(db, DEFAULT_RECOVERY_LEASE)
    }

    /// Construct with an explicit recovery-lease TTL. A non-terminal row
    /// older than this is eligible for reclaim on the next
    /// `begin_or_replay` call for the same fingerprint.
    pub fn with_recovery_lease(db: Arc<::libsql::Database>, recovery_lease: Duration) -> Self {
        Self { db, recovery_lease }
    }

    async fn connect(&self) -> Result<::libsql::Connection, ProductWorkflowError> {
        self.db.connect().map_err(libsql_error)
    }

    /// Conflict-path handler for `begin_or_replay`. The INSERT lost the
    /// UNIQUE race; a row already exists for this fingerprint. The three
    /// branches:
    ///
    /// * `Settled` / `DeduplicatedReplay` — return `Replay(row)`. Caller
    ///   replays the prior outcome.
    /// * `Received` / `Dispatched` with `received_at` < `now -
    ///   recovery_lease` — the prior reservation is abandoned (timeout,
    ///   crash, cancelled spawn). Atomically claim it: UPDATE in place
    ///   with a fresh `action_id` and `received_at`, keep `phase = Received`.
    ///   Return `New(reclaimed_action)`.
    /// * `Received` / `Dispatched` within lease — return `Transient`. Caller
    ///   should retry later.
    ///
    /// The reclaim UPDATE is gated on the same phase-and-age predicate so
    /// two concurrent callers that both observe a stale row only let one
    /// of them claim it; the other sees `rows_affected = 0` and re-reads
    /// the now-fresh row (and returns Transient because the new claim is
    /// in-flight).
    async fn handle_conflict(
        &self,
        conn: &::libsql::Connection,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let row = fetch_row(conn, &fingerprint).await?.ok_or_else(|| {
            transient("ledger UNIQUE constraint fired but row not visible on follow-up SELECT")
        })?;
        let phase = parse_phase(&row.phase)?;
        match phase {
            ActionPhase::Settled | ActionPhase::DeduplicatedReplay => {
                let action = row_into_action(row, fingerprint)?;
                Ok(IdempotencyDecision::Replay(action))
            }
            ActionPhase::Received | ActionPhase::Dispatched => {
                let row_received_at = parse_timestamp(&row.received_at)?;
                let lease_chrono = chrono::Duration::from_std(self.recovery_lease)
                    .map_err(|e| transient(format!("recovery lease out of range: {e}")))?;
                let stale_threshold = received_at - lease_chrono;
                if row_received_at >= stale_threshold {
                    // Still in flight within the lease window.
                    return Err(transient(
                        "idempotency fingerprint already in flight; retry after recovery lease",
                    ));
                }
                // Stale. Atomically claim by UPDATE-with-WHERE. The WHERE
                // includes the phase-and-age predicate so a concurrent
                // caller that also saw the stale row only lets one of us
                // win; the loser observes rows_affected = 0 and re-reads
                // (the row now carries the winner's fresh received_at, so
                // the follow-through hits the Transient branch).
                let claimed = ProductInboundAction::begin(fingerprint.clone(), received_at);
                let claimed_action_id_str = claimed.action_id.as_uuid().to_string();
                let claimed_received_at_str = format_timestamp(claimed.received_at);
                let row_received_at_str = row.received_at.clone();
                let affected = conn
                    .execute(
                        "UPDATE product_inbound_actions \
                         SET action_id = ?1, received_at = ?2, phase = 'received', \
                             dispatch_kind_json = NULL, outcome_json = NULL, settled_at = NULL \
                         WHERE adapter_id = ?3 \
                           AND installation_id = ?4 \
                           AND source_binding_key = ?5 \
                           AND external_event_id = ?6 \
                           AND phase IN ('received', 'dispatched') \
                           AND received_at = ?7",
                        ::libsql::params![
                            claimed_action_id_str,
                            claimed_received_at_str,
                            claimed.fingerprint.adapter_id.as_str(),
                            claimed.fingerprint.installation_id.as_str(),
                            claimed.fingerprint.source_binding_key.as_str(),
                            claimed.fingerprint.external_event_id.as_str(),
                            row_received_at_str,
                        ],
                    )
                    .await
                    .map_err(libsql_error)?;
                if affected == 1 {
                    tracing::warn!(
                        fingerprint = ?claimed.fingerprint,
                        prior_received_at = %row_received_at,
                        lease_secs = self.recovery_lease.as_secs(),
                        "reclaimed stale non-terminal ledger row after recovery lease elapsed"
                    );
                    Ok(IdempotencyDecision::New(claimed))
                } else {
                    // Another caller claimed it between our SELECT and
                    // UPDATE. Their claim is in-flight; we surface
                    // Transient so the protocol layer retries.
                    Err(transient(
                        "idempotency fingerprint reclaimed by concurrent caller; retry",
                    ))
                }
            }
        }
    }
}

/// Internal row representation read from the table.
struct LedgerRow {
    action_id: String,
    phase: String,
    dispatch_kind_json: Option<String>,
    outcome_json: Option<String>,
    received_at: String,
    settled_at: Option<String>,
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, ProductWorkflowError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| transient(format!("invalid timestamp '{value}': {e}")))
}

fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn row_into_action(
    row: LedgerRow,
    fingerprint: ActionFingerprintKey,
) -> Result<ProductInboundAction, ProductWorkflowError> {
    let action_id_uuid = Uuid::parse_str(&row.action_id)
        .map_err(|e| transient(format!("invalid action_id uuid '{}': {e}", row.action_id)))?;
    let action_id = ProductActionId::from_uuid(action_id_uuid);

    let phase = parse_phase(&row.phase)?;
    let dispatch_kind = row
        .dispatch_kind_json
        .map(|s| {
            serde_json::from_str::<ActionDispatchKind>(&s)
                .map_err(|e| transient(format!("invalid dispatch_kind_json: {e}")))
        })
        .transpose()?;
    let outcome = row
        .outcome_json
        .map(|s| {
            serde_json::from_str::<ProductInboundAck>(&s)
                .map_err(|e| transient(format!("invalid outcome_json: {e}")))
        })
        .transpose()?;
    let received_at = parse_timestamp(&row.received_at)?;
    let settled_at = row.settled_at.as_deref().map(parse_timestamp).transpose()?;

    Ok(ProductInboundAction {
        action_id,
        fingerprint,
        phase,
        dispatch_kind,
        outcome,
        received_at,
        settled_at,
    })
}

async fn fetch_row(
    conn: &::libsql::Connection,
    fingerprint: &ActionFingerprintKey,
) -> Result<Option<LedgerRow>, ProductWorkflowError> {
    let mut rows = conn
        .query(
            "SELECT action_id, phase, dispatch_kind_json, outcome_json, received_at, settled_at \
             FROM product_inbound_actions \
             WHERE adapter_id = ?1 \
               AND installation_id = ?2 \
               AND source_binding_key = ?3 \
               AND external_event_id = ?4",
            ::libsql::params![
                fingerprint.adapter_id.as_str(),
                fingerprint.installation_id.as_str(),
                fingerprint.source_binding_key.as_str(),
                fingerprint.external_event_id.as_str(),
            ],
        )
        .await
        .map_err(libsql_error)?;

    let Some(row) = rows.next().await.map_err(libsql_error)? else {
        return Ok(None);
    };

    let action_id: String = row.get(0).map_err(libsql_error)?;
    let phase: String = row.get(1).map_err(libsql_error)?;
    let dispatch_kind_json: Option<String> = row.get(2).map_err(libsql_error)?;
    let outcome_json: Option<String> = row.get(3).map_err(libsql_error)?;
    let received_at: String = row.get(4).map_err(libsql_error)?;
    let settled_at: Option<String> = row.get(5).map_err(libsql_error)?;

    Ok(Some(LedgerRow {
        action_id,
        phase,
        dispatch_kind_json,
        outcome_json,
        received_at,
        settled_at,
    }))
}

#[async_trait]
impl IdempotencyLedger for LibSqlProductIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        // INSERT-first to close the SELECT-then-INSERT TOCTOU window
        // (zmanian's review on PR #3590, item #1). Two concurrent callers
        // with the same fingerprint used to both miss a prior SELECT and
        // race the UNIQUE constraint at INSERT; the loser surfaced a raw DB
        // error instead of the expected `Transient` reply.
        let conn = self.connect().await?;
        let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
        let action_id_str = action.action_id.as_uuid().to_string();
        let received_at_str = format_timestamp(action.received_at);

        match conn
            .execute(
                "INSERT INTO product_inbound_actions \
                 (action_id, adapter_id, installation_id, source_binding_key, external_event_id, \
                  phase, dispatch_kind_json, outcome_json, received_at, settled_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, NULL)",
                ::libsql::params![
                    action_id_str,
                    action.fingerprint.adapter_id.as_str(),
                    action.fingerprint.installation_id.as_str(),
                    action.fingerprint.source_binding_key.as_str(),
                    action.fingerprint.external_event_id.as_str(),
                    phase_to_str(action.phase),
                    received_at_str,
                ],
            )
            .await
        {
            Ok(_) => Ok(IdempotencyDecision::New(action)),
            // UNIQUE constraint (extended SQLite code 2067 =
            // SQLITE_CONSTRAINT_UNIQUE). libsql 0.6 surfaces the extended
            // code, not the primary code 19; matching on 19 alone
            // silently misses this case. We then take the conflict path,
            // which may either reclaim a stale non-terminal row (recovery
            // lease) or return Replay/Transient based on the winner's
            // phase.
            Err(::libsql::Error::SqliteFailure(2067, _)) => {
                self.handle_conflict(&conn, fingerprint, received_at).await
            }
            Err(other) => Err(libsql_error(other)),
        }
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let conn = self.connect().await?;
        let outcome_json = action
            .outcome
            .as_ref()
            .map(|o| {
                serde_json::to_string(o).map_err(|e| transient(format!("serialize outcome: {e}")))
            })
            .transpose()?;
        let dispatch_kind_json = action
            .dispatch_kind
            .as_ref()
            .map(|d| {
                serde_json::to_string(d)
                    .map_err(|e| transient(format!("serialize dispatch_kind: {e}")))
            })
            .transpose()?;
        let settled_at_str = action.settled_at.map(format_timestamp);

        let affected = conn
            .execute(
                "UPDATE product_inbound_actions \
                 SET phase = ?1, \
                     dispatch_kind_json = ?2, \
                     outcome_json = ?3, \
                     settled_at = ?4 \
                 WHERE adapter_id = ?5 \
                   AND installation_id = ?6 \
                   AND source_binding_key = ?7 \
                   AND external_event_id = ?8",
                ::libsql::params![
                    phase_to_str(action.phase),
                    dispatch_kind_json,
                    outcome_json,
                    settled_at_str,
                    action.fingerprint.adapter_id.as_str(),
                    action.fingerprint.installation_id.as_str(),
                    action.fingerprint.source_binding_key.as_str(),
                    action.fingerprint.external_event_id.as_str(),
                ],
            )
            .await
            .map_err(libsql_error)?;

        if affected == 0 {
            tracing::warn!(
                fingerprint = ?action.fingerprint,
                "settle: no row matched fingerprint; settlement ignored"
            );
        }
        Ok(())
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let conn = self.connect().await?;
        // Only release in-flight rows. A settled row stays put so future
        // retries replay the prior outcome.
        conn.execute(
            "DELETE FROM product_inbound_actions \
             WHERE adapter_id = ?1 \
               AND installation_id = ?2 \
               AND source_binding_key = ?3 \
               AND external_event_id = ?4 \
               AND phase IN ('received', 'dispatched')",
            ::libsql::params![
                action.fingerprint.adapter_id.as_str(),
                action.fingerprint.installation_id.as_str(),
                action.fingerprint.source_binding_key.as_str(),
                action.fingerprint.external_event_id.as_str(),
            ],
        )
        .await
        .map_err(libsql_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_adapters::{
        AdapterInstallationId, ExternalEventId, ProductAdapterId, ProductInboundAck,
    };
    use ironclaw_product_workflow::SourceBindingKey;
    use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

    /// Mirrors `src/db/libsql_migrations.rs` migration V26. Keep in sync.
    const TEST_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS product_inbound_actions (
    action_id TEXT PRIMARY KEY,
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    source_binding_key TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    phase TEXT NOT NULL,
    dispatch_kind_json TEXT,
    outcome_json TEXT,
    received_at TEXT NOT NULL,
    settled_at TEXT,
    UNIQUE (adapter_id, installation_id, source_binding_key, external_event_id)
);
"#;

    async fn ledger() -> (LibSqlProductIdempotencyLedger, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.db");
        let db = ::libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build db");
        let conn = db.connect().expect("connect");
        conn.execute_batch(TEST_SCHEMA).await.expect("schema");
        (LibSqlProductIdempotencyLedger::new(Arc::new(db)), dir)
    }

    fn fingerprint(event_id: &str) -> ActionFingerprintKey {
        ActionFingerprintKey::new(
            ProductAdapterId::new("telegram_v2").expect("adapter id"),
            AdapterInstallationId::new("install_default").expect("installation id"),
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
        let (ledger, _dir) = ledger().await;
        let now = Utc::now();
        let decision = ledger
            .begin_or_replay(fingerprint("evt_1"), now)
            .await
            .expect("begin_or_replay");
        assert!(matches!(decision, IdempotencyDecision::New(_)));
    }

    #[tokio::test]
    async fn second_begin_while_in_flight_is_transient() {
        let (ledger, _dir) = ledger().await;
        let fp = fingerprint("evt_inflight");
        ledger
            .begin_or_replay(fp.clone(), Utc::now())
            .await
            .expect("first");
        let result = ledger.begin_or_replay(fp, Utc::now()).await;
        assert!(matches!(
            result,
            Err(ProductWorkflowError::Transient { .. })
        ));
    }

    #[tokio::test]
    async fn settle_then_begin_returns_replay() {
        let (ledger, _dir) = ledger().await;
        let fp = fingerprint("evt_settle");
        let decision = ledger
            .begin_or_replay(fp.clone(), Utc::now())
            .await
            .expect("begin");
        let mut action = match decision {
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

    #[tokio::test]
    async fn release_drops_inflight_row_and_allows_new_begin() {
        let (ledger, _dir) = ledger().await;
        let fp = fingerprint("evt_release");
        let decision = ledger
            .begin_or_replay(fp.clone(), Utc::now())
            .await
            .expect("begin");
        let action = match decision {
            IdempotencyDecision::New(a) => a,
            other => panic!("expected New, got {other:?}"),
        };
        ledger.release(action).await.expect("release");

        // After release, a fresh begin should succeed (not transient).
        let second = ledger
            .begin_or_replay(fp, Utc::now())
            .await
            .expect("second begin");
        assert!(matches!(second, IdempotencyDecision::New(_)));
    }

    /// Regression for zmanian's PR #3590 review item #1 — concurrent
    /// `begin_or_replay` calls with the same fingerprint must never surface
    /// a raw DB UNIQUE-constraint error. Exactly one task sees `New`; the
    /// rest see `Transient` (an honest "in-flight" reply, retryable later)
    /// or `Replay` (if one of them settled first). No `Err` other than
    /// `Transient` is allowed.
    #[tokio::test]
    async fn concurrent_begin_funnels_through_unique_constraint() {
        let (ledger, _dir) = ledger().await;
        let fp = fingerprint("evt_concurrent");

        let mut handles = Vec::new();
        for _ in 0..8 {
            let l = ledger.clone();
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
                    // Possible if the winner settles in time, though not
                    // expected in this test because nobody calls settle.
                }
                Err(ProductWorkflowError::Transient { .. }) => transient_count += 1,
                Err(other) => {
                    panic!("concurrent begin must surface Transient on conflict, not {other:?}")
                }
            }
        }
        assert_eq!(new_count, 1, "exactly one task should win the insert");
        assert!(transient_count >= 1, "losers must surface as Transient");
    }

    /// Regression for Henry's PR #3590 review item #1 — the recovery-lease
    /// contract. A non-terminal row whose `received_at` is older than the
    /// configured lease must be atomically reclaimed by the next caller's
    /// `begin_or_replay`, returning `New`. Drives the actual public API
    /// (not just helper internals), per the "Test Through the Caller" rule.
    #[tokio::test]
    async fn stale_inflight_row_is_reclaimed_on_next_begin() {
        let (db, _dir) = build_test_db().await;
        // Short lease so the test doesn't have to wait. 1ms means any row
        // we hand-age by even 10ms is well past the threshold.
        let ledger = LibSqlProductIdempotencyLedger::with_recovery_lease(
            Arc::clone(&db),
            Duration::from_millis(1),
        );
        let fp = fingerprint("evt_stale_reclaim");

        // First begin: ordinary New + in-flight reservation.
        let first = ledger
            .begin_or_replay(fp.clone(), Utc::now())
            .await
            .expect("first begin");
        let first_action = match first {
            IdempotencyDecision::New(a) => a,
            other => panic!("expected New, got {other:?}"),
        };
        // Simulate "process crashed before settle/release": hand-age the
        // row's received_at to a value well past the 1ms lease window.
        let conn = db.connect().expect("connect");
        let aged = Utc::now() - chrono::Duration::seconds(10);
        let aged_str = format_timestamp(aged);
        let affected = conn
            .execute(
                "UPDATE product_inbound_actions SET received_at = ?1 \
                 WHERE adapter_id = ?2 AND installation_id = ?3 \
                   AND source_binding_key = ?4 AND external_event_id = ?5",
                ::libsql::params![
                    aged_str,
                    fp.adapter_id.as_str(),
                    fp.installation_id.as_str(),
                    fp.source_binding_key.as_str(),
                    fp.external_event_id.as_str(),
                ],
            )
            .await
            .expect("age the row");
        assert_eq!(affected, 1, "must hand-age exactly one row");

        // Second begin: stale row should be reclaimed atomically.
        let second = ledger
            .begin_or_replay(fp, Utc::now())
            .await
            .expect("second begin");
        match second {
            IdempotencyDecision::New(reclaimed) => {
                assert_ne!(
                    reclaimed.action_id, first_action.action_id,
                    "reclaim must mint a fresh action_id"
                );
            }
            other => panic!("stale row must be reclaimed and surface as New, got {other:?}"),
        }
    }

    /// Counter-test: a *fresh* non-terminal row within the lease window
    /// must continue to surface as Transient. Prevents the reclaim path
    /// from over-firing and clobbering honest slow dispatches.
    #[tokio::test]
    async fn fresh_inflight_row_stays_transient_within_lease() {
        let (db, _dir) = build_test_db().await;
        // Generous lease so the second begin happens well inside it.
        let ledger = LibSqlProductIdempotencyLedger::with_recovery_lease(
            Arc::clone(&db),
            Duration::from_secs(3600),
        );
        let fp = fingerprint("evt_fresh_inflight");
        let _first = ledger
            .begin_or_replay(fp.clone(), Utc::now())
            .await
            .expect("first begin");
        let err = ledger
            .begin_or_replay(fp, Utc::now())
            .await
            .expect_err("fresh in-flight must be Transient");
        assert!(
            matches!(err, ProductWorkflowError::Transient { .. }),
            "fresh in-flight must be Transient, got {err:?}"
        );
    }

    /// Helper: build a fresh in-memory libSQL DB with the V26 schema.
    /// Extracted from `ledger()` so tests that need a custom-lease ledger
    /// can keep the same DB lifecycle.
    async fn build_test_db() -> (Arc<::libsql::Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.db");
        let db = ::libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build db");
        let conn = db.connect().expect("connect");
        conn.execute_batch(TEST_SCHEMA).await.expect("schema");
        (Arc::new(db), dir)
    }

    #[tokio::test]
    async fn release_does_not_drop_settled_row() {
        let (ledger, _dir) = ledger().await;
        let fp = fingerprint("evt_settled_release");
        let decision = ledger
            .begin_or_replay(fp.clone(), Utc::now())
            .await
            .expect("begin");
        let mut action = match decision {
            IdempotencyDecision::New(a) => a,
            other => panic!("expected New, got {other:?}"),
        };
        action.settle(sample_ack());
        ledger.settle(action.clone()).await.expect("settle");

        // release on a settled action is a no-op; future begin still replays.
        ledger.release(action).await.expect("release");
        let after = ledger.begin_or_replay(fp, Utc::now()).await.expect("after");
        assert!(matches!(after, IdempotencyDecision::Replay(_)));
    }
}
