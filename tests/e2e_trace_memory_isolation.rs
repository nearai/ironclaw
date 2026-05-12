//! E2E trace tests: memory write/read round trip + protected-path rejection
//! through the agent tool layer.
//!
//! Tier B coverage of PR #3180 invariants 1 and 3, asserted at the caller
//! tier (`memory_*` tools dispatched through the agent loop). Today these
//! still hit the legacy host workspace; once PR 7 swaps in
//! `RepositoryMemoryBackend`, the same fixtures auto-cover the reborn
//! substrate. Per `.claude/rules/testing.md` "Test Through the Caller", this
//! file pins observable invariants that survive the substrate swap.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    /// PR #3180 invariant 1: a memory_write through the agent persists exactly
    /// the bytes the agent sent, and a follow-up memory_read returns those
    /// same bytes — no substrate-side mutation.
    #[tokio::test]
    async fn memory_write_then_read_round_trip_persists_payload_through_tool_layer() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/memory/write_then_read_same_scope.json"
        ))
        .expect("failed to load write_then_read_same_scope.json trace fixture");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.run_and_verify_trace(&trace, Duration::from_secs(20))
            .await;

        // Tool result for memory_read must round-trip the exact marker —
        // checked as a loose contains() first so a tool that drops the
        // payload entirely fails fast with a readable error...
        let results = rig.tool_results();
        let read_result = results
            .iter()
            .find(|(name, _)| name == "memory_read")
            .map(|(_, preview)| preview.clone())
            .expect("memory_read result must be captured");
        assert!(
            read_result.contains("deterministic-marker-42"),
            "memory_read tool result must round-trip the marker; got {read_result:?}",
        );

        // ...then go to the source of truth and assert exact persisted
        // bytes. The tool output is a renderable preview and may decorate
        // the content; `contains()` alone would still pass if the tool
        // returned the marker plus stale or mutated surrounding bytes.
        // The Database trait's `get_document_by_path` reads the persisted
        // row directly, which is the byte-level invariant PR #3180
        // invariant 1 actually pins.
        let doc = rig
            .database()
            .get_document_by_path(rig.channel_user_id(), None, "notes/round-trip.md")
            .await
            .expect("notes/round-trip.md must be persisted under channel user");
        assert_eq!(
            doc.content, "deterministic-marker-42",
            "persisted content must equal exactly the bytes the agent sent; \
             got {:?}",
            doc.content,
        );

        rig.shutdown();
    }

    /// PR #3180 invariant 3: writes to protected paths (e.g. SOUL.md) carrying
    /// high-risk content do not persist. This test will start passing the day
    /// PR 7 wires `RepositoryMemoryBackend` behind the host workspace; today
    /// the legacy host-workspace `memory_write` path does not consult the
    /// reborn substrate's `PromptWriteSafetyPolicy`, so the assertion would
    /// fail prematurely.
    #[tokio::test]
    #[cfg_attr(
        not(feature = "pr7-ready"),
        ignore = "tool-layer rejection requires PR 7 (product-tool migration) to route memory_write through ironclaw_memory's PromptWriteSafetyPolicy. Today the tool path runs against the legacy host workspace, which does not consult the substrate, so this test would fail prematurely. Distinct from `pr3180-ready` (substrate-level guards) — split here because the substrate and the tool routing land in separate PRs. Enable with --features pr7-ready when PR 7 lands. Substrate-level SOUL.md rejection has always-on coverage in `crates/ironclaw_memory/tests/e2e_scope_isolation_safety.rs::protected_paths_high_risk_writes_blocked_at_libsql_backend`."
    )]
    async fn memory_write_to_soul_md_rejects_through_tool_layer_no_persistence() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/memory/write_to_protected_path_rejected.json"
        ))
        .expect("failed to load write_to_protected_path_rejected.json trace fixture");

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .build()
            .await;

        let _ = rig.run_trace(&trace, Duration::from_secs(15)).await;

        // The memory_write call must have been attempted but reported as failed.
        let completed = rig.tool_calls_completed();
        let soul_call = completed
            .iter()
            .find(|(name, _)| name == "memory_write")
            .expect("memory_write attempt must be recorded");
        assert!(
            !soul_call.1,
            "memory_write to SOUL.md must report failure, got success",
        );

        // SOUL.md must not have a persisted row. The Database trait exposes
        // `get_document_by_path(user_id, agent_id, path)` which returns
        // `Err(WorkspaceError::NotFound)` when the row is absent — the cleanest
        // way to assert non-persistence without bypassing the trait into
        // dialect-specific raw SQL.
        //
        // Query under `rig.channel_user_id()`, NOT `rig.owner_id()`: the
        // memory_write tool runs as the effective channel user (default
        // `"test-user"`), and a regression that persists SOUL.md would
        // write it under that scope. An owner-keyed lookup (config owner =
        // `"default"`) would silently miss the persisted row and the
        // non-persistence assertion would become a false negative.
        let lookup = rig
            .database()
            .get_document_by_path(rig.channel_user_id(), None, "SOUL.md")
            .await;
        assert!(
            lookup.is_err(),
            "SOUL.md must not be persisted under channel user {}; got {lookup:?}",
            rig.channel_user_id(),
        );
        // Defense-in-depth: also check the owner scope, so a regression
        // that mis-routes the write to the owner identity is still caught.
        let owner_lookup = rig
            .database()
            .get_document_by_path(rig.owner_id(), None, "SOUL.md")
            .await;
        assert!(
            owner_lookup.is_err(),
            "SOUL.md must not be persisted under owner {}; got {owner_lookup:?}",
            rig.owner_id(),
        );

        rig.shutdown();
    }
}
