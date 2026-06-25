//! libSQL-backed durable gate-resolution store.
//!
//! Spec: `docs/reborn/2026-06-08-subagent-durability-spec.md` §1.3 + §1.6.
//!
//! # Scope-predicate convention
//!
//! Every query that filters by scope MUST use the conditional
//! `<agent_predicate>`:
//! - When `scope.agent_id` is `Some(id)`: `agent_id = ?` bound to `id`.
//! - When `scope.agent_id` is `None`: `agent_id IS NULL`.
//!
//! NEVER use `(agent_id = ? OR agent_id IS NULL)` — it allows agent-scoped
//! callers to reach system-level (NULL agent_id) rows.
//!
//! # First-writer-wins
//!
//! All INSERT paths use `INSERT OR IGNORE` keyed on the PRIMARY KEY so that
//! duplicate rows (replay, concurrent settlement) are silently dropped.
//!
//! # libSQL-specific notes
//!
//! - No `GREATEST()` — use `MAX(a, b)` for the floor-at-zero decrement.
//! - `BEGIN IMMEDIATE` for the spawn transaction (per-scope serialization).
//! - Boolean columns are `INTEGER` (0/1).
//! - Timestamps are `TEXT` in ISO-8601 format.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_turns::{GateRef, LoopResultRef, TurnRunId, TurnScope, TurnStatus};

use crate::gate_resolution::{
    AwaitedChildRecord, AwaitedChildRow, DurableSubagentGateResolutionStore, DurableTerminalEvent,
    GateResolutionStoreError, MAX_GATE_RECORDS, child_bucket,
};

/// libSQL connection handle type alias (avoids importing libsql directly at call sites).
type LibSqlDb = Arc<libsql::Database>;

/// libSQL-backed durable gate-resolution store.
///
/// Owns a reference to the libSQL `Database`. Each async method opens a
/// fresh connection and runs in a single transaction. For the spawn path
/// `BEGIN IMMEDIATE` serializes per-scope capacity accounting.
#[derive(Clone)]
pub struct LibSqlGateResolutionStore {
    db: LibSqlDb,
    k_buckets: u32,
    /// Per-scope cap; defaults to `MAX_GATE_RECORDS`. Tests may inject a
    /// smaller value via [`Self::new_with_limit`] to avoid inserting thousands
    /// of rows. The production default is never changed.
    max_records: u32,
}

impl LibSqlGateResolutionStore {
    /// Build a new store from a libSQL database handle.
    ///
    /// `k_buckets` is the number of capacity-counter buckets; pass
    /// `effective_capacity_counter_buckets()` for the operator-tunable value.
    pub fn new(db: LibSqlDb, k_buckets: u32) -> Self {
        Self {
            db,
            k_buckets,
            max_records: MAX_GATE_RECORDS,
        }
    }

    /// Build a new store with an overridden per-scope capacity limit.
    ///
    /// **For tests only.** The production default (`MAX_GATE_RECORDS`) is not
    /// changed by this constructor. Use this to avoid inserting thousands of
    /// rows in capacity-limit tests.
    pub fn new_with_limit(db: LibSqlDb, k_buckets: u32, max_records: u32) -> Self {
        Self {
            db,
            k_buckets,
            max_records,
        }
    }

    async fn conn(&self) -> Result<libsql::Connection, GateResolutionStoreError> {
        self.db
            .connect()
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

/// Decode a libSQL row from `subagent_gate_awaited_children`.
fn decode_row(row: &libsql::Row) -> Result<AwaitedChildRow, GateResolutionStoreError> {
    macro_rules! col_str {
        ($row:expr, $idx:expr) => {
            $row.get::<String>($idx)
                .map_err(|e| GateResolutionStoreError::io("decode_row", e.to_string()))?
        };
    }
    macro_rules! col_opt_str {
        ($row:expr, $idx:expr) => {
            $row.get::<Option<String>>($idx)
                .map_err(|e| GateResolutionStoreError::io("decode_row", e.to_string()))?
        };
    }
    macro_rules! col_i64 {
        ($row:expr, $idx:expr) => {
            $row.get::<i64>($idx)
                .map_err(|e| GateResolutionStoreError::io("decode_row", e.to_string()))?
        };
    }

    let gate_ref_str = col_str!(row, 0);
    let child_run_id_str = col_str!(row, 1);
    let parent_run_id_str = col_str!(row, 2);
    let tree_root_run_id_str = col_str!(row, 3);
    let child_scope_json = col_str!(row, 4);
    let parent_run_context_json = col_str!(row, 5);
    let source_binding_ref = col_str!(row, 6);
    let reply_target_binding_ref = col_str!(row, 7);
    let subagent_kind = col_str!(row, 8);
    let spawn_capability_id = col_str!(row, 9);
    let result_ref_str = col_str!(row, 10);
    let spawn_mode = col_str!(row, 11);
    let terminal_status_raw: Option<String> = col_opt_str!(row, 12);
    let terminal_event_json: Option<String> = col_opt_str!(row, 13);
    let terminal_result_written: i64 = col_i64!(row, 14);
    let terminal_byte_len: i64 = col_i64!(row, 15);
    let delivery_claimed: i64 = col_i64!(row, 16);
    let delivered_to_parent: i64 = col_i64!(row, 17);

    let gate_ref = GateRef::new(&gate_ref_str)
        .map_err(|e| GateResolutionStoreError::io("decode_row/gate_ref", e))?;
    let child_run_id = parse_run_id(&child_run_id_str, "child_run_id")?;
    let parent_run_id = parse_run_id(&parent_run_id_str, "parent_run_id")?;
    let tree_root_run_id = parse_run_id(&tree_root_run_id_str, "tree_root_run_id")?;
    let result_ref = LoopResultRef::new(&result_ref_str)
        .map_err(|e| GateResolutionStoreError::io("decode_row/result_ref", e))?;
    let terminal_status = terminal_status_raw.map(|s| parse_status(&s)).transpose()?;

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
        terminal_result_written: terminal_result_written != 0,
        terminal_byte_len: terminal_byte_len as u64,
        delivery_claimed: delivery_claimed != 0,
        delivered_to_parent: delivered_to_parent != 0,
    })
}

fn parse_run_id(s: &str, field: &'static str) -> Result<TurnRunId, GateResolutionStoreError> {
    TurnRunId::parse(s).map_err(|e| GateResolutionStoreError::io(field, e.to_string()))
}

// ── trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl DurableSubagentGateResolutionStore for LibSqlGateResolutionStore {
    async fn record_awaited_child(
        &self,
        scope: &TurnScope,
        record: AwaitedChildRecord,
    ) -> Result<(), GateResolutionStoreError> {
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());
        let k = self.k_buckets;
        let bucket = child_bucket(&record.child_run_id.to_string(), k) as i64;

        // BEGIN IMMEDIATE: per-scope serialization for capacity accounting.
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("BEGIN IMMEDIATE: {e}")))?;

        // Initialize bucket row if missing.
        // Use ON CONFLICT targeting the COALESCE expression index so that
        // NULL agent_id rows share the same unique (tenant, user, '', bucket) key.
        // Plain INSERT OR IGNORE with the former composite PK would treat each
        // NULL agent_id as distinct, creating a new counter row per agentless spawn.
        let init_sql = "INSERT INTO subagent_gate_capacity_counter             (tenant_id, user_id, agent_id, bucket, undelivered) VALUES (?, ?, ?, ?, 0)             ON CONFLICT (tenant_id, user_id, COALESCE(agent_id, ''), bucket) DO NOTHING";
        conn.execute(
            init_sql,
            libsql::params![tenant_id.clone(), user_id.clone(), agent_id.clone(), bucket],
        )
        .await
        .map_err(|e| GateResolutionStoreError::io("init_bucket", e.to_string()))?;

        // Cap check: SUM(undelivered) across all buckets for this scope.
        let sum_sql = if agent_id.is_some() {
            "SELECT COALESCE(SUM(undelivered), 0) FROM subagent_gate_capacity_counter \
             WHERE tenant_id = ? AND user_id = ? AND agent_id = ?"
        } else {
            "SELECT COALESCE(SUM(undelivered), 0) FROM subagent_gate_capacity_counter \
             WHERE tenant_id = ? AND user_id = ? AND agent_id IS NULL"
        };

        let mut rows = if let Some(ref aid) = agent_id {
            conn.query(
                sum_sql,
                libsql::params![tenant_id.clone(), user_id.clone(), aid.clone()],
            )
            .await
        } else {
            conn.query(sum_sql, libsql::params![tenant_id.clone(), user_id.clone()])
                .await
        }
        .map_err(|e| GateResolutionStoreError::io("cap_check", e.to_string()))?;

        let cap_row = rows
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("cap_check_row", e.to_string()))?;
        // COALESCE in the SQL already returns 0 for an empty scope, so a missing
        // row is a hard error (not a legitimate zero). Decode failure maps to Io.
        let total: i64 = match cap_row {
            Some(r) => r
                .get::<i64>(0)
                .map_err(|e| GateResolutionStoreError::io("cap_check_decode", e.to_string()))?,
            None => {
                // The COALESCE always returns a row even when there are no counter
                // rows; None here indicates a backend error.
                return Err(GateResolutionStoreError::io(
                    "cap_check_decode",
                    "SUM query returned no row",
                ));
            }
        };

        if total >= self.max_records as i64 {
            conn.execute("ROLLBACK", ()).await.ok();
            return Err(GateResolutionStoreError::CapacityExceeded);
        }

        // INSERT OR IGNORE primary row (first-writer-wins per spec §1.6).
        let insert_sql = "INSERT OR IGNORE INTO subagent_gate_awaited_children \
            (tenant_id, user_id, agent_id, gate_ref, parent_run_id, tree_root_run_id, \
             child_run_id, child_thread_id, child_scope_json, parent_run_context_json, \
             source_binding_ref, reply_target_binding_ref, subagent_kind, spawn_capability_id, \
             result_ref, spawn_mode, counter_bucket) \
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        let rows_inserted = conn
            .execute(
                insert_sql,
                libsql::params![
                    tenant_id.clone(),
                    user_id.clone(),
                    agent_id.clone(),
                    record.gate_ref.as_str().to_string(),
                    record.parent_run_id.to_string(),
                    record.tree_root_run_id.to_string(),
                    record.child_run_id.to_string(),
                    record.child_thread_id.clone(),
                    record.child_scope_json.clone(),
                    record.parent_run_context_json.clone(),
                    record.source_binding_ref.clone(),
                    record.reply_target_binding_ref.clone(),
                    record.subagent_kind.clone(),
                    record.spawn_capability_id.clone(),
                    record.result_ref.as_str().to_string(),
                    record.spawn_mode.clone(),
                    bucket
                ],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("insert_awaited_child", e.to_string()))?;

        // INSERT OR IGNORE reverse-index row.
        let idx_sql = "INSERT OR IGNORE INTO subagent_gate_child_index \
            (tenant_id, user_id, agent_id, child_run_id, gate_ref) VALUES (?, ?, ?, ?, ?)";
        conn.execute(
            idx_sql,
            libsql::params![
                tenant_id.clone(),
                user_id.clone(),
                agent_id.clone(),
                record.child_run_id.to_string(),
                record.gate_ref.as_str().to_string()
            ],
        )
        .await
        .map_err(|e| GateResolutionStoreError::io("insert_child_index", e.to_string()))?;

        // F2: only increment the counter when a NEW row was actually inserted.
        // Replayed / duplicate calls skip the INSERT (0 rows affected) and must
        // NOT touch the counter — otherwise each replay inflates capacity.
        if rows_inserted > 0 {
            let incr_sql = if agent_id.is_some() {
                "UPDATE subagent_gate_capacity_counter                   SET undelivered = undelivered + 1                 WHERE tenant_id = ? AND user_id = ? AND agent_id = ? AND bucket = ?"
            } else {
                "UPDATE subagent_gate_capacity_counter                   SET undelivered = undelivered + 1                 WHERE tenant_id = ? AND user_id = ? AND agent_id IS NULL AND bucket = ?"
            };
            if let Some(ref aid) = agent_id {
                conn.execute(
                    incr_sql,
                    libsql::params![tenant_id.clone(), user_id.clone(), aid.clone(), bucket],
                )
                .await
            } else {
                conn.execute(
                    incr_sql,
                    libsql::params![tenant_id.clone(), user_id.clone(), bucket],
                )
                .await
            }
            .map_err(|e| GateResolutionStoreError::io("incr_bucket", e.to_string()))?;
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("COMMIT: {e}")))?;
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

        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        let event_json = serde_json::json!({
            "status": status_str(event.status),
            "kind": event.kind,
            "cursor": event.cursor,
            "sanitized_reason": event.sanitized_reason,
            "owner_user_id": event.owner_user_id.as_ref().map(|u| u.as_str()),
        })
        .to_string();

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("BEGIN: {e}")))?;

        // UPDATE only when terminal_status IS NULL (first-writer-wins).
        let update_sql = build_agent_predicate_update(
            "UPDATE subagent_gate_awaited_children \
              SET terminal_status = ?, terminal_event_json = ?, settled_at = datetime('now') \
            WHERE gate_ref = ? AND child_run_id = ? AND terminal_status IS NULL \
              AND tenant_id = ? AND user_id = ? AND",
            agent_id.is_some(),
        );
        let rows_changed = if let Some(ref aid) = agent_id {
            conn.execute(
                &update_sql,
                libsql::params![
                    status_str(event.status),
                    event_json.clone(),
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.execute(
                &update_sql,
                libsql::params![
                    status_str(event.status),
                    event_json.clone(),
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("record_terminal_update", e.to_string()))?;

        if rows_changed > 0 {
            // Append settlement log row (only when terminal was freshly written).
            let log_sql = "INSERT INTO subagent_gate_settlement_log \
                (tenant_id, user_id, agent_id, gate_ref, child_run_id, result_ref, \
                 parent_run_id, terminal_status, terminal_kind, event_cursor, \
                 terminal_byte_len, sanitized_reason, owner_user_id) \
                SELECT tenant_id, user_id, agent_id, gate_ref, child_run_id, result_ref, \
                       parent_run_id, ?, ?, ?, 0, ?, ? \
                  FROM subagent_gate_awaited_children \
                 WHERE gate_ref = ? AND child_run_id = ? \
                   AND tenant_id = ? AND user_id = ?";
            let owner_str = event.owner_user_id.as_ref().map(|u| u.as_str().to_string());
            conn.execute(
                log_sql,
                libsql::params![
                    status_str(event.status),
                    event.kind.clone(),
                    event.cursor as i64,
                    event.sanitized_reason.clone(),
                    owner_str,
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("settlement_log_insert", e.to_string()))?;

            // Insert deliverable queue entry (INSERT OR IGNORE for idempotency).
            let queue_sql = "INSERT OR IGNORE INTO subagent_gate_deliverable_queue \
                (tenant_id, user_id, agent_id, child_run_id, gate_ref) VALUES (?, ?, ?, ?, ?)";
            conn.execute(
                queue_sql,
                libsql::params![
                    tenant_id.clone(),
                    user_id.clone(),
                    agent_id.clone(),
                    child_run_id.to_string(),
                    gate_ref.as_str().to_string()
                ],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("queue_insert", e.to_string()))?;
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("COMMIT: {e}")))?;
        Ok(())
    }

    async fn mark_terminal_result_written(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
        child_run_id: TurnRunId,
        byte_len: u64,
    ) -> Result<(), GateResolutionStoreError> {
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        let update_sql = build_agent_predicate_update(
            "UPDATE subagent_gate_awaited_children \
              SET terminal_result_written = 1, terminal_byte_len = ? \
            WHERE gate_ref = ? AND child_run_id = ? AND terminal_result_written = 0 \
              AND tenant_id = ? AND user_id = ? AND",
            agent_id.is_some(),
        );
        if let Some(ref aid) = agent_id {
            conn.execute(
                &update_sql,
                libsql::params![
                    byte_len as i64,
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id,
                    user_id,
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.execute(
                &update_sql,
                libsql::params![
                    byte_len as i64,
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id,
                    user_id
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("mark_result_written", e.to_string()))?;
        Ok(())
    }

    async fn mark_child_delivered(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
        child_run_id: TurnRunId,
    ) -> Result<bool, GateResolutionStoreError> {
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("BEGIN: {e}")))?;

        // Fetch the counter_bucket for this specific spawn.
        let bucket_sql = "SELECT counter_bucket FROM subagent_gate_awaited_children \
            WHERE gate_ref = ? AND child_run_id = ?";
        let mut bucket_rows = conn
            .query(
                bucket_sql,
                libsql::params![gate_ref.as_str().to_string(), child_run_id.to_string()],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("fetch_bucket", e.to_string()))?;
        let bucket: i64 = match bucket_rows
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("fetch_bucket_row", e.to_string()))?
        {
            Some(r) => r
                .get::<i64>(0)
                .map_err(|e| GateResolutionStoreError::io("fetch_bucket_col", e.to_string()))?,
            None => {
                conn.execute("ROLLBACK", ()).await.ok();
                return Ok(false);
            }
        };

        // Flip delivered flags (guard: delivered_to_parent = 0).
        let upd_sql = build_agent_predicate_update(
            "UPDATE subagent_gate_awaited_children \
              SET delivery_claimed = 1, delivered_to_parent = 1 \
            WHERE gate_ref = ? AND child_run_id = ? AND delivered_to_parent = 0 \
              AND tenant_id = ? AND user_id = ? AND",
            agent_id.is_some(),
        );
        let delivered_rows = if let Some(ref aid) = agent_id {
            conn.execute(
                &upd_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.execute(
                &upd_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("mark_delivered_update", e.to_string()))?;

        // F3: only decrement the counter and remove the queue entry when the
        // guarding UPDATE actually flipped the row (delivered_to_parent 0 -> 1).
        // If 0 rows were updated, the row was already delivered (retry / replay)
        // and we must NOT double-decrement capacity.
        if delivered_rows > 0 {
            // Decrement the capacity bucket (floor-at-zero via MAX).
            // libSQL has no GREATEST() — use MAX(undelivered - 1, 0).
            let decr_sql = build_agent_predicate_update(
                "UPDATE subagent_gate_capacity_counter                   SET undelivered = MAX(undelivered - 1, 0)                 WHERE tenant_id = ? AND user_id = ? AND bucket = ? AND",
                agent_id.is_some(),
            );
            // Note: for this query the positional params are tenant_id, user_id, bucket, [agent_id]
            if let Some(ref aid) = agent_id {
                conn.execute(
                    &decr_sql,
                    libsql::params![tenant_id.clone(), user_id.clone(), bucket, aid.clone()],
                )
                .await
            } else {
                conn.execute(
                    &decr_sql,
                    libsql::params![tenant_id.clone(), user_id.clone(), bucket],
                )
                .await
            }
            .map_err(|e| GateResolutionStoreError::io("decr_bucket", e.to_string()))?;

            // Delete the specific child's queue entry.
            let del_queue_sql = build_agent_predicate_update(
                "DELETE FROM subagent_gate_deliverable_queue                 WHERE gate_ref = ? AND child_run_id = ? AND tenant_id = ? AND user_id = ? AND",
                agent_id.is_some(),
            );
            if let Some(ref aid) = agent_id {
                conn.execute(
                    &del_queue_sql,
                    libsql::params![
                        gate_ref.as_str().to_string(),
                        child_run_id.to_string(),
                        tenant_id.clone(),
                        user_id.clone(),
                        aid.clone()
                    ],
                )
                .await
            } else {
                conn.execute(
                    &del_queue_sql,
                    libsql::params![
                        gate_ref.as_str().to_string(),
                        child_run_id.to_string(),
                        tenant_id.clone(),
                        user_id.clone()
                    ],
                )
                .await
            }
            .map_err(|e| GateResolutionStoreError::io("del_queue_entry", e.to_string()))?;
        }

        // Check if ALL children under this gate are now delivered.
        let all_sql = "SELECT COUNT(*) FROM subagent_gate_awaited_children \
            WHERE gate_ref = ? AND delivered_to_parent = 0";
        let mut check_rows = conn
            .query(all_sql, libsql::params![gate_ref.as_str().to_string()])
            .await
            .map_err(|e| GateResolutionStoreError::io("all_delivered_check", e.to_string()))?;
        let remaining: i64 = match check_rows
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("all_delivered_row", e.to_string()))?
        {
            Some(r) => r
                .get::<i64>(0)
                .map_err(|e| GateResolutionStoreError::io("all_delivered_col", e.to_string()))?,
            // COUNT(*) always returns a row; None here is a backend error.
            None => {
                return Err(GateResolutionStoreError::io(
                    "all_delivered_col",
                    "COUNT query returned no row",
                ));
            }
        };

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("COMMIT: {e}")))?;
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
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        // Join deliverable queue to primary table to fetch all rows.
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
              WHERE q.child_run_id = ? \
                AND q.tenant_id = ? AND q.user_id = ? AND q.agent_id = ? \
                AND c.delivered_to_parent = 0 AND c.terminal_status IS NOT NULL"
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
              WHERE q.child_run_id = ? \
                AND q.tenant_id = ? AND q.user_id = ? AND q.agent_id IS NULL \
                AND c.delivered_to_parent = 0 AND c.terminal_status IS NOT NULL"
        };

        let mut rows = if let Some(ref aid) = agent_id {
            conn.query(
                select_sql,
                libsql::params![child_run_id.to_string(), tenant_id, user_id, aid.clone()],
            )
            .await
        } else {
            conn.query(
                select_sql,
                libsql::params![child_run_id.to_string(), tenant_id, user_id],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("claim_query", e.to_string()))?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("claim_fetch", e.to_string()))?
        {
            result.push(decode_row(&row)?);
        }
        Ok(result)
    }

    async fn delete_awaited_child(
        &self,
        scope: &TurnScope,
        gate_ref: &GateRef,
    ) -> Result<(), GateResolutionStoreError> {
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("BEGIN: {e}")))?;

        // Count undelivered rows by bucket for decrementing.
        let count_sql = build_agent_predicate_select(
            "SELECT counter_bucket, COUNT(*) FROM subagent_gate_awaited_children \
            WHERE gate_ref = ? AND tenant_id = ? AND user_id = ? AND delivered_to_parent = 0 AND",
            "GROUP BY counter_bucket",
            agent_id.is_some(),
        );
        let mut count_rows = if let Some(ref aid) = agent_id {
            conn.query(
                &count_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.query(
                &count_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("delete_count_buckets", e.to_string()))?;

        let mut bucket_counts: Vec<(i64, i64)> = Vec::new();
        while let Some(row) = count_rows
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("delete_count_row", e.to_string()))?
        {
            let b: i64 = row.get::<i64>(0).map_err(|e| {
                GateResolutionStoreError::io("delete_count_bucket_col", e.to_string())
            })?;
            let n: i64 = row
                .get::<i64>(1)
                .map_err(|e| GateResolutionStoreError::io("delete_count_n_col", e.to_string()))?;
            bucket_counts.push((b, n));
        }

        // Delete from all three tables.
        let del_queue_sql = build_agent_predicate_delete(
            "DELETE FROM subagent_gate_deliverable_queue \
            WHERE gate_ref = ? AND tenant_id = ? AND user_id = ? AND",
            agent_id.is_some(),
        );
        if let Some(ref aid) = agent_id {
            conn.execute(
                &del_queue_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.execute(
                &del_queue_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("del_queue_gate", e.to_string()))?;

        let del_idx_sql = build_agent_predicate_delete(
            "DELETE FROM subagent_gate_child_index \
            WHERE gate_ref = ? AND tenant_id = ? AND user_id = ? AND",
            agent_id.is_some(),
        );
        if let Some(ref aid) = agent_id {
            conn.execute(
                &del_idx_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.execute(
                &del_idx_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("del_child_idx", e.to_string()))?;

        let del_primary_sql = build_agent_predicate_delete(
            "DELETE FROM subagent_gate_awaited_children \
            WHERE gate_ref = ? AND tenant_id = ? AND user_id = ? AND",
            agent_id.is_some(),
        );
        if let Some(ref aid) = agent_id {
            conn.execute(
                &del_primary_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.execute(
                &del_primary_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("del_primary", e.to_string()))?;

        // Decrement each touched bucket (MAX floor-at-zero).
        for (bucket, n) in bucket_counts {
            let decr_sql = build_agent_predicate_update(
                "UPDATE subagent_gate_capacity_counter \
                  SET undelivered = MAX(undelivered - ?, 0) \
                WHERE tenant_id = ? AND user_id = ? AND bucket = ? AND",
                agent_id.is_some(),
            );
            if let Some(ref aid) = agent_id {
                conn.execute(
                    &decr_sql,
                    libsql::params![n, tenant_id.clone(), user_id.clone(), bucket, aid.clone()],
                )
                .await
            } else {
                conn.execute(
                    &decr_sql,
                    libsql::params![n, tenant_id.clone(), user_id.clone(), bucket],
                )
                .await
            }
            .map_err(|e| GateResolutionStoreError::io("decr_bucket_delete", e.to_string()))?;
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("COMMIT: {e}")))?;
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
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        // Build IN clause placeholders.
        let placeholders: Vec<String> = gate_refs.iter().map(|_| "?".to_string()).collect();
        let in_clause = placeholders.join(", ");

        let agent_pred = if agent_id.is_some() {
            "agent_id = ?".to_string()
        } else {
            "agent_id IS NULL".to_string()
        };
        let sql = format!(
            "SELECT DISTINCT gate_ref FROM subagent_gate_awaited_children \
             WHERE tenant_id = ? AND user_id = ? AND {agent_pred} \
               AND gate_ref IN ({in_clause})"
        );

        let mut params: Vec<libsql::Value> = vec![tenant_id.into(), user_id.into()];
        if let Some(ref aid) = agent_id {
            params.push(aid.clone().into());
        }
        for gr in &gate_refs {
            params.push(gr.as_str().to_string().into());
        }

        let mut rows = conn
            .query(&sql, params)
            .await
            .map_err(|e| GateResolutionStoreError::io("gates_exist_batch", e.to_string()))?;

        let mut found = HashSet::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("gates_exist_row", e.to_string()))?
        {
            let s: String = row
                .get::<String>(0)
                .map_err(|e| GateResolutionStoreError::io("gates_exist_col", e.to_string()))?;
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
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("BEGIN: {e}")))?;

        // Check existence — scoped to the caller's (tenant_id, user_id, agent_id)
        // so a foreign (gate_ref, child_run_id) pair cannot be rebound into
        // this caller's deliverable queue (F4 security fix).
        let exists_sql = if agent_id.is_some() {
            "SELECT 1 FROM subagent_gate_awaited_children                 WHERE gate_ref = ? AND child_run_id = ?                   AND tenant_id = ? AND user_id = ? AND agent_id = ? LIMIT 1"
        } else {
            "SELECT 1 FROM subagent_gate_awaited_children                 WHERE gate_ref = ? AND child_run_id = ?                   AND tenant_id = ? AND user_id = ? AND agent_id IS NULL LIMIT 1"
        };
        let mut exists_rows = if let Some(ref aid) = agent_id {
            conn.query(
                exists_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.query(
                exists_sql,
                libsql::params![
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("redeliver_check", e.to_string()))?;
        if exists_rows
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("redeliver_check_row", e.to_string()))?
            .is_none()
        {
            conn.execute("ROLLBACK", ()).await.ok();
            return Ok(false); // gate row vanished or belongs to a different scope
        }

        // Set terminal flags (UPDATE WHERE terminal_status IS NULL = idempotent).
        let upd_sql = build_agent_predicate_update(
            "UPDATE subagent_gate_awaited_children \
              SET terminal_status = ?, terminal_result_written = 1, \
                  result_ref = ?, settled_at = COALESCE(settled_at, datetime('now')) \
            WHERE gate_ref = ? AND child_run_id = ? AND terminal_status IS NULL \
              AND tenant_id = ? AND user_id = ? AND",
            agent_id.is_some(),
        );
        if let Some(ref aid) = agent_id {
            conn.execute(
                &upd_sql,
                libsql::params![
                    status_str(terminal_status),
                    result_ref.as_str().to_string(),
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone(),
                    aid.clone()
                ],
            )
            .await
        } else {
            conn.execute(
                &upd_sql,
                libsql::params![
                    status_str(terminal_status),
                    result_ref.as_str().to_string(),
                    gate_ref.as_str().to_string(),
                    child_run_id.to_string(),
                    tenant_id.clone(),
                    user_id.clone()
                ],
            )
            .await
        }
        .map_err(|e| GateResolutionStoreError::io("redeliver_update", e.to_string()))?;

        // Ensure deliverable queue entry exists (INSERT OR IGNORE).
        let queue_sql = "INSERT OR IGNORE INTO subagent_gate_deliverable_queue \
            (tenant_id, user_id, agent_id, child_run_id, gate_ref) VALUES (?, ?, ?, ?, ?)";
        conn.execute(
            queue_sql,
            libsql::params![
                tenant_id.clone(),
                user_id.clone(),
                agent_id.clone(),
                child_run_id.to_string(),
                gate_ref.as_str().to_string()
            ],
        )
        .await
        .map_err(|e| GateResolutionStoreError::io("redeliver_queue", e.to_string()))?;

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("COMMIT: {e}")))?;
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
        let conn = self.conn().await?;
        let tenant_id = scope.tenant_id.as_str().to_string();
        let user_id = user_id_str(scope);
        let agent_id: Option<String> = scope.agent_id.as_ref().map(|a| a.as_str().to_string());

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("BEGIN: {e}")))?;

        for (gate_ref, child_run_id) in rows {
            // Look up the counter_bucket for this row.
            let bucket_sql = "SELECT counter_bucket FROM subagent_gate_awaited_children                 WHERE gate_ref = ? AND child_run_id = ?";
            let mut bucket_rows = conn
                .query(
                    bucket_sql,
                    libsql::params![gate_ref.as_str().to_string(), child_run_id.to_string()],
                )
                .await
                .map_err(|e| GateResolutionStoreError::io("rub_fetch_bucket", e.to_string()))?;
            let bucket: i64 =
                match bucket_rows.next().await.map_err(|e| {
                    GateResolutionStoreError::io("rub_fetch_bucket_row", e.to_string())
                })? {
                    Some(r) => r.get::<i64>(0).map_err(|e| {
                        GateResolutionStoreError::io("rub_fetch_bucket_col", e.to_string())
                    })?,
                    None => continue, // row already gone — skip
                };

            // Flip delivered flags (guard: delivered_to_parent = 0).
            let upd_sql = build_agent_predicate_update(
                "UPDATE subagent_gate_awaited_children                   SET delivery_claimed = 1, delivered_to_parent = 1                 WHERE gate_ref = ? AND child_run_id = ? AND delivered_to_parent = 0                   AND tenant_id = ? AND user_id = ? AND",
                agent_id.is_some(),
            );
            let delivered_rows = if let Some(ref aid) = agent_id {
                conn.execute(
                    &upd_sql,
                    libsql::params![
                        gate_ref.as_str().to_string(),
                        child_run_id.to_string(),
                        tenant_id.clone(),
                        user_id.clone(),
                        aid.clone()
                    ],
                )
                .await
            } else {
                conn.execute(
                    &upd_sql,
                    libsql::params![
                        gate_ref.as_str().to_string(),
                        child_run_id.to_string(),
                        tenant_id.clone(),
                        user_id.clone()
                    ],
                )
                .await
            }
            .map_err(|e| GateResolutionStoreError::io("rub_mark_delivered", e.to_string()))?;

            // F3: only decrement + delete queue if the UPDATE actually flipped the row.
            if delivered_rows > 0 {
                let decr_sql = build_agent_predicate_update(
                    "UPDATE subagent_gate_capacity_counter                       SET undelivered = MAX(undelivered - 1, 0)                     WHERE tenant_id = ? AND user_id = ? AND bucket = ? AND",
                    agent_id.is_some(),
                );
                if let Some(ref aid) = agent_id {
                    conn.execute(
                        &decr_sql,
                        libsql::params![tenant_id.clone(), user_id.clone(), bucket, aid.clone()],
                    )
                    .await
                } else {
                    conn.execute(
                        &decr_sql,
                        libsql::params![tenant_id.clone(), user_id.clone(), bucket],
                    )
                    .await
                }
                .map_err(|e| GateResolutionStoreError::io("rub_decr_bucket", e.to_string()))?;

                let del_queue_sql = build_agent_predicate_update(
                    "DELETE FROM subagent_gate_deliverable_queue                     WHERE gate_ref = ? AND child_run_id = ? AND tenant_id = ? AND user_id = ? AND",
                    agent_id.is_some(),
                );
                if let Some(ref aid) = agent_id {
                    conn.execute(
                        &del_queue_sql,
                        libsql::params![
                            gate_ref.as_str().to_string(),
                            child_run_id.to_string(),
                            tenant_id.clone(),
                            user_id.clone(),
                            aid.clone()
                        ],
                    )
                    .await
                } else {
                    conn.execute(
                        &del_queue_sql,
                        libsql::params![
                            gate_ref.as_str().to_string(),
                            child_run_id.to_string(),
                            tenant_id.clone(),
                            user_id.clone()
                        ],
                    )
                    .await
                }
                .map_err(|e| GateResolutionStoreError::io("rub_del_queue", e.to_string()))?;
            }
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| GateResolutionStoreError::unavailable(format!("COMMIT: {e}")))?;
        Ok(())
    }
}

// ── query-building helpers ────────────────────────────────────────────────────

/// Appends the agent predicate (`agent_id = ?` or `agent_id IS NULL`) to the
/// end of a partial SQL UPDATE statement.
fn build_agent_predicate_update(prefix: &str, has_agent: bool) -> String {
    if has_agent {
        format!("{prefix} agent_id = ?")
    } else {
        format!("{prefix} agent_id IS NULL")
    }
}

/// Appends the agent predicate to a partial SQL DELETE statement.
fn build_agent_predicate_delete(prefix: &str, has_agent: bool) -> String {
    if has_agent {
        format!("{prefix} agent_id = ?")
    } else {
        format!("{prefix} agent_id IS NULL")
    }
}

/// Appends the agent predicate and a trailing suffix to a SELECT statement.
fn build_agent_predicate_select(prefix: &str, suffix: &str, has_agent: bool) -> String {
    let pred = if has_agent {
        "agent_id = ?"
    } else {
        "agent_id IS NULL"
    };
    format!("{prefix} {pred} {suffix}")
}

/// Run all pending incremental migrations against the libSQL database.
///
/// Creates the `_reborn_migrations` tracking table first, then applies each
/// entry in [`super::migrations::INCREMENTAL_MIGRATIONS`] that is absent from
/// the table. All DDL uses `IF NOT EXISTS` for idempotency.
pub async fn run_libsql_gate_migrations(
    db: &libsql::Database,
) -> Result<(), GateResolutionStoreError> {
    use crate::libsql::migrations::{INCREMENTAL_MIGRATIONS, MIGRATIONS_TABLE_DDL};

    let conn = db
        .connect()
        .map_err(|e| GateResolutionStoreError::unavailable(e.to_string()))?;

    conn.execute(MIGRATIONS_TABLE_DDL, ())
        .await
        .map_err(|e| GateResolutionStoreError::io("create_migrations_table", e.to_string()))?;

    for (version, name, ddl) in INCREMENTAL_MIGRATIONS {
        let already_applied_rows = conn
            .query(
                "SELECT 1 FROM _reborn_migrations WHERE version = ?",
                libsql::params![*version],
            )
            .await
            .map_err(|e| GateResolutionStoreError::io("check_migration", e.to_string()))?
            .next()
            .await
            .map_err(|e| GateResolutionStoreError::io("check_migration_row", e.to_string()))?;
        if already_applied_rows.is_some() {
            continue;
        }

        // Each migration may contain multiple statements separated by semicolons.
        // Split and execute each individually (libsql does not support multi-statement exec).
        for stmt in ddl.split(';') {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                continue;
            }
            conn.execute(stmt, ())
                .await
                .map_err(|e| GateResolutionStoreError::io("run_migration_stmt", e.to_string()))?;
        }

        conn.execute(
            "INSERT INTO _reborn_migrations (version, name) VALUES (?, ?)",
            libsql::params![*version, *name],
        )
        .await
        .map_err(|e| GateResolutionStoreError::io("record_migration", e.to_string()))?;

        tracing::debug!(
            version = *version,
            name = *name,
            "reborn gate-resolution migration applied"
        );
    }
    Ok(())
}
