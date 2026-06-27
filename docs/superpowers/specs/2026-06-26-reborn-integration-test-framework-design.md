# Reborn Integration-Test Framework — Design

**Date:** 2026-06-26
**Status:** Design (pre-implementation) — revised after multi-lens review (approach / local-patterns / maintainability / thermo-nuclear)
**Scope:** Extend the existing Reborn test harness (`tests/support/reborn/`) so integration tests run the full internal stack against a real SQL persistence backend while intercepting the model at the vendor-SDK seam — with test bodies that stay short (~3–12 lines).

---

## 1. Goal & Motivation

Adopt the hermes-agent test philosophy for the Reborn stack: **run all internal logic for real; mock only the external edges (model vendor, inbound payload); use a real-but-ephemeral database.** Investigation of hermes-agent (see `docs/plans/2026-06-26-hermes-agent-test-ci-replication.md`) confirmed that pattern, and that the bulk of their tests stay 3–12 lines because all ceremony lives in fixtures + factory helpers with safe defaults.

IronClaw already has the building blocks, but the current Reborn harness has two gaps relative to this goal:

1. **LLM is mocked too high.** `RebornTraceReplayModelGateway` swaps the entire `HostManagedModelGateway`, bypassing *all* of `ironclaw_llm` (model-profile resolution, `CompletionRequest` build, retry/routing/circuit-breaker/safety decorators, and rig-core request shaping). Tests therefore never exercise the provider chain.
2. **No real SQL persistence.** The harness hardcodes `LocalFilesystem`(TempDir) for product/thread state and `InMemoryBackend` for turn state. The production `RootFilesystem` SQL backends (`LibSqlRootFilesystem`, `PostgresRootFilesystem`) are never touched in tests.

This design closes both gaps **without** increasing per-test verbosity.

### Language: Rust only

These are **in-process Rust** tests in `tests/reborn_*.rs`. The two requirements — intercepting beneath the `ironclaw_llm` decorator chain, and constructing a real `LibSqlRootFilesystem` on tmp — are compiled-Rust seams unreachable from Python or TypeScript. Python/TS would force a black-box HTTP boundary, putting the mock back at the network edge (the opposite of "mock at the very end"). The existing Python (`tests/e2e/` Playwright) and TS (`webui_v2` `node --test`) suites cover the **browser/frontend** layer and are out of scope here.

### Non-goals

- Not touching the v1 `TestRig` stack (`tests/support/test_rig.rs`). Reborn-only.
- Not implementing the Postgres test lane now (no reserved enum variant — added when the CI container lane lands).
- Not migrating existing `RebornBinaryE2EHarness`-based tests. They stay; this adds a new integration tier alongside them.
- Not exercising real channel-adapter parsing (Slack/Telegram HMAC verification, event dispatch). Inbound is a synthetic envelope (§3.6); real `slack_v2_adapter`/`telegram_v2_adapter` parse paths stay covered by their own adapter unit tests.
- Not a coverage plan. This is the framework that *enables* a later, overlap-minimized coverage effort.

---

## 2. Key Decisions (locked)

| Axis | Decision |
|---|---|
| Target stack | **Reborn only** (`tests/support/reborn/`). |
| LLM seam | **One seam: raw-provider stub beneath the real decorator chain**, routed through a real `LlmProviderModelGateway`. No `GatewaySwap` mode in the new builder — orchestration-only tests keep using the existing `RebornBinaryE2EHarness::with_model_gateway` constructors. |
| DB backends | **InMemory (default) + libSQL-on-tmp.** No Postgres yet (added later as a one-line enum case + CI lane). |
| Param mechanism | **Add `rstest` dev-dependency** for named `#[case]` backend parametrization and fixture injection. **No proc-macro.** |
| Hermetic setup | Baked **unconditionally into `build()`** (keychain disable, `TZ=UTC`, tmp dirs, **`LLM_MAX_RETRIES=0`**, **and unset `NEARAI_CHEAP_MODEL` / `NEARAI_FALLBACK_MODEL` / `LLM_CIRCUIT_BREAKER_THRESHOLD` / `LLM_RESPONSE_CACHE_ENABLED`** — so all five decorators (`retry`, `routing`, `failover`, `circuit`, `cache`) become passthrough around the single scripted raw provider and no live vendor sub-provider is built), so every test form (`#[tokio::test]`, `#[rstest]`) inherits it. |
| Verbosity | Per-test scripting is **terse**; all wiring lives once in the harness builder. |
| Local runs | **Any developer runs the full default suite with one plain command, zero setup** — see §4.3. No services, no API keys, no `integration` feature, no Docker, no special linker. |
| External edges | **Every network/IO boundary is captured or inert by default** (§3.6) — LLM, tool HTTP, channel in/out, secrets, embeddings, Trace Commons, and shell/process (clock/wall-time excepted — runs live). A default test reaches no network, no real OS process, no real channel. Live variants (HTTP, shell) are explicit per-test opt-ins. |

---

## 3. Architecture

### 3.1 The single LLM seam

```
Reborn agent loop
   │
   ▼
HostManagedModelGateway
   │  ── real LlmProviderModelGateway (profile resolve, CompletionRequest build, tool-def assembly)
   ▼
LlmProvider decorator chain          built by the NEW extracted apply_decorator_chain():
   │  (retry → routing → circuit → safety → cache)   runs for real
   ▼
raw provider (rig-core "SDK")        ◄── scripted fake injected HERE
   │                                      (TraceLlm fed an in-memory LlmTrace,
   ▼                                       built by the RebornScriptedReply façade)
vendor HTTP                          (never reached)
```

IronClaw's vendor SDK is the rig-core `Client` inside each `RigAdapter`. The faithful interception point that still runs every internal layer is **the raw provider at the bottom of the decorator chain**. Profile policy, retry, routing, circuit-breaker, safety sanitization, and `CompletionRequest`/tool-definition assembly all execute; only the vendor call returns scripted output.

**Why a single seam (no `GatewaySwap`):** the chain above the raw provider is a handful of in-memory state checks — its cost is negligible, so a "fast gateway-swap mode" is not justified, and keeping it would (a) contradict the locked "mock at the SDK seam" requirement and (b) let any chain bug be silently bypassed. Orchestration-only tests that genuinely want to skip `ironclaw_llm` already have a home: the existing `RebornBinaryE2EHarness::with_model_gateway` static constructors, which are untouched by this work.

### 3.2 Storage seam

```
CompositeRootFilesystem (control-plane: turn / thread / product state)
   ├─ StorageMode::InMemory  → InMemoryBackend                 [default, fast]
   └─ StorageMode::LibSql    → LibSqlRootFilesystem (tmp .db)  [real SQL + migrations]
```

The existing harness wires **three** backends separately (`product`/`thread` on `LocalFilesystem`(TempDir), `turn` on `InMemoryBackend`). `StorageMode::LibSql` constructs **one** `LibSqlRootFilesystem` over a tmp `.db` and mounts it across the control-plane paths of the composite, **reusing the production mount helper** (`mount_local_dev_database_roots` in `crates/ironclaw_reborn_composition/src/factory.rs`, the same call the libSQL local-dev boot path uses) rather than hand-rolling the mount wiring. This avoids a second copy of the mount truth.

**Visibility prerequisite:** `mount_local_dev_database_roots` is currently private to `factory.rs`, so the cross-crate test builder cannot call it as-is. The reuse requires promoting it (or a thin wrapper) to `pub`/`pub(crate)`-plus-a-`#[cfg(any(test, feature = "test-support"))]` accessor following the `ironclaw_reborn_composition` CLAUDE.md test-support pattern. This promotion is part of Step 4 scope — without it the only alternatives are duplicating the mount wiring (the exact second-copy this design rejects) or an ungoverned visibility change.

### 3.3 New components (each in its OWN file — `harness.rs` is already 3,755 lines)

| Component | File | Responsibility |
|---|---|---|
| scripted raw provider | `tests/support/reborn/scripted_provider.rs` | Bottom-of-chain provider. **Reuse `TraceLlm`'s replay engine** (`tests/support/trace_llm.rs`, already an `LlmProvider` with sequential step replay + tool-call steps + template substitution). Decision locked — see §3.3.1. The façade builds an in-memory `LlmTrace` and hands a `TraceLlm` to the chain; no new replay provider is written. |
| `RebornScriptedReply` constructors | `tests/support/reborn/reply.rs` | `RebornScriptedReply::text(s)` and `RebornScriptedReply::tool_call(capability_id, json)` — exactly two, each mapping 1:1 to one `TraceStep`. (No `tool_call_then_text`: `TraceResponse::ToolCalls` hardcodes `content: None` in the engine, so a combined step would force a `TraceLlm` modification; "tool call then reply" is already two clean array entries.) Produce provider-level scripted steps. **Distinct from** the existing gateway-level `RebornModelReplayStep` DSL — a new, narrower vocabulary for the new tier, not a shared one. |
| `StorageMode` | `tests/support/reborn/builder.rs` | `InMemory` \| `LibSql`. Selects the control-plane backend. |
| `RebornIntegrationHarness` + `::builder()` | `tests/support/reborn/builder.rs` | New integration tier. Single entry point; defaults absorb ceremony. Named to avoid collision with the existing `RebornBinaryE2EHarness` / `RebornHarnessSharedStorage`. |
| `assert_*` helpers | `tests/support/reborn/assertions.rs` | `assert_reply_contains`, `assert_capability_denied`, `assert_capability_order` over the existing `HarnessCapabilityRecorder` + milestone sink. |

### 3.3.1 Why reuse `TraceLlm`'s engine (decision, not an open question)

`TraceLlm` (`tests/support/trace_llm.rs`, impl `LlmProvider`) was built to replay *recorded* JSON traces (`RecordingLlm` → JSON → `from_file`) for the v1 replay gate — that recorded-fixture path is the majority (113 JSON fixtures under `tests/fixtures/llm_traces/`). But `TraceLlm` also accepts **hand-built in-memory** traces via `LlmTrace::new` + `TraceLlm::from_trace`, and 7 v1-stack test files use exactly that in-memory pattern (e.g. `e2e_builtin_tool_coverage.rs`, `e2e_response_order.rs`, `multi_tenant_system_prompt.rs`). So in-memory scripting through `TraceLlm`'s engine is a supported, exercised path — reusing it for Reborn is not a repurposing. The recorded-only fields (`memory_snapshot`, `http_exchanges`) default empty and are ignored by in-memory builders.

(Note: the Reborn tier has **no** prior `TraceLlm` usage — this design introduces it at the raw-provider seam. The existing `reborn_qa_recorded_behavior.rs` uses `RebornTraceReplayModelGateway` at the *gateway* seam, which is the higher-level mock this design deliberately moves below.)

Reusing the engine avoids reimplementing sequential replay, tool-call steps, hint-based scanning (concurrent threads), and template substitution. `StubLlm` was rejected: it returns one fixed string with `tool_calls: Vec::new()` and can never script a tool-call → text sequence.

**But the raw construction API is the verbosity we are escaping.** A current hand-built trace costs ~44 lines for "two tool calls then a text reply" (`e2e_builtin_tool_coverage.rs:1131`): nested `TraceStep { request_hint: None, response: TraceResponse::ToolCalls { tool_calls: vec![TraceToolCall { id, name, arguments }], input_tokens, output_tokens }, expected_tool_results: Vec::new() }` per step. That is exactly what new tests must not look like. The `RebornScriptedReply` façade exists to collapse each of those steps to one line, auto-filling `id`, token counts, `request_hint: None`, and `expected_tool_results: []`. The façade is therefore **mandatory**, not polish — it is the mechanism that delivers the readability contract in §4.2.

### 3.4 Tool-call name contract (must be explicit)

`RebornScriptedReply::tool_call(capability_id, json)` accepts a **CapabilityId-format** name (e.g. `"builtin.file_read"`). At the raw-provider seam the scripted step is realized as a `ToolCall` whose `name` is in **ProviderToolName format** (`"builtin__file_read"`): the provider resolves the matching `ToolDefinition` from the incoming `ToolCompletionRequest.tools` list via the reversible `'.' → "__"` mapping, then emits the provider-format name `LlmProviderModelGateway` expects (`provider_tool_call_from_llm` → `ProviderToolName::new`). Without this, every `tool_call` step would silently mis-match. The conversion lives in one place (the scripted provider), and `RebornScriptedReply`'s accepted format is documented at the constructor.

### 3.5 Reused unchanged

`RebornTestIngress` / `RebornTestProductAdapter` synthetic inbound; `HarnessCapabilityRecorder`; `InMemoryLoopHostMilestoneSink`; `TurnRunScheduler` wiring; production `LlmProviderModelGateway`; production `LibSqlRootFilesystem` + migrations; the `factory.rs` mount helper.

### 3.6 External-boundary capture matrix

The LLM is one egress; a real turn crosses several. The contract is **every network/IO boundary is captured or inert by default** — a default-built test reaches no network, no real process, no real channel. (Clock/wall-time is *not* mocked: turn timestamps and scheduler timing run on real time, so assert on behavior, not on timing windows.) The harness builder wires each port below; most reuse recording ports the Reborn harness already has.

| Boundary | Seam (real) | Default in harness | Captured via / opt-in |
|---|---|---|---|
| LLM / model | `HostManagedModelGateway` → `LlmProvider` | scripted | `TraceLlm` at raw-provider seam (§3.1) |
| DB / state | `RootFilesystem` | InMemory; `.storage(LibSql)` | §3.2 |
| Channel inbound | `ProductAdapter::parse_inbound` | **synthetic** `RebornTestProductAdapter` (auth bypassed, no HMAC/parse) | real `slack_v2_adapter`/`telegram_v2_adapter` parsing is **out of scope** — covered by separate adapter unit tests (§1 non-goals) |
| Channel outbound / delivery | `OutboundDeliverySink` / `ProtocolHttpEgress` | `RecordingOutboundDeliverySink` (records `DeliveryStatus`, no HTTP) | `tests/support/reborn/delivery.rs`; asserted via `assert_delivered`/milestones |
| Tool HTTP egress | `RuntimeHttpEgress` (first-party) + `NetworkHttpEgress` (WASM / network-policy) | `RecordingRuntimeHttpEgress` (`harness.rs:3034`) + `RecordingNetworkHttpEgress` (`harness.rs:3091`) — scripted body, no network; builder wires **both** (different tool-call paths) | live HTTP only via explicit `with_live_http_egress` |
| Shell / process | `RuntimeProcessPort` | **inert `RecordingProcessPort`** (scripted output, no real process) — **NEW, see below** | real shell only via explicit `.with_live_shell()` |
| Secrets / OAuth | `SecretStore` | `StaticSecretStore` (fixture handles); refresh worker not spawned | `.with_secret(handle, value)` to seed |
| Approval gates | approval store | in-memory auto-approve | `.deny_capability` / explicit gate resolution |
| Embeddings | `EmbeddingProvider` | none wired; `InMemoryBackend` linear-scan | **caveat:** semantic-ordering assertions are unreliable — assert membership, not vector rank |
| Trace Commons (telemetry) | `ContributionHttpSink` (`ironclaw_reborn_traces`) | **already captured** — the agent path (`HostEgressContributionSink`) routes through `RuntimeHttpEgress`, so `RecordingRuntimeHttpEgress` records it; no new sink needed | assert by filtering `recorded_egress.requests()` by the contribution URL |
| MCP servers | MCP client | not wired by default | opt-in: `.with_mock_mcp(server)` wiring the existing `MockMcpServer` — **P1 ergonomics** |

**NEW — inert process port (locked, safety requirement).** Today `HostRuntimeServices::new()` hardcodes `process_port: Arc::new(LocalHostProcessPort::new())` (`crates/ironclaw_host_runtime/src/services.rs:329`), which runs **real OS processes** via `tokio::process::Command`. A scripted model `tool_call("builtin.shell", …)` would execute on the developer's machine — the exact incident class that motivated hermes-agent's live-system guard. The harness default must be a `RecordingProcessPort` (impl `RuntimeProcessPort`, in `tests/support/reborn/process.rs`) that returns scripted/empty output and records the attempted command; **no real process ever runs by default**. Real shell is explicit per-test opt-in via `.with_live_shell()`. This is required by the zero-setup local-run guarantee (§4.3): a default test can never mutate the dev machine.

Injection seam: `RebornBuildInput` → `build_local_runtime` does not expose a process-port override today (`apply_runtime_process_binding` handles only `TenantSandbox`/`None`). Wiring the recording port needs a small `#[cfg(any(test, feature = "test-support"))]` accessor in `ironclaw_reborn_composition::factory` accepting an injectable `Arc<dyn RuntimeProcessPort>` — the same visibility-promotion pattern Step 4 uses for `mount_local_dev_database_roots`. Scoped into Step 5b.

**Trace Commons — no new type.** `ironclaw_reborn_traces::ContributionHttpSink` looks like a separate egress, but the agent-invoked path (`HostEgressContributionSink`, `trace_commons.rs`) delegates to `RuntimeHttpEgress` — so it is *already* captured by the default `RecordingRuntimeHttpEgress`. A test asserts contribution behavior by filtering `recorded_egress.requests()` for the contribution URL. (The CLI/background-worker path uses a direct client, but that worker is not spawned in the harness — §3.6 "Secrets/OAuth" note.) No `RecordingContributionSink` is built.

**P1 ergonomics (not blockers):** a URL-keyed scripting layer over `RecordingRuntimeHttpEgress` for multi-step tool-HTTP flows, and a `.with_mock_mcp(...)` constructor wiring the existing `MockMcpServer` into the Reborn `ExtensionRegistry`. Both are additive; the default capture matrix above is the required floor.

### 3.7 Interception model & extending the framework

There is **no single outbound chokepoint** — production has four distinct seam families, so the harness uses a **two-tier** interception model (matching the industry split: trait-level fakes for orchestration logic, request-matcher mocks for HTTP adapters):

- **Tier 1 — trait-level fakes** (for logic that runs *above* the call): `LlmProvider` (scripted `TraceLlm`, §3.1), `EmbeddingProvider` (fake), `RuntimeProcessPort` (inert `RecordingProcessPort`), and **channel delivery** (`OutboundDeliverySink` — captured by `RecordingOutboundDeliverySink` at its own trait boundary, *not* through `RuntimeHttpEgress`; see §3.6). LLM/embeddings hold their own `reqwest` clients and are deliberately **not** mocked at HTTP — mocking at the provider trait runs the real chain/loop (the "FakeChatModel" lesson: HTTP-level mocks silently pass when the SDK/loop layer above them changes).
- **Tier 2 — recording interceptor over the HTTP-egress family**: MCP, OAuth, OAuth-refresh, first-party HTTP tools (and Trace Commons) route through the single `RuntimeHttpEgress::execute(RuntimeHttpEgressRequest{ runtime, capability_id, url, method, … })` trait; WASM/network-policy tool calls go through the sibling `NetworkHttpEgress`. The harness wires both recorders (`RecordingRuntimeHttpEgress` + `RecordingNetworkHttpEgress`) — see §3.6 for the authoritative per-boundary list. Today they record a scripted FIFO body; the P1 ergonomics extension (§3.6) is a URL/`capability_id`-keyed matcher over the same recorders.

**Rejected: a single HTTP interceptor for *everything*.** Routing LLM/embeddings/process through one HTTP layer would rip provider auth/retry/circuit out of `ironclaw_llm`, force non-HTTP boundaries (process, future CLI) into an HTTP shape, lose type safety (match on serialized bodies), and reintroduce the per-provider HTTP fixtures §3.1 rejects. It contradicts the locked SDK-seam requirement.

**Extending — adding a new egress point (the key extensibility property):**

1. **New egress behind an *existing* trait** — a new tool, OAuth provider, MCP server, or future CLI-over-HTTP — is a new `RuntimeKind`/`capability_id` value flowing through `RuntimeHttpEgress`, which the one interceptor **already** records and can match. **Framework change: none.** This is the common case and the design's main payoff: production already funnels the HTTP family through one trait, and the harness rides it.
2. **A genuinely new *kind* of I/O (a new port trait)** — write a concrete recording struct for that trait, mirroring the existing `Recording*` types (`RecordingRuntimeHttpEgress`, `RecordingNetworkHttpEgress`, `RecordingOutboundDeliverySink`): a small struct holding scripted responses + a captured-calls `Vec`, implementing the production trait. These single-method request→response ports are ~25–35 lines each and need no new framework support.

**No shared generic is built now.** The existing `Recording*` structs are deliberately concrete. *If* a future port produces a third near-identical recorder, extract a shared generic (`Recording<P>` with `type Req`/`type Resp` + a `respond(&req)` match-and-record core) from the concrete code that then exists — a genuine rule-of-three lift from real duplication, not a speculative type written ahead of need. Write each new recorder in that extractable shape (scripted-responses + captured-calls, no bespoke control flow) so the eventual lift is mechanical. No derive/attribute macro unless the port count grows far enough to justify a proc-macro crate.

---

## 4. Test-Authoring Ergonomics (the anti-verbosity contract)

Mapped from hermes's playbook (their bodies are 3–12 lines):

1. **Builder defaults absorb invariant setup + hermeticity.** `RebornIntegrationHarness::builder()` / `::test_default()` default to `StorageMode::InMemory`, echo capability port, auto `conversation_id`, and apply hermetic env unconditionally in `build()` (`LLM_MAX_RETRIES=0` + unsetting cheap/fallback/circuit/cache — see §2 — so the decorator chain is genuinely passthrough and no real sub-provider is constructed beneath the scripted raw provider; note this is necessary for error-path tests, where a non-zero retry count would re-invoke the exhausted script three times before propagating).
2. **Free constructors with safe defaults**, never hand-built structs: `RebornScriptedReply::text("hi")`, `RebornScriptedReply::tool_call("builtin.file_read", json!({"path":"/x"}))`.
3. **Script set at builder time** (`.script(conv, [...])` on the builder) — immutable after `build()`, matching the existing harness's construction-time queue. No post-build mutation, no `Mutex`.
4. **rstest parametrization** cascades the backend matrix with named cases; plain `#[tokio::test]` for single-backend tests. No proc-macro.
5. **Recorder-backed one-liner assertions** replace manual introspection.

### 4.1 Canonical templates

Minimal single-backend test (the floor — 3 meaningful lines):

```rust
#[tokio::test]
async fn replies_to_greeting() {
    let h = RebornIntegrationHarness::test_default()      // InMemory, hermetic, real chain
        .script(CONV, [RebornScriptedReply::text("done")])
        .build().await;
    h.submit_turn(CONV, "do something").await;
    h.assert_reply_contains(CONV, "done").await;
}
```

Backend-matrix test (real libSQL + real provider chain, mocked SDK):

```rust
#[rstest]
#[case(StorageMode::InMemory)]
#[case(StorageMode::LibSql)]
#[tokio::test]
async fn refuses_when_tool_denied(#[case] storage: StorageMode) {
    let h = RebornIntegrationHarness::builder()
        .storage(storage)                                  // real SQL on tmp when LibSql
        .deny_capability("builtin.file_read")
        .script(CONV, [
            RebornScriptedReply::tool_call("builtin.file_read", json!({"path": "/etc/passwd"})),
            RebornScriptedReply::text("ok, stopping"),
        ])
        .build().await;
    h.submit_turn(CONV, "read /etc/passwd").await;
    h.assert_capability_denied("builtin.file_read").await;
    h.assert_completed(CONV).await;
}
```

No HTTP fixtures, no manual chain wiring, no per-backend copy-paste, no magic attribute.

### 4.2 Readability contract (enforced, not aspirational)

Every new `tests/reborn_*.rs` integration test must be readable top-to-bottom by a human or an LLM with **zero harness knowledge**. The rules:

1. **One scripted model turn = one line** — `RebornScriptedReply::text(...)` or `RebornScriptedReply::tool_call(capability_id, args)`. Nothing else expresses model output.
2. **Raw `LlmTrace::new` / `TraceTurn` / `TraceStep` / `TraceResponse` / `TraceToolCall` construction is forbidden in new Reborn integration tests.** That nested-struct form (≈44 lines for 3 steps, per `e2e_builtin_tool_coverage.rs:1131`) is exactly what `RebornScriptedReply` replaces. Existing v1 tests keep theirs; this work does not migrate them.
3. **No nested structs in a test body.** Setup = a builder chain; script = a list of one-line replies; assertions = `assert_*` one-liners.
4. **Shape is fixed**: `build → submit_turn → assert`. A reader sees inputs, the scripted model, and the expected outcome with no indirection.
5. **Target ≤ ~12 lines** for a complete test; the floor is ~3 (see §4.1).

Enforcement: a review-checklist item, plus a pre-commit grep that flags `TraceStep {` / `TraceResponse::` / `LlmTrace::new` added under `tests/reborn_*.rs` (suppressible with `// raw-trace-exempt: <reason>` for the rare deliberate case). This is a **test-style** check, not a safety invariant — implement it as a separate `scripts/check-reborn-test-style.sh` invoked by the pre-commit hook (or, if folded into `scripts/pre-commit-safety.sh`, under a clearly-labeled "test-style (non-safety)" section) so a future maintainer doesn't mistake it for a safety guard. It reuses the existing added-line grep mechanics.

### 4.3 Local-run contract (zero-setup)

The full default suite (InMemory + libSQL backends) must run for any developer with a single, obvious command and **no environment setup**:

```bash
cargo test --test reborn_<name>        # one file
cargo test --test 'reborn_*'           # the whole tier
```

Guarantees this design must hold to:

1. **No external services.** InMemory needs nothing; libSQL is an embedded SQLite file in a `TempDir`. `libsql` is already in `default` features (`Cargo.toml:294`), so neither a feature flag nor a server is needed.
2. **No API keys / network / real process.** The SDK seam is scripted and `build()` unsets provider env (§2); all other egress is captured or inert by default (§3.6) — including the **inert process port**, so a scripted `shell` tool call never spawns a real process. A default run never reaches a vendor, reads a credential, hits a channel, or mutates the machine. Works fully offline.
3. **No `integration` feature, no Postgres, no Docker** for the default suite. The `integration` feature and a live Postgres are reserved for the deferred Postgres lane only (§8) — the default run must never depend on them.
4. **No special toolchain.** mold linker and `CARGO_INCREMENTAL=0` are CI-only speed/OOM knobs (`reborn-tests.yml`); a developer uses the stock linker. The design must not bake mold or any CI-only flag into a test's correctness.
5. **Same code path in CI and locally.** CI adds parallelism/sharding and may pass `--features libsql` explicitly (a no-op since it is default), but runs the same `cargo test` against the same tests — no local-only or CI-only code branch, so "passes locally" means "passes in CI."
6. **Discoverability.** A short "How to run" block in `tests/support/reborn/` (README/module doc) states the one command and the (empty) prerequisites. An optional `scripts/` wrapper may exist, but the bare `cargo test` command must always work — the wrapper is convenience, never a requirement.

A framework self-test (or a doc check) asserts the libSQL-backed case builds and runs under default features without `integration`.

---

## 5. Data Flow

**Inbound (synthetic):** `h.submit_turn(conv, text)` → `RebornTestIngress` → `RebornTestProductAdapter` → `DefaultProductWorkflow` → `DefaultInboundTurnService` → `DefaultTurnCoordinator` → `TurnRunScheduler` → planned agent loop. No HTTP server.

**Model call:** agent loop → real `LlmProviderModelGateway` → real decorator chain (via the extracted `apply_decorator_chain`) → scripted raw provider pops the next `RebornScriptedReply` → returns a scripted provider response. Retry/routing/safety/tool-def assembly all execute.

**Persistence (`LibSql`):** turn/thread/product `put`/`get`/`append`/`query` → `CompositeRootFilesystem` → single `LibSqlRootFilesystem` → real SQLite file in `TempDir` (migrations run at build). `TempDir` dropped at test end.

**Capability/tool calls:** routed through the capability port; `RecordingTestCapabilityPort::echo()` (default) records without executing, or a real `HostRuntimeCapabilityHarness` executes file/memory/HTTP for fuller tests.

---

## 6. Error Handling & Edge Behavior

- **Script underflow** (loop requests more turns than scripted): the scripted provider returns a typed error → test fails with "script exhausted at step N" — not a hang. (If reusing `TraceLlm`, its existing exhaustion behavior is the baseline; wrap to produce a clear message.)
- **libSQL migration failure**: surfaced at `build()`, fails fast with the migration error.
- **Backend parity divergence** (`InMemory` passes, `LibSql` fails): a real persistence/CAS bug the matrix is designed to catch (version/CAS semantics, BLOB round-trip).
- **Tool-name mis-mapping**: prevented by §3.4; covered by a self-test.

---

## 7. Testing the Framework Itself

- Scripted provider: sequential replay, tool-call step, underflow error, and the §3.4 CapabilityId→ProviderToolName mapping.
- One golden scenario run through both `InMemory` and `LibSql` asserting identical outcome (proves backend parity for the harness plumbing).
- A probe test asserting `build()` applied the hermetic env (`TZ=UTC`, keychain disabled, zero backoff, and the cheap/fallback/circuit/cache env vars unset so the chain is a passthrough around the scripted provider).

---

## 8. CI Integration

- New tests are ordinary `tests/reborn_*.rs` files — picked up by the existing `reborn-tests.yml` matrix. No new workflow.
- CI runs the **same** `cargo test` invocation a developer runs locally (§4.3), wrapped only with parallelism/sharding and CI-only speed knobs (mold, `CARGO_INCREMENTAL=0`). No CI-only code path — "passes locally" ⇒ "passes in CI."
- `InMemory` + `LibSql` cases run in the default lane under default features (`libsql` is default); no `integration` feature, no service container.
- **Postgres lane (later):** when added, it reuses the existing `pgvector/pgvector:pg16` service-container pattern (`coverage.yml` / hooks-parity), gated to a dedicated job so PR bulk stays fast. At that point `StorageMode::Postgres` is a one-line `#[case]` + a builder arm constructing `PostgresRootFilesystem`, mirroring `LocalDevStorageBackendInput::Postgres` in `factory.rs`.

---

## 9. Build Order (for the implementation plan)

0. **(Prerequisite, production crate)** Extract `apply_decorator_chain(raw: Arc<dyn LlmProvider>, config: &LlmConfig, session: Arc<SessionManager>) -> Arc<dyn LlmProvider>` from `build_provider_chain_components_with_options` in `crates/ironclaw_llm/src/lib.rs` (the decorator stack at lines 1004–1118). **Minimal approach (preferred):** `build_provider_chain_components_with_options` internally calls `apply_decorator_chain` over the raw provider it already builds (lines 997–1001); `build_static_provider_chain` is **unchanged** (still calls the wrapper). The new `apply_decorator_chain` becomes `pub` so the test harness can call it directly with the scripted provider. Behavior-preserving — no change to any production call path. Ship as its own reviewable PR. Verified necessary: no injection seam exists today (the raw provider is built internally at lib.rs:997–1001).
1. Scripted-provider adapter (`scripted_provider.rs`) + unit tests. Wraps `TraceLlm`'s engine (decision locked, §3.3.1) — no new replay provider. Build an in-memory `LlmTrace` from the façade's replies and hand a `TraceLlm` to the chain. Include the §3.4 name mapping.
2. `RebornScriptedReply` constructors (`reply.rs`) — the mandatory façade (§3.3.1, §4.2): each reply maps to one `TraceStep`, auto-filling `id` / tokens / `request_hint` / `expected_tool_results`.
3. `RebornIntegrationHarness` + `::builder()`/`::test_default()` (`builder.rs`): wire the real `apply_decorator_chain` + `LlmProviderModelGateway` over the scripted provider; bake hermetic env into `build()`; `.script()` at builder time.
4. `StorageMode` + `LibSql` wiring in the builder, reusing the `factory.rs` mount helper over one `LibSqlRootFilesystem` on tmp. **Includes promoting `mount_local_dev_database_roots` to a test-callable visibility** (`pub`/`pub(crate)` + `#[cfg(any(test, feature = "test-support"))]` accessor) — a small, behavior-preserving change to `ironclaw_reborn_composition`.
5. `assert_*` helpers (`assertions.rs`).
5b. **External-boundary capture defaults (§3.6):** add `RecordingProcessPort` (`process.rs`, inert by default) and a `test-support` accessor in `ironclaw_reborn_composition::factory` to inject `Arc<dyn RuntimeProcessPort>` (same pattern as Step 4's mount-helper promotion); wire the existing `RecordingOutboundDeliverySink` / `RecordingRuntimeHttpEgress` + `RecordingNetworkHttpEgress` / `StaticSecretStore` into the builder defaults. Add `.with_live_shell()` / `.with_live_http_egress()` opt-ins. (Trace Commons needs no new sink — captured via `RecordingRuntimeHttpEgress`, §3.6. P1 ergonomics — URL-keyed HTTP scripting, `.with_mock_mcp()` — deferred.)
6. Add `rstest` dev-dep; write the backend-matrix template; framework self-tests (§7), including one asserting a scripted `shell` call records-but-does-not-execute (no real process).
7. Migrate 2–3 existing reborn scenarios as exemplars; document the templates in `tests/support/reborn/` (README or module doc).

Each step is independently landable. Step 0 is a standalone production-crate PR that gates step 3.

---

## 10. Resolved Review Decisions

- **No `GatewaySwap` / `ProviderMode`.** Single seam. (thermo F1 — highest-value simplification.)
- **`RebornScriptedReply` is a new, distinct provider-level API**, not a shared DSL with `RebornModelReplayStep`. The earlier "identical steps / does not fork" claim was incorrect and is removed. (thermo F2, local-patterns LP-01.)
- **No `#[reborn_test]` proc-macro.** Hermetic setup in `build()`; rstest fixtures + `#[tokio::test]`. (all reviewers.)
- **No reserved `StorageMode::Postgres`.** Added when the lane lands. (thermo F5, maintainability.)
- **Chain-injection promoted to Step 0** and verified necessary. (thermo F4/F7, approach.)
- **Naming**: `RebornIntegrationHarness`, `RebornScriptedReply` — both carry the `Reborn*` prefix per sibling convention and are free of collisions with `RebornBinaryE2EHarness` / `RebornHarnessSharedStorage` / `RebornModelReplayStep::Response`. (No new provider type — the engine is `TraceLlm`.) (local-patterns LP-01/02/03 + verification NEW-LP-01.)
- **Hermetic `build()` unsets cheap/fallback/circuit/cache env vars** so a developer `.env` can't make `apply_decorator_chain` instantiate a live vendor sub-provider beneath the scripted raw provider. (verification: approach F2.)
- **`mount_local_dev_database_roots` visibility promotion** folded into Step 4 — required for the cross-crate reuse the design depends on. (verification: approach F1 / thermo F1.)
- **Script at builder time**, not post-build mutation. (local-patterns LP-06.)
- **New files, not `harness.rs`** growth. (thermo F6, `.claude/rules/architecture.md` §5.)
- **StorageMode reuses the `factory.rs` mount helper**, not a parallel mount path. (approach F3, maintainability.)

- **Scripted provider = reuse `TraceLlm`'s engine** (§3.3.1). Verified: `LlmTrace`/`TraceLlm::from_trace` are in-memory constructable and already the dominant scripting pattern (8+ files incl. a Reborn test); `StubLlm` can't script tool-call sequences. No new replay provider.
- **`RebornScriptedReply` façade is mandatory**, and raw `TraceStep`/`LlmTrace` construction is **forbidden** in new Reborn integration tests (§4.2) — the readability contract is enforced by review + a pre-commit grep. This is the mechanism that keeps tests simple for humans and LLMs.

- **Zero-setup local runs** (§4.3): the default suite (InMemory + libSQL) runs on one plain `cargo test` with no services, keys, `integration` feature, Docker, or mold. Verified: `libsql` is in `default` features; CI and local share the identical invocation. (user requirement.)
- **Full external-boundary capture** (§3.6): every network/IO edge — LLM, tool HTTP, channel in/out, secrets/OAuth, embeddings, Trace Commons, shell/process — is captured or inert by default (clock/wall-time excepted — runs live). Verified against the existing recording ports (`RecordingOutboundDeliverySink`, `RecordingRuntimeHttpEgress` @`harness.rs:3034`, `RecordingNetworkHttpEgress` @`harness.rs:3091`, `StaticSecretStore`). One NEW port: inert `RecordingProcessPort` (closes the real-shell-execution gap — a safety requirement; needs a test-support process-port injection accessor, Step 5b). Trace Commons needs **no** new sink — its agent path routes through `RuntimeHttpEgress`, already captured (thermo round-4 F3 — dropped `RecordingContributionSink`). Channel inbound is synthetic; real adapter parsing is out of scope. (user requirement.)

- **Hermetic env must include `LLM_MAX_RETRIES=0`** (§2/§4.1): without it `apply_decorator_chain` wraps the scripted provider in `RetryProvider(max_retries=3)` (lib.rs:1004–1017), which is not passthrough and would re-invoke an exhausted script 3× on error-path tests. (cold thermo round-5 F3 — a real functional fix.)
- **§3.3.1 evidence corrected** (cold round-5 F1/F2): the TraceLlm-reuse decision is unchanged and correct on its merits, but the supporting data was wrong — recorded JSON fixtures are the *majority* (113 files), in-memory `from_trace` is used by 7 v1 files, and the Reborn tier had no prior `TraceLlm` use (`reborn_qa_recorded_behavior.rs` uses the gateway-level mock, not `TraceLlm`).
- **Step 0 scope pinned** (round-5 F6): `build_static_provider_chain` stays unchanged; `apply_decorator_chain` is extracted as a `pub` fn the harness calls directly. **Pre-commit readability check** is a separate test-style script, not a safety-hook check (round-5 F5).

No open implementation-time questions remain.
