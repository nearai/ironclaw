# Reborn Integration Tests

In-process tests that run a **whole Reborn turn** with the real internal stack —
product workflow, turn coordinator, scheduler, the agent loop, the real
`LlmProviderModelGateway` + the real `ironclaw_llm` decorator chain, and real
`RootFilesystem` persistence. The **only** thing faked is the bottom of the
stack: a scripted model at the vendor-SDK seam. A test reaches no network, no
real process, no real channel, and needs no setup.

This is a distinct tier from `RebornBinaryE2EHarness` (which swaps the whole
`HostManagedModelGateway` with `RebornTraceReplayModelGateway` at the *gateway*
seam, skipping `ironclaw_llm`). This tier mocks one layer lower so the decorator
chain runs for real.

## How it works

```
submit_turn(text)                      ← synthetic inbound (no HTTP, no Slack parsing)
   → product workflow → turn coordinator → scheduler → agent loop
       → LlmProviderModelGateway → real decorator chain (passthrough)
           → scripted model (TraceLlm fed an in-memory trace)   ← THE ONLY FAKE
   → turn state + assistant reply persisted to RootFilesystem (InMemory)
assert_* reads the persisted reply / state
```

The fake sits beneath the real chain (`apply_decorator_chain`), so
retry/smart-routing/failover/circuit-breaker/response-cache and `CompletionRequest`+tool-def assembly all execute. Mocking
higher (at the gateway) is wrong — it skips `ironclaw_llm`.

## Writing a test (the shape — always)

```rust
#[tokio::test]
async fn replies_to_greeting() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("done")])
        .build().await.expect("harness builds");
    h.submit_turn("do something").await.expect("turn completes");
    h.assert_reply_contains("done").await.expect("reply finalized");
}
```

`build → submit_turn → assert`. Script each model turn with
`RebornScriptedReply::text(..)` — one line each. The harness is single-
conversation; `submit_turn`/`assert_reply_contains` take just the text.

## Requirements & expectations (non-negotiable)

1. **Test-first & consolidate.** Per root `CLAUDE.md` → Testing Discipline (and
   `.claude/rules/testing.md`): write/update the test first and watch it fail for
   the right reason; extend an existing test rather than standing up a redundant
   one, and say why if you add a new one.
2. **Readability contract.** ~3–12 lines, `build → submit_turn → assert`, no
   nested structs in the body. **Never** hand-build raw `TraceStep` /
   `LlmTrace::new` in a Reborn test — that is the verbosity the `RebornScriptedReply`
   façade removes.
3. **Mock only at the SDK seam.** Use `RebornScriptedReply`; do not swap the
   gateway or stub internals.
4. **Zero setup.** Must pass offline via a plain `cargo test --test reborn_<name>`
   — no services, no API keys, no `integration` feature, no Docker, no special
   linker. Hermetic env (keychain off, `TZ=UTC`, passthrough LLM config) is baked
   into `build()`.
5. **Minimal, inert edges.** The harness defaults every network/IO boundary to
   captured or inert — no real network, process, or channel. Wire only the
   boundaries your scenario actually crosses; a text-only turn needs no
   DB/HTTP/process setup.
6. **Test through the real path**, asserting on the persisted reply / recorded
   boundary calls / state — not on internals.

## Files

- `scripted_provider.rs` — `scripted_trace_llm(..)`, the `TraceLlm` raw-provider seam.
- `reply.rs` — `RebornScriptedReply` (the one-line-per-turn façade).
- `builder.rs` — `RebornIntegrationHarness` + builder, hermetic env, the
  slice-1/2 asserts (`assert_reply_contains` / `assert_tool_invoked` /
  `assert_egress_request_matching`, co-located with the harness fields) plus the
  slice-5 asserts (`assert_shell_command_recorded` / `assert_no_real_process_executed`)
  and the `pub(super)` capture accessors (`captured_egress_requests` /
  `captured_capability_results`) the assertion file reads.
- `process.rs` — `RecordingProcessPort`, the inert process port (slice 5): records
  every `CommandExecutionRequest.command` and returns exit 0 / empty output without
  spawning any OS process. Injected by default when `with_builtin_http_tools()` is
  used; the `.with_live_shell()` opt-in skips injection so the real
  `LocalHostProcessPort` executes instead.
- `http_matcher.rs` — `ScriptedHttpResponse`, the URL/method/capability-keyed
  HTTP scripting layer over `RecordingRuntimeHttpEgress` (install via
  `.with_keyed_http_responses([..])`).
- `assertions.rs` — the richer egress + tool-result assertions
  (`assert_egress_count` / `assert_egress_url_order` / `assert_egress_method_order`
  / `assert_egress_body_contains` / `assert_tool_result_contains`).
- Tests live as flat `tests/reborn_*.rs` (Cargo requires top-level test files).

Slice 6 MCP support lives in `harness.rs` (the `LoopbackMcpRuntimeHttpEgress`,
`LoopbackMcpRuntime` type alias, `mock_mcp_extension_package`,
`local_dev_host_runtime_with_registry_egress_and_mcp`, and
`HostRuntimeCapabilityHarness::mock_mcp_tools`) and `builder.rs`
(`MockMcp` variant, `.with_mock_mcp(mcp_url)`, `assert_mcp_tool_called`).

Module paths: each `tests/reborn_*.rs` declares both `#[path = "support/reborn/mod.rs"] mod reborn_support;` and `mod support;`, then `use reborn_support::builder::RebornIntegrationHarness;` / `use reborn_support::reply::RebornScriptedReply;`. Inside the support tree, siblings reference each other via `super::` and `trace_llm` via `crate::support::trace_llm` (there is no `crate::support::reborn` path). Copy the includes from `tests/reborn_integration_greeting.rs`.

Design: `docs/superpowers/specs/2026-06-26-reborn-integration-test-framework-design.md`.

## Implemented now vs planned

Slice 1 ships the spine + one text-reply test. Slice 2 ships
`RebornScriptedReply::tool_call(..)` + the CapabilityId→ProviderToolName mapping
+ `RecordingRuntimeHttpEgress` (FIFO body) + `assert_tool_invoked` /
`assert_egress_request_matching` (substring). Slice 4 ships the §3.6
P1-ergonomics **URL/method/capability-keyed HTTP matcher**
(`ScriptedHttpResponse` + `.with_keyed_http_responses([..])` — different scripted
body per URL for a multi-step tool-HTTP flow) and the richer **egress assertion
API** in the now-extracted `assertions.rs` (count / URL order / method order /
per-URL body / surfaced tool result). The keyed matcher is the canonical
HTTP-matcher API; an MCP/OAuth interceptor with per-URL response needs folds
into `ScriptedHttpResponse` rather than adding a parallel matcher.

Slice 3 ships `StorageMode { InMemory, LibSql }` (design spec §3.2, §3.8):
- `RebornIntegrationHarness::builder(..).storage(mode)` selects the backend.
- Both modes ride **one** `CompositeRootFilesystem` at the production path layout
  `/tenants/<tenant>/users/<user>/...` — the only difference is the `RootFilesystem`
  mounted under `/tenants`, `/memory`, and `/events`.
- Integration tier: threads at `/tenants/.../threads`; turns at `/tenants/.../turns`.
  **Binary-E2E tier is unchanged**: `RebornThreadHarness<LocalFilesystem>` still uses
  `/engine/tenants/...`; `scoped_turns_fs` in `harness.rs` keeps `/engine` prefix.
- `assert_reply_persists_after_reopen(text)` reopens the thread service over the same
  composite backend and asserts the reply survives (SQLite durability for LibSql;
  in-process re-instantiation for InMemory).
- `reborn_integration_backend_matrix.rs`: `backend_parity_replies_to_greeting`
  (`#[rstest] #[case(InMemory)] #[case(LibSql)]`) + `libsql_persists_reply_across_reopen`.
- Product / auth / approval / install / skill / secret stores join the same composite in
  their own §3.8 coverage slices; this slice covers thread + turn only.

Slice 5 ships the **inert process port + `.with_live_shell()` opt-in** (design spec §3.6):
- `RecordingProcessPort` (`process.rs`): impl `RuntimeProcessPort`, records every
  `command` string, returns exit 0 / empty output, never spawns a real OS process.
- Injected by default when `with_builtin_http_tools()` is used: the
  `local_dev_host_runtime_with_http_egress` helper calls
  `HostRuntimeServices::with_runtime_process_port_dyn(port)` — the existing pub builder
  method. **No production change was needed** (the injection seam was already public).
- `SHELL_CAPABILITY_ID` added to `core_builtin_tools_from_runtime`'s `capability_ids`
  so scripted `builtin.shell` calls surface to the model.
- `assert_shell_command_recorded(substr)` + `assert_no_real_process_executed()` on
  `RebornIntegrationHarness` (in `builder.rs`).
- `.with_live_shell()` builder opt-in skips recording-port injection so the real
  `LocalHostProcessPort` executes instead (use only for hermetic commands).
- `reborn_integration_process_port.rs`: `shell_call_recorded_not_executed` (end-to-end
  safety invariant) + `shell_assertions_fail_when_no_shell_call_ran` (guard).

Slice 6 ships the **real MCP runtime wired to a loopback mock MCP server** (design spec §3.6):
- `LoopbackMcpRuntimeHttpEgress`: test-only `RuntimeHttpEgress` making real HTTP
  connections to a loopback `MockMcpServer`; injects `Authorization: Bearer mock-mcp-test-token`;
  hermetic guard rejects any URL not prefixed by the configured `mcp_url`.
- `LoopbackMcpRuntime` type alias: the concrete `McpRuntime<...>` parameterisation used
  in test compositions.
- `mock_mcp_extension_package`: builds an `ExtensionPackage` via
  `from_host_bundled_manifest_with_inline_dynamic_schemas` so `parameters_schema` is the
  inline `{"type":"object"}` (not a `$ref`); this prevents `surface_descriptor` from
  attempting a filesystem read of a schema file that doesn't exist for a test-only extension.
- `local_dev_host_runtime_with_registry_egress_and_mcp`: wires the above into
  `HostRuntimeServices` with both first-party egress and the MCP runtime.
- `HostRuntimeCapabilityHarness::mock_mcp_tools`: convenience constructor for the above.
- `.with_mock_mcp(mcp_url)` on `RebornIntegrationHarnessBuilder`: switches to the MCP backend.
- `assert_mcp_tool_called(tool_name)` on `RebornIntegrationHarness`: asserts `"<provider>.<tool_name>"` was invoked.
- `reborn_integration_mcp.rs`: `mcp_tool_call_reaches_mock_server` (positive) +
  `assert_mcp_tool_called_fails_when_no_mcp_call_ran` (guard).

**Planned (do not assume present; add behind a test that exercises it — no dead code):**
`StorageMode::Postgres` (CI container lane); product/auth/approval/install/skill/secret
stores on the composite; `.with_live_http_egress()` opt-in; outbound/secrets capture wiring;
the pre-commit test-style check.
