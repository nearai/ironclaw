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
//! ## Regression coverage
//!
//! These tests guard the four bugs surfaced during the test-driven
//! review of the engine-native external-tools path and fixed in the
//! follow-up commit:
//!
//! 1. **Thread-id mismatch (Bug 1)** — `engine_pauses_when_llm_calls_registered_external_tool`.
//!    Catalog is keyed by engine `ThreadId`; the responses_api handler
//!    registers under the conversation_scope UUID it generates. The
//!    bridge `transfer` hook in `bridge::handle_with_engine_inner`
//!    re-keys onto the actual ThreadId after `handle_user_message`
//!    returns. If that hook regresses, this test panics on
//!    "engine never paused on external tool".
//! 2. **Resume payload materialisation (Bug 2)** — `round_trip_resume_payload_reaches_llm`.
//!    The bridge's `resolve_gate` path for `GateResolution::ExternalCallback`
//!    used to consult `pending.resume_output` only — which is `None`
//!    for tool-flavoured pauses, so it would re-run the action and
//!    re-pause forever. The fix special-cases `ext_tool:` callback
//!    ids and synthesises an `ActionResult` ThreadMessage from the
//!    resolution payload, which the LLM sees on its next call.
//! 3. **Collision rejection (Bug 3)** — covered at the HTTP boundary
//!    in `tests/responses_api_path_prefix.rs::external_tool_name_shadowing_registered_action_is_rejected`.
//!    Caller-supplied tool names that shadow registered actions are
//!    rejected up-front so a confused LLM can't be tricked into
//!    running caller code while believing it ran the internal tool.
//! 4. **Catalog cleanup on terminal state (Bug 4)** —
//!    `catalog_cleared_on_terminal_completed_outcome`. After
//!    `await_thread_outcome` joins on a non-`GatePaused` outcome,
//!    the catalog entry for the thread is dropped so it can't leak
//!    monotonically.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::sync::OnceLock;
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;
    use ironclaw::bridge::ExternalToolCatalog;
    use ironclaw_engine::{ActionDef, EffectType, ModelToolSurface, ThreadId};
    use tokio::sync::Mutex;

    const TIMEOUT: Duration = Duration::from_secs(10);

    /// Serializes the engine-touching tests in this module. The
    /// engine v2 bridge holds its state in a process-global
    /// `OnceLock<RwLock<Option<EngineState>>>` (see
    /// `src/bridge/router.rs::ENGINE_STATE`), so two parallel tests
    /// would each spin up a `TestRig` whose messages are routed
    /// through whichever rig won the `init_engine` race — one test's
    /// messages disappear into the other's engine. Per-test state
    /// isolation would need to land in the bridge itself; until then,
    /// any test that drives the engine end-to-end via `TestRigBuilder`
    /// must hold this mutex for its duration.
    fn engine_state_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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
    async fn round_trip_resume_payload_reaches_llm() {
        let _engine_guard = engine_state_lock().lock().await;
        // Drop any engine state left over from a prior test before
        // building this rig so the global ENGINE_STATE is bound to
        // *this* TestRig's components, not a previous one's.
        let trace = round_trip_trace();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        let scope_uuid = uuid::Uuid::new_v4();
        let scope_str = scope_uuid.to_string();

        // Register tools under the conversation_scope UUID — the
        // bridge transfer hook re-keys to the engine's actual
        // ThreadId once `handle_user_message` returns (Bug 1 fix).
        register_under_scope(scope_uuid, "lookup_weather").await;

        let msg = ironclaw::channels::IncomingMessage::new(
            "gateway",
            "test-user",
            "Look up the weather in NYC.",
        )
        .with_thread(scope_str);
        rig.send_incoming(msg).await;

        let (request_id, action_name) = wait_for_external_pending_gate("test-user", TIMEOUT)
            .await
            .expect(
                "engine never paused on external tool — Bug 1 \
                     (thread-id transfer) may have regressed",
            );
        assert_eq!(action_name, "lookup_weather");

        // Resume with the OpenAI-shaped output payload the
        // responses_api handler builds out of `function_call_output`
        // items. Bug 2 fix synthesizes an ActionResult ThreadMessage
        // from the matching entry; without that fix, the LLM's
        // second-turn context wouldn't see the output and
        // `verify_trace_expects` would fail on `response_contains:
        // ["sunny", "72F"]`.
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

    /// **Bug 4 regression**: the bridge clears catalog entries when
    /// a thread reaches a terminal `Completed` outcome. Without the
    /// `await_thread_outcome` cleanup hook, this catalog entry would
    /// leak forever.
    #[tokio::test]
    async fn catalog_cleared_on_terminal_completed_outcome() {
        let _engine_guard = engine_state_lock().lock().await;
        // Drop any engine state left over from a prior test before
        // building this rig so the global ENGINE_STATE is bound to
        // *this* TestRig's components, not a previous one's.
        // A simple text-only trace so the thread completes immediately
        // (no gate, no pause). We register a catalog entry under the
        // request's conversation_scope, verify it gets transferred
        // onto the engine ThreadId, then verify it's gone after the
        // thread completes.
        let trace = LlmTrace::from_file(format!(
            "{}/tests/fixtures/llm_traces/engine_v2/smoke_text.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("smoke text trace");
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        let scope_uuid = uuid::Uuid::new_v4();
        let scope_str = scope_uuid.to_string();

        // Use a unique action name so the cleanup-poll check can
        // verify "no entry anywhere has this action" regardless of
        // what other concurrent tests have registered. Keying off
        // `catalog.len()` was racy under parallel runs because the
        // engine_external_tool_catalog is process-global.
        let unique_action = format!("cleanup_marker_{}", uuid::Uuid::new_v4().simple());
        register_under_scope(scope_uuid, &unique_action).await;

        let catalog = ironclaw::bridge::engine_external_tool_catalog()
            .await
            .expect("catalog");
        assert!(
            catalog.contains_action_anywhere(&unique_action).await,
            "marker action must be present immediately after registration"
        );

        let msg = ironclaw::channels::IncomingMessage::new(
            "gateway",
            "test-user",
            "Hello! Introduce yourself briefly.",
        )
        .with_thread(scope_str);
        rig.send_incoming(msg).await;

        // Wait for the thread to complete. We don't strictly assert
        // on the response count: under engine v2, completed threads
        // primarily broadcast the response over SSE (which the test
        // channel doesn't subscribe to) and only return through
        // `Channel::respond` for some outcomes. The catalog-cleanup
        // poll below is the substantive assertion.
        let _ = rig.wait_for_responses(1, TIMEOUT).await;

        // Cleanup runs on the same task that does the join. Poll for
        // up to TIMEOUT — under parallel load the join task can be
        // scheduled later than the response broadcast, so we mirror
        // the response timeout rather than picking a tighter bound.
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            if !catalog.contains_action_anywhere(&unique_action).await {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "catalog entry for marker action {unique_action:?} was not cleared \
                     after thread completion — terminal cleanup hook may have regressed"
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        rig.shutdown();
    }

    /// Smoke variant of the round-trip: register a caller tool under
    /// a `conversation_scope` UUID, send a message that carries that
    /// UUID as its scope, and verify the engine pauses on the
    /// resulting tool_call. This exercises the `transfer` hook in
    /// `bridge::handle_with_engine_inner` that re-keys the catalog
    /// from the handler-supplied UUID onto the engine's actual
    /// `ThreadId` before the LLM call lands.
    #[tokio::test]
    async fn engine_pauses_when_llm_calls_registered_external_tool() {
        let _engine_guard = engine_state_lock().lock().await;
        // Drop any engine state left over from a prior test before
        // building this rig so the global ENGINE_STATE is bound to
        // *this* TestRig's components, not a previous one's.
        let trace = round_trip_trace();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        let scope_uuid = uuid::Uuid::new_v4();
        let scope_str = scope_uuid.to_string();

        // Register tools under the conversation_scope UUID. The bridge
        // transfer hook will rebind onto the actual engine ThreadId
        // once `handle_user_message` returns. Using
        // `register_under_scope` rather than `engine_external_tool_catalog`
        // directly is the resilient way to bootstrap from a clean
        // rig — see the helper for the polling rationale.
        register_under_scope(scope_uuid, "lookup_weather").await;

        // Send the user message with the matching conversation_scope.
        let msg = ironclaw::channels::IncomingMessage::new(
            "gateway",
            "test-user",
            "Look up the weather in NYC.",
        )
        .with_thread(scope_str);
        rig.send_incoming(msg).await;

        let pending = wait_for_external_pending_gate("test-user", TIMEOUT).await;
        assert!(
            pending.is_some(),
            "expected an external-tool pending gate to fire after the LLM emitted \
             tool_calls for the registered name; found none"
        );
        rig.shutdown();
    }

    /// Lazily register a caller tool under a `scope_uuid` ThreadId.
    /// The engine catalog only exists after `init_engine` runs; that
    /// happens on the first message routed through the bridge. To
    /// bootstrap from a clean rig, we poll until the catalog is
    /// available, then register. In practice the responses_api
    /// handler depends on the same lazy bootstrap — so this mirrors
    /// production behaviour.
    async fn register_under_scope(scope_uuid: uuid::Uuid, action_name: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        loop {
            if let Some(catalog) = ironclaw::bridge::engine_external_tool_catalog().await {
                catalog
                    .register(
                        ThreadId(scope_uuid),
                        vec![caller_action(action_name, "caller-supplied test tool")],
                    )
                    .await;
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "engine catalog never initialised; the bridge may not have \
                     bootstrapped engine v2 — verify TestRigBuilder.with_engine_v2() ran"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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

    /// Direct unit-level proof that `clear()` removes a registered
    /// entry. The production cleanup (Bug 4 fix) is wired in
    /// `await_thread_outcome` and exercised end-to-end by
    /// `catalog_cleared_on_terminal_completed_outcome` above; this
    /// test is the narrow primitive coverage for the helper itself.
    #[tokio::test]
    async fn catalog_clear_removes_entry() {
        let catalog = ExternalToolCatalog::new();
        let thread = ThreadId::new();
        catalog
            .register(thread, vec![caller_action("a", "a")])
            .await;
        assert!(!catalog.is_empty().await);

        catalog.clear(thread).await;
        assert!(catalog.is_empty().await);
    }

    // Note on collision rejection: the request-validation rejection
    // for caller-supplied tool names that shadow registered actions
    // is exercised at the HTTP level in
    // `tests/responses_api_path_prefix.rs::external_tool_name_shadowing_registered_action_is_rejected`.
    // That test asserts the handler returns 400 mentioning the
    // colliding name. A cargo-feature-gated registry isn't reachable
    // from this engine-tier file, so the check belongs at the wire
    // boundary.
}
