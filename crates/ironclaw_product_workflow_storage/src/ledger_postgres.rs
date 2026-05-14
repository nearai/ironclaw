//! Postgres-backed [`IdempotencyLedger`] implementation.
//!
//! Schema lives in `migrations/V28__product_inbound_actions_and_bindings.sql`.
//! This file only deals with row layout and state transitions.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use ironclaw_product_adapters::ProductInboundAck;
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger,
    ProductActionId, ProductInboundAction, ProductWorkflowError,
};
use uuid::Uuid;

use crate::error::{pool_error, postgres_error, transient};
use crate::phase::{parse_phase, phase_to_str};

#[derive(Clone)]
pub struct PostgresProductIdempotencyLedger {
    pool: Pool,
}

impl PostgresProductIdempotencyLedger {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

fn row_into_action(
    row: &::tokio_postgres::Row,
    fingerprint: ActionFingerprintKey,
) -> Result<ProductInboundAction, ProductWorkflowError> {
    // `action_id` is `UUID` in Postgres (see migration V28); tokio-postgres
    // maps it to `uuid::Uuid` directly when `with-uuid-1` is enabled.
    let action_id_uuid: Uuid = row.get("action_id");
    let action_id = ProductActionId::from_uuid(action_id_uuid);

    let phase_str: String = row.get("phase");
    let phase = parse_phase(&phase_str)?;

    let dispatch_kind_json: Option<String> = row.get("dispatch_kind_json");
    let outcome_json: Option<String> = row.get("outcome_json");
    let received_at: DateTime<Utc> = row.get("received_at");
    let settled_at: Option<DateTime<Utc>> = row.get("settled_at");

    let dispatch_kind = dispatch_kind_json
        .map(|s| {
            serde_json::from_str::<ActionDispatchKind>(&s)
                .map_err(|e| transient(format!("invalid dispatch_kind_json: {e}")))
        })
        .transpose()?;
    let outcome = outcome_json
        .map(|s| {
            serde_json::from_str::<ProductInboundAck>(&s)
                .map_err(|e| transient(format!("invalid outcome_json: {e}")))
        })
        .transpose()?;

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

#[async_trait]
impl IdempotencyLedger for PostgresProductIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        // Single-roundtrip INSERT-or-replay using `ON CONFLICT DO NOTHING
        // RETURNING action_id`. Closes the SELECT-then-INSERT TOCTOU window
        // from the prior implementation (zmanian's review on PR #3590,
        // item #1). If no row is returned, a concurrent caller — or an
        // honest retry — beat us; a second SELECT fetches the canonical row
        // and decides Replay vs Transient based on its phase.
        let client = self.pool.get().await.map_err(pool_error)?;
        let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
        let action_id_uuid = action.action_id.as_uuid();

        let inserted = client
            .query_opt(
                "INSERT INTO product_inbound_actions \
                 (action_id, adapter_id, installation_id, source_binding_key, external_event_id, \
                  phase, dispatch_kind_json, outcome_json, received_at, settled_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL, $7, NULL) \
                 ON CONFLICT (adapter_id, installation_id, source_binding_key, external_event_id) \
                 DO NOTHING \
                 RETURNING action_id",
                &[
                    &action_id_uuid,
                    &action.fingerprint.adapter_id.as_str(),
                    &action.fingerprint.installation_id.as_str(),
                    &action.fingerprint.source_binding_key.as_str(),
                    &action.fingerprint.external_event_id.as_str(),
                    &phase_to_str(action.phase),
                    &action.received_at,
                ],
            )
            .await
            .map_err(postgres_error)?;

        if inserted.is_some() {
            return Ok(IdempotencyDecision::New(action));
        }

        // Conflict path: another caller owns the row. Read it and decide.
        let row = client
            .query_opt(
                "SELECT action_id, phase, dispatch_kind_json, outcome_json, received_at, settled_at \
                 FROM product_inbound_actions \
                 WHERE adapter_id = $1 \
                   AND installation_id = $2 \
                   AND source_binding_key = $3 \
                   AND external_event_id = $4",
                &[
                    &fingerprint.adapter_id.as_str(),
                    &fingerprint.installation_id.as_str(),
                    &fingerprint.source_binding_key.as_str(),
                    &fingerprint.external_event_id.as_str(),
                ],
            )
            .await
            .map_err(postgres_error)?
            .ok_or_else(|| {
                transient(
                    "ledger ON CONFLICT fired but row not visible on follow-up SELECT",
                )
            })?;

        let phase_str: String = row.get("phase");
        let phase = parse_phase(&phase_str)?;
        let action = row_into_action(&row, fingerprint)?;
        match phase {
            ActionPhase::Settled | ActionPhase::DeduplicatedReplay => {
                Ok(IdempotencyDecision::Replay(action))
            }
            ActionPhase::Received | ActionPhase::Dispatched => Err(transient(
                "idempotency fingerprint already in flight; retry after recovery lease",
            )),
        }
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let client = self.pool.get().await.map_err(pool_error)?;
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

        let affected = client
            .execute(
                "UPDATE product_inbound_actions \
                 SET phase = $1, \
                     dispatch_kind_json = $2, \
                     outcome_json = $3, \
                     settled_at = $4 \
                 WHERE adapter_id = $5 \
                   AND installation_id = $6 \
                   AND source_binding_key = $7 \
                   AND external_event_id = $8",
                &[
                    &phase_to_str(action.phase),
                    &dispatch_kind_json,
                    &outcome_json,
                    &action.settled_at,
                    &action.fingerprint.adapter_id.as_str(),
                    &action.fingerprint.installation_id.as_str(),
                    &action.fingerprint.source_binding_key.as_str(),
                    &action.fingerprint.external_event_id.as_str(),
                ],
            )
            .await
            .map_err(postgres_error)?;

        if affected == 0 {
            tracing::warn!(
                fingerprint = ?action.fingerprint,
                "settle: no row matched fingerprint; settlement ignored"
            );
        }
        Ok(())
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let client = self.pool.get().await.map_err(pool_error)?;
        client
            .execute(
                "DELETE FROM product_inbound_actions \
                 WHERE adapter_id = $1 \
                   AND installation_id = $2 \
                   AND source_binding_key = $3 \
                   AND external_event_id = $4 \
                   AND phase IN ('received', 'dispatched')",
                &[
                    &action.fingerprint.adapter_id.as_str(),
                    &action.fingerprint.installation_id.as_str(),
                    &action.fingerprint.source_binding_key.as_str(),
                    &action.fingerprint.external_event_id.as_str(),
                ],
            )
            .await
            .map_err(postgres_error)?;
        Ok(())
    }
}
