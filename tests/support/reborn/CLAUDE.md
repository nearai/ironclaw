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
- `builder.rs` — `RebornIntegrationHarness` + builder, hermetic env, core assertions
  (`assert_reply_contains` / `assert_tool_invoked` / `assert_egress_request_matching`,
  co-located with the harness fields), shell assertions
  (`assert_shell_command_recorded` / `assert_shell_ran_through_inert_port`),
  MCP assertion (`assert_mcp_tool_called`), approval methods
  (`submit_turn_until_blocked` / `approve_gate` / `deny_gate` / `enable_auto_approve`),
  and the `pub(super)` capture accessors (`captured_egress_requests` /
  `captured_capability_results`) the assertion file reads.
- **`harness.rs` split follow-up** — the MCP/process-port wiring block
  (`LoopbackMcpRuntimeHttpEgress`, `mock_mcp_extension_package`,
  `local_dev_host_runtime_with_registry_egress_and_mcp`,
  `HostRuntimeCapabilityHarness::mock_mcp_tools`, and the `LoopbackMcpRuntime`
  type alias) is a tracked follow-up to extract into a `harness_mcp.rs` sub-module
  (the file exceeds 4000 lines; see arch-exempt annotation near `LoopbackMcpRuntime`).
- `process.rs` — `RecordingProcessPort`, the inert process port: records every
  `CommandExecutionRequest.command` and returns exit 0 / empty output without
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

Module paths: each `tests/reborn_*.rs` declares both `#[path = "support/reborn/mod.rs"] mod reborn_support;` and `mod support;`, then `use reborn_support::builder::RebornIntegrationHarness;` / `use reborn_support::reply::RebornScriptedReply;`. Inside the support tree, siblings reference each other via `super::` and `trace_llm` via `crate::support::trace_llm` (there is no `crate::support::reborn` path). Copy the includes from `tests/reborn_integration_greeting.rs`.

Design rationale: see git history.

## Capabilities

Capabilities are opt-in. The default harness is text-only (no tools, no storage,
no process, no MCP). Wire only what your scenario crosses.

### Storage

`StorageMode` selects the durable backend mounted into the harness's
`CompositeRootFilesystem`. Both modes use one composite at the production path
layout `/tenants/<tenant>/users/<user>/...`.

| Mode | What it provides | Opt-in |
|---|---|---|
| `InMemory` (default) | Fast, no filesystem I/O; supports service re-instantiation. | Default — no builder call needed. |
| `LibSql` | Real SQLite on a per-test `TempDir`; full SQL migrations + CAS. | `.storage(StorageMode::LibSql)` |

**`assert_reply_persists_after_reopen(text)`** — reopens the thread service over
the same composite backend and asserts the reply survives. For `LibSql`: opens a
genuinely fresh `libsql::Database` connection to the on-disk `.db` file
(independent of the live `Arc`) — data not serialized to disk will not appear,
proving real on-disk durability. For `InMemory`: re-instantiates the service over
the same in-process handle (no disk involved; proves service re-instantiation, not
durability).

### Tool calls

Script a tool call with `RebornScriptedReply::tool_call(capability_id, args)`,
where `capability_id` is the full id (e.g. `"builtin.http"`). Enable the real
first-party runtime with `.with_builtin_http_tools()`. The recording
`RuntimeHttpEgress` intercepts every outbound HTTP call at the runtime boundary
(no real network); responses come from the script.

Core assertions in `builder.rs`:
- `assert_tool_invoked(capability_id)` — proves the named capability was dispatched through the real capability path.
- `assert_egress_request_matching(url_substr)` — proves a runtime HTTP egress crossed the recording boundary (URL contains the substring).

Richer assertions in `assertions.rs` (all check the `[baseline..]` delta per thread):
- `assert_egress_count(n)` — exact count of captured egress requests.
- `assert_egress_url_order(&[substrings])` — URLs in call order, each containing the matching substring; also checks count.
- `assert_egress_method_order(&[methods])` — HTTP methods in call order (case-insensitive).
- `assert_egress_body_contains(url_substr, body_substr)` — body of the captured egress request whose URL contains the substring.
- `assert_tool_result_contains(needle)` — a recorded capability result's output contains the text (proves the scripted body surfaced back to the model).

### Keyed HTTP responses

For multi-step tool flows where each `builtin.http` call to a different URL
needs a different scripted body, install keyed responses via
`.with_keyed_http_responses([..])`. This implies `.with_builtin_http_tools()`.

`ScriptedHttpResponse` — first-match-wins in scripted order:

- `ScriptedHttpResponse::for_url(url_substr, body)` — matches any request whose URL contains the substring.
- `.with_method(method)` — narrow to a specific HTTP method (lowercase, e.g. `"post"`).
- `.with_capability(capability_id)` — narrow to a specific capability id (e.g. `"builtin.http"`).

Requests that match no scripted response fall back to the recording egress default body. The keyed matcher is the canonical HTTP-scripting API; new per-URL response needs fold into `ScriptedHttpResponse` rather than adding a parallel matcher.

### Shell / process

When `.with_builtin_http_tools()` is active, `builtin.shell` turns are dispatched
through the inert `RecordingProcessPort` by default. It records every
`CommandExecutionRequest.command` string and returns exit 0 / empty output without
spawning any OS process.

- `assert_shell_command_recorded(substr)` — the recorded command string contains `substr`.
- `assert_shell_ran_through_inert_port()` — at least one shell command was recorded by the inert port (proves no real OS process ran).

**`.with_live_shell()`** — opt-in; skips recording-port injection so the real
`LocalHostProcessPort` executes instead. Use only for hermetic commands
(no network, no external state, reproducible on any machine).
Implies `.with_builtin_http_tools()`.

### MCP

`.with_mock_mcp(mcp_url)` wires the real MCP runtime to a loopback
`MockMcpServer`. `LoopbackMcpRuntimeHttpEgress` makes real HTTP connections to
the mock server on a loopback port, injecting `Authorization: Bearer mock-mcp-test-token`
so the mock's OAuth gate passes; it rejects any URL not prefixed by the configured
`mcp_url`. A single MCP capability `"mock-mcp.search"` is registered.

Script with `RebornScriptedReply::tool_call("mock-mcp.search", json!({}))`.

- `assert_mcp_tool_called(tool_name)` — maps `tool_name` → `"mock-mcp.<tool_name>"` and delegates to `assert_tool_invoked`.

### OAuth / product-auth

Available from crate `ironclaw_reborn_composition::test_support`, gated on
`#[cfg(feature = "test-support")]`.

**`ScriptedOAuthTokenEgress`** — `RuntimeHttpEgress` impl returning a fixed
token-exchange JSON body and recording all calls. Ignores the actual URL;
every call consumes the next scripted response.

Constructors:
- `ScriptedOAuthTokenEgress::with_access_token(token)` — default scripted `200` response with `access_token`.
- `ScriptedOAuthTokenEgress::with_access_and_refresh_token(access, refresh)` — scripted `200` with both tokens; required for refresh-sweep tests that need a `refresh_token` in the initial exchange body.
- `ScriptedOAuthTokenEgress::with_error_response(status, error_code)` — scripted non-200 body (e.g. `400`, `"invalid_grant"`) as the default for every call.
- `push_response(status, body)` — enqueue a per-call FIFO override; consumed before the default response.

Assertion accessors: `captured_count()` / `captured_grant_types()` (returns only the non-secret `grant_type` discriminator — e.g. `"authorization_code"` or `"refresh_token"` — extracted from each captured request body, never raw secrets).

**`OAuthProductAuthTestBundle`** — bundles `Arc<RebornProductAuthServices>` wired
over real `FilesystemAuthProductServices<InMemoryBackend>` alongside a
`ScriptedOAuthTokenEgress`. Construct via:

- `build_oauth_product_auth_for_test()` — generic provider; no refresh token in egress body.
- `build_google_oauth_product_auth_for_test()` — `provider_id = "google"`, includes `refresh_token` in the egress body; wires the provider client so `refresh_credential_account` does not short-circuit.

**`OAuthProductAuthTestBundle::sweep_for_refresh(candidates, settings, now)`** —
drives one sweep tick with an always-leader lock and a frozen clock, exercising
`sweep_once` → `ProviderBackedCredentialAccountService::refresh_account` →
`HostOAuthProviderClient::refresh_token` → scripted egress. Requires
`--features libsql`.

### Approvals (group tests only)

`RebornIntegrationGroup::live_approvals()` constructs a group with real file-tool
approval stores (`write_file`/`read_file` at `PermissionMode::Ask`). Auto-approve
is disabled for the group scope at construction so gated tool calls raise real
`BlockedApproval` gates.

On a harness built from a `live_approvals` group:

- `submit_turn_until_blocked(text)` — submits a turn and waits for `TurnStatus::BlockedApproval`, returning `(run_id, gate_ref)`.
- `approve_gate(run_id, &gate_ref)` — resolves the persisted approval request to an issued lease and resumes the run.
- `deny_gate(run_id, &gate_ref)` — resolves to `Denied` and resumes with `GateResumeDisposition::Denied`; the executor surfaces a non-retryable authorization failure to the model.
- `wait_for_status(run_id, expected)` — polls the turn-state store until the run reaches `expected`; fails fast on a different terminal status.
- `enable_auto_approve()` — flips the per-`(tenant, user)` CAS-persisted auto-approve toggle ON; the flip persists across threads in the group because the store is shared.

### Test-support crate accessors

`HostRuntimeCapabilityHarness` (in `harness.rs`, gated on `#[cfg(feature = "test-support")]`) exposes:

- `extension_installation_store_for_test()` — returns the `Option<Arc<dyn ExtensionInstallationStore>>` wired into the local-dev extension management port; mirrors the production installation store for test read-back assertions. Returns `None` when the local runtime has no extension management wired.

`ironclaw_reborn_composition::test_support` exposes:

- `build_local_dev_secret_store_for_test(root, scoped)` — constructs the `LocalDevSecretStore` used by production local-dev composition; for store read-back in secrets tests.

Both are zero-byte in production builds (gated on the `test-support` feature).

## Group tests

`RebornIntegrationGroup` (in `group.rs`) owns shared storage and a shared
capability backend once; each `.thread(conv_id)` builds a per-thread turn
runtime over those shared pieces. Cross-thread persistence is real — thread A
writes, thread B sees it. Single-shot `test_default()` is a degenerate
one-thread group (its own storage, baseline = 0); all existing tests are
byte-identical after this refactor.

### When to use a group (vs a flat test)

Use a group **only** when the scenario needs cross-thread persistence — e.g.,
thread A submits a tool call that raises an approval gate; thread B resolves
the gate; thread A resumes. A scenario that submits + asserts in one thread
belongs in a flat `tests/reborn_integration_*.rs` test as always.

### Group test binary layout

Group tests live in subdirectories under `tests/`:

```
tests/reborn_group_approvals/
    main.rs                              # one #[tokio::test], drives scenarios in order
    scenario_gate_then_resolve.rs        # pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()>
    scenario_approve_always_persists.rs
```

Cargo discovers multi-file integration test binaries via `[[test]]` entries in
`Cargo.toml`:

```toml
[[test]]
name = "reborn_group_approvals"
path = "tests/reborn_group_approvals/main.rs"
```

### `main.rs` boilerplate (required)

```rust
#[allow(dead_code)] #[path = "../support/reborn/mod.rs"] mod reborn_support;
#[allow(dead_code)] #[path = "../support/mod.rs"] mod support;

mod scenario_gate_then_resolve;
mod scenario_approve_always_persists;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::{HarnessResult, RebornIntegrationGroup, ScenarioReport};
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn approvals_group() {
    let g = RebornIntegrationGroup::live_approvals().await.expect("group builds");
    let mut report = ScenarioReport::new();
    // dependent: must pass before subsequent scenarios consume its side-effect
    scenario_gate_then_resolve::run(&g).await.expect("gate+resolve");
    // independent: failure recorded, driver continues
    report.record("approve_always_persists", scenario_approve_always_persists::run(&g).await);
    report.assert_all_passed();
}
```

Both `#[path]` declarations with `#[allow(dead_code)]` are mandatory — bare
`mod support;` resolves relative to the source file, not `tests/`.

### Scenario shape

```rust
// scenario_gate_then_resolve.rs
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g.thread("conv-gate-resolve")
        .script([RebornScriptedReply::tool_call("builtin.write_file", serde_json::json!({}))])
        .build().await?;
    let (run_id, gate_ref) = h.submit_turn_until_blocked("write something").await?;
    h.approve_gate(run_id, &gate_ref).await?;
    h.wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed).await?;
    Ok(())
}
```

### Available constructors

| Constructor | Capability | Auto-approve |
|---|---|---|
| `RebornIntegrationGroup::live_approvals()` | file tools (write_file/read_file @ Ask) | disabled |
| `RebornIntegrationGroup::builtin_tools()` | core built-in (http/echo/time/json/shell) | enabled |
| `RebornIntegrationGroup::extension_lifecycle()` | extension_search/install/activate/remove | enabled |
| `RebornIntegrationGroup::builder().storage(LibSql).live_approvals()` | same + LibSql storage | disabled |

### Key accessors on `RebornIntegrationGroup`

- `turn_composite()` — the thread/turn `CompositeRootFilesystem`; use for
  thread-history and turn-state read-back only.
- `capability_harness()` — `Option<&Arc<HostRuntimeCapabilityHarness>>`; use
  for capability stores (memory, projects, extensions, approval, auto-approve).
  Returns `None` for the Echo backend.

### Per-thread baseline (R2)

Each `RebornIntegrationHarness` records `baseline_invocation_count`,
`baseline_egress_count`, and `baseline_result_count` at construction from the
shared recorder's current lengths. All assertion methods (`assert_tool_invoked`,
`assert_egress_request_matching`, etc.) slice `[baseline..]` so a thread never
spuriously passes on a prior thread's entries.
