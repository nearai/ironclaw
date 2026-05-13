//! libSQL-backed [`IdempotencyLedger`] implementation.
//!
//! Schema lives in the host crate (`src/db/libsql_migrations.rs` migration V26).
//! This file only deals with row layout and state transitions.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_product_adapters::ProductInboundAck;
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger,
    ProductActionId, ProductInboundAction, ProductWorkflowError,
};
use uuid::Uuid;

use crate::error::{libsql_error, transient};

/// libSQL-backed durable idempotency ledger.
///
/// Webhook retries for the same external event return a `Replay` of the prior
/// outcome instead of double-dispatching downstream. Required-but-in-flight
/// duplicates surface as transient errors so the protocol layer can retry
/// after the in-flight action settles (or after recovery lease TTL elapses,
/// which is a separate sweep concern).
#[derive(Clone)]
pub struct LibSqlProductIdempotencyLedger {
    db: Arc<::libsql::Database>,
}

impl LibSqlProductIdempotencyLedger {
    pub fn new(db: Arc<::libsql::Database>) -> Self {
        Self { db }
    }

    async fn connect(&self) -> Result<::libsql::Connection, ProductWorkflowError> {
        self.db.connect().map_err(libsql_error)
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

/// Convert the stored `phase` column value back to an `ActionPhase`.
/// Exhaustive on the persisted wire spelling — any new variant added to
/// `ActionPhase` must be added here too, or this stops compiling.
fn parse_phase(value: &str) -> Result<ActionPhase, ProductWorkflowError> {
    match value {
        "received" => Ok(ActionPhase::Received),
        "dispatched" => Ok(ActionPhase::Dispatched),
        "settled" => Ok(ActionPhase::Settled),
        "deduplicated_replay" => Ok(ActionPhase::DeduplicatedReplay),
        other => Err(transient(format!("invalid phase '{other}'"))),
    }
}

fn phase_to_str(phase: ActionPhase) -> &'static str {
    // Keep in lock-step with `parse_phase` above. Both must enumerate every
    // `ActionPhase` variant or one of them will be silently lossy.
    match phase {
        ActionPhase::Received => "received",
        ActionPhase::Dispatched => "dispatched",
        ActionPhase::Settled => "settled",
        ActionPhase::DeduplicatedReplay => "deduplicated_replay",
    }
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
        let conn = self.connect().await?;

        if let Some(row) = fetch_row(&conn, &fingerprint).await? {
            let phase = parse_phase(&row.phase)?;
            let action = row_into_action(row, fingerprint.clone())?;
            return match phase {
                ActionPhase::Settled | ActionPhase::DeduplicatedReplay => {
                    Ok(IdempotencyDecision::Replay(action))
                }
                ActionPhase::Received | ActionPhase::Dispatched => Err(transient(
                    "idempotency fingerprint already in flight; retry after recovery lease",
                )),
            };
        }

        let action = ProductInboundAction::begin(fingerprint, received_at);
        let action_id_str = action.action_id.as_uuid().to_string();
        let received_at_str = format_timestamp(action.received_at);
        conn.execute(
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
        .map_err(libsql_error)?;

        Ok(IdempotencyDecision::New(action))
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
