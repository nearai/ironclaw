# Reborn Integration-Test Framework — Slice 2 Implementation Plan

**Date:** 2026-06-27
**Branch:** `feat/reborn-integration-test-framework`
**Base:** slice 1 (`0364c72a8`) + docs (`11f9b12d0`)

## Goal (locked)

Extend the framework so a Reborn integration test can drive a **tool-calling turn**
and verify it — including the **Tier-2 egress capture** — while keeping tests terse
(§4.2) and every edge captured by default (§3.6). Prove the tool path + the two-tier
egress design end-to-end with running code.

## Tool path chosen: first-party HTTP egress (`builtin.http`) — the Tier-2 proof

The scripted model emits a `builtin.http` tool call; the real first-party tool runtime
executes it through `RuntimeHttpEgress`, captured by the existing `RecordingRuntimeHttpEgress`
(scripted body, no network). This is the §3.7 "Tier-2 consumer #1" and the strongest
proof requested by the slice. It is **not** disproportionately heavy because the existing
`HostRuntimeCapabilityHarness::core_builtin_tools()` already wires exactly this
(HTTP capability + `RecordingRuntimeHttpEgress` body `{"accepted":true}`,
network policy `http_test_policy()` allowing `https://api.example.test`) and exposes
`capability_factory()` / `capability_result_writer()` / `runtime_http_requests()`. We reuse it.

## File-by-file

### 1. `tests/support/reborn/reply.rs` (+ ~20 lines)
Add `RebornScriptedReply::tool_call(capability_id, args)`:
- maps `capability_id: &str` → ProviderToolName via the §3.4 base mapping `'.' → "__"`
  (`capability_id.replace('.', "__")`). CapabilityIds are validated dot-separated
  alphanumeric segments, so this is the exact, reversible base mapping the gateway ships
  (`provider_tool_name_base` in `capability_port.rs`). No digest: digests only appear on
  provider-name *collisions*, which cannot happen for the distinct capabilities a test scripts.
- builds one `TraceStep { request_hint: None, response: TraceResponse::ToolCalls {
  tool_calls: vec![TraceToolCall { id, name, arguments }], input_tokens: 0, output_tokens: 0 },
  expected_tool_results: [] }`.
- auto-fills `id` from a process-global `AtomicU64` (`call-1`, `call-2`, …) so multiple
  scripted tool calls across a test get distinct, deterministic ids.
- doc-comment states the accepted format is CapabilityId (e.g. `"builtin.http"`), per §3.4.

Mapping lives at construction (one place). `TraceLlm` is unchanged (no new replay provider),
honoring §3.3.1.

### 2. `tests/support/reborn/harness.rs` — visibility bumps only (no logic change)
Promote the existing, tested wiring so the sibling `builder.rs` can reuse it (single mechanism):
- `enum HarnessCapabilityMode` → `pub(crate)` (+ variants are already in-module; enum bump suffices).
- `type HarnessCapabilityParts` → `pub(crate)`.
- `fn HarnessCapabilityMode::into_parts` → `pub(crate)`.
- `enum HarnessCapabilityRecorder` → `pub(crate)`; methods `invocations()` and
  `runtime_http_requests()` → `pub(crate)`.
- `struct HostRuntimeCapabilityHarness` → `pub(crate)`; `core_builtin_tools()` → `pub(crate)`.

No behavior change — these are the exact constructs `RebornBinaryE2EHarness` already uses.

### 3. `tests/support/reborn/builder.rs` (~+35 / -20 lines)
- Add a private `enum RebornCapabilityBackend { Echo, BuiltinHttpTools }` (default `Echo`).
- Builder field `capability: RebornCapabilityBackend`; method
  `pub fn with_builtin_http_tools(mut self) -> Self`.
- In `build()`: create `milestone_sink` earlier; resolve the backend to a
  `HarnessCapabilityMode` (`Echo` → `Recording(RecordingTestCapabilityPort::echo())`,
  `BuiltinHttpTools` → `HostRuntime(Arc::new(HostRuntimeCapabilityHarness::core_builtin_tools().await?))`),
  then `into_parts(milestone_sink.clone())?` → `(factory, surface_resolver, input_resolver,
  result_writer, recorder)`. This **replaces** the slice-1 inline echo wiring (port,
  `capability_io`, factory, `CapabilityAllowSet::All` resolver) with the shared path —
  one mechanism for both backends.
- Pass `factory` / `surface_resolver` / `result_writer` to `build_default_planned_runtime`,
  and `JsonSpawnSubagentInputCodec::new(input_resolver)`.
- Store `recorder: HarnessCapabilityRecorder` on `RebornIntegrationHarness`.
- Add assertion helpers (co-located with the harness fields they read, per slice-1 note):
  - `assert_tool_invoked(&self, capability_id: &str)` — recorder `invocations()` contains it.
  - `assert_egress_request_matching(&self, url_substr: &str)` — recorder
    `runtime_http_requests()` has a request whose `url` contains the substring (Tier-2 proof).

Note: the echo arm's surface allow-set moves from `All` to the echo port's allowlist
(`[test.echo]`); the slice-1 greeting turn invokes no tool, so it is unaffected — verified
in Phase D by re-running `reborn_integration_greeting`.

### 4. `tests/reborn_integration_tool_call.rs` (NEW, ≤15 lines body)
Test-first. Shape `build → script → submit_turn → assert`:
```rust
let h = RebornIntegrationHarness::test_default()
    .with_builtin_http_tools()
    .script([
        RebornScriptedReply::tool_call("builtin.http",
            json!({"url": "https://api.example.test/v1/items", "method": "GET"})),
        RebornScriptedReply::text("fetched"),
    ])
    .build().await.expect("harness builds");
h.submit_turn("fetch items").await.expect("turn completes");
h.assert_tool_invoked("builtin.http").await.expect("http tool ran");
h.assert_egress_request_matching("api.example.test").await.expect("egress captured");
h.assert_reply_contains("fetched").await.expect("final reply");
```
Includes the standard `#[path]` module preamble copied from `reborn_integration_greeting.rs`.

## Reused unchanged
`TraceLlm` engine; `RecordingRuntimeHttpEgress`; `HostRuntimeCapabilityHarness::core_builtin_tools`;
`HarnessCapabilityMode`/`into_parts`/`HarnessCapabilityRecorder`; `LlmProviderModelGateway` +
real decorator chain; hermetic env.

## Deferred (NOT built — no test exercises them)
`StorageMode::LibSql` + matrix; inert `RecordingProcessPort` + `.with_live_shell()` /
`.with_live_http_egress()`; URL-keyed matcher API; MCP/OAuth tests; generic `Recording<P>`;
proc-macro. `apply_decorator_chain` visibility is left exactly as shipped (pending human decision).

## Thermo self-review
- **No over-engineering:** reuse `core_builtin_tools()` + `into_parts` rather than a new
  runtime or generic recorder. Mapping is a one-line `replace`, not a tools-resolution engine.
- **No dead code:** every new symbol is exercised by the one new test (or the greeting test
  via the unified echo path).
- **No two-mechanisms:** echo and HTTP backends both flow through `HarnessCapabilityMode::into_parts`.
- **Right layer:** mapping at reply construction (`reply.rs`); capture reuse at the builder seam.
- **Extractable-not-generic:** `RecordingRuntimeHttpEgress` stays the concrete scripted-body +
  captured-`Vec` shape (§3.7); no `Recording<P>`.

## As-built note

Implemented as planned. Deltas worth recording:

- **§3.4 mapping** is applied at reply-construction time (`capability_id.replace('.', "__")`),
  not by resolving against `ToolCompletionRequest.tools` at the seam — equivalent for the
  collision-free capability IDs scripted tests use, and keeps `TraceLlm` unchanged. Spec §3.4
  trued-up to match.
- **Capability wiring** reuses the existing `HarnessCapabilityMode::into_parts` (promoted to
  `pub(crate)` along with `HarnessCapabilityRecorder` / `HostRuntimeCapabilityHarness` /
  `core_builtin_tools`) — both Echo (default) and `BuiltinHttpTools` flow through it (single
  mechanism). The Echo arm's surface allow-set is now the port allowlist (was
  `CapabilityAllowSet::All`); benign — a text turn invokes no tool. Slice-1 greeting test
  re-verified green.
- **Test tool path:** `builtin.http` over the recording `RuntimeHttpEgress` (the §3.7 Tier-2
  proof). `method` is omitted (defaults to `get`); passing `"GET"` uppercase fails the tool's
  lowercase `method` enum.
- **Tests:** `tests/reborn_integration_tool_call.rs` — the positive tool-call+egress test plus
  one negative test (`assertions_fail_when_tool_did_not_run`) guarding the assertion-helper
  `Err` branches (added per code-review).
- **`apply_decorator_chain`** left exactly as shipped (`pub`). A relayed request to narrow it
  to `pub(crate)` + a `testing` re-export does not compile (`pub use` cannot re-export a
  `pub(crate)` item — E0364) and contradicts the original task instruction, so it was reverted;
  `ironclaw_llm` has zero diff.
- **Deferred items confirmed not built:** no `StorageMode::LibSql`, no `RecordingProcessPort`,
  no live opt-ins, no URL-keyed matcher, no generic `Recording<P>`, no proc-macro.
