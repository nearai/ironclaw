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

### Script discipline — one script entry per model call

The script is a FIFO of model calls, consumed across every turn on the harness.
Count entries by model calls, not by "turns":

- Plain reply turn = **1** entry (`text(..)`).
- Ungated tool-call turn = **2** entries: `tool_call(..)` + the post-execution
  model call (usually `text(..)`).
- Gated tool-call turn (approval OR auth) = **exactly 2 entries**, whether the
  gate is approved or denied: `tool_call(..)` + one post-resume model call
  (reacting to the granted result or the authorization-failure tool error). A
  third entry over-runs the FIFO and silently bleeds into the next turn.

So a two-turn thread where both turns raise and resolve a gate needs 4 entries
(`tool_call, text, tool_call, text`).

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

- `scope_gateway.rs` — `ScopeRegistryGateway`, a `HostManagedModelGateway` dispatcher that routes model calls to per-thread scripted gateways by `TurnScope` (looked up in a `Mutex<HashMap>`). Sits at the `HostManagedModelGateway` seam but routes to REAL `LlmProviderModelGateway` instances over the `ironclaw_llm` chain — the single-fake-at-the-vendor-SDK-seam invariant (CLAUDE.md lines 5–8, 28) is preserved. Its own `stream_model` is a `ConfigurationError` sentinel (never reached when routing succeeds); `resolve_for_scope` does the actual lookup.
- `scripted_provider.rs` — `scripted_trace_llm(..)`, the `TraceLlm` raw-provider seam.
- `reply.rs` — `RebornScriptedReply` (the one-line-per-turn façade).
- `builder.rs` — `RebornIntegrationHarness` + builder, hermetic env, core assertions
  (`assert_reply_contains` / `assert_tool_invoked` / `assert_egress_request_matching`,
  co-located with the harness fields), shell assertions
  (`assert_shell_command_recorded` / `assert_shell_ran_through_inert_port`),
  MCP assertion (`assert_mcp_tool_called`), approval methods
  (`submit_turn_until_blocked` / `approve_gate` / `deny_gate` / `enable_auto_approve`),
  and the `pub(super)` capture accessors (`captured_egress_requests` /
  `captured_capability_results` / `captured_system_prompts`) the assertion file reads.
- `harness_mcp.rs` — the mock-MCP scaffolding extracted from the harness:
  `LoopbackMcpRuntimeHttpEgress` (the real-HTTP loopback egress), the
  `LoopbackMcpRuntime` type alias + `build_loopback_mcp_runtime` factory,
  `mock_mcp_extension_package`, `local_dev_host_runtime_with_registry_egress_and_mcp`,
  and the MCP trust/network policies. `HostRuntimeCapabilityHarness::mock_mcp_tools`
  lives in `harness/profiles/mock_mcp.rs` (part of the `ToolsProfile` split below);
  it delegates the MCP wiring to the `pub(super)` factories in `harness_mcp.rs`.
- `harness/` — the `HostRuntimeCapabilityHarness` split (was a single `harness.rs`):
  `harness/mod.rs` (the central struct, `new_with_options`, and constructors),
  `harness/assembly.rs` (composition/wiring helpers), `harness/options.rs`
  (`HostRuntimeHarnessOptions`, `ToolsProfile` + `build()`), `harness/recorder.rs`
  (`HarnessCapabilityRecorder`, `RecordedCapabilityResult`), and
  `harness/profiles/<domain>.rs` — one file per capability domain (`attachment`,
  `coding_read`, `core_builtin`, `extension`, `file`, `github`, `mock_mcp`,
  `outbound`, `process`, `profile`, `project`, `qa_smoke`, `skill`,
  `trace_commons`, `trigger`, `web_access`) — each returning a `ToolsProfile` via
  a constructor like `profiles::file::file_tools_requiring_approval()` or
  `profiles::core_builtin::core_builtin_tools(CoreBuiltinOptions)`.
- `doubles/` — one file per production-port test double substituted into the
  harness (`RecordingTestCapabilityPort`, `RecordingHostRuntime`,
  `RecordingRuntimeHttpEgress`, `RecordingNetworkHttpEgress`,
  `RecordingApprovalRequestStore`, `RecordingCapabilityResultWriter`,
  `RecordingSecurityAuditSink`, `UnavailableProjectService`,
  `GithubHarnessAuthorizer`, `StaticSecretStore`,
  `StaticCapabilitySurfaceProfileResolver`,
  `FixedRuntimeCredentialAccountResolver`, `EmptyIdentityContextSource`,
  `HarnessCapabilityPortFactory`, `HostRuntimeHarnessCapabilityPortFactory`,
  `ParkingCapabilityGate`/`ParkingHostRuntime` — the tool-path analog of
  `ParkingModelGate`/`ParkingLlm`; parks a `HostRuntime` capability dispatch
  until released, used by `lease_wedge.rs`), re-exported from
  `doubles/mod.rs`.
- `group_constructors.rs` — the per-capability `RebornIntegrationGroup` /
  `RebornIntegrationGroupBuilder` preset constructors (`live_approvals`,
  `builtin_tools`, `extension_lifecycle`, `skill_management_tools`, etc.), a
  private child module of `group.rs` (same `harness_mcp.rs` split precedent)
  so it can reach `group.rs`'s shared assembly internals at module-private
  visibility. See [Group tests](#group-tests) below.
- `process.rs` — `RecordingProcessPort`, the inert process port: records every
  `CommandExecutionRequest.command` and returns exit 0 / empty output without
  spawning any OS process. Injected by default when `with_builtin_http_tools()` is
  used; the `.with_live_shell()` opt-in skips injection so the real
  `HostProcessPort` executes instead.
- `http_matcher.rs` — `ScriptedHttpResponse`, the URL/method/capability-keyed
  HTTP scripting layer over `RecordingRuntimeHttpEgress` (install via
  `.with_keyed_http_responses([..])`).
- `assertions.rs` — the richer egress + tool-result assertions
  (`assert_egress_count` / `assert_egress_url_order` / `assert_egress_method_order`
  / `assert_egress_body_contains` / `assert_tool_result_contains` /
  `assert_tool_error` / `assert_network_egress_header_contains`), plus the
  model-prompt assertion `assert_system_prompt_contains` (reads the scripted
  `TraceLlm`'s captured requests via `captured_system_prompts`, not the egress
  log).
- Tests live as bins anywhere under `tests/integration/` — flat
  (`tests/integration/<name>.rs`) or inside a domain folder — each registered
  as its own `[[test]]` in the workspace `Cargo.toml` with
  `name = "reborn_integration_<name>"`. Domain folders group related bins by
  pointing the `[[test]]` `path` into the folder — `tests/integration/auth/`
  holds the auth user-journey bins (`oauth_connect`, `oauth_popup_journeys`,
  `oauth_refresh`, `auth_gate`, `auth_failure`, `reopen_resume_through_gate`;
  binary names unchanged), with shared fixtures in `auth/common.rs` mounted
  per-bin via `#[path = "common.rs"] mod common;`. Files one level deeper
  mount the support trees as `#[path = "../support/mod.rs"]` and
  `#[path = "../../support/mod.rs"]`. Extension-lifecycle journeys live as
  scenarios under `group_extensions/` (see Group tests below).

Module paths: each flat `tests/integration/<name>.rs` declares both
`#[path = "support/mod.rs"] mod reborn_support;` and
`#[path = "../support/mod.rs"] mod support;` (bins one folder deeper prepend
one more `../` to each, as above), then
`use reborn_support::builder::RebornIntegrationHarness;` /
`use reborn_support::reply::RebornScriptedReply;`. Inside the support tree,
siblings reference each other via `super::` and `trace_llm` via
`crate::support::trace_llm` (the top-level `tests/support/` tree, mounted
separately from `tests/integration/support/` via the second `#[path]`). Copy
the includes from `tests/integration/greeting.rs`.

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
- `assert_egress_body_contains(url_substr, body_substr)` — body of the (first) captured egress request whose URL contains the substring.
- `assert_egress_body_contains_any(url_substr, body_substr)` — body of ANY (not just the first) captured egress request whose URL contains the substring; for a multi-request handshake where every leg shares one URL (e.g. web-access's Exa MCP `initialize`/`notifications/initialized`/`tools/call` sequence) and only one leg's body carries the substring under test. Prefer `assert_egress_body_contains` whenever `url_substr` matches exactly one request.
- `assert_tool_result_contains(needle)` — a recorded capability result's output contains the text (proves the scripted body surfaced back to the model on the *Completed* path; reads the in-process recorder).
- `assert_tool_error(class, reason)` — a persisted `ToolResultReference` envelope's parsed `safe_summary` field is of outcome `class` (`ToolErrorClass::{Failed, Denied}`) and carries `reason`. Distinct from `assert_tool_result_contains`: this reads the *Failed*/*Denied* capability-error path (persisted via `append_tool_result_reference`), not the in-process recorder, so it's the assertion for `egress_error`-scripted responses and other capability failures/denials. `class` is a typed arg (not a needle prefix) so it discriminates Failed-vs-Denied structurally — a `Failed{PolicyDenied}` and a `Denied{policy_denied}` render the same `reason` token but different classes. Parses the `safe_summary` field (not a raw-JSON substring). Scans full thread history (not baseline-sliced) — on multi-turn threads use the `*_since` variant with a `history_len()` baseline (see below).
- `assert_no_tool_error(class, reason)` — the inverse of `assert_tool_error`: passes when NO persisted `ToolResultReference` summary matches `class`'s prefix and contains `reason`, and fails (listing what was found) when one is. Built on the same collector, so it shares the same full-thread-history caveat. Prefer this over pattern-matching `assert_tool_error`'s `Err` string when a test needs to prove absence — matching the negative directly avoids coupling the test to `assert_tool_error`'s diagnostic wording.
- `assert_tool_error_summary_contains(text)` — raw `safe_summary` substring check on a persisted `ToolResultReference`, with NO class-prefix requirement. Use for `CapabilityErrorSummary`s the executor builds via `SanitizedStrategySummary::from_trusted_static` (`crates/ironclaw_agent_loop/src/executor/capabilities.rs`: filtered-surface denial, stale-surface retry, gate-declined short-circuit) — those are fixed host-authored literals with no host-returned text to prefix, so `assert_tool_error`'s `capability_{failed,denied}_summary` prefix match never succeeds for them. Scans full thread history (not baseline-sliced) — on multi-turn threads use the `*_since` variant with a `history_len()` baseline (see below).
- `assert_network_egress_header_contains(url_substr, header_name, value_substr)` — reads the **network** egress lane (`captured_network_requests()`), not the runtime lane the four assertions above read. Needed for `.with_github_issue_tools()`: that harness's `try_with_host_http_egress` overwrites the runtime port with the host egress pipeline over the network recorder, so the runtime-lane `assert_egress_*` family is inert for it — assert here instead.
- `tool_result_output(capability_id)` — the parsed JSON output of the most-recent recorded capability result for that id, for reading server-minted fields (e.g. `trigger_id`) a static script can't reference ahead of time.

Multi-turn (baseline-sliced) variants: the `assert_tool_error*` and
`assert_conversation_history_*` families scan the FULL thread history — safe
only on a single-turn harness. On multi-turn threads, capture
`history_len()` at the START of the turn under test and pass it as `baseline`
to the `*_since` forms (`assert_tool_error_since`, `assert_no_tool_error_since`,
`assert_tool_error_summary_contains_since`,
`assert_conversation_history_contains_since`) so only that turn's appended
messages are considered. `assert_conversation_history_contains(needle)` checks
persisted-transcript containment across ANY role (the stored-history analogue
of `assert_system_prompt_contains`, which reads only System-role model
requests); `assert_conversation_history_role_contains(kind, needle)` restricts
it to one `MessageKind`. Contracts (class discrimination, fail-loud decode)
match the full-history siblings — see their doc-comments. Demo + regression:
`tests/integration/http_matcher.rs::multi_turn_baseline_sliced_history_assertions`.

### Keyed HTTP responses

For multi-step tool flows where each `builtin.http` call to a different URL
needs a different scripted body, install keyed responses via
`.with_keyed_http_responses([..])`. This implies `.with_builtin_http_tools()`.

`ScriptedHttpResponse` — first-match-wins in scripted order:

- `ScriptedHttpResponse::for_url(url_substr, body)` — matches any request whose URL contains the substring; defaults to a `200` body.
- `.with_method(method)` — narrow to a specific HTTP method (lowercase, e.g. `"post"`).
- `.with_capability(capability_id)` — narrow to a specific capability id (e.g. `"builtin.http"`).
- `.with_status(status)` — override the status of a `for_url` body response (e.g. `404`, `500`). Still a successful egress call — `builtin.http` surfaces it as a Completed tool result carrying that status. Panics if called on an `egress_error` response (mutually exclusive outcomes).
- `ScriptedHttpResponse::egress_error(url_substr, error)` — scripts a runtime egress failure (`Err(RuntimeHttpEgressError)` from `execute`) instead of a body, driving `builtin.http`'s error-mapping paths (e.g. `policy_denied` → `Denied`, `response_body_limit_exceeded` → `Failed{OutputTooLarge}`). Prefer the two named wrappers below so test bodies select the scenario by name instead of hand-building the nested error struct:
  - `ScriptedHttpResponse::network_error(url_substr, reason)` — a `RuntimeHttpEgressError::Network` with `reason` (e.g. `"policy_denied"` → `Denied`).
  - `ScriptedHttpResponse::response_error(url_substr, reason)` — a `RuntimeHttpEgressError::Response` with `reason` (e.g. `RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED` → `Failed{OutputTooLarge}`).

Requests that match no scripted response fall back to the recording egress default body. The keyed matcher is the canonical HTTP-scripting API; new per-URL response needs fold into `ScriptedHttpResponse` rather than adding a parallel matcher.

### Shell / process

When `.with_builtin_http_tools()` is active, `builtin.shell` turns are dispatched
through the inert `RecordingProcessPort` by default. It records every
`CommandExecutionRequest.command` string and returns exit 0 / empty output without
spawning any OS process.

- `assert_shell_command_recorded(substr)` — the recorded command string contains `substr`.
- `assert_shell_ran_through_inert_port()` — at least one shell command was recorded by the inert port (proves no real OS process ran).

**`.with_live_shell()`** — opt-in; skips recording-port injection so the real
`HostProcessPort` executes instead. Use only for hermetic commands
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

### Credential injection (GitHub)

`.with_github_issue_tools()` wires the real GitHub first-party WASM
capabilities behind a `GithubHarnessAuthorizer`, which authorizes every
dispatch with an `InjectCredentialAccountOnce` obligation. A scripted
`github.*` tool call executes the real WASM module; its outbound HTTP
request gets a synthetic `Authorization: Bearer <token>` credential
injected by the host egress pipeline before it reaches the recording
network egress. This is the credential-injection-reaches-the-wire proof
(T0-SECRET-INJECT).

Script with `RebornScriptedReply::tool_call("github.get_repo", json!({"owner": ..., "repo": ...}))`
followed by a trailing `RebornScriptedReply::text(..)` turn.

- `assert_network_egress_header_contains(url_substr, header_name, value_substr)` — see the "Richer assertions" list below; this is the assertion for this capability.

### Turn lifecycle events

`.with_turn_event_sink()` (harness or group builder) installs an in-memory
`ironclaw_turns::InMemoryTurnEventSink` into the ONE planned runtime via the
production `lifecycle_bus.subscribe_best_effort` seam — the entry point Trace
Commons capture and skill learning hang off in production. Off by default
(`turn_event_sink: None`, matching every pre-existing test).

- `assert_turn_event_recorded(kind)` — at least one recorded `TurnLifecycleEvent`
  of that `TurnEventKind` (e.g. `Completed`). The sink is group-shared, but the
  harness records `baseline_turn_event_count` at construction and
  `recorded_turn_events` slices `[baseline..]` (R2), so each thread only sees
  events its own turns published — a sibling thread's earlier event can't make
  this assertion pass.

### Attachments (multimodal)

`RebornIntegrationGroup::attachment_tools()` wires the real production
attachment pair over the local-dev workspace filesystem:
`ProjectScopedAttachmentLander` (lands inbound bytes) +
`ProjectScopedAttachmentReader` (the `attachment_read_port` the loop model port
reads bytes back through). Route the thread at a vision-capable model id with
`.with_model_override(model_id)` (see `ironclaw_llm::vision_models`) — the
scripted default id is not a vision pattern, so image parts are dropped for it.

- `submit_turn_with_image_attachment(text, filename, mime, bytes)` — lands the
  image through the real `submit_inbound_with_attachments` entry point and waits
  for completion. Errors if the harness has no lander (i.e. not an
  `attachment_tools()` group).
- `assert_model_saw_image_attachment(mime, bytes)` — a captured model request
  carried a `ContentPart::ImageUrl` part whose `data:` URL is exactly
  `data:<mime>;base64,<encode(bytes)>` — byte-fidelity through lander →
  filesystem → read port, not just the textual `<attachments>` pointer.

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
the default backend build.

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

`HostRuntimeCapabilityHarness` (in `harness/mod.rs`, gated on `#[cfg(feature = "test-support")]`) exposes:

- `extension_installation_store_for_test()` — returns the `Option<Arc<dyn ExtensionInstallationStore>>` wired into the local-dev extension management port; mirrors the production installation store for test read-back assertions. Returns `None` when the local runtime has no extension management wired.

`ironclaw_reborn_composition::test_support` exposes:

- `build_secret_store_for_test(root, scoped)` — constructs the `LocalDevSecretStore` used by production local-dev composition; for store read-back in secrets tests.

`RebornRuntime` (returned by `build_runtime`, methods defined in `crates/ironclaw_reborn_composition/src/runtime/test_support.rs`) exposes:

- `local_dev_approval_interaction_service_for_test(turn_coordinator)` — real `DefaultApprovalInteractionService` wired like `build_reborn_runtime`. `None` without a local-dev runtime.
- `local_dev_auth_interaction_service_for_test(turn_coordinator)` — WebUI auth-interaction service via the same `build_webui_auth_interaction_service` helper production uses. `None` only without a local-dev runtime; falls back to `UnavailableAuthInteractionService` when `product_auth` has no flow-record source.

Both take `turn_coordinator: Arc<dyn TurnCoordinator>` explicitly rather than reading `self.turn_coordinator` — a `RebornRuntime` from `build_runtime` alone carries a different coordinator instance than a caller-built planned runtime; pass the coordinator your harness's turns actually run against.

All of the above are zero-byte in production builds (gated on the `test-support` feature).

## Group tests

`RebornIntegrationGroup` (in `group.rs`) owns shared storage, a shared
capability backend, and one shared turn runtime (coordinator + scheduler) once;
each `.thread(conv_id)` builds a per-thread workflow over that one shared
runtime. Cross-thread persistence is real — thread A
writes, thread B sees it. Single-shot `test_default()` is a degenerate
one-thread group (its own storage, baseline = 0); all existing tests are
byte-identical after this refactor.

### When to use a group (vs a flat test)

Use a group when the scenario needs **multiple threads over shared state or
the shared runtime** — either:
- **cross-thread persistence** — thread A submits a tool call that raises an
  approval gate; thread B resolves the gate; thread A resumes; or
- **shared-coordinator/runtime behavior** — two threads parked on gates at the
  same time, resolved independently by `run_id` (see
  `scenario_concurrent_dual_gate_resume`), which only the one-shared-runtime
  model can exercise.

A scenario that submits + asserts in one thread belongs in a flat
`tests/integration/<name>.rs` test as always.

### Group test binary layout

Group tests live in subdirectories under `tests/integration/`:

```
tests/integration/group_approvals/
    main.rs                              # one #[tokio::test], drives scenarios in order
    scenario_gate_then_resolve.rs        # pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()>
    scenario_approve_always_persists.rs
```

Cargo discovers multi-file integration test binaries via `[[test]]` entries in
`Cargo.toml`:

```toml
[[test]]
name = "reborn_group_approvals"
path = "tests/integration/group_approvals/main.rs"
```

### `main.rs` boilerplate (required)

```rust
#[allow(dead_code)] #[path = "../support/mod.rs"] mod reborn_support;
#[allow(dead_code)] #[path = "../../support/mod.rs"] mod support;

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
| `RebornIntegrationGroup::extension_lifecycle()` | extension_search/install/remove | enabled |
| `RebornIntegrationGroup::triggers()` | trigger_create/list/pause/resume/remove | enabled |
| `RebornIntegrationGroup::skill_management_tools()` | skill_list/skill_install/skill_remove | enabled |
| `RebornIntegrationGroup::attachment_tools()` | attachment lander + read port (no tool dispatch) | n/a (no capability dispatch) |
| `RebornIntegrationGroup::live_auth_and_approval()` | file tools @ Ask + unseeded `github.get_repo` (raises BlockedApproval AND BlockedAuth on one runtime; C-JOURNEY) | disabled |
| `RebornIntegrationGroup::builder().storage(LibSql).live_approvals()` | same + LibSql storage | disabled |
| `RebornIntegrationGroup::multiuser_memory_tools()` | core built-in (memory/http/shell/…) with per-actor run-owner-scoped capability dispatch (C-MULTIUSER) | enabled |
| `RebornIntegrationGroup::multiuser_approvals()` | file tools (write_file/read_file @ Ask) with per-actor capability scoping (C-MULTIUSER) | enabled per owner (default; toggle per-owner in test) |
| `RebornIntegrationGroup::outbound_target_tools()` | `outbound_delivery_targets_list`/`target_set` over an injected `FakeOutboundPreferencesFacade` (C-SYNTH) | enabled |

### Auth-gate resolution (C-JOURNEY)

On a `live_auth_and_approval()` group thread:

- `resolve_auth_gate(run_id, &gate_ref)` — the "user submitted credentials"
  happy arm: seeds a real GitHub credential account WITH secret material
  through the production manual-token flow
  (`request_manual_token_setup` -> `submit_manual_token`), then resumes with
  `ResumeTurnPrecondition::BlockedAuthGate`; the parked `github.*` capability
  re-dispatches and completes. Only valid on this group (needs the
  `build_reborn_services` product-auth wiring; `live_auth_gate()`'s
  lower-level fixture cannot complete an auth resume).
- `deny_auth_gate(run_id, &gate_ref)` — works on both auth-gate groups.

Multi-turn journey chaining (gate -> resolve -> next turn on ONE
conversation) is pinned by `tests/integration/group_journeys/`; per-turn resume
idempotency keys are `(run_id, gate_ref)`-scoped so one run can resume
through an approval gate and a subsequent auth gate without replay collision.

### Distinct actors per thread (E-MULTIUSER)

`g.thread(conv).with_actor_id("some-actor")` resolves that thread's binding
under a DISTINCT actor instead of the default `HARNESS_ACTOR_ID` — both at
the build-time binding probe and at every `submit_turn` for that thread, so
probe and submit always resolve the same binding/owner. The group's one
shared runtime resolves each turn's thread by the run's own owner
(production's `ThreadScopeResolver::resolve_for_turn` over the harness's
per-op `/threads` mount), so two actors' threads coexist over one
coordinator with their history isolated under separate
`/tenants/<tenant>/users/<user>/threads` subtrees. Driving test:
`tests/integration/group_multiuser/`.

### Key accessors on `RebornIntegrationGroup`

- `turn_composite()` — the thread/turn `CompositeRootFilesystem`; use for
  thread-history and turn-state read-back only.
- `capability_harness()` — `Option<&Arc<HostRuntimeCapabilityHarness>>`; use
  for capability stores (memory, projects, extensions, approval, auto-approve).
  Returns `None` for the Echo backend.

### Per-thread baseline (R2)

Each `RebornIntegrationHarness` records `baseline_invocation_count`,
`baseline_egress_count`, `baseline_result_count`, `baseline_process_count`, and
`baseline_network_count` at construction from the shared recorder's current
lengths. All assertion methods (`assert_tool_invoked`,
`assert_egress_request_matching`, `assert_network_egress_header_contains`,
etc.) slice `[baseline..]` so a thread never spuriously passes on a prior
thread's entries.
