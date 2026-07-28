# Reborn Integration-Test Framework — Slice 1 Implementation Plan

**Date:** 2026-06-26
**Scope:** Minimal spine + exactly ONE passing test, per the design spec
(`docs/superpowers/specs/2026-06-26-reborn-integration-test-framework-design.md`).
Build orders 0–3 + a minimal slice of 5 (assertions). Everything else
(libSQL backend, backend matrix, tool/HTTP/shell/MCP capture, pre-commit
style check, tool-call façade, rstest) is an EXPLICIT follow-up — NOT built now.

The single test exercises the full real path: synthetic inbound → product
workflow → scheduler → planned agent loop → real `LlmProviderModelGateway` →
real `apply_decorator_chain` (genuine passthrough) → scripted `TraceLlm` →
assistant reply finalized in thread history → assertion. InMemory storage only.

---

## File-by-file

### 1. Step 0 — production crate (`crates/ironclaw_llm/src/lib.rs`)

Extract the decorator stack (current lines ~1004–1118) into:

```rust
/// Apply the LLM decorator chain (retry → smart-routing → failover →
/// circuit-breaker → response-cache) over a raw provider. Each decorator is
/// configured from `config`; with the corresponding config field
/// disabled/zero the decorator is a passthrough (returns its inner provider
/// unchanged). This is the single source of truth for chain assembly.
pub(crate) async fn apply_decorator_chain(
    raw: Arc<dyn LlmProvider>,
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    // (body = current lines 1004–1118, with the initial `let llm = raw;`)
    Ok(llm)
}
```

The function is crate-internal (`pub(crate)`). Cross-crate test access goes through
`ironclaw_llm::testing::provider_chain_over` (a feature-gated forwarding fn in
`crates/ironclaw_llm/src/testing/mod.rs`), not via a `pub use` re-export.

`build_provider_chain_components_with_options` is rewritten to call it:

```rust
let llm = if config.backend == "openai_codex" {
    create_openai_codex_provider(config).await?
} else {
    create_llm_provider(config, session.clone()).await?
};
tracing::debug!("LLM provider initialized: {}", llm.model_name());
let llm = apply_decorator_chain(llm, config, session.clone()).await?;
let cheap_llm = if include_standalone_cheap {
    create_cheap_llm_provider(config, session)?
} else { None };
// ... unchanged Ok(ProviderChainComponents { primary: llm, cheap: cheap_llm })
```

- `build_static_provider_chain` is UNCHANGED (still goes through the wrapper).
- Behavior-preserving: same decorators, same order, same config reads, same
  `session.clone()` usage. The only change is hoisting the stack into a named
  `pub` fn. All private helpers it calls (`create_cheap_provider_for_backend`,
  `create_llm_provider_with_config`, the `RetryProvider`/`SmartRoutingProvider`/
  `FailoverProvider`/`CircuitBreakerProvider`/`CachedProvider` constructors)
  remain in the same module, so visibility is unaffected.
- Verify: `cargo test -p ironclaw_llm` (the existing chain/hot-reload tests in
  `lib.rs` `mod tests` exercise this path) stays green.

Est. net change: ~+8 lines (a fn header + the rewritten caller), no logic delta.

### 2. Scripted LLM seam — `tests/support/reborn/scripted_provider.rs` (NEW, ~30 lines)

Reuse `TraceLlm`'s replay engine; do not write a new provider.

Module-path convention (verified against `model_replay.rs`/`harness.rs`): reborn
siblings reference each other via `super::`, and `trace_llm` via the absolute
`crate::support::trace_llm` (the test binary declares both `mod reborn_support`
(`#[path="support/reborn/mod.rs"]`) and `mod support`). There is NO
`crate::support::reborn` path.

```rust
use super::reply::RebornScriptedReply;
use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceTurn};

/// Default model name surfaced by the scripted provider. Non-empty and not
/// "default" so the gateway's model-override resolution accepts it.
pub const SCRIPTED_MODEL_NAME: &str = "scripted/integration-test";

/// Build a `TraceLlm` that replays the given scripted replies in order.
pub fn scripted_trace_llm(
    replies: impl IntoIterator<Item = RebornScriptedReply>,
) -> TraceLlm {
    let steps = replies.into_iter().map(RebornScriptedReply::into_step).collect();
    let trace = LlmTrace::new(
        SCRIPTED_MODEL_NAME,
        vec![TraceTurn { user_input: "(scripted)".to_string(), steps, expects: Default::default() }],
    );
    TraceLlm::from_trace(trace)
}
```

(The §3.4 CapabilityId→ProviderToolName mapping is NOT added here — the one
test uses no tool calls, so adding it now would be untested speculative code.
Deferred with the `tool_call` façade.)

### 3. Reply façade — `tests/support/reborn/reply.rs` (NEW, ~25 lines)

```rust
use crate::support::trace_llm::{TraceResponse, TraceStep};   // absolute path; see scripted_provider.rs note

/// One scripted model turn. Each maps 1:1 to a `TraceStep`, auto-filling
/// id/tokens/request_hint/expected_tool_results so test bodies stay one line
/// per model turn (design §4.2). Raw `TraceStep`/`LlmTrace` construction is
/// forbidden in new Reborn integration tests.
pub struct RebornScriptedReply {
    step: TraceStep,
}

impl RebornScriptedReply {
    /// A plain assistant text reply.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            step: TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: content.into(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
                expected_tool_results: Vec::new(),
            },
        }
    }

    pub(crate) fn into_step(self) -> TraceStep { self.step }
}
```

(`tool_call(...)` is deferred — no test exercises it in slice 1; adding it now
would be dead/untested code that the thermo gate rejects.)

### 4. Harness + builder — `tests/support/reborn/builder.rs` (NEW, ~150 lines)

`StorageMode` enum is NOT added (only `InMemory` would exist → a one-variant
enum is dead abstraction). The builder defaults to InMemory directly. A doc
comment notes libSQL is the planned next variant so the addition is non-breaking.

```rust
pub struct RebornIntegrationHarness { /* coordinator, scheduler_handle (Option),
    workflow, ingress, binding, turn_scope, thread_scope, turn_store,
    thread_harness, _turn_root, _product_harness */ }

pub struct RebornIntegrationHarnessBuilder {
    conversation_id: String,
    replies: Vec<RebornScriptedReply>,
}

impl RebornIntegrationHarness {
    pub fn test_default() -> RebornIntegrationHarnessBuilder { Self::builder("conv-itest") }
    pub fn builder(conversation_id: impl Into<String>) -> RebornIntegrationHarnessBuilder { ... }
}

impl RebornIntegrationHarnessBuilder {
    pub fn script(mut self, replies: impl IntoIterator<Item = RebornScriptedReply>) -> Self { ... }
    pub async fn build(self) -> HarnessResult<RebornIntegrationHarness> { ... }
}
```

`build()` steps (mirrors the existing low-level constructor, but with the REAL
gateway and only the parts the greeting needs):

1. **Hermetic env (unconditional framework invariant, locked spec §2/§4.1):**
   set `IRONCLAW_DISABLE_OS_KEYCHAIN=1`, `TZ=UTC`, `LLM_MAX_RETRIES=0`; unset
   `NEARAI_CHEAP_MODEL`, `NEARAI_FALLBACK_MODEL`, `LLM_CIRCUIT_BREAKER_THRESHOLD`,
   `LLM_RESPONSE_CACHE_ENABLED`. This is a framework-wide hermetic contract (so
   every future test form inherits it and a developer `.env` can never reach a
   vendor), not test-specific — keep the full set even though slice 1's explicit
   passthrough config (step 7) already makes the LLM-chain ones inert.
   NOTE: `std::env::set_var`/`remove_var` require `unsafe` unconditionally in this
   crate (edition 2024). Wrap in `unsafe { .. }` — no edition check needed. See
   the `apply_hermetic_env` pattern in `tests/support/reborn/builder.rs`.
2. Adapter/ingress: `RebornTestProductAdapter::new("reborn-itest","itest-install")`,
   `RebornTestIngress::new(adapter)`.
3. Product harness: `RebornProductWorkflowHarness::filesystem_temp(product_scope)`
   where `product_scope = test_product_scope("tenant-itest","host-user","agent-itest",Some("project-itest"))`.
   Resolve binding via `product_harness.binding_service()?.resolve_binding(req)`,
   where `req` is a `ResolveBindingRequest` assembled inline from a
   `verified_text_envelope` (route_kind = `Direct` — slice 1 is DirectChat only).
4. Derive `thread_scope` (inline `ThreadScope { .. }` from binding fields) and
   `turn_scope` (`TurnScope::new_with_owner(..)`).
5. Thread harness: `RebornThreadHarness::filesystem_temp(thread_scope.clone())`.
6. Turn store (InMemory): `turn_backend = Arc::new(BlockingTurnStatePutFilesystem::new(InMemoryBackend::new()))`;
   `turn_store = Arc::new(TurnStateRowStore::new(scoped_turns_fs(turn_backend, &binding)?))`.
   (`scoped_turns_fs` + the two turn type aliases promoted to `pub(crate)` in harness.rs — see §6.)
7. **Real model gateway:**
   ```rust
   let raw: Arc<dyn LlmProvider> = Arc::new(scripted_trace_llm(self.replies));
   let session = ironclaw_llm::create_session_manager(SessionConfig::default()).await;
   let config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME); // passthrough caps
   let provider = ironclaw_llm::apply_decorator_chain(raw, &config, session).await?;
   let model_profile_id = ModelProfileId::new("interactive_model")?;
   let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None);
   let gateway: Arc<dyn HostManagedModelGateway> =
       Arc::new(LlmProviderModelGateway::new(provider, policy));
   ```
8. Capability parts (echo, no tools invoked): reuse the canonical harness glue
   (promoted to `pub(crate)`, §6) — NO new glue structs in builder.rs:
   `port = Arc::new(RecordingTestCapabilityPort::echo())`;
   `io = Arc::new(ProductLiveCapabilityIo::default())`;
   `factory = Arc::new(HarnessCapabilityPortFactory { port: port.clone() })`;
   `resolver = Arc::new(StaticCapabilitySurfaceProfileResolver { allow_set: CapabilityAllowSet::All })`
   (AllowAll — slice 1 never invokes a tool; the resolver already exists, reused
   with the `All` variant rather than the allowlist path). input_resolver =
   result_writer = `io`.
9. Loop-exit evidence: `ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
   thread_harness.service.clone(), turn_store.clone() as Arc<dyn TurnStateStore>,
   loop_checkpoint_store, thread_scope.clone())` (plain; the harness's
   blocked-evidence wrapper is only needed for approval tests — out of scope).
10. `build_default_planned_runtime(DefaultPlannedRuntimeParts { model_gateway: gateway, .. })`
    — same field set as the existing constructor (subagent stores, codec,
    `EmptyUserProfileSource`, the reused `EmptyIdentityContextSource` (§6),
    `poll_interval: 10ms`, all `Option` extensions `None`).
11. Workflow: `DefaultProductWorkflow::new(DefaultInboundTurnService::new(binding_service,
    thread_harness.service_instance()?, composition.coordinator.clone()),
    product_harness.idempotency_ledger(), binding_service)`.
12. Store coordinator + scheduler_handle + the bits needed for submit/assert.

`submit_turn(&self, text)`:
- Build a `verified_text_envelope_with_trigger(event_id, "host-user", conv, text, DirectChat)`,
  `workflow.accept_inbound(envelope).await` → `ProductInboundAck::Accepted { submitted_run_id }`.
- Poll `turn_store.get_run_state(..)` until `TurnStatus::Completed` (or terminal/timeout):
  10ms poll, ~10s deadline. Return on Completed; error with last status on timeout.

### 5. Assertions — `assert_reply_contains`

**As-built (commit 0364c72a8):** with a single assertion in slice 1, this method
was co-located in `builder.rs` (on the `impl RebornIntegrationHarness` block,
beside the fields it reads) rather than in a separate `assertions.rs`. A dedicated
`assertions.rs` is deferred until the `assert_*` family grows (see *Explicitly
deferred*). The method body below is exactly what shipped:

```rust
impl RebornIntegrationHarness {
    /// Assert the finalized assistant reply in thread history contains `text`.
    pub async fn assert_reply_contains(&self, text: &str) -> HarnessResult<()> {
        self.thread_harness
            .assert_final_reply(self.binding.thread_id.clone(), text)
            .await
            .map_err(Into::into)
    }
}
```

Reuses `RebornThreadHarness::assert_final_reply` (the proven reply source —
thread history, same as the existing harness). No outbound-sink wiring in
slice 1 (RecordingOutboundDeliverySink capture is a §3.6 follow-up).

### 6. harness.rs visibility promotions (NO new logic, NO line growth)

`pub(crate)` on existing items so builder.rs reuses the canonical helpers
(turn-store mount truth + capability/identity glue) instead of duplicating them
(thermo rubric #6 — reuse canonical, no near-duplicate glue):
- `type HarnessTurnStorageBackend`, `type HarnessTurnBackend`
- `fn scoped_turns_fs`
- `struct HarnessCapabilityPortFactory` (+ its `port` field)
- `struct StaticCapabilitySurfaceProfileResolver` (+ its `allow_set` field)
- `struct EmptyIdentityContextSource`

(`turn_state_root_filesystem` and `local_dev_mount_descriptor` stay private —
reached only through `scoped_turns_fs`.) Visibility-only: zero line growth.

### 7. mod.rs — register new submodules

Add `pub mod builder;`, `pub mod reply;`, and `pub mod scripted_provider;` to
`tests/support/reborn/mod.rs`. (No `pub mod assertions;` — see §5 as-built note:
`assert_reply_contains` lives in `builder.rs`.)

### 8. THE ONE TEST — `tests/reborn_integration_greeting.rs` (NEW, ~12 lines)

```rust
#[path = "support/reborn/mod.rs"]
mod reborn_support;            // mirror the existing include convention
mod support;                  // required: reborn_support files reference crate::support::*

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn replies_to_greeting() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("Hello! How can I help?")])
        .build()
        .await
        .expect("harness builds");
    harness.submit_turn("hi there").await.expect("turn completes");
    harness
        .assert_reply_contains("Hello! How can I help?")
        .await
        .expect("reply finalized in thread history");
}
```

Runs under default features, no services/keys/Docker/integration feature.

---

## Verification (Phase C)
- `cargo test -p ironclaw_llm` — Step 0 behavior-preserving.
- `cargo build` + `cargo clippy --all-targets` for touched crates.
- `cargo test --test reborn_integration_greeting` — the one test, green.

## Known accepted tradeoff (thermo self-review)
`builder.rs::build()` re-assembles ~120 lines of composition wiring that
parallels the existing `RebornBinaryE2EHarness` low-level constructor. The only
real difference is the injected gateway (real `LlmProviderModelGateway` over the
decorator chain vs. the gateway-level `RebornTraceReplayModelGateway`). The
code-judo move — make the existing constructor generic over `G:
HostManagedModelGateway` — is blocked: the `RebornBinaryE2EHarness` struct field
and its `model_requests()`/`assert_model_exhausted()` methods are concrete to
`RebornTraceReplayModelGateway`, so genericizing ripples to ~30 call sites in a
3.7k-line file the task forbids touching. The design spec deliberately chose
"new files, not harness.rs growth." Duplication is minimized by reusing the
product/thread harness, ingress, capability port, planned-runtime builder, and
the promoted `scoped_turns_fs`/glue (§6). If both tiers later converge, the
shared core is the natural extraction point.

## Explicitly deferred (follow-ups, NOT slice 1)
libSQL backend + `StorageMode` enum + `factory.rs` mount-helper promotion;
backend matrix + rstest; `RebornScriptedReply::tool_call` + §3.4 name mapping;
inert `RecordingProcessPort` + process-port injection accessor; outbound /
tool-HTTP / secrets / MCP capture wiring; pre-commit test-style check;
migration of existing scenarios.
