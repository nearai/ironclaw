//! Tool-calling turn: proves the §3.7 two-tier egress design end-to-end —
//! scripted `builtin.http` call → real `RuntimeHttpEgress` → recording egress
//! (Tier-2) → finalized reply. Same scripted `TraceLlm` seam as other harness tests.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_threads::MessageKind;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn runs_http_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("Tier-2 egress captured");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// Guards against vacuous pass: with no scripted tool call, both
/// `assert_tool_invoked` and `assert_egress_request_matching` must return `Err`.
#[tokio::test]
async fn assertions_fail_when_tool_did_not_run() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no tool")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("just talk").await.expect("turn completes");
    assert!(h.assert_tool_invoked("builtin.http").await.is_err());
    assert!(
        h.assert_egress_request_matching("api.example.test")
            .await
            .is_err()
    );
}

/// Proves the assertions discriminate when the invocation + egress lists are
/// NON-empty: a real `builtin.http` call runs, but assertions for a *different*
/// capability/host must still return `Err` (the "present but no match" branch).
#[tokio::test]
async fn assertions_fail_when_tool_present_but_requested_tool_or_url_does_not_match() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    // Prove capture lists are NON-empty first, so the checks below exercise the
    // mismatch branch, not the empty-list branch.
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran before mismatch assertions");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http egress captured before mismatch assertions");
    // Non-empty invocation list — wrong capability id must fail.
    assert!(
        h.assert_tool_invoked("some.other.capability")
            .await
            .is_err()
    );
    // Non-empty egress list — non-matching host substring must fail.
    assert!(
        h.assert_egress_request_matching("nonmatching.host.test")
            .await
            .is_err()
    );
}

/// Proves the multi-segment `builtin.http.save` capability id (`.`→`__`
/// encoding to `builtin__http__save` at the provider seam) resolves end-to-end,
/// writing to the `/workspace` mount `core_builtin_tools` provides read-write.
#[tokio::test]
async fn runs_http_save_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http.save",
                json!({"url": HTTP_TOOL_URL, "save_to": "/workspace/response.json"}),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch and save")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.http.save")
        .await
        .expect("http.save tool ran");
    // The save path must reach the real `RuntimeHttpEgress`.
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http.save egress captured");
    h.assert_reply_contains("saved")
        .await
        .expect("final reply finalized");
}

/// Regression for #5817: a decimal lifted from prose (`0.95`) tokenizes as
/// `digits.digits`, satisfying the capability-id shape check. The guard must
/// not mistake it for a requested-but-unavailable capability and suppress the
/// model's real `builtin.http` call.
#[tokio::test]
async fn decimal_number_in_prompt_does_not_suppress_tool_call() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn(
        "compute the correlation-adjusted 95% = 0.95 (use 0.95 in formulas), then fetch items",
    )
    .await
    .expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran; guard must not misfire on the decimal 0.95");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("scripted http call crossed the recording egress");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

/// Regression for #5782: a backticked code reference (`playwright.sync_api`,
/// a Python module) tokenizes like a capability id sitting after "use". The
/// guard must not mistake it for a capability request and suppress the
/// model's real `builtin.http` call.
#[tokio::test]
async fn backticked_code_reference_in_prompt_does_not_suppress_tool_call() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("use `playwright.sync_api` (Python sync API) as reference, then fetch items")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran; guard must not misfire on the code reference playwright.sync_api");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("scripted http call crossed the recording egress");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

/// The globally-disabled `builtin.spawn_subagent` capability (configured
/// through `DefaultPlannedRuntimeConfig::disabled_capability_ids`, applied as
/// the OUTERMOST `PerSurfaceCapabilityDenyDecorator` in
/// `build_default_planned_runtime_inner` — see that function's doc comments)
/// must never reach the model-facing tool list, whichever port would
/// otherwise have surfaced it: the flavor-aware `SubagentSpawnCapabilityDecorator`
/// (always wired, independent of any harness extension registry) or the
/// host-runtime first-party manifest stub (`builtin_first_party_package()` in
/// `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`, included in
/// `core_builtin_tools()`'s registry unconditionally).
///
/// Non-vacuity: confirmed by direct inspection that `core_builtin_tools()`'s
/// capability port surfaces `builtin__spawn_subagent` when the deny decorator
/// is bypassed (i.e. `spawn_decorator` runs before the outermost deny filter
/// in composition order) — so this assertion is pinning a real strip, not
/// asserting absence from an already-empty surface. `builtin__http` is
/// asserted present as the non-vacuity control for THIS test's own capture.
#[tokio::test]
async fn disabled_spawn_subagent_capability_is_stripped_from_model_surface() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hello").await.expect("turn completes");

    let captured = h.scripted_llm.captured_tool_definitions();
    let names: Vec<&str> = captured
        .iter()
        .flatten()
        .map(|def| def.name.as_str())
        .collect();

    // Neither encoding of the disabled capability id may appear in what the
    // model was shown (provider-seam `__` encoding, or the raw dotted id).
    assert!(
        !names.contains(&"builtin__spawn_subagent"),
        "disabled capability's provider seam name must not be advertised: {names:?}"
    );
    assert!(
        !names.contains(&"builtin.spawn_subagent"),
        "disabled capability's raw dotted id must not be advertised: {names:?}"
    );
    // Control: a real capability IS present, so the absence asserts above are
    // not vacuously true against an empty surface.
    assert!(
        names.contains(&"builtin__http"),
        "control tool builtin__http must be present: {names:?}"
    );
}

/// A model that calls the disabled `builtin.spawn_subagent` anyway is rejected
/// at the gateway (`CapabilitySurfaceDenyFilter`, before
/// `register_provider_tool_call` ever stages an invocation) — the whole
/// provider response fails with `InvalidOutput` → `Unavailable`, reaching a
/// terminal `TurnStatus::Failed`/`"model_unavailable"` after exactly one
/// scripted turn. No `ToolResultReference` is persisted; `assert_tool_invoked`
/// returning `Err` proves the capability was never dispatched.
#[tokio::test]
async fn disabled_spawn_subagent_capability_call_anyway_fails_the_run() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::tool_call(
            "builtin.spawn_subagent",
            json!({"goal": "test"}),
        )])
        .build()
        .await
        .expect("harness builds");

    let run_id = h
        .submit_turn_async("spawn a subagent")
        .await
        .expect("turn submitted");
    let state = h
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Failed)
        .await
        .expect("run reaches Failed after the disabled capability is rejected at the gateway");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(
        failure.category(),
        "model_unavailable",
        "expected the Unavailable fidelity category (InvalidOutput -> Unavailable), got {failure:?}"
    );

    // No side effect: the capability was rejected before dispatch, so it was
    // never invoked.
    assert!(
        h.assert_tool_invoked("builtin.spawn_subagent")
            .await
            .is_err(),
        "disabled capability must never be dispatched, even when the model calls it anyway"
    );
}

/// A `read_file` result large enough to exceed `TOOL_RESULT_RECORD_READ_MAX_BYTES`
/// (2048 bytes) once serialized, so both durable-projection tests below
/// exercise truncation. Every line is distinct so `TAIL_MARKER` (the last
/// line) can only appear once the raw payload's tail is reached.
const DURABLE_CONTENT_LINES: usize = 400;
const TAIL_MARKER: &str = "line-0399";

fn large_durable_file_content() -> String {
    (0..DURABLE_CONTENT_LINES)
        .map(|i| format!("line-{i:04} filler filler filler filler"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Durable tool-result projection (issue #5838 / PR #5902): a `read_file`
/// result routed through the REAL `LocalDevCapabilityIo`
/// (`.with_durable_capability_io_file_tools()`, which wires
/// `new_with_durable_previews` over this harness's own local-dev session
/// thread service — mirrors production's `capability_wiring`) must reach the
/// model as a truncated `ResultReference` preview
/// (`local_dev_result_reference_observation`), never the raw payload.
///
/// RED evidence for this PR: against the harness's `ProductLive` default
/// (`ProductLiveCapabilityIo::write_capability_result`, which sets no
/// `model_observation`), this assertion fails — the executor falls back to
/// embedding the full raw output with no truncation summary at all. Verified
/// by running this test body against `RebornIntegrationHarness::test_default()`
/// with only `.with_builtin_http_tools()`'s file-tool sibling (no durable
/// opt-in) before adding the harness seam; swapping in
/// `.with_durable_capability_io_file_tools()` is what turns it green.
#[tokio::test]
async fn durable_large_read_file_result_reaches_model_as_truncated_preview() {
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/durable.txt", "content": large_durable_file_content()}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.read_file",
                json!({"path": "/workspace/durable.txt"}),
            ),
            RebornScriptedReply::text("read it"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("write then read the durable file")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.read_file")
        .await
        .expect("read_file ran");

    // Model-visible seam: the persisted ToolResultReference message (what the
    // conversation history — and thus the next model request — actually
    // carries) contains the host-authored truncation summary...
    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        "preview truncated",
    )
    .await
    .expect("model-visible history carries the ResultReference truncation summary");
    // ...and never the raw payload's tail. Scoped to ToolResultReference-kind
    // messages (not ANY role): the model's OWN `write_file` tool-call
    // arguments legitimately echo the full content elsewhere in history —
    // this asserts the absence specifically from the persisted TOOL RESULT
    // side, which is what `LocalDevCapabilityIo` controls.
    assert!(
        h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, TAIL_MARKER)
            .await
            .is_err(),
        "raw payload tail must not reach the model-visible tool-result transcript"
    );
}

/// `result_read` continuation (issue #5838): a second scripted turn on the
/// SAME thread calls `builtin.result_read` (`RESULT_READ_CAPABILITY_ID`,
/// `runtime/local_dev/result_read.rs`) with the durable `result_ref` and
/// `next_offset` the first turn's `read_file` observation reported —
/// discovered via `latest_tool_result_ref`/`latest_tool_result_next_offset`
/// (a static script cannot know a server-minted ref ahead of time) and
/// injected with `push_script`. Asserts the returned chunk continues
/// byte-exactly from the SAME canonical serialization `tool_result_output`
/// returns for `read_file` — no gap, no overlap — and reports the true
/// `total_bytes` of the durable record.
#[tokio::test]
async fn result_read_continues_a_durable_result_byte_exactly() {
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/durable.txt", "content": large_durable_file_content()}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.read_file",
                json!({"path": "/workspace/durable.txt"}),
            ),
            RebornScriptedReply::text("read it"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("write then read the durable file")
        .await
        .expect("turn completes");

    let raw_output = h
        .tool_result_output("builtin.read_file")
        .await
        .expect("read_file result recorded");
    let serialized = serde_json::to_vec(&raw_output).expect("read_file output serializes");
    let result_ref = h
        .latest_tool_result_ref()
        .await
        .expect("read_file's durable result_ref is persisted");
    let next_offset = h
        .latest_tool_result_next_offset()
        .await
        .expect("read_file's observation reports a continuation offset");
    assert!(
        (next_offset as usize) < serialized.len(),
        "test fixture must exceed the preview cutoff to exercise continuation"
    );

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": next_offset,
                "max_bytes": ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES,
            }),
        ),
        RebornScriptedReply::text("continued"),
    ]);
    h.submit_turn("continue reading the file")
        .await
        .expect("second turn completes");

    let chunk = h
        .tool_result_output("builtin.result_read")
        .await
        .expect("result_read result recorded");
    let chunk_content = chunk["content"].as_str().expect("chunk content is text");
    let offset = next_offset as usize;
    let expected = &serialized[offset..offset + chunk_content.len()];
    assert_eq!(
        chunk_content.as_bytes(),
        expected,
        "result_read chunk must continue byte-exactly from the preview cutoff (no gap/overlap)"
    );
    assert_eq!(
        chunk["total_bytes"].as_u64(),
        Some(serialized.len() as u64),
        "result_read must report the true total byte length of the durable record"
    );
}
