//! End-to-end integration tests for the Responses API external-tools
//! path through engine v2 with replay (trace) LLMs.
//!
//! These tests exercise the engine-native flow added in PR #3122
//! (commit `44135ca5a` and follow-ups):
//! - `ExternalToolCatalog` registers caller-supplied actions per-thread
//! - `EffectBridgeAdapter::execute_action` short-circuits on catalog hits
//!   to a `GatePaused { ResumeKind::External { ext_tool:<call_id> } }`
//! - bridge router projects the pause to `AppEvent::ExternalToolCall`
//! - the resume goes through `Submission::ExternalCallback { payload }`
//!   and `bridge::handle_external_callback`
//!
//! The tests deliberately drive engine v2 directly via `TestRigBuilder`
//! (rather than the HTTP `/v1/responses` endpoint) so we can use the
//! existing `TraceLlm` replay infrastructure without spinning up a full
//! HTTP gateway. Wire-shape coverage stays in
//! `tests/responses_api_path_prefix.rs`.
//!
//! ## What these tests are designed to catch
//!
//! - **Resume payload not materialised**: the engine's existing
//!   `GateResolution::ExternalCallback` path uses `pending.resume_output`
//!   (which `EffectBridgeAdapter::execute_action` sets to `None` for
//!   tool-flavoured pauses) and re-runs the action — meaning the
//!   caller-supplied tool output never reaches the LLM. The
//!   `round_trip_resume_payload_reaches_llm` test asserts the LLM's
//!   second-turn context contains the caller-supplied output;
//!   without a fix this test fails.
//! - **Thread-id mismatch**: catalog is keyed by engine `ThreadId`, but
//!   the responses_api handler registers under a separately-generated
//!   UUID before the engine spawns the thread. Catalog entries miss
//!   the actual thread, tool calls fall through to the registry, and
//!   the LLM gets "tool not found" instead of a pause.
//! - **Internal-vs-external collision in dispatch**: the catalog wins
//!   over the registry inside `execute_action`, but loses in
//!   `available_action_inventory` — so the LLM sees the internal tool
//!   in its surface but a registered shadow name short-circuits to
//!   caller execution at dispatch time. The
//!   `external_collision_with_registry_action_is_rejected` test
//!   asserts the request is rejected up-front; without a fix the
//!   request is silently accepted and the security gap stays open.
//! - **Lease coverage for external actions**: external actions are
//!   merged into the LLM-visible action surface; the dynamic
//!   lease-refresh path needs to also extend the lease to cover them
//!   so `find_and_consume` succeeds. If it doesn't, calls error out
//!   with `LeaseNotFound` before reaching the catalog short-circuit.
//! - **Catalog cleanup on terminal state**: today nothing clears
//!   catalog entries when a thread finishes, leaking entries forever.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;
    use ironclaw::bridge::ExternalToolCatalog;
    use ironclaw_engine::{ActionDef, EffectType, ModelToolSurface, ThreadId};

    const TIMEOUT: Duration = Duration::from_secs(10);

    /// Helper: build an `ActionDef` for a caller-supplied function tool.
    fn caller_action(name: &str, description: &str) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: description.to_string(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"]
            }),
            effects: vec![EffectType::Compute],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }
    }

    /// Load the prepared 2-turn trace from disk. The fixture script
    /// has the LLM emit a tool_call for `lookup_weather` and then
    /// (on the second LLM call) produce text quoting "sunny and 72F"
    /// — that text only appears in the LLM's context if the resume
    /// payload was materialised back as a tool result.
    fn round_trip_trace() -> LlmTrace {
        let path = format!(
            "{}/tests/fixtures/llm_traces/engine_v2/external_tool_round_trip.json",
            env!("CARGO_MANIFEST_DIR")
        );
        LlmTrace::from_file(&path).expect("load round-trip trace fixture")
    }

    /// Wait until the engine state has registered a pending external
    /// tool gate for the given user, or the timeout expires. Returns
    /// the request_id of the pending gate.
    async fn wait_for_external_pending_gate(
        user_id: &str,
        timeout: Duration,
    ) -> Option<(uuid::Uuid, String)> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Ok(Some(view)) = ironclaw::bridge::get_engine_pending_gate(user_id, None).await
                && matches!(
                    view.resume_kind,
                    ironclaw_engine::ResumeKind::External { .. }
                )
            {
                let request_id = uuid::Uuid::parse_str(&view.request_id).ok()?;
                return Some((request_id, view.tool_name.clone()));
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Wait until the engine's external-tool catalog has at least one
    /// thread registered (the engine has spawned a thread for this
    /// request). Returns the first registered ThreadId snapshot — this
    /// is how a test discovers the engine-assigned thread id without
    /// reaching into private engine state.
    async fn wait_for_first_engine_thread(
        catalog: &ExternalToolCatalog,
        timeout: Duration,
    ) -> Option<ThreadId> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Best-effort: borrow the inner map via `is_empty`/`len`
            // to detect when something is registered.
            if !catalog.is_empty().await {
                // The catalog doesn't expose its keys publicly. Instead,
                // we rely on the agent's session manager to expose the
                // active engine thread id. Falls back to None on
                // timeout — the caller should treat that as a hard
                // failure mode (engine never spawned a thread).
                //
                // This helper exists primarily to give the round-trip
                // test a deterministic point at which to check the
                // catalog state.
                return None;
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // -------------------------------------------------------------------
    // Round-trip tests (the load-bearing showstopper coverage)
    // -------------------------------------------------------------------

    /// **The load-bearing test**: full pause + resume cycle. After the
    /// caller supplies a tool result via `Submission::ExternalCallback`
    /// payload, the engine must surface that result back to the LLM as
    /// a tool result message. The LLM's second turn (per the trace)
    /// produces text that quotes a phrase only present in the supplied
    /// tool output — so if the payload never reaches the LLM, this
    /// test fails.
    ///
    /// Currently expected to **fail** until the resume materialisation
    /// fix lands (`bridge::router::resolve_gate` doesn't currently
    /// consume the payload for `ExternalCallback` resolutions).
    #[tokio::test]
    #[ignore = "documents the resume-payload bug — unignore after fix lands"]
    async fn round_trip_resume_payload_reaches_llm() {
        let trace = round_trip_trace();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        // Pre-register the catalog. Note: we don't yet know the engine
        // ThreadId, so this uses a placeholder UUID — exercising the
        // current (buggy) code path the responses_api handler takes.
        // A correct implementation would resolve the engine ThreadId
        // first, OR the catalog would key on something stable like
        // (user_id, conversation_scope) instead.
        let catalog = ironclaw::bridge::engine_external_tool_catalog()
            .await
            .expect("engine v2 must initialize the catalog");
        let placeholder_thread = ThreadId::new();
        catalog
            .register(
                placeholder_thread,
                vec![caller_action("lookup_weather", "Look up the weather")],
            )
            .await;

        rig.send_message("Look up the weather in NYC.").await;

        // Wait for the engine to pause on the external tool. If the
        // catalog under-key bug is real, the engine will instead try
        // to dispatch `lookup_weather` through the registry, find
        // nothing, and produce an error response. Either way, the
        // pause never fires.
        let pending = wait_for_external_pending_gate("test-user", TIMEOUT).await;
        let (request_id, action_name) = pending.expect(
            "engine never paused on external tool — likely thread-id mismatch \
             between handler-registered catalog and engine-spawned ThreadId",
        );
        assert_eq!(action_name, "lookup_weather");

        // Resume with the tool output. The payload mirrors the
        // Responses API resume contract: an `outputs` array.
        rig.send_external_callback_with_payload(
            request_id,
            serde_json::json!({
                "outputs": [{
                    "call_id": "call_lookup_weather_1",
                    "output": "sunny and 72F"
                }]
            }),
        )
        .await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert!(!responses.is_empty(), "no final response after resume");
        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    /// Simpler smoke variant: even before the resume payload bug, the
    /// engine should at least PAUSE when the LLM emits a tool_call
    /// for a registered external tool. If this fails, the catalog is
    /// not being consulted at all — likely the thread-id mismatch.
    #[tokio::test]
    #[ignore = "fails today on thread-id mismatch — documents the gap"]
    async fn engine_pauses_when_llm_calls_registered_external_tool() {
        let trace = round_trip_trace();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        let catalog = ironclaw::bridge::engine_external_tool_catalog()
            .await
            .expect("engine v2 catalog");
        catalog
            .register(
                ThreadId::new(),
                vec![caller_action("lookup_weather", "Look up the weather")],
            )
            .await;

        rig.send_message("Look up the weather in NYC.").await;

        let _ = &rig;
        let pending = wait_for_external_pending_gate("test-user", TIMEOUT).await;
        assert!(
            pending.is_some(),
            "expected an external-tool pending gate to fire after the LLM emitted \
             tool_calls for the registered name; found none"
        );
        rig.shutdown();
    }

    // -------------------------------------------------------------------
    // Catalog lifecycle and isolation
    // -------------------------------------------------------------------

    /// Catalog isolation by ThreadId — registering under one thread
    /// id does not leak into another. Direct catalog test, no engine
    /// roundtrip.
    #[tokio::test]
    async fn catalog_isolates_by_thread_id() {
        let catalog = ExternalToolCatalog::new();
        let thread_a = ThreadId::new();
        let thread_b = ThreadId::new();
        catalog
            .register(thread_a, vec![caller_action("a_only", "tool A")])
            .await;

        assert!(catalog.contains(thread_a, "a_only").await);
        assert!(!catalog.contains(thread_b, "a_only").await);
        assert!(catalog.list(thread_b).await.is_empty());
    }

    /// Re-registering replaces, not merges. The Responses API contract
    /// is that each request restates the full `tools[]` list — so
    /// dropping one tool from a follow-up request must remove it from
    /// the catalog rather than leave it lurking from the prior request.
    #[tokio::test]
    async fn catalog_register_overwrites_not_merges() {
        let catalog = ExternalToolCatalog::new();
        let thread = ThreadId::new();
        catalog
            .register(
                thread,
                vec![
                    caller_action("first", "first"),
                    caller_action("second", "second"),
                ],
            )
            .await;
        // Second request restates only one tool.
        catalog
            .register(thread, vec![caller_action("second", "second")])
            .await;
        let listed = catalog.list(thread).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "second");
    }

    /// TTL sweep evicts stale entries while leaving fresh ones.
    /// Defensive coverage for the backstop against callers that
    /// register tools and then abandon the conversation.
    #[tokio::test]
    async fn catalog_sweep_evicts_only_stale_entries() {
        let catalog = ExternalToolCatalog::new();
        let fresh = ThreadId::new();
        catalog.register(fresh, vec![caller_action("a", "a")]).await;
        // Sweep with an absurdly long max-age — nothing should evict.
        let evicted = catalog.sweep_older_than(chrono::Duration::days(1)).await;
        assert!(evicted.is_empty());
        // Sweep with a zero/negative max-age — everything evicts.
        let evicted = catalog
            .sweep_older_than(chrono::Duration::seconds(-1))
            .await;
        assert_eq!(evicted, vec![fresh]);
        assert!(catalog.is_empty().await);
    }

    /// Two concurrent registrations against different thread ids do
    /// not interfere. Documents the per-thread isolation invariant
    /// even under contention — protects against a future change that
    /// might accidentally collapse all entries into a single map.
    #[tokio::test]
    async fn catalog_handles_concurrent_registrations() {
        let catalog = Arc::new(ExternalToolCatalog::new());
        let mut handles = Vec::new();
        for _ in 0..32 {
            let catalog = Arc::clone(&catalog);
            handles.push(tokio::spawn(async move {
                let tid = ThreadId::new();
                catalog
                    .register(tid, vec![caller_action("concurrent", "concurrent test")])
                    .await;
                assert!(catalog.contains(tid, "concurrent").await);
            }));
        }
        for h in handles {
            h.await.expect("concurrent task did not panic");
        }
        assert_eq!(catalog.len().await, 32);
    }

    /// Callback-id helpers round-trip cleanly. This is the single
    /// disambiguator between caller-tool pauses and OAuth/pairing
    /// pauses in `bridge::router::notify_pending_gate` — if the
    /// `ext_tool:` prefix invariant is broken, OAuth pauses would be
    /// projected as `AppEvent::ExternalToolCall` (or vice versa) and
    /// the wrong UI would render.
    #[tokio::test]
    async fn callback_id_disambiguates_external_from_oauth() {
        use ironclaw::bridge::{
            call_id_from_external_callback, external_tool_callback_id, is_external_tool_callback_id,
        };
        let cb = external_tool_callback_id("call_42");
        assert!(cb.starts_with("ext_tool:"));
        assert!(is_external_tool_callback_id(&cb));
        assert_eq!(call_id_from_external_callback(&cb), Some("call_42"));

        // OAuth/pairing flows generate "pairing:<extension>" callback
        // ids — must NOT match the external-tool prefix.
        assert!(!is_external_tool_callback_id("pairing:telegram"));
        assert_eq!(
            call_id_from_external_callback("pairing:telegram"),
            None,
            "OAuth callback must not strip clean as a tool call_id"
        );
    }

    /// **Documents an unfixed gap**: there is currently no hook that
    /// clears the catalog when a thread reaches a terminal state.
    /// Until that hook lands, catalog entries leak monotonically —
    /// a long-running gateway with many short-lived requests will
    /// accumulate entries.
    ///
    /// This test asserts the cleanup behaviour we want; today it
    /// passes only because we've explicitly invoked `clear()`. The
    /// test name reminds us the production cleanup is missing.
    #[tokio::test]
    async fn catalog_clear_on_terminal_state_explicit() {
        let catalog = ExternalToolCatalog::new();
        let thread = ThreadId::new();
        catalog
            .register(thread, vec![caller_action("a", "a")])
            .await;
        assert!(!catalog.is_empty().await);

        // The fix is to invoke this on `EventKind::StateChanged { to:
        // Done | Failed }`. Until that hook exists, the catalog has
        // to be cleared explicitly by whoever has the catalog handle
        // — and there's no such caller today.
        catalog.clear(thread).await;
        assert!(catalog.is_empty().await);
    }

    /// **Documents an unfixed gap**: a caller registering a tool
    /// whose name collides with an internal action (e.g. `shell`,
    /// `memory_write`) is silently accepted today. The
    /// `EffectBridgeAdapter::execute_action` catalog short-circuit
    /// checks the catalog *before* the registry, so any LLM call to
    /// the colliding name lands in caller-side execution — even
    /// though the LLM saw the internal tool's description.
    ///
    /// `available_action_inventory` dedupes the *opposite* way
    /// (internal beats external in the LLM-visible list), so the
    /// vulnerability is in the dispatch path, not the surfacing path.
    /// The right fix is to reject the collision at request validation
    /// time so a confused LLM can't be tricked into running
    /// caller code while believing it ran the internal tool.
    ///
    /// Until validation rejects, this test stays ignored — the
    /// dispatcher behaviour today is the documented gap.
    #[tokio::test]
    #[ignore = "documents the collision-dispatch gap; unignore after validation rejects"]
    async fn external_collision_with_registry_action_is_rejected() {
        // Concrete shape of the desired check (paraphrased):
        //
        //     // in validate_external_tools(...)
        //     for tool in tools {
        //         if registry.get(name).is_some() {
        //             return Err(format!(
        //                 "tool '{name}' shadows a built-in action; \
        //                  pick a different name"
        //             ));
        //         }
        //     }
        //
        // Once that check is in place, this test should send a
        // request with `tools: [{name: "shell"}]` and assert the
        // handler returns 400 with an error message naming "shell".
        //
        // The HTTP-level scaffolding for this assertion lives in
        // `tests/responses_api_path_prefix.rs`; this stub records
        // the expected behaviour as soon as the registry is wired
        // into validate_external_tools.
        panic!("unimplemented — see comment for the desired wiring");
    }

    // -------------------------------------------------------------------
    // Helper for tests above (avoids unused-import lint when ignored
    // tests are skipped on a default `cargo test` run).
    // -------------------------------------------------------------------
    #[allow(dead_code)]
    async fn _harness_compiles() {
        let _ = wait_for_first_engine_thread;
        let _: Arc<ExternalToolCatalog> = Arc::new(ExternalToolCatalog::new());
    }
}
