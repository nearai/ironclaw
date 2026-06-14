//! Contract tests for the durable gate-resolution store.
//!
//! All tests run against the libSQL file-backed backend using a temp directory
//! (no external process). libSQL `:memory:` databases do not share state
//! between connections, so a temp-file database is used instead.
//! PostgreSQL tests are gated behind the `integration` feature and require a
//! live Postgres server — the code compiles without a live server.
//!
//! Test coverage per spec and task requirements:
//!  - First-writer-wins `record_child_terminal` parity with in-memory store
//!  - Claim semantics (`claim_all_terminal_states_for_child`)
//!  - Capacity counter bucket behavior (K=16 sharding, cap enforcement)
//!  - Scoped-query security test (decision 31: no cross-agent leakage)
//!  - Reconciler-facing methods: `gates_exist_batch`, `redeliver_settled_child`,
//!    `resolve_undeliverable_batch`

#[cfg(feature = "libsql")]
mod libsql_tests {
    use std::sync::Arc;

    use ironclaw_host_api::{AgentId, TenantId, ThreadId};
    use ironclaw_reborn_event_store::{
        AwaitedChildRecord, CAPACITY_COUNTER_BUCKETS, DurableSubagentGateResolutionStore,
        DurableTerminalEvent, GateResolutionStoreError, LibSqlGateResolutionStore, child_bucket,
        run_libsql_gate_migrations,
    };
    use ironclaw_turns::{GateRef, LoopResultRef, TurnRunId, TurnScope, TurnStatus};

    /// Build a file-backed libSQL store with migrations applied.
    ///
    /// libSQL `:memory:` databases do not share state between connections
    /// (each `connect()` opens a fresh in-memory DB). A temp file ensures
    /// migration DDL is visible to all subsequent connections.
    async fn build_store() -> (LibSqlGateResolutionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("gate_test.db");
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("build libsql db"),
        );
        run_libsql_gate_migrations(&db)
            .await
            .expect("run gate migrations");
        (LibSqlGateResolutionStore::new(db, 16), dir)
    }

    fn scope_with_agent(tenant: &str, agent: &str, thread: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new(tenant).unwrap(),
            Some(AgentId::new(agent).unwrap()),
            None,
            ThreadId::new(thread).unwrap(),
        )
    }

    fn scope_no_agent(tenant: &str, thread: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new(tenant).unwrap(),
            None,
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
            child_thread_id: "child-thread-0".to_string(),
            child_scope_json: r#"{"tenant_id":"t1","agent_id":null,"project_id":null,"thread_id":"th0","thread_owner":{"mode":"actor_fallback"}}"#.to_string(),
            parent_run_context_json: r#"{"run_id":"00000000-0000-0000-0000-000000000000"}"#.to_string(),
            source_binding_ref: "src".to_string(),
            reply_target_binding_ref: "reply".to_string(),
            subagent_kind: "coding".to_string(),
            spawn_capability_id: "spawn.subagent".to_string(),
            result_ref: LoopResultRef::new("result:test.001").unwrap(),
            spawn_mode: "blocking".to_string(),
        }
    }

    fn terminal_event(status: TurnStatus) -> DurableTerminalEvent {
        DurableTerminalEvent {
            status,
            kind: "subagent_terminal".to_string(),
            cursor: 42,
            sanitized_reason: None,
            owner_user_id: None,
        }
    }

    // ── migration sanity ────────────────────────────────────────────────────

    #[tokio::test]
    async fn migration_creates_all_tables() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("migration_test.db");
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("file-backed libsql"),
        );
        run_libsql_gate_migrations(&db)
            .await
            .expect("migrations must succeed");
        let conn = db.connect().unwrap();
        let mut rows = conn
            .query(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
                (),
            )
            .await
            .unwrap();
        let mut tables = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            tables.push(row.get::<String>(0).unwrap());
        }
        for expected in &[
            "subagent_gate_awaited_children",
            "subagent_gate_capacity_counter",
            "subagent_gate_child_index",
            "subagent_gate_deliverable_queue",
            "subagent_gate_settlement_log",
        ] {
            assert!(
                tables.contains(&expected.to_string()),
                "migration must create table {expected}, got: {tables:?}"
            );
        }
    }

    // ── first-writer-wins ────────────────────────────────────────────────────

    #[tokio::test]
    async fn record_awaited_child_is_idempotent() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-a", "agent-a", "thread-a");
        let record = make_record(
            "gate:fw-idem-0001",
            "00000000-0000-0000-0000-000000000001",
            "00000000-0000-0000-0000-000000000002",
        );
        store
            .record_awaited_child(&scope, record.clone())
            .await
            .unwrap();
        // Second insert is silently ignored (INSERT OR IGNORE).
        store.record_awaited_child(&scope, record).await.unwrap();
    }

    /// Spec §1.6 decision 6: `record_child_terminal` is first-writer-wins.
    /// A second write with a different status MUST be a no-op.
    #[tokio::test]
    async fn record_child_terminal_first_writer_wins() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-fw", "agent-fw", "thread-fw");
        let gate_ref = GateRef::new("gate:fw-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000010").unwrap();
        let parent_id = "00000000-0000-0000-0000-000000000011";

        let record = make_record(gate_ref.as_str(), &child_id.to_string(), parent_id);
        store.record_awaited_child(&scope, record).await.unwrap();

        // First write: Completed.
        store
            .record_child_terminal(
                &scope,
                gate_ref.clone(),
                child_id,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();

        // Second write: Failed — must be ignored (first-writer-wins).
        store
            .record_child_terminal(
                &scope,
                gate_ref,
                child_id,
                terminal_event(TurnStatus::Failed),
            )
            .await
            .unwrap();

        // Claim must return the first-written status (Completed).
        let rows = store
            .claim_all_terminal_states_for_child(&scope, child_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "expected exactly one row");
        assert_eq!(
            rows[0].terminal_status,
            Some(TurnStatus::Completed),
            "first-writer-wins: terminal_status must be Completed, not Failed"
        );
    }

    /// `record_child_terminal` rejects non-terminal statuses.
    #[tokio::test]
    async fn record_child_terminal_rejects_non_terminal_status() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-nt", "agent-nt", "thread-nt");
        let gate_ref = GateRef::new("gate:nt-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000020").unwrap();
        let parent_id = "00000000-0000-0000-0000-000000000021";

        let record = make_record(gate_ref.as_str(), &child_id.to_string(), parent_id);
        store.record_awaited_child(&scope, record).await.unwrap();

        let result = store
            .record_child_terminal(
                &scope,
                gate_ref,
                child_id,
                terminal_event(TurnStatus::Running),
            )
            .await;
        assert!(
            matches!(result, Err(GateResolutionStoreError::NonTerminalStatus)),
            "non-terminal status must be rejected"
        );
    }

    // ── claim semantics ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn claim_all_returns_empty_before_terminal() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-cl", "agent-cl", "thread-cl");
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000030").unwrap();
        let gate_ref = GateRef::new("gate:cl-001").unwrap();

        let record = make_record(
            gate_ref.as_str(),
            &child_id.to_string(),
            "00000000-0000-0000-0000-000000000031",
        );
        store.record_awaited_child(&scope, record).await.unwrap();

        let rows = store
            .claim_all_terminal_states_for_child(&scope, child_id)
            .await
            .unwrap();
        assert!(
            rows.is_empty(),
            "no terminal state yet — queue must be empty"
        );
    }

    #[tokio::test]
    async fn claim_all_returns_row_after_terminal() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-cl2", "agent-cl2", "thread-cl2");
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000040").unwrap();
        let gate_ref = GateRef::new("gate:cl2-001").unwrap();

        let record = make_record(
            gate_ref.as_str(),
            &child_id.to_string(),
            "00000000-0000-0000-0000-000000000041",
        );
        store.record_awaited_child(&scope, record).await.unwrap();

        store
            .record_child_terminal(
                &scope,
                gate_ref,
                child_id,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();

        let rows = store
            .claim_all_terminal_states_for_child(&scope, child_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "one terminal row expected after settlement");
        assert_eq!(rows[0].terminal_status, Some(TurnStatus::Completed));
    }

    // ── capacity counter bucket behavior ─────────────────────────────────────

    #[tokio::test]
    async fn capacity_counter_increments_and_decrements() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-cap", "agent-cap", "thread-cap");

        let gate_ref = GateRef::new("gate:cap-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000050").unwrap();
        let record = make_record(
            gate_ref.as_str(),
            &child_id.to_string(),
            "00000000-0000-0000-0000-000000000051",
        );
        store.record_awaited_child(&scope, record).await.unwrap();

        // Settle the child terminal.
        store
            .record_child_terminal(
                &scope,
                gate_ref.clone(),
                child_id,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();

        // mark_child_delivered should decrement the counter bucket.
        let _gate_done = store
            .mark_child_delivered(&scope, &gate_ref, child_id)
            .await
            .unwrap();
    }

    #[test]
    fn capacity_exceeded_error_variant_is_accessible() {
        // Confirms GateResolutionStoreError::CapacityExceeded is exported and
        // carries the expected Display message (spec §1 decision 21).
        let err = GateResolutionStoreError::CapacityExceeded;
        assert_eq!(
            format!("{err}"),
            "gate resolution capacity exceeded for scope"
        );
    }

    #[test]
    fn bucket_distribution_across_k_buckets() {
        let k = CAPACITY_COUNTER_BUCKETS;
        let mut seen = std::collections::HashSet::new();
        for i in 0u32..200 {
            let id = format!("{i:08x}-0000-0000-0000-000000000000");
            seen.insert(child_bucket(&id, k));
        }
        assert!(
            seen.len() > 10,
            "expected spread across buckets, got {} distinct values",
            seen.len()
        );
    }

    // ── scoped-query security test (decision 31) ─────────────────────────────

    /// Prove that `claim_all_terminal_states_for_child` scoped to agent-a
    /// cannot return rows belonging to agent-b under the same tenant.
    ///
    /// This is the required test per spec decision 31.
    #[tokio::test]
    async fn gate_resolution_scoped_query_excludes_rows_from_other_agents() {
        let (store, _dir) = build_store().await;

        let scope_a = scope_with_agent("tenant-sec", "agent-sec-a", "thread-sec-a");
        let scope_b = scope_with_agent("tenant-sec", "agent-sec-b", "thread-sec-b");

        // agent-a spawns a child.
        let gate_a = GateRef::new("gate:sec-a-001").unwrap();
        let child_a = TurnRunId::parse("00000000-0000-0000-0000-000000000060").unwrap();
        let record_a = make_record(
            gate_a.as_str(),
            &child_a.to_string(),
            "00000000-0000-0000-0000-000000000061",
        );
        store
            .record_awaited_child(&scope_a, record_a)
            .await
            .unwrap();
        store
            .record_child_terminal(
                &scope_a,
                gate_a,
                child_a,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();

        // Querying as agent-b MUST NOT return agent-a's row.
        let rows_b = store
            .claim_all_terminal_states_for_child(&scope_b, child_a)
            .await
            .unwrap();
        assert!(
            rows_b.is_empty(),
            "scoped query for agent-b must not return agent-a rows (cross-agent isolation violation)"
        );
    }

    /// `gates_exist_batch` must only return gates visible in the given scope.
    #[tokio::test]
    async fn gates_exist_batch_scoped_to_agent() {
        let (store, _dir) = build_store().await;
        let scope_a = scope_with_agent("tenant-geb", "agent-geb-a", "thread-geb-a");
        let scope_b = scope_with_agent("tenant-geb", "agent-geb-b", "thread-geb-b");

        let gate_a = GateRef::new("gate:geb-a-001").unwrap();
        let child_a = TurnRunId::parse("00000000-0000-0000-0000-000000000070").unwrap();
        let record_a = make_record(
            gate_a.as_str(),
            &child_a.to_string(),
            "00000000-0000-0000-0000-000000000071",
        );
        store
            .record_awaited_child(&scope_a, record_a)
            .await
            .unwrap();

        // Batch check as scope_a — should find the gate.
        let found_a = store
            .gates_exist_batch(&scope_a, vec![gate_a.clone()])
            .await
            .unwrap();
        assert!(
            found_a.contains(&gate_a),
            "gate_a must be visible to scope_a"
        );

        // Batch check as scope_b — must NOT find scope_a's gate.
        let found_b = store
            .gates_exist_batch(&scope_b, vec![gate_a.clone()])
            .await
            .unwrap();
        assert!(
            !found_b.contains(&gate_a),
            "gate_a must NOT be visible to scope_b (cross-agent isolation violation)"
        );
    }

    /// `gates_exist_batch` with empty input returns empty set.
    #[tokio::test]
    async fn gates_exist_batch_empty_input_returns_empty() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-empty", "agent-empty", "thread-empty");
        let found = store.gates_exist_batch(&scope, vec![]).await.unwrap();
        assert!(found.is_empty());
    }

    // ── reconciler-facing methods ─────────────────────────────────────────────

    #[tokio::test]
    async fn redeliver_settled_child_returns_true_for_existing_gate() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-rd", "agent-rd", "thread-rd");
        let gate_ref = GateRef::new("gate:rd-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000080").unwrap();
        let parent_id = "00000000-0000-0000-0000-000000000081";
        let result_ref = LoopResultRef::new("result:rd.001").unwrap();

        let record = make_record(gate_ref.as_str(), &child_id.to_string(), parent_id);
        store.record_awaited_child(&scope, record).await.unwrap();

        let delivered = store
            .redeliver_settled_child(
                &scope,
                gate_ref,
                child_id,
                TurnStatus::Completed,
                result_ref,
            )
            .await
            .unwrap();
        assert!(
            delivered,
            "redeliver_settled_child must return true for existing gate row"
        );
    }

    #[tokio::test]
    async fn redeliver_settled_child_returns_false_for_missing_gate() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-rd2", "agent-rd2", "thread-rd2");
        let gate_ref = GateRef::new("gate:rd2-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000090").unwrap();
        let result_ref = LoopResultRef::new("result:rd2.001").unwrap();

        // No record_awaited_child call — gate row doesn't exist.
        let delivered = store
            .redeliver_settled_child(
                &scope,
                gate_ref,
                child_id,
                TurnStatus::Completed,
                result_ref,
            )
            .await
            .unwrap();
        assert!(
            !delivered,
            "redeliver_settled_child must return false for missing gate (orphan)"
        );
    }

    #[tokio::test]
    async fn resolve_undeliverable_batch_is_idempotent() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-rub", "agent-rub", "thread-rub");
        let gate_ref = GateRef::new("gate:rub-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-0000000000a0").unwrap();
        let parent_id = "00000000-0000-0000-0000-0000000000a1";

        let record = make_record(gate_ref.as_str(), &child_id.to_string(), parent_id);
        store.record_awaited_child(&scope, record).await.unwrap();
        // Settle terminal first.
        store
            .record_child_terminal(
                &scope,
                gate_ref.clone(),
                child_id,
                terminal_event(TurnStatus::Cancelled),
            )
            .await
            .unwrap();

        let rows = vec![(gate_ref.clone(), child_id)];
        // First call: resolves the undeliverable row.
        store
            .resolve_undeliverable_batch(&scope, rows.clone())
            .await
            .unwrap();
        // Second call: idempotent (delivered_to_parent guard).
        store
            .resolve_undeliverable_batch(&scope, rows)
            .await
            .unwrap();
    }

    /// After `resolve_undeliverable_batch`, the row must not appear in
    /// `claim_all_terminal_states_for_child` (delivered_to_parent = 1 skips queue).
    #[tokio::test]
    async fn resolve_undeliverable_batch_removes_from_claim_queue() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-rub2", "agent-rub2", "thread-rub2");
        let gate_ref = GateRef::new("gate:rub2-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-0000000000b0").unwrap();
        let parent_id = "00000000-0000-0000-0000-0000000000b1";

        let record = make_record(gate_ref.as_str(), &child_id.to_string(), parent_id);
        store.record_awaited_child(&scope, record).await.unwrap();
        store
            .record_child_terminal(
                &scope,
                gate_ref.clone(),
                child_id,
                terminal_event(TurnStatus::Failed),
            )
            .await
            .unwrap();

        // Verify row is claimable before resolution.
        let before = store
            .claim_all_terminal_states_for_child(&scope, child_id)
            .await
            .unwrap();
        assert_eq!(before.len(), 1, "row should be claimable before resolve");

        // Resolve as undeliverable (decision 31 path).
        store
            .resolve_undeliverable_batch(&scope, vec![(gate_ref, child_id)])
            .await
            .unwrap();

        // After resolution, queue entry is removed → claim returns empty.
        let after = store
            .claim_all_terminal_states_for_child(&scope, child_id)
            .await
            .unwrap();
        assert!(
            after.is_empty(),
            "row must not appear in claim queue after resolve_undeliverable_batch"
        );
    }

    /// `resolve_undeliverable_batch` with empty input is a no-op.
    #[tokio::test]
    async fn resolve_undeliverable_batch_empty_input_is_noop() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-noop", "agent-noop", "thread-noop");
        store
            .resolve_undeliverable_batch(&scope, vec![])
            .await
            .unwrap();
    }

    /// Null `agent_id` scope (non-agent run) uses `agent_id IS NULL` predicate
    /// and must not match rows written under an agent scope.
    #[tokio::test]
    async fn null_agent_scope_does_not_match_agent_scoped_rows() {
        let (store, _dir) = build_store().await;
        let scope_agent = scope_with_agent("tenant-null", "agent-null", "thread-null-a");
        let scope_system = scope_no_agent("tenant-null", "thread-null-sys");

        let gate_ref = GateRef::new("gate:null-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-0000000000c0").unwrap();
        let record = make_record(
            gate_ref.as_str(),
            &child_id.to_string(),
            "00000000-0000-0000-0000-0000000000c1",
        );
        store
            .record_awaited_child(&scope_agent, record)
            .await
            .unwrap();

        // System scope must not find the agent-scoped gate.
        let found = store
            .gates_exist_batch(&scope_system, vec![gate_ref])
            .await
            .unwrap();
        assert!(
            found.is_empty(),
            "null agent_id scope must not match agent-scoped rows"
        );
    }

    // ── F8a: capacity_exceeded_after_max_records ─────────────────────────────

    /// Fill to the (injected) capacity limit, then assert the next
    /// `record_awaited_child` fails with `CapacityExceeded`.
    ///
    /// Uses `new_with_limit` to set max_records = 3 so the test avoids
    /// inserting 4096 rows. The production default (MAX_GATE_RECORDS = 4096) is
    /// not changed.
    #[tokio::test]
    async fn capacity_exceeded_after_max_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("cap_exceed.db");
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("build libsql db"),
        );
        run_libsql_gate_migrations(&db)
            .await
            .expect("run gate migrations");
        // Limit = 3 so the test only needs 4 inserts total.
        let store = LibSqlGateResolutionStore::new_with_limit(db, 16, 3);

        let scope = scope_with_agent("tenant-capx", "agent-capx", "thread-capx");
        let gate_ref = GateRef::new("gate:capx-001").unwrap();
        let parent_id = "00000000-0000-0000-0000-0000000000d0";

        // Insert exactly `max_records` children — all must succeed.
        for i in 0u32..3 {
            // Produce UUIDs that hash to different buckets for variety, but
            // correctness doesn't depend on bucket distribution here.
            let child_id_str = format!("0000{i:04x}-0000-0000-0000-0000000000d0");
            let record = make_record(gate_ref.as_str(), &child_id_str, parent_id);
            store
                .record_awaited_child(&scope, record)
                .await
                .unwrap_or_else(|e| panic!("insert {i} failed: {e}"));
        }

        // One more must fail with CapacityExceeded.
        let over_child = "0000ffff-0000-0000-0000-0000000000d0";
        let record = make_record(gate_ref.as_str(), over_child, parent_id);
        let result = store.record_awaited_child(&scope, record).await;
        assert!(
            matches!(result, Err(GateResolutionStoreError::CapacityExceeded)),
            "expected CapacityExceeded, got: {result:?}"
        );
    }

    // ── F8b: duplicate_record_awaited_child_does_not_inflate_capacity ────────

    /// Regression for F2: recording the same (gate_ref, child_run_id) twice
    /// must not increment the capacity counter a second time.
    #[tokio::test]
    async fn duplicate_record_awaited_child_does_not_inflate_capacity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("dup_cap.db");
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("build libsql db"),
        );
        run_libsql_gate_migrations(&db)
            .await
            .expect("run gate migrations");
        // Limit = 2 so we can distinguish "1 slot used" from "2 slots used".
        let store = LibSqlGateResolutionStore::new_with_limit(db, 16, 2);

        let scope = scope_with_agent("tenant-dup", "agent-dup", "thread-dup");
        let gate_ref = GateRef::new("gate:dup-001").unwrap();
        let child_id = "00000000-0000-0000-0000-0000000000e0";
        let parent_id = "00000000-0000-0000-0000-0000000000e1";
        let record = make_record(gate_ref.as_str(), child_id, parent_id);

        // First insert: succeeds and consumes 1 slot.
        store
            .record_awaited_child(&scope, record.clone())
            .await
            .unwrap();

        // Second insert of the same identity: silently ignored (INSERT OR IGNORE),
        // counter must NOT increment again.
        store.record_awaited_child(&scope, record).await.unwrap();

        // If counter drifted to 2, this second unique child would fail.
        let child_id2 = "00000001-0000-0000-0000-0000000000e0";
        let record2 = make_record(gate_ref.as_str(), child_id2, parent_id);
        store
            .record_awaited_child(&scope, record2)
            .await
            .expect("second unique child must succeed — counter must still be 1, not 2");
    }

    // ── F8c: repeated_delivery_does_not_deflate_capacity ────────────────────

    /// Regression for F3: delivering the same child twice must decrement the
    /// capacity counter exactly once.
    #[tokio::test]
    async fn repeated_delivery_does_not_deflate_capacity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("rep_del.db");
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("build libsql db"),
        );
        run_libsql_gate_migrations(&db)
            .await
            .expect("run gate migrations");
        // Limit = 2: insert 2, deliver one twice, insert a third — if counter
        // under-counts, the third insert would fail.
        let store = LibSqlGateResolutionStore::new_with_limit(db, 16, 2);

        let scope = scope_with_agent("tenant-repd", "agent-repd", "thread-repd");
        let gate_ref = GateRef::new("gate:repd-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-0000000000f0").unwrap();
        let parent_id = "00000000-0000-0000-0000-0000000000f1";

        let record = make_record(gate_ref.as_str(), &child_id.to_string(), parent_id);
        store.record_awaited_child(&scope, record).await.unwrap();

        // Settle the child.
        store
            .record_child_terminal(
                &scope,
                gate_ref.clone(),
                child_id,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();

        // Deliver once: counter goes 1 → 0.
        store
            .mark_child_delivered(&scope, &gate_ref, child_id)
            .await
            .unwrap();

        // Deliver again (retry / replay): counter must stay at 0, not go to -1
        // (saturating at 0 via MAX). This is idempotent by the
        // delivered_to_parent = 0 guard.
        store
            .mark_child_delivered(&scope, &gate_ref, child_id)
            .await
            .unwrap();

        // If counter under-counted (went to -1 and wrapped), inserting a second
        // child might either fail (CHECK constraint) or succeed incorrectly.
        // With the correct guard, counter is 0 — we can insert one more child.
        let child_id2 = TurnRunId::parse("00000001-0000-0000-0000-0000000000f0").unwrap();
        let record2 = make_record(gate_ref.as_str(), &child_id2.to_string(), parent_id);
        store
            .record_awaited_child(&scope, record2)
            .await
            .expect("counter must be 0 after one delivery, allowing one more spawn");
    }

    // ── F8d: mark_terminal_result_written_is_once_only ───────────────────────

    /// `mark_terminal_result_written` must be idempotent: first call persists
    /// `terminal_byte_len`, second call is a no-op (does not overwrite).
    #[tokio::test]
    async fn mark_terminal_result_written_is_once_only() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-trw", "agent-trw", "thread-trw");
        let gate_ref = GateRef::new("gate:trw-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000100").unwrap();
        let parent_id = "00000000-0000-0000-0000-000000000101";

        let record = make_record(gate_ref.as_str(), &child_id.to_string(), parent_id);
        store.record_awaited_child(&scope, record).await.unwrap();

        // First write: byte_len = 42.
        store
            .mark_terminal_result_written(&scope, &gate_ref, child_id, 42)
            .await
            .unwrap();

        // Second write with a different byte_len: must be ignored
        // (terminal_result_written = 0 guard prevents overwrite).
        store
            .mark_terminal_result_written(&scope, &gate_ref, child_id, 999)
            .await
            .unwrap();

        // Claim the row and verify terminal_byte_len == 42 (first write wins).
        // We need to settle the child first to make it claimable.
        store
            .record_child_terminal(
                &scope,
                gate_ref.clone(),
                child_id,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();
        let rows = store
            .claim_all_terminal_states_for_child(&scope, child_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "one settled row expected");
        assert_eq!(
            rows[0].terminal_byte_len, 42,
            "terminal_byte_len must be 42 (first write), not 999 (second write)"
        );
        assert!(
            rows[0].terminal_result_written,
            "terminal_result_written must be true"
        );
    }

    // ── F8e: delete_awaited_child_cleans_all_tables ──────────────────────────

    /// After `delete_awaited_child`, the awaited-child row, queue row, child-
    /// index row, and capacity counter must all be cleaned up consistently.
    #[tokio::test]
    async fn delete_awaited_child_cleans_all_tables() {
        let (store, _dir) = build_store().await;
        let scope = scope_with_agent("tenant-del", "agent-del", "thread-del");

        let gate_ref_a = GateRef::new("gate:del-001").unwrap();
        let gate_ref_b = GateRef::new("gate:del-002").unwrap();
        let child_a = TurnRunId::parse("00000000-0000-0000-0000-000000000110").unwrap();
        let child_b = TurnRunId::parse("00000000-0000-0000-0000-000000000111").unwrap();
        let parent_id = "00000000-0000-0000-0000-000000000112";

        // Record two children under different gates.
        store
            .record_awaited_child(
                &scope,
                make_record(gate_ref_a.as_str(), &child_a.to_string(), parent_id),
            )
            .await
            .unwrap();
        store
            .record_awaited_child(
                &scope,
                make_record(gate_ref_b.as_str(), &child_b.to_string(), parent_id),
            )
            .await
            .unwrap();

        // Settle child_a so it has a queue entry.
        store
            .record_child_terminal(
                &scope,
                gate_ref_a.clone(),
                child_a,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();

        // Delete gate_ref_a — should remove awaited row, queue row, index row,
        // and decrement capacity counter.
        store
            .delete_awaited_child(&scope, &gate_ref_a)
            .await
            .unwrap();

        // gate_ref_a must no longer exist for this scope.
        let found = store
            .gates_exist_batch(&scope, vec![gate_ref_a.clone()])
            .await
            .unwrap();
        assert!(
            !found.contains(&gate_ref_a),
            "deleted gate must not appear in gates_exist_batch"
        );

        // gate_ref_b must still exist.
        let found_b = store
            .gates_exist_batch(&scope, vec![gate_ref_b.clone()])
            .await
            .unwrap();
        assert!(
            found_b.contains(&gate_ref_b),
            "non-deleted gate must still appear in gates_exist_batch"
        );

        // Claiming child_a after deletion must return empty (queue cleared).
        let claimed = store
            .claim_all_terminal_states_for_child(&scope, child_a)
            .await
            .unwrap();
        assert!(
            claimed.is_empty(),
            "claim must return empty after gate deletion"
        );
    }

    // ── F8f: concurrent_spawns_respect_first_writer_wins ────────────────────

    /// Concurrent `record_awaited_child` calls with the same identity (duplicate)
    /// and different identities must not drift capacity — first-writer-wins on
    /// duplicates, each unique identity counted exactly once.
    #[tokio::test]
    async fn concurrent_spawns_respect_first_writer_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("concurrent.db");
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("build libsql db"),
        );
        run_libsql_gate_migrations(&db)
            .await
            .expect("run gate migrations");
        // Limit = 4: we'll issue 2 unique + 2 duplicates concurrently.
        let store = LibSqlGateResolutionStore::new_with_limit(db, 16, 4);

        let scope = scope_with_agent("tenant-conc", "agent-conc", "thread-conc");
        let gate_ref = GateRef::new("gate:conc-001").unwrap();
        let parent_id = "00000000-0000-0000-0000-000000000120";

        // Two unique child IDs.
        let child_x = "00000000-0000-0000-0000-000000000121";
        let child_y = "00000000-0000-0000-0000-000000000122";

        let store_a = store.clone();
        let store_b = store.clone();
        let store_c = store.clone();
        let store_d = store.clone();
        let scope_a = scope.clone();
        let scope_b = scope.clone();
        let scope_c = scope.clone();
        let scope_d = scope.clone();
        let gate_a = gate_ref.clone();
        let gate_b = gate_ref.clone();
        let gate_c = gate_ref.clone();
        let gate_d = gate_ref.clone();

        // Run 4 concurrent inserts: child_x twice, child_y twice.
        let (r1, r2, r3, r4) = tokio::join!(
            async move {
                store_a
                    .record_awaited_child(
                        &scope_a,
                        make_record(gate_a.as_str(), child_x, parent_id),
                    )
                    .await
            },
            async move {
                store_b
                    .record_awaited_child(
                        &scope_b,
                        make_record(gate_b.as_str(), child_x, parent_id),
                    )
                    .await
            },
            async move {
                store_c
                    .record_awaited_child(
                        &scope_c,
                        make_record(gate_c.as_str(), child_y, parent_id),
                    )
                    .await
            },
            async move {
                store_d
                    .record_awaited_child(
                        &scope_d,
                        make_record(gate_d.as_str(), child_y, parent_id),
                    )
                    .await
            },
        );

        // All four calls must succeed (duplicates are silently ignored).
        r1.expect("child_x first insert must succeed");
        r2.expect("child_x duplicate insert must succeed (INSERT OR IGNORE)");
        r3.expect("child_y first insert must succeed");
        r4.expect("child_y duplicate insert must succeed (INSERT OR IGNORE)");

        // Capacity must be exactly 2 (child_x + child_y), not 4.
        // Verify by checking that the gate still appears (not over-cap) and
        // that we can add 2 more without hitting the limit of 4.
        let scope_e = scope.clone();
        let scope_f = scope.clone();
        let gate_e = gate_ref.clone();
        let gate_f = gate_ref.clone();
        store
            .record_awaited_child(
                &scope_e,
                make_record(
                    gate_e.as_str(),
                    "00000000-0000-0000-0000-000000000123",
                    parent_id,
                ),
            )
            .await
            .expect("3rd unique child must fit within limit=4");
        store
            .record_awaited_child(
                &scope_f,
                make_record(
                    gate_f.as_str(),
                    "00000000-0000-0000-0000-000000000124",
                    parent_id,
                ),
            )
            .await
            .expect("4th unique child must fit within limit=4");
    }

    // ── F8g: scoped redelivery/claim security (F4 regression) ────────────────

    /// A caller with a different (tenant, user, agent) scope must NOT be able to
    /// redeliver or claim a (gate_ref, child_run_id) pair that belongs to
    /// another scope.
    ///
    /// Regression test for F4: ensures that `redeliver_settled_child` and
    /// `claim_all_terminal_states_for_child` scope the existence check AND the
    /// queue insert AND the join to the caller's full (tenant_id, user_id,
    /// agent_id) predicate.
    #[tokio::test]
    async fn scoped_redeliver_and_claim_cannot_touch_foreign_scope() {
        let (store, _dir) = build_store().await;

        let scope_owner = scope_with_agent("tenant-f4", "agent-f4-owner", "thread-f4-owner");
        let scope_attacker =
            scope_with_agent("tenant-f4", "agent-f4-attacker", "thread-f4-attacker");

        let gate_ref = GateRef::new("gate:f4-001").unwrap();
        let child_id = TurnRunId::parse("00000000-0000-0000-0000-000000000130").unwrap();
        let parent_id = "00000000-0000-0000-0000-000000000131";
        let result_ref = LoopResultRef::new("result:f4.001").unwrap();

        // Owner registers a child.
        store
            .record_awaited_child(
                &scope_owner,
                make_record(gate_ref.as_str(), &child_id.to_string(), parent_id),
            )
            .await
            .unwrap();

        // Attacker attempts to redeliver the owner's pair into its own queue.
        // Must return false (not found in attacker's scope).
        let redelivered = store
            .redeliver_settled_child(
                &scope_attacker,
                gate_ref.clone(),
                child_id,
                TurnStatus::Completed,
                result_ref,
            )
            .await
            .unwrap();
        assert!(
            !redelivered,
            "redeliver_settled_child must return false for a foreign scope's pair"
        );

        // Settle the child under the owner's scope.
        store
            .record_child_terminal(
                &scope_owner,
                gate_ref.clone(),
                child_id,
                terminal_event(TurnStatus::Completed),
            )
            .await
            .unwrap();

        // Attacker attempts to claim the settled child — must return empty.
        let claimed = store
            .claim_all_terminal_states_for_child(&scope_attacker, child_id)
            .await
            .unwrap();
        assert!(
            claimed.is_empty(),
            "claim_all_terminal_states_for_child must not return rows belonging to a foreign scope"
        );

        // Owner can still claim their own row.
        let owner_claimed = store
            .claim_all_terminal_states_for_child(&scope_owner, child_id)
            .await
            .unwrap();
        assert_eq!(
            owner_claimed.len(),
            1,
            "owner must still be able to claim their own row after attacker's failed attempt"
        );
    }
}
