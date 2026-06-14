//! PostgreSQL integration tests for gate-resolution migrations and store ops.
//!
//! Gated behind `cfg(all(feature = "postgres", feature = "integration"))` so
//! they compile without a live Postgres server and only connect at runtime
//! when both feature flags are present.
//!
//! Run with:
//!   cargo test -p ironclaw_reborn_event_store --features postgres,integration \
//!     --test postgres_gate_migrations
//!
//! Requires environment variable:
//!   IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL=postgres://user:pass@localhost/db

#[cfg(all(feature = "postgres", feature = "integration"))]
mod pg_gate_tests {
    use ironclaw_host_api::{AgentId, TenantId, ThreadId};
    use ironclaw_reborn_event_store::{
        AwaitedChildRecord, DurableSubagentGateResolutionStore, DurableTerminalEvent,
        PostgresGateResolutionStore, open_postgres_pool, run_postgres_gate_migrations,
    };
    use ironclaw_turns::{GateRef, LoopResultRef, TurnRunId, TurnScope, TurnStatus};
    use secrecy::SecretString;

    /// Build a deadpool_postgres::Pool from the standard env var.
    ///
    /// Returns `None` and prints a skip message when the env var is unset.
    fn build_pool() -> Option<deadpool_postgres::Pool> {
        let Ok(url) = std::env::var("IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL") else {
            eprintln!(
                "skipping postgres gate migration tests: \
                 IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL not set"
            );
            return None;
        };
        let pool = open_postgres_pool(SecretString::new(url.into_boxed_str()))
            .expect("open_postgres_pool must succeed with valid URL");
        Some(pool)
    }

    fn scope_with_agent(tenant: &str, agent: &str, thread: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new(tenant).unwrap(),
            Some(AgentId::new(agent).unwrap()),
            None,
            ThreadId::new(thread).unwrap(),
        )
    }

    fn make_record(gate: &str, child: &str, parent: &str) -> AwaitedChildRecord {
        AwaitedChildRecord {
            gate_ref: GateRef::new(gate).unwrap(),
            parent_run_id: TurnRunId::parse(parent).unwrap(),
            tree_root_run_id: TurnRunId::parse(parent).unwrap(),
            child_run_id: TurnRunId::parse(child).unwrap(),
            child_thread_id: "child-thread-pg-0".to_string(),
            child_scope_json: r#"{"tenant_id":"t1","agent_id":null,"project_id":null,"thread_id":"th0","thread_owner":{"mode":"actor_fallback"}}"#.to_string(),
            parent_run_context_json: r#"{"run_id":"00000000-0000-0000-0000-000000000000"}"#
                .to_string(),
            source_binding_ref: "src".to_string(),
            reply_target_binding_ref: "reply".to_string(),
            subagent_kind: "coding".to_string(),
            spawn_capability_id: "spawn.subagent".to_string(),
            result_ref: LoopResultRef::new("result:pg-test.001").unwrap(),
            spawn_mode: "blocking".to_string(),
        }
    }

    fn terminal_event(status: TurnStatus) -> DurableTerminalEvent {
        DurableTerminalEvent {
            status,
            kind: "subagent_terminal".to_string(),
            cursor: 1,
            sanitized_reason: None,
            owner_user_id: None,
        }
    }

    /// F9: Run migrations twice — second run must be an idempotent no-op.
    ///
    /// Asserts that all expected gate tables exist and that
    /// `_reborn_migrations` contains the version-1 entry after both runs.
    #[tokio::test]
    async fn pg_gate_migrations_are_idempotent() {
        let Some(pool) = build_pool() else {
            return;
        };

        // First run: creates tables and records the migration.
        run_postgres_gate_migrations(&pool)
            .await
            .expect("first migration run must succeed");

        // Second run: must be a no-op — no error, no duplicate rows.
        run_postgres_gate_migrations(&pool)
            .await
            .expect("second migration run must be an idempotent no-op");

        // Verify all expected tables exist.
        let client = pool.get().await.expect("pool get");
        for table in &[
            "subagent_gate_awaited_children",
            "subagent_gate_capacity_counter",
            "subagent_gate_child_index",
            "subagent_gate_deliverable_queue",
            "subagent_gate_settlement_log",
            "_reborn_migrations",
        ] {
            let row = client
                .query_one(
                    "SELECT to_regclass($1::text) IS NOT NULL",
                    &[&format!("public.{table}")],
                )
                .await
                .unwrap_or_else(|e| panic!("table-existence query failed for {table}: {e}"));
            let exists: bool = row
                .try_get::<_, bool>(0)
                .unwrap_or_else(|e| panic!("decode failed for {table}: {e}"));
            assert!(exists, "expected table {table} to exist after migrations");
        }

        // Verify _reborn_migrations has exactly one row for version 1 (idempotent
        // second run must NOT insert a duplicate).
        let count_row = client
            .query_one(
                "SELECT COUNT(*) FROM _reborn_migrations WHERE version = 1",
                &[],
            )
            .await
            .expect("count _reborn_migrations version 1");
        let count: i64 = count_row
            .try_get::<_, i64>(0)
            .expect("decode migration count");
        assert_eq!(
            count, 1,
            "_reborn_migrations must have exactly one row for version 1 after idempotent re-run"
        );
    }

    /// F9 / F2 / F3: record_awaited_child increments the counter; a replay call
    /// (duplicate insert) must NOT increment again.  mark_child_delivered must
    /// decrement the counter exactly once.
    ///
    /// This exercises the F2 (increment only on actual insert) and F3 (decrement
    /// only when the guarding UPDATE flips the row) fixes at runtime against live PG.
    #[tokio::test]
    async fn pg_counter_increments_once_and_decrements_once() {
        let Some(pool) = build_pool() else {
            return;
        };

        run_postgres_gate_migrations(&pool)
            .await
            .expect("migrations");

        // Use a unique suffix to avoid cross-test interference.
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let tenant = format!("pg-f2f3-tenant-{suffix}");
        let agent = format!("pg-f2f3-agent-{suffix}");
        let scope = scope_with_agent(&tenant, &agent, "thread-pg-f2f3");

        let gate = format!("gate:pg-f2f3-{suffix}");
        let child_id = "00000000-0000-0000-0000-000000000001";
        let parent_id = "00000000-0000-0000-0000-000000000002";

        let store = PostgresGateResolutionStore::new(pool.clone(), 16);

        // Insert the awaited-child row once.
        store
            .record_awaited_child(&scope, make_record(&gate, child_id, parent_id))
            .await
            .expect("first record_awaited_child must succeed");

        // Replay the same call — ON CONFLICT DO NOTHING; counter must NOT grow.
        store
            .record_awaited_child(&scope, make_record(&gate, child_id, parent_id))
            .await
            .expect("replay record_awaited_child must succeed (idempotent)");

        // Read the counter directly to verify exactly 1 undelivered.
        let client = pool.get().await.expect("pool get");
        let sum_row = client
            .query_one(
                "SELECT COALESCE(SUM(undelivered), 0) \
                   FROM subagent_gate_capacity_counter \
                  WHERE tenant_id = $1 AND user_id != '' AND agent_id = $2",
                &[&tenant, &agent],
            )
            .await
            .expect("sum query");
        let total: i64 = sum_row.try_get::<_, i64>(0).expect("decode sum");
        assert_eq!(
            total, 1,
            "counter must be exactly 1 after one unique insert + one replay (F2)"
        );

        // Settle the child terminal so mark_child_delivered can proceed.
        let gate_ref = GateRef::new(&gate).unwrap();
        let child_run_id = TurnRunId::parse(child_id).unwrap();
        store
            .record_child_terminal(
                &scope,
                gate_ref.clone(),
                child_run_id,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .expect("record_child_terminal");

        // mark_child_delivered — first call; must decrement and return true.
        let gate_done = store
            .mark_child_delivered(&scope, &gate_ref, child_run_id)
            .await
            .expect("first mark_child_delivered");
        assert!(
            gate_done,
            "gate must be fully delivered after only child is delivered"
        );

        // mark_child_delivered replay — guarding UPDATE sees 0 rows; counter
        // must NOT be double-decremented (F3).
        let gate_done_replay = store
            .mark_child_delivered(&scope, &gate_ref, child_run_id)
            .await
            .expect("replay mark_child_delivered must succeed (idempotent)");
        // The gate is already done; replay returns false (no new delivery).
        assert!(
            !gate_done_replay,
            "replay mark_child_delivered must return false (already delivered)"
        );

        // Verify counter is now 0, not negative (no double-decrement).
        let sum_after = client
            .query_one(
                "SELECT COALESCE(SUM(undelivered), 0) \
                   FROM subagent_gate_capacity_counter \
                  WHERE tenant_id = $1 AND user_id != '' AND agent_id = $2",
                &[&tenant, &agent],
            )
            .await
            .expect("sum after delivery");
        let total_after: i64 = sum_after.try_get::<_, i64>(0).expect("decode sum after");
        assert_eq!(
            total_after, 0,
            "counter must be exactly 0 after delivery — no double-decrement (F3)"
        );
    }
}
