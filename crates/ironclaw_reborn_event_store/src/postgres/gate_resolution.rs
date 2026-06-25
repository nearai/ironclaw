//! PostgreSQL-backed durable gate-resolution store.
//!
//! Spec: `docs/reborn/2026-06-08-subagent-durability-spec.md` §1.4 + §1.6.
//!
//! # Scope-predicate convention
//!
//! Every query that filters by scope MUST use the conditional
//! `<agent_predicate>`:
//! - When `scope.agent_id` is `Some(id)`: `agent_id = $N` bound to `id`.
//! - When `scope.agent_id` is `None`: `agent_id IS NULL`.
//!
//! NEVER use `(agent_id = $N OR agent_id IS NULL)` — it allows agent-scoped
//! callers to reach system-level (NULL agent_id) rows.
//!
//! # First-writer-wins
//!
//! All INSERT paths use `ON CONFLICT DO NOTHING` so that duplicate rows
//! (replay, concurrent settlement) are silently dropped.
//!
//! # PostgreSQL-specific notes
//!
//! - `GREATEST(a, b)` is available in PostgreSQL.
//! - `SELECT ... FOR UPDATE` locks the capacity bucket row before the SUM
//!   read (per-bucket lock, not full-table lock), preventing TOCTOU drift.
//! - JSONB is used for `child_scope_json` and `terminal_event_json` columns.
//! - Boolean columns are `BOOLEAN` (TRUE/FALSE).
//! - Timestamps are `TIMESTAMPTZ`.
//! - The capacity-counter table has no declared PK due to nullable `agent_id`;
//!   the COALESCE expression index (`idx_sgcc_pk`) is the conflict target.

use std::collections::HashSet;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use ironclaw_turns::{GateRef, LoopResultRef, TurnRunId, TurnScope, TurnStatus};

use crate::gate_resolution::{
    AwaitedChildRecord, AwaitedChildRow, DurableSubagentGateResolutionStore, DurableTerminalEvent,
    GateResolutionStoreError, MAX_GATE_RECORDS, child_bucket,
};

/// PostgreSQL-backed durable gate-resolution store.
///
/// Owns a `deadpool_postgres::Pool`. Each async method acquires a client from
/// the pool and runs in a single transaction. For the spawn path a
/// `SELECT ... FOR UPDATE` locks the capacity bucket row before the SUM read
/// (drift bound: at most K-1 rows over cap under maximum concurrency).
#[derive(Clone)]
pub struct PostgresGateResolutionStore {
    pool: Pool,
    k_buckets: u32,
}

impl PostgresGateResolutionStore {
    /// Build a new store from a connection pool.
    ///
    /// `k_buckets` is the number of capacity-counter buckets; pass
    /// `effective_capacity_counter_buckets()` for the operator-tunable value.
    pub fn new(pool: Pool, k_buckets: u32) -> Self {
        Self { pool, k_buckets }
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, GateResolutionStoreError> {
        self.pool
            .get()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Resolve `user_id` from a `TurnScope`, falling back to the system sentinel.
fn user_id_str(scope: &TurnScope) -> String {
    scope
        .explicit_owner_user_id()
        .map(|uid| uid.as_str().to_string())
        .unwrap_or_else(|| ironclaw_host_api::SYSTEM_RESERVED_ID.to_string())
}

/// Parse a `TurnStatus` from a TEXT column value.
fn parse_status(s: &str) -> Result<TurnStatus, GateResolutionStoreError> {
    match s {
        "completed" => Ok(TurnStatus::Completed),
        "failed" => Ok(TurnStatus::Failed),
        "cancelled" => Ok(TurnStatus::Cancelled),
        "cancel_requested" => Ok(TurnStatus::CancelRequested),
        "running" => Ok(TurnStatus::Running),
        "queued" => Ok(TurnStatus::Queued),
        "blocked_approval" => Ok(TurnStatus::BlockedApproval),
        "blocked_auth" => Ok(TurnStatus::BlockedAuth),
        "blocked_resource" => Ok(TurnStatus::BlockedResource),
        "blocked_dependent_run" => Ok(TurnStatus::BlockedDependentRun),
        "recovery_required" => Ok(TurnStatus::RecoveryRequired),
        other => Err(GateResolutionStoreError::io(
            "parse_status",
            format!("unknown TurnStatus value: {other}"),
        )),
    }
}

fn status_str(s: TurnStatus) -> &'static str {
    match s {
        TurnStatus::Completed => "completed",
        TurnStatus::Failed => "failed",
        TurnStatus::Cancelled => "cancelled",
        TurnStatus::CancelRequested => "cancel_requested",
        TurnStatus::Running => "running",
        TurnStatus::Queued => "queued",
        TurnStatus::BlockedApproval => "blocked_approval",
        TurnStatus::BlockedAuth => "blocked_auth",
        TurnStatus::BlockedResource => "blocked_resource",
        TurnStatus::BlockedDependentRun => "blocked_dependent_run",
        TurnStatus::RecoveryRequired => "recovery_required",
    }
}

/// Decode a tokio_postgres `Row` from `subagent_gate_awaited_children`.
fn decode_row(row: &tokio_postgres::Row) -> Result<AwaitedChildRow, GateResolutionStoreError> {
    let gate_ref_str: String = row
        .try_get::<_, String>(0)
        .map_err(|e| GateResolutionStoreError::io("decode_row/gate_ref", e.to_string()))?;
    let child_run_id_str: String = row
        .try_get::<_, String>(1)
        .map_err(|e| GateResolutionStoreError::io("decode_row/child_run_id", e.to_string()))?;
    let parent_run_id_str: String = row
        .try_get::<_, String>(2)
        .map_err(|e| GateResolutionStoreError::io("decode_row/parent_run_id", e.to_string()))?;
    let tree_root_run_id_str: String = row
        .try_get::<_, String>(3)
        .map_err(|e| GateResolutionStoreError::io("decode_row/tree_root_run_id", e.to_string()))?;
    // child_scope_json is JSONB in PG; fetch as serde_json::Value then stringify
    let child_scope_json_val: serde_json::Value = row
        .try_get::<_, serde_json::Value>(4)
        .map_err(|e| GateResolutionStoreError::io("decode_row/child_scope_json", e.to_string()))?;
    let parent_run_context_json_val: serde_json::Value =
        row.try_get::<_, serde_json::Value>(5).map_err(|e| {
            GateResolutionStoreError::io("decode_row/parent_run_context_json", e.to_string())
        })?;
    let source_binding_ref: String = row.try_get::<_, String>(6).map_err(|e| {
        GateResolutionStoreError::io("decode_row/source_binding_ref", e.to_string())
    })?;
    let reply_target_binding_ref: String = row.try_get::<_, String>(7).map_err(|e| {
        GateResolutionStoreError::io("decode_row/reply_target_binding_ref", e.to_string())
    })?;
    let subagent_kind: String = row
        .try_get::<_, String>(8)
        .map_err(|e| GateResolutionStoreError::io("decode_row/subagent_kind", e.to_string()))?;
    let spawn_capability_id: String = row.try_get::<_, String>(9).map_err(|e| {
        GateResolutionStoreError::io("decode_row/spawn_capability_id", e.to_string())
    })?;
    let result_ref_str: String = row
        .try_get::<_, String>(10)
        .map_err(|e| GateResolutionStoreError::io("decode_row/result_ref", e.to_string()))?;
    let spawn_mode: String = row
        .try_get::<_, String>(11)
        .map_err(|e| GateResolutionStoreError::io("decode_row/spawn_mode", e.to_string()))?;
    let terminal_status_raw: Option<String> = row
        .try_get::<_, Option<String>>(12)
        .map_err(|e| GateResolutionStoreError::io("decode_row/terminal_status", e.to_string()))?;
    let terminal_event_json_val: Option<serde_json::Value> = row
        .try_get::<_, Option<serde_json::Value>>(13)
        .map_err(|e| {
            GateResolutionStoreError::io("decode_row/terminal_event_json", e.to_string())
        })?;
    let terminal_result_written: bool = row.try_get::<_, bool>(14).map_err(|e| {
        GateResolutionStoreError::io("decode_row/terminal_result_written", e.to_string())
    })?;
    let terminal_byte_len: i64 = row
        .try_get::<_, i64>(15)
        .map_err(|e| GateResolutionStoreError::io("decode_row/terminal_byte_len", e.to_string()))?;
    let delivery_claimed: bool = row
        .try_get::<_, bool>(16)
        .map_err(|e| GateResolutionStoreError::io("decode_row/delivery_claimed", e.to_string()))?;
    let delivered_to_parent: bool = row.try_get::<_, bool>(17).map_err(|e| {
        GateResolutionStoreError::io("decode_row/delivered_to_parent", e.to_string())
    })?;

    let gate_ref = GateRef::new(&gate_ref_str)
        .map_err(|e| GateResolutionStoreError::io("decode_row/gate_ref_parse", e))?;
    let child_run_id = TurnRunId::parse(&child_run_id_str).map_err(|e| {
        GateResolutionStoreError::io("decode_row/child_run_id_parse", e.to_string())
    })?;
    let parent_run_id = TurnRunId::parse(&parent_run_id_str).map_err(|e| {
        GateResolutionStoreError::io("decode_row/parent_run_id_parse", e.to_string())
    })?;
    let tree_root_run_id = TurnRunId::parse(&tree_root_run_id_str).map_err(|e| {
        GateResolutionStoreError::io("decode_row/tree_root_run_id_parse", e.to_string())
    })?;
    let result_ref = LoopResultRef::new(&result_ref_str)
        .map_err(|e| GateResolutionStoreError::io("decode_row/result_ref_parse", e))?;
    let terminal_status = terminal_status_raw.map(|s| parse_status(&s)).transpose()?;

    let child_scope_json = serde_json::to_string(&child_scope_json_val)
        .map_err(|e| GateResolutionStoreError::serialization(e.to_string()))?;
    let parent_run_context_json = serde_json::to_string(&parent_run_context_json_val)
        .map_err(|e| GateResolutionStoreError::serialization(e.to_string()))?;
    let terminal_event_json = terminal_event_json_val
        .map(|v| {
            serde_json::to_string(&v)
                .map_err(|e| GateResolutionStoreError::serialization(e.to_string()))
        })
        .transpose()?;

    Ok(AwaitedChildRow {
        gate_ref,
        child_run_id,
        parent_run_id,
        tree_root_run_id,
        child_scope_json,
        parent_run_context_json,
        source_binding_ref,
        reply_target_binding_ref,
        subagent_kind,
        spawn_capability_id,
        result_ref,
        spawn_mode,
        terminal_status,
        terminal_event_json,
        terminal_result_written,
        terminal_byte_len: terminal_byte_len as u64,
        delivery_claimed,
        delivered_to_parent,
    })
}

// ── trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl DurableSubagentGateResolutionStore for PostgresGateResolutionStore {
    async fn record_awaited_child(
        &self,
        scope: &TurnScope,
        record: AwaitedChildRecord,
    ) -> Result<(), GateResolutionStoreError> {
        let mut client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());
        let k = self.k_buckets;
        let bucket = child_bucket(&record.child_run_id.to_string(), k) as i16;

        let transaction = client
            .build_transaction()
            .start()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

        // Initialize bucket row if missing (ON CONFLICT DO NOTHING with expression index).
        let init_sql = if agent_id.is_some() {
            "INSERT INTO subagent_gate_capacity_counter \
                (tenant_id, user_id, agent_id, bucket, undelivered) \
             VALUES ($1, $2, $3, $4, 0) \
             ON CONFLICT (tenant_id, user_id, COALESCE(agent_id, '__non_agent__'), bucket) \
             DO NOTHING"
        } else {
            "INSERT INTO subagent_gate_capacity_counter \
                (tenant_id, user_id, agent_id, bucket, undelivered) \
             VALUES ($1, $2, NULL, $3, 0) \
             ON CONFLICT (tenant_id, user_id, COALESCE(agent_id, '__non_agent__'), bucket) \
             DO NOTHING"
        };
        {
            let p: &[&(dyn tokio_postgres::types::ToSql + Sync)] = if let Some(ref aid) = agent_id {
                &[&tenant_id, &user_id, aid, &bucket]
            } else {
                &[&tenant_id, &user_id, &bucket]
            };
            transaction
                .execute(init_sql, p)
                .await
                .map_err(|e| GateResolutionStoreError::io("pg_init_bucket", e.to_string()))?;
        }

        // Lock the bucket row for this spawn (SELECT FOR UPDATE on our bucket).
        // This prevents TOCTOU drift bounded to K-1 rows under maximum concurrency.
        let lock_sql = if agent_id.is_some() {
            "SELECT undelivered FROM subagent_gate_capacity_counter \
              WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND bucket = $4 \
              FOR UPDATE"
        } else {
            "SELECT undelivered FROM subagent_gate_capacity_counter \
              WHERE tenant_id = $1 AND user_id = $2 AND agent_id IS NULL AND bucket = $3 \
              FOR UPDATE"
        };
        {
            let p: &[&(dyn tokio_postgres::types::ToSql + Sync)] = if let Some(ref aid) = agent_id {
                &[&tenant_id, &user_id, aid, &bucket]
            } else {
                &[&tenant_id, &user_id, &bucket]
            };
            transaction
                .query_opt(lock_sql, p)
                .await
                .map_err(|e| GateResolutionStoreError::io("pg_lock_bucket", e.to_string()))?;
        }

        // Cap check: SUM(undelivered) across all buckets for this scope.
        let sum_sql = if agent_id.is_some() {
            "SELECT COALESCE(SUM(undelivered), 0) FROM subagent_gate_capacity_counter \
              WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3"
        } else {
            "SELECT COALESCE(SUM(undelivered), 0) FROM subagent_gate_capacity_counter \
              WHERE tenant_id = $1 AND user_id = $2 AND agent_id IS NULL"
        };
        let sum_row = if let Some(ref aid) = agent_id {
            transaction
                .query_one(sum_sql, &[&tenant_id, &user_id, aid])
                .await
        } else {
            transaction
                .query_one(sum_sql, &[&tenant_id, &user_id])
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_cap_check", e.to_string()))?;

        let total: i64 = sum_row
            .try_get::<_, i64>(0)
            .map_err(|e| GateResolutionStoreError::io("pg_cap_check_decode", e.to_string()))?;

        if total >= MAX_GATE_RECORDS as i64 {
            transaction.rollback().await.ok();
            return Err(GateResolutionStoreError::CapacityExceeded);
        }

        // Parse child_scope_json for JSONB column.
        let child_scope_val: serde_json::Value = serde_json::from_str(&record.child_scope_json)
            .map_err(|e| GateResolutionStoreError::serialization(e.to_string()))?;
        let parent_ctx_val: serde_json::Value =
            serde_json::from_str(&record.parent_run_context_json)
                .map_err(|e| GateResolutionStoreError::serialization(e.to_string()))?;

        // INSERT ON CONFLICT DO NOTHING primary row (first-writer-wins per spec §1.6).
        let insert_sql = if agent_id.is_some() {
            "INSERT INTO subagent_gate_awaited_children \
                (tenant_id, user_id, agent_id, gate_ref, parent_run_id, tree_root_run_id, \
                 child_run_id, child_thread_id, child_scope_json, parent_run_context_json, \
                 source_binding_ref, reply_target_binding_ref, subagent_kind, spawn_capability_id, \
                 result_ref, spawn_mode, counter_bucket) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) \
             ON CONFLICT (gate_ref, child_run_id) DO NOTHING"
        } else {
            "INSERT INTO subagent_gate_awaited_children \
                (tenant_id, user_id, agent_id, gate_ref, parent_run_id, tree_root_run_id, \
                 child_run_id, child_thread_id, child_scope_json, parent_run_context_json, \
                 source_binding_ref, reply_target_binding_ref, subagent_kind, spawn_capability_id, \
                 result_ref, spawn_mode, counter_bucket) \
             VALUES ($1,$2,NULL,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) \
             ON CONFLICT (gate_ref, child_run_id) DO NOTHING"
        };

        let gate_ref_str = record.gate_ref.as_str().to_string();
        let parent_run_id_str = record.parent_run_id.to_string();
        let tree_root_str = record.tree_root_run_id.to_string();
        let child_run_id_str = record.child_run_id.to_string();
        let result_ref_str = record.result_ref.as_str().to_string();

        let rows_inserted = {
            let p: &[&(dyn tokio_postgres::types::ToSql + Sync)] = if let Some(ref aid) = agent_id {
                &[
                    &tenant_id,
                    &user_id,
                    aid,
                    &gate_ref_str,
                    &parent_run_id_str,
                    &tree_root_str,
                    &child_run_id_str,
                    &record.child_thread_id,
                    &child_scope_val,
                    &parent_ctx_val,
                    &record.source_binding_ref,
                    &record.reply_target_binding_ref,
                    &record.subagent_kind,
                    &record.spawn_capability_id,
                    &result_ref_str,
                    &record.spawn_mode,
                    &bucket,
                ]
            } else {
                &[
                    &tenant_id,
                    &user_id,
                    &gate_ref_str,
                    &parent_run_id_str,
                    &tree_root_str,
                    &child_run_id_str,
                    &record.child_thread_id,
                    &child_scope_val,
                    &parent_ctx_val,
                    &record.source_binding_ref,
                    &record.reply_target_binding_ref,
                    &record.subagent_kind,
                    &record.spawn_capability_id,
                    &result_ref_str,
                    &record.spawn_mode,
                    &bucket,
                ]
            };
            transaction.execute(insert_sql, p).await.map_err(|e| {
                GateResolutionStoreError::io("pg_insert_awaited_child", e.to_string())
            })?
        };

        // INSERT ON CONFLICT DO NOTHING reverse-index row.
        let idx_sql = if agent_id.is_some() {
            "INSERT INTO subagent_gate_child_index \
                (tenant_id, user_id, agent_id, child_run_id, gate_ref) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (child_run_id, gate_ref) DO NOTHING"
        } else {
            "INSERT INTO subagent_gate_child_index \
                (tenant_id, user_id, agent_id, child_run_id, gate_ref) \
             VALUES ($1, $2, NULL, $3, $4) \
             ON CONFLICT (child_run_id, gate_ref) DO NOTHING"
        };
        if let Some(ref aid) = agent_id {
            transaction
                .execute(
                    idx_sql,
                    &[&tenant_id, &user_id, aid, &child_run_id_str, &gate_ref_str],
                )
                .await
        } else {
            transaction
                .execute(
                    idx_sql,
                    &[&tenant_id, &user_id, &child_run_id_str, &gate_ref_str],
                )
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_insert_child_index", e.to_string()))?;

        // F2: only increment the counter when a NEW row was actually inserted.
        // Replayed / duplicate calls skip the INSERT (0 rows affected) and must
        // NOT touch the counter — otherwise each replay inflates capacity.
        if rows_inserted > 0 {
            let incr_sql = if agent_id.is_some() {
                "UPDATE subagent_gate_capacity_counter                    SET undelivered = undelivered + 1                  WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND bucket = $4"
            } else {
                "UPDATE subagent_gate_capacity_counter                    SET undelivered = undelivered + 1                  WHERE tenant_id = $1 AND user_id = $2 AND agent_id IS NULL AND bucket = $3"
            };
            {
                let p: &[&(dyn tokio_postgres::types::ToSql + Sync)] =
                    if let Some(ref aid) = agent_id {
                        &[&tenant_id, &user_id, aid, &bucket]
                    } else {
                        &[&tenant_id, &user_id, &bucket]
                    };
                transaction
                    .execute(incr_sql, p)
                    .await
                    .map_err(|e| GateResolutionStoreError::io("pg_incr_bucket", e.to_string()))?;
            }
        }

        transaction
            .commit()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;
        Ok(())
    }

    async fn record_child_terminal(
        &self,
        scope: &TurnScope,
        gate_ref: GateRef,
        child_run_id: TurnRunId,
        event: DurableTerminalEvent,
    ) -> Result<(), GateResolutionStoreError> {
        if !event.status.is_terminal() {
            return Err(GateResolutionStoreError::NonTerminalStatus);
        }

        let mut client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        let event_json_val = serde_json::json!({
            "status": status_str(event.status),
            "kind": event.kind,
            "cursor": event.cursor,
            "sanitized_reason": event.sanitized_reason,
            "owner_user_id": event.owner_user_id.as_ref().map(|u| u.as_str()),
        });

        let gate_ref_str = gate_ref.as_str().to_string();
        let child_run_id_str = child_run_id.to_string();
        let status_s = status_str(event.status);

        let transaction = client
            .build_transaction()
            .start()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

        // UPDATE only when terminal_status IS NULL (first-writer-wins).
        let update_sql = if agent_id.is_some() {
            "UPDATE subagent_gate_awaited_children \
               SET terminal_status = $1, terminal_event_json = $2, settled_at = NOW() \
             WHERE gate_ref = $3 AND child_run_id = $4 AND terminal_status IS NULL \
               AND tenant_id = $5 AND user_id = $6 AND agent_id = $7"
        } else {
            "UPDATE subagent_gate_awaited_children \
               SET terminal_status = $1, terminal_event_json = $2, settled_at = NOW() \
             WHERE gate_ref = $3 AND child_run_id = $4 AND terminal_status IS NULL \
               AND tenant_id = $5 AND user_id = $6 AND agent_id IS NULL"
        };
        let rows_changed = {
            let p: &[&(dyn tokio_postgres::types::ToSql + Sync)] = if let Some(ref aid) = agent_id {
                &[
                    &status_s,
                    &event_json_val,
                    &gate_ref_str,
                    &child_run_id_str,
                    &tenant_id,
                    &user_id,
                    aid,
                ]
            } else {
                &[
                    &status_s,
                    &event_json_val,
                    &gate_ref_str,
                    &child_run_id_str,
                    &tenant_id,
                    &user_id,
                ]
            };
            transaction.execute(update_sql, p).await.map_err(|e| {
                GateResolutionStoreError::io("pg_record_terminal_update", e.to_string())
            })?
        };

        if rows_changed > 0 {
            // Append settlement log row.
            let owner_str = event.owner_user_id.as_ref().map(|u| u.as_str().to_string());
            let cursor_i64 = event.cursor as i64;
            let log_sql = if agent_id.is_some() {
                "INSERT INTO subagent_gate_settlement_log \
                    (tenant_id, user_id, agent_id, gate_ref, child_run_id, result_ref, \
                     parent_run_id, terminal_status, terminal_kind, event_cursor, \
                     terminal_byte_len, sanitized_reason, owner_user_id) \
                 SELECT c.tenant_id, c.user_id, c.agent_id, c.gate_ref, c.child_run_id, \
                        c.result_ref, c.parent_run_id, $1, $2, $3, 0, $4, $5 \
                   FROM subagent_gate_awaited_children c \
                  WHERE c.gate_ref = $6 AND c.child_run_id = $7 \
                    AND c.tenant_id = $8 AND c.user_id = $9 AND c.agent_id = $10"
            } else {
                "INSERT INTO subagent_gate_settlement_log \
                    (tenant_id, user_id, agent_id, gate_ref, child_run_id, result_ref, \
                     parent_run_id, terminal_status, terminal_kind, event_cursor, \
                     terminal_byte_len, sanitized_reason, owner_user_id) \
                 SELECT c.tenant_id, c.user_id, c.agent_id, c.gate_ref, c.child_run_id, \
                        c.result_ref, c.parent_run_id, $1, $2, $3, 0, $4, $5 \
                   FROM subagent_gate_awaited_children c \
                  WHERE c.gate_ref = $6 AND c.child_run_id = $7 \
                    AND c.tenant_id = $8 AND c.user_id = $9 AND c.agent_id IS NULL"
            };
            {
                let p: &[&(dyn tokio_postgres::types::ToSql + Sync)] =
                    if let Some(ref aid) = agent_id {
                        &[
                            &status_s,
                            &event.kind,
                            &cursor_i64,
                            &event.sanitized_reason,
                            &owner_str,
                            &gate_ref_str,
                            &child_run_id_str,
                            &tenant_id,
                            &user_id,
                            aid,
                        ]
                    } else {
                        &[
                            &status_s,
                            &event.kind,
                            &cursor_i64,
                            &event.sanitized_reason,
                            &owner_str,
                            &gate_ref_str,
                            &child_run_id_str,
                            &tenant_id,
                            &user_id,
                        ]
                    };
                transaction.execute(log_sql, p).await.map_err(|e| {
                    GateResolutionStoreError::io("pg_settlement_log_insert", e.to_string())
                })?;
            }

            // Insert deliverable queue entry (ON CONFLICT DO NOTHING).
            let queue_sql = if agent_id.is_some() {
                "INSERT INTO subagent_gate_deliverable_queue \
                    (tenant_id, user_id, agent_id, child_run_id, gate_ref) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (child_run_id, gate_ref) DO NOTHING"
            } else {
                "INSERT INTO subagent_gate_deliverable_queue \
                    (tenant_id, user_id, agent_id, child_run_id, gate_ref) \
                 VALUES ($1, $2, NULL, $3, $4) \
                 ON CONFLICT (child_run_id, gate_ref) DO NOTHING"
            };
            if let Some(ref aid) = agent_id {
                transaction
                    .execute(
                        queue_sql,
                        &[&tenant_id, &user_id, aid, &child_run_id_str, &gate_ref_str],
                    )
                    .await
            } else {
                transaction
                    .execute(
                        queue_sql,
                        &[&tenant_id, &user_id, &child_run_id_str, &gate_ref_str],
                    )
                    .await
            }
            .map_err(|e| GateResolutionStoreError::io("pg_queue_insert", e.to_string()))?;
        }

        transaction
            .commit()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;
        Ok(())
    }

    async fn mark_terminal_result_written(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
        child_run_id: TurnRunId,
        byte_len: u64,
    ) -> Result<(), GateResolutionStoreError> {
        let client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());
        let byte_len_i64 = byte_len as i64;
        let gate_ref_str = gate_ref.as_str().to_string();
        let child_run_id_str = child_run_id.to_string();

        let update_sql = if agent_id.is_some() {
            "UPDATE subagent_gate_awaited_children \
               SET terminal_result_written = TRUE, terminal_byte_len = $1 \
             WHERE gate_ref = $2 AND child_run_id = $3 AND terminal_result_written = FALSE \
               AND tenant_id = $4 AND user_id = $5 AND agent_id = $6"
        } else {
            "UPDATE subagent_gate_awaited_children \
               SET terminal_result_written = TRUE, terminal_byte_len = $1 \
             WHERE gate_ref = $2 AND child_run_id = $3 AND terminal_result_written = FALSE \
               AND tenant_id = $4 AND user_id = $5 AND agent_id IS NULL"
        };
        if let Some(ref aid) = agent_id {
            client
                .execute(
                    update_sql,
                    &[
                        &byte_len_i64,
                        &gate_ref_str,
                        &child_run_id_str,
                        &tenant_id,
                        &user_id,
                        aid,
                    ],
                )
                .await
        } else {
            client
                .execute(
                    update_sql,
                    &[
                        &byte_len_i64,
                        &gate_ref_str,
                        &child_run_id_str,
                        &tenant_id,
                        &user_id,
                    ],
                )
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_mark_result_written", e.to_string()))?;
        Ok(())
    }

    async fn mark_child_delivered(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
        child_run_id: TurnRunId,
    ) -> Result<bool, GateResolutionStoreError> {
        let mut client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());
        let gate_ref_str = gate_ref.as_str().to_string();
        let child_run_id_str = child_run_id.to_string();

        let transaction = client
            .build_transaction()
            .start()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

        // Fetch the counter_bucket for this specific spawn.
        let bucket_row = transaction
            .query_opt(
                "SELECT counter_bucket FROM subagent_gate_awaited_children \
                  WHERE gate_ref = $1 AND child_run_id = $2",
                &[&gate_ref_str, &child_run_id_str],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("pg_fetch_bucket", e.to_string()))?;
        let bucket: i16 = match bucket_row {
            Some(r) => r
                .try_get::<_, i16>(0)
                .map_err(|e| GateResolutionStoreError::io("pg_decode_bucket", e.to_string()))?,
            None => {
                transaction.rollback().await.ok();
                return Ok(false);
            }
        };

        // Flip delivered flags (guard: delivered_to_parent = FALSE).
        let upd_sql = if agent_id.is_some() {
            "UPDATE subagent_gate_awaited_children \
               SET delivery_claimed = TRUE, delivered_to_parent = TRUE \
             WHERE gate_ref = $1 AND child_run_id = $2 AND delivered_to_parent = FALSE \
               AND tenant_id = $3 AND user_id = $4 AND agent_id = $5"
        } else {
            "UPDATE subagent_gate_awaited_children \
               SET delivery_claimed = TRUE, delivered_to_parent = TRUE \
             WHERE gate_ref = $1 AND child_run_id = $2 AND delivered_to_parent = FALSE \
               AND tenant_id = $3 AND user_id = $4 AND agent_id IS NULL"
        };
        let delivered_rows = if let Some(ref aid) = agent_id {
            transaction
                .execute(
                    upd_sql,
                    &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id, aid],
                )
                .await
        } else {
            transaction
                .execute(
                    upd_sql,
                    &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id],
                )
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_mark_delivered_update", e.to_string()))?;

        // F3: only decrement the counter and remove the queue entry when the
        // guarding UPDATE actually flipped the row (delivered_to_parent FALSE -> TRUE).
        // Retries / replays that observe 0 rows affected must NOT double-decrement.
        if delivered_rows > 0 {
            // Decrement the capacity bucket (GREATEST floor-at-zero).
            let decr_sql = if agent_id.is_some() {
                "UPDATE subagent_gate_capacity_counter                    SET undelivered = GREATEST(undelivered - 1, 0)                  WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND bucket = $4"
            } else {
                "UPDATE subagent_gate_capacity_counter                    SET undelivered = GREATEST(undelivered - 1, 0)                  WHERE tenant_id = $1 AND user_id = $2 AND agent_id IS NULL AND bucket = $3"
            };
            if let Some(ref aid) = agent_id {
                transaction
                    .execute(decr_sql, &[&tenant_id, &user_id, aid, &bucket])
                    .await
            } else {
                transaction
                    .execute(decr_sql, &[&tenant_id, &user_id, &bucket])
                    .await
            }
            .map_err(|e| GateResolutionStoreError::io("pg_decr_bucket", e.to_string()))?;

            // Delete the specific child's queue entry.
            let del_queue_sql = if agent_id.is_some() {
                "DELETE FROM subagent_gate_deliverable_queue                   WHERE gate_ref = $1 AND child_run_id = $2                     AND tenant_id = $3 AND user_id = $4 AND agent_id = $5"
            } else {
                "DELETE FROM subagent_gate_deliverable_queue                   WHERE gate_ref = $1 AND child_run_id = $2                     AND tenant_id = $3 AND user_id = $4 AND agent_id IS NULL"
            };
            if let Some(ref aid) = agent_id {
                transaction
                    .execute(
                        del_queue_sql,
                        &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id, aid],
                    )
                    .await
            } else {
                transaction
                    .execute(
                        del_queue_sql,
                        &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id],
                    )
                    .await
            }
            .map_err(|e| GateResolutionStoreError::io("pg_del_queue_entry", e.to_string()))?;
        }

        // Check if ALL children under this gate are now delivered.
        let all_row = transaction
            .query_one(
                "SELECT COUNT(*) FROM subagent_gate_awaited_children \
                  WHERE gate_ref = $1 AND delivered_to_parent = FALSE",
                &[&gate_ref_str],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("pg_all_delivered_check", e.to_string()))?;
        let remaining: i64 = all_row
            .try_get::<_, i64>(0)
            .map_err(|e| GateResolutionStoreError::io("pg_all_delivered_decode", e.to_string()))?;

        transaction
            .commit()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;
        Ok(remaining == 0)
    }

    async fn claim_next_terminal_state_for_child(
        &self,
        scope: &TurnScope,
        child_run_id: TurnRunId,
    ) -> Result<Option<AwaitedChildRow>, GateResolutionStoreError> {
        let mut results = self
            .claim_all_terminal_states_for_child(scope, child_run_id)
            .await?;
        Ok(results.drain(..1).next())
    }

    async fn claim_all_terminal_states_for_child(
        &self,
        scope: &TurnScope,
        child_run_id: TurnRunId,
    ) -> Result<Vec<AwaitedChildRow>, GateResolutionStoreError> {
        let client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());
        let child_run_id_str = child_run_id.to_string();

        // F4: scope BOTH the queue row (q) AND the child row (c) to the caller's
        // (tenant_id, user_id, agent_id).  Without scoping c, a caller who somehow
        // held a queue row pointing at a foreign (gate_ref, child_run_id) pair could
        // read that row's parent_run_context_json / terminal_event_json.
        let select_sql = if agent_id.is_some() {
            "SELECT c.gate_ref, c.child_run_id, c.parent_run_id, c.tree_root_run_id, \
                    c.child_scope_json, c.parent_run_context_json, c.source_binding_ref, \
                    c.reply_target_binding_ref, c.subagent_kind, c.spawn_capability_id, \
                    c.result_ref, c.spawn_mode, c.terminal_status, c.terminal_event_json, \
                    c.terminal_result_written, c.terminal_byte_len, \
                    c.delivery_claimed, c.delivered_to_parent \
               FROM subagent_gate_deliverable_queue q \
               JOIN subagent_gate_awaited_children c \
                 ON c.gate_ref = q.gate_ref AND c.child_run_id = q.child_run_id \
              WHERE q.child_run_id = $1 \
                AND q.tenant_id = $2 AND q.user_id = $3 AND q.agent_id = $4 \
                AND c.tenant_id = $2 AND c.user_id = $3 AND c.agent_id = $4 \
                AND c.delivered_to_parent = FALSE AND c.terminal_status IS NOT NULL"
        } else {
            "SELECT c.gate_ref, c.child_run_id, c.parent_run_id, c.tree_root_run_id, \
                    c.child_scope_json, c.parent_run_context_json, c.source_binding_ref, \
                    c.reply_target_binding_ref, c.subagent_kind, c.spawn_capability_id, \
                    c.result_ref, c.spawn_mode, c.terminal_status, c.terminal_event_json, \
                    c.terminal_result_written, c.terminal_byte_len, \
                    c.delivery_claimed, c.delivered_to_parent \
               FROM subagent_gate_deliverable_queue q \
               JOIN subagent_gate_awaited_children c \
                 ON c.gate_ref = q.gate_ref AND c.child_run_id = q.child_run_id \
              WHERE q.child_run_id = $1 \
                AND q.tenant_id = $2 AND q.user_id = $3 AND q.agent_id IS NULL \
                AND c.tenant_id = $2 AND c.user_id = $3 AND c.agent_id IS NULL \
                AND c.delivered_to_parent = FALSE AND c.terminal_status IS NOT NULL"
        };

        let rows = if let Some(ref aid) = agent_id {
            client
                .query(select_sql, &[&child_run_id_str, &tenant_id, &user_id, aid])
                .await
        } else {
            client
                .query(select_sql, &[&child_run_id_str, &tenant_id, &user_id])
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_claim_query", e.to_string()))?;

        rows.iter().map(decode_row).collect()
    }

    async fn delete_awaited_child(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
    ) -> Result<(), GateResolutionStoreError> {
        let mut client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());
        let gate_ref_str = gate_ref.as_str().to_string();

        let transaction = client
            .build_transaction()
            .start()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

        // Count undelivered rows by bucket for decrementing.
        let count_sql = if agent_id.is_some() {
            "SELECT counter_bucket, COUNT(*) FROM subagent_gate_awaited_children \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id = $4 \
                AND delivered_to_parent = FALSE \
              GROUP BY counter_bucket"
        } else {
            "SELECT counter_bucket, COUNT(*) FROM subagent_gate_awaited_children \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id IS NULL \
                AND delivered_to_parent = FALSE \
              GROUP BY counter_bucket"
        };
        let count_rows = if let Some(ref aid) = agent_id {
            transaction
                .query(count_sql, &[&gate_ref_str, &tenant_id, &user_id, aid])
                .await
        } else {
            transaction
                .query(count_sql, &[&gate_ref_str, &tenant_id, &user_id])
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_delete_count_buckets", e.to_string()))?;

        let mut bucket_counts: Vec<(i16, i64)> = Vec::new();
        for row in &count_rows {
            let b: i16 = row.try_get::<_, i16>(0).map_err(|e| {
                GateResolutionStoreError::io("pg_delete_decode_bucket", e.to_string())
            })?;
            let n: i64 = row.try_get::<_, i64>(1).map_err(|e| {
                GateResolutionStoreError::io("pg_delete_decode_count", e.to_string())
            })?;
            bucket_counts.push((b, n));
        }

        // Delete from all three tables.
        let del_queue_sql = if agent_id.is_some() {
            "DELETE FROM subagent_gate_deliverable_queue \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id = $4"
        } else {
            "DELETE FROM subagent_gate_deliverable_queue \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id IS NULL"
        };
        if let Some(ref aid) = agent_id {
            transaction
                .execute(del_queue_sql, &[&gate_ref_str, &tenant_id, &user_id, aid])
                .await
        } else {
            transaction
                .execute(del_queue_sql, &[&gate_ref_str, &tenant_id, &user_id])
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_del_queue_gate", e.to_string()))?;

        let del_idx_sql = if agent_id.is_some() {
            "DELETE FROM subagent_gate_child_index \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id = $4"
        } else {
            "DELETE FROM subagent_gate_child_index \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id IS NULL"
        };
        if let Some(ref aid) = agent_id {
            transaction
                .execute(del_idx_sql, &[&gate_ref_str, &tenant_id, &user_id, aid])
                .await
        } else {
            transaction
                .execute(del_idx_sql, &[&gate_ref_str, &tenant_id, &user_id])
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_del_child_idx", e.to_string()))?;

        let del_primary_sql = if agent_id.is_some() {
            "DELETE FROM subagent_gate_awaited_children \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id = $4"
        } else {
            "DELETE FROM subagent_gate_awaited_children \
              WHERE gate_ref = $1 AND tenant_id = $2 AND user_id = $3 AND agent_id IS NULL"
        };
        if let Some(ref aid) = agent_id {
            transaction
                .execute(del_primary_sql, &[&gate_ref_str, &tenant_id, &user_id, aid])
                .await
        } else {
            transaction
                .execute(del_primary_sql, &[&gate_ref_str, &tenant_id, &user_id])
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_del_primary", e.to_string()))?;

        // Decrement each touched bucket (GREATEST floor-at-zero).
        for (bucket, n) in bucket_counts {
            let decr_sql = if agent_id.is_some() {
                "UPDATE subagent_gate_capacity_counter \
                   SET undelivered = GREATEST(undelivered - $1, 0) \
                 WHERE tenant_id = $2 AND user_id = $3 AND agent_id = $4 AND bucket = $5"
            } else {
                "UPDATE subagent_gate_capacity_counter \
                   SET undelivered = GREATEST(undelivered - $1, 0) \
                 WHERE tenant_id = $2 AND user_id = $3 AND agent_id IS NULL AND bucket = $4"
            };
            {
                let p: &[&(dyn tokio_postgres::types::ToSql + Sync)] =
                    if let Some(ref aid) = agent_id {
                        &[&n, &tenant_id, &user_id, aid, &bucket]
                    } else {
                        &[&n, &tenant_id, &user_id, &bucket]
                    };
                transaction.execute(decr_sql, p).await.map_err(|e| {
                    GateResolutionStoreError::io("pg_decr_bucket_delete", e.to_string())
                })?;
            }
        }

        transaction
            .commit()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;
        Ok(())
    }

    // ── Reconciler-facing ──────────────────────────────────────────────────

    async fn gates_exist_batch(
        &self,
        scope: &TurnScope,
        gate_refs: Vec<GateRef>,
    ) -> Result<HashSet<GateRef>, GateResolutionStoreError> {
        if gate_refs.is_empty() {
            return Ok(HashSet::new());
        }
        let client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        // Build $N placeholders for the IN clause.
        let base = if agent_id.is_some() { 4 } else { 3 };
        let placeholders: Vec<String> = gate_refs
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", base + i))
            .collect();
        let in_clause = placeholders.join(", ");

        let agent_pred = if agent_id.is_some() {
            "agent_id = $3"
        } else {
            "agent_id IS NULL"
        };
        let sql = format!(
            "SELECT DISTINCT gate_ref FROM subagent_gate_awaited_children \
             WHERE tenant_id = $1 AND user_id = $2 AND {agent_pred} \
               AND gate_ref IN ({in_clause})"
        );

        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            vec![&tenant_id, &user_id];
        let agent_clone: Option<String> = agent_id.clone();
        if let Some(ref aid) = agent_clone {
            params.push(aid);
        }
        let ref_strs: Vec<String> = gate_refs.iter().map(|g| g.as_str().to_string()).collect();
        for s in &ref_strs {
            params.push(s);
        }

        let rows = client
            .query(&sql, &params)
            .await
            .map_err(|e| GateResolutionStoreError::io("pg_gates_exist_batch", e.to_string()))?;

        let mut found = HashSet::new();
        for row in &rows {
            let s: String = row
                .try_get::<_, String>(0)
                .map_err(|e| GateResolutionStoreError::io("pg_gates_exist_col", e.to_string()))?;
            if let Ok(gr) = GateRef::new(&s) {
                found.insert(gr);
            }
        }
        Ok(found)
    }

    async fn redeliver_settled_child(
        &self,
        scope: &TurnScope,
        gate_ref: GateRef,
        child_run_id: TurnRunId,
        terminal_status: TurnStatus,
        result_ref: LoopResultRef,
    ) -> Result<bool, GateResolutionStoreError> {
        let mut client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());
        let gate_ref_str = gate_ref.as_str().to_string();
        let child_run_id_str = child_run_id.to_string();
        let status_s = status_str(terminal_status);
        let result_ref_str = result_ref.as_str().to_string();

        let transaction = client
            .build_transaction()
            .start()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

        // Check existence — scoped to the caller's (tenant_id, user_id, agent_id)
        // so a foreign (gate_ref, child_run_id) pair cannot be rebound into
        // this caller's deliverable queue (F4 security fix).
        let exists_sql = if agent_id.is_some() {
            "SELECT 1 FROM subagent_gate_awaited_children               WHERE gate_ref = $1 AND child_run_id = $2                 AND tenant_id = $3 AND user_id = $4 AND agent_id = $5 LIMIT 1"
        } else {
            "SELECT 1 FROM subagent_gate_awaited_children               WHERE gate_ref = $1 AND child_run_id = $2                 AND tenant_id = $3 AND user_id = $4 AND agent_id IS NULL LIMIT 1"
        };
        let exists_row = if let Some(ref aid) = agent_id {
            transaction
                .query_opt(
                    exists_sql,
                    &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id, aid],
                )
                .await
        } else {
            transaction
                .query_opt(
                    exists_sql,
                    &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id],
                )
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_redeliver_check", e.to_string()))?;
        if exists_row.is_none() {
            transaction.rollback().await.ok();
            return Ok(false); // gate row vanished or belongs to a different scope
        }

        // Set terminal flags (idempotent: WHERE terminal_status IS NULL).
        let upd_sql = if agent_id.is_some() {
            "UPDATE subagent_gate_awaited_children \
               SET terminal_status = $1, terminal_result_written = TRUE, \
                   result_ref = $2, settled_at = COALESCE(settled_at, NOW()) \
             WHERE gate_ref = $3 AND child_run_id = $4 AND terminal_status IS NULL \
               AND tenant_id = $5 AND user_id = $6 AND agent_id = $7"
        } else {
            "UPDATE subagent_gate_awaited_children \
               SET terminal_status = $1, terminal_result_written = TRUE, \
                   result_ref = $2, settled_at = COALESCE(settled_at, NOW()) \
             WHERE gate_ref = $3 AND child_run_id = $4 AND terminal_status IS NULL \
               AND tenant_id = $5 AND user_id = $6 AND agent_id IS NULL"
        };
        if let Some(ref aid) = agent_id {
            transaction
                .execute(
                    upd_sql,
                    &[
                        &status_s,
                        &result_ref_str,
                        &gate_ref_str,
                        &child_run_id_str,
                        &tenant_id,
                        &user_id,
                        aid,
                    ],
                )
                .await
        } else {
            transaction
                .execute(
                    upd_sql,
                    &[
                        &status_s,
                        &result_ref_str,
                        &gate_ref_str,
                        &child_run_id_str,
                        &tenant_id,
                        &user_id,
                    ],
                )
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_redeliver_update", e.to_string()))?;

        // Ensure deliverable queue entry exists (ON CONFLICT DO NOTHING).
        let queue_sql = if agent_id.is_some() {
            "INSERT INTO subagent_gate_deliverable_queue \
                (tenant_id, user_id, agent_id, child_run_id, gate_ref) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (child_run_id, gate_ref) DO NOTHING"
        } else {
            "INSERT INTO subagent_gate_deliverable_queue \
                (tenant_id, user_id, agent_id, child_run_id, gate_ref) \
             VALUES ($1, $2, NULL, $3, $4) \
             ON CONFLICT (child_run_id, gate_ref) DO NOTHING"
        };
        if let Some(ref aid) = agent_id {
            transaction
                .execute(
                    queue_sql,
                    &[&tenant_id, &user_id, aid, &child_run_id_str, &gate_ref_str],
                )
                .await
        } else {
            transaction
                .execute(
                    queue_sql,
                    &[&tenant_id, &user_id, &child_run_id_str, &gate_ref_str],
                )
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("pg_redeliver_queue", e.to_string()))?;

        transaction
            .commit()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;
        Ok(true)
    }

    async fn resolve_undeliverable_batch(
        &self,
        scope: &TurnScope,
        rows: Vec<(GateRef, TurnRunId)>,
    ) -> Result<(), GateResolutionStoreError> {
        if rows.is_empty() {
            return Ok(());
        }

        // F5: apply the whole batch in ONE transaction — one BEGIN/COMMIT, N
        // per-row statements inside. This avoids partial commits on mid-batch
        // failure and N round trips.
        let mut client = self.client().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        let transaction = client
            .build_transaction()
            .start()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

        for (gate_ref, child_run_id) in rows {
            let gate_ref_str = gate_ref.as_str().to_string();
            let child_run_id_str = child_run_id.to_string();

            // Look up the counter_bucket for this row.
            let bucket_row = transaction
                .query_opt(
                    "SELECT counter_bucket FROM subagent_gate_awaited_children                       WHERE gate_ref = $1 AND child_run_id = $2",
                    &[&gate_ref_str, &child_run_id_str],
                )
                .await
                .map_err(|e| GateResolutionStoreError::io("pg_rub_fetch_bucket", e.to_string()))?;
            let bucket: i16 = match bucket_row {
                Some(r) => r.try_get::<_, i16>(0).map_err(|e| {
                    GateResolutionStoreError::io("pg_rub_decode_bucket", e.to_string())
                })?,
                None => continue, // row already gone — skip
            };

            // Flip delivered flags (guard: delivered_to_parent = FALSE).
            let upd_sql = if agent_id.is_some() {
                "UPDATE subagent_gate_awaited_children                    SET delivery_claimed = TRUE, delivered_to_parent = TRUE                  WHERE gate_ref = $1 AND child_run_id = $2 AND delivered_to_parent = FALSE                    AND tenant_id = $3 AND user_id = $4 AND agent_id = $5"
            } else {
                "UPDATE subagent_gate_awaited_children                    SET delivery_claimed = TRUE, delivered_to_parent = TRUE                  WHERE gate_ref = $1 AND child_run_id = $2 AND delivered_to_parent = FALSE                    AND tenant_id = $3 AND user_id = $4 AND agent_id IS NULL"
            };
            let delivered_rows = if let Some(ref aid) = agent_id {
                transaction
                    .execute(
                        upd_sql,
                        &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id, aid],
                    )
                    .await
            } else {
                transaction
                    .execute(
                        upd_sql,
                        &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id],
                    )
                    .await
            }
            .map_err(|e| GateResolutionStoreError::io("pg_rub_mark_delivered", e.to_string()))?;

            // F3: only decrement + delete queue if the UPDATE actually flipped the row.
            if delivered_rows > 0 {
                let decr_sql = if agent_id.is_some() {
                    "UPDATE subagent_gate_capacity_counter                        SET undelivered = GREATEST(undelivered - 1, 0)                      WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND bucket = $4"
                } else {
                    "UPDATE subagent_gate_capacity_counter                        SET undelivered = GREATEST(undelivered - 1, 0)                      WHERE tenant_id = $1 AND user_id = $2 AND agent_id IS NULL AND bucket = $3"
                };
                if let Some(ref aid) = agent_id {
                    transaction
                        .execute(decr_sql, &[&tenant_id, &user_id, aid, &bucket])
                        .await
                } else {
                    transaction
                        .execute(decr_sql, &[&tenant_id, &user_id, &bucket])
                        .await
                }
                .map_err(|e| GateResolutionStoreError::io("pg_rub_decr_bucket", e.to_string()))?;

                let del_queue_sql = if agent_id.is_some() {
                    "DELETE FROM subagent_gate_deliverable_queue                       WHERE gate_ref = $1 AND child_run_id = $2                         AND tenant_id = $3 AND user_id = $4 AND agent_id = $5"
                } else {
                    "DELETE FROM subagent_gate_deliverable_queue                       WHERE gate_ref = $1 AND child_run_id = $2                         AND tenant_id = $3 AND user_id = $4 AND agent_id IS NULL"
                };
                if let Some(ref aid) = agent_id {
                    transaction
                        .execute(
                            del_queue_sql,
                            &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id, aid],
                        )
                        .await
                } else {
                    transaction
                        .execute(
                            del_queue_sql,
                            &[&gate_ref_str, &child_run_id_str, &tenant_id, &user_id],
                        )
                        .await
                }
                .map_err(|e| GateResolutionStoreError::io("pg_rub_del_queue", e.to_string()))?;
            }
        }

        transaction
            .commit()
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;
        Ok(())
    }
}

/// Run all pending incremental migrations against the PostgreSQL pool.
///
/// Creates the `_reborn_migrations` tracking table first, then applies each
/// entry in [`super::migrations::INCREMENTAL_MIGRATIONS`] that is absent from
/// the table. All DDL uses `IF NOT EXISTS` for idempotency.
pub async fn run_postgres_gate_migrations(pool: &Pool) -> Result<(), GateResolutionStoreError> {
    use crate::postgres::migrations::{INCREMENTAL_MIGRATIONS, MIGRATIONS_TABLE_DDL};

    let client = pool
        .get()
        .await
        .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

    client
        .execute(MIGRATIONS_TABLE_DDL, &[])
        .await
        .map_err(|e| GateResolutionStoreError::io("pg_create_migrations_table", e.to_string()))?;

    for (version, name, ddl) in INCREMENTAL_MIGRATIONS {
        let already_applied = client
            .query_opt(
                "SELECT 1 FROM _reborn_migrations WHERE version = $1",
                &[version],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("pg_check_migration", e.to_string()))?;
        if already_applied.is_some() {
            continue;
        }

        // Execute each semicolon-separated DDL statement individually.
        for stmt in ddl.split(';') {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                continue;
            }
            client.execute(stmt, &[]).await.map_err(|e| {
                GateResolutionStoreError::io("pg_run_migration_stmt", e.to_string())
            })?;
        }

        client
            .execute(
                "INSERT INTO _reborn_migrations (version, name) VALUES ($1, $2)",
                &[version, name],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("pg_record_migration", e.to_string()))?;

        tracing::debug!(
            version = *version,
            name = *name,
            "reborn gate-resolution pg migration applied"
        );
    }
    Ok(())
}
