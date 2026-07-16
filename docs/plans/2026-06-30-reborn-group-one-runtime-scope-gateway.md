# Converge reborn_group_* harness to ONE runtime per group + scope-selected scripted gateway (Option P)

## Why
`reborn_group_*` integration binaries flake ~1.4–5% under CPU contention.
Proven root cause: the group builds **N per-thread runtimes** (each its own
`TurnRunScheduler` worker + coordinator + per-thread `scripted_trace_llm`
gateway) over **one shared turn-run queue**. The scheduler is a global pool
(claims any run), so under load a worker for thread B claims thread C's run and
runs it on B's scripted gateway → B's deque exhausts → `Failed{ModelError}`,
masked as `driver_protocol_violation` (evidence port omits
`with_checkpoint_state_store`).

Option P deletes the N-runtime fan-out: the group owns **one** runtime (one
scheduler/coordinator/executor over the shared queue — exactly prod's shape),
and per-conversation scripted replies are selected **by scope at host
construction** (no model-hot-path store lookup). Chosen over the smaller
"Option C" scheduler scope-filter because this is a coverage roadmap: future
group tests should sit on a prod-faithful, scalable, one-runtime harness, and P
adds genuinely-new coverage (concurrent multi-scope dispatch + gate resume).

## Design (to be grounded against tip of main, then finalized by review loop)

### 1. Scope-selected scripted gateway — **DECISION: S2** (grounded at tip 09e7eb909)
The model request is scope-thin (`HostManagedModelRequest` carries
`run_id`/`turn_id`, not scope). But `create_host` builds the per-run host and
**already has `run_context.scope`** (`claimed.state.scope`, loop_driver_host.rs
:2062; `effective_scope` :1388) and already wraps the gateway as
`ThreadResolvingLoopModelGateway { thread_scope, host_gateway }` (:1636). So
per-scope selection happens **at host construction**, off the `stream_model`
hot path — NO turn-state-store read per model call.

**Why S2 over S1:** prod ALREADY instantiates `G = dyn HostManagedModelGateway`
(composition/runtime.rs:3770/3105 build `Arc<dyn HostManagedModelGateway>`); the
generic `G` exists only so tests can pass concrete gateway structs. So S2 needs
**zero generic-signature churn** and prod stays **byte-identical**, whereas S1
(replace `Arc<G>` with a resolver across `RebornLoopDriverHostFactory<S,G>` /
`DefaultPlannedRuntimeParts<G>` / `RebornRuntimeLoopComposition<S,G>` /
`build_*_planned_runtime<G>` + every concrete-`G` test call site) is strictly
wider for identical behavior.

**S2 edit surface (prod ≈ 12 lines, default-None → byte-identical):**
- `crates/ironclaw_loop_host/src/lib.rs` (~6 lines): add to the
  `HostManagedModelGateway` trait a default method
  `fn resolve_for_scope(&self, _scope: &TurnScope) -> Option<Arc<dyn HostManagedModelGateway>> { None }`.
  Every real gateway inherits the default → prod resolves to its own gateway.
- `crates/ironclaw_runner/src/loop_driver_host.rs` (~6 lines, 2 sites): before
  the wrap at :1636, `let host_gateway = self.model_gateway.resolve_for_scope(&request.loop_run_context.scope).unwrap_or_else(|| Arc::clone(&self.model_gateway))`
  and use it; apply the same one-line resolve in `build_compaction_ports`
  (:1062-1065) so compaction/system-inference also hit the right scope.
- Test support — **new file `tests/support/reborn/scope_gateway.rs`** (~40-60
  lines, NOT inlined into group.rs; own file so the routing mutation-test is a
  ~20-line standalone `#[cfg(test)] mod tests` without group scaffolding):
  `ScopeRegistryGateway` implementing `HostManagedModelGateway` —
  `resolve_for_scope(scope)` looks up a
  `Mutex<HashMap<TurnScope, Arc<dyn HostManagedModelGateway>>>` populated by
  `.thread(conv).script([...])` (each value = today's
  `LlmProviderModelGateway` over `scripted_trace_llm(replies)`, one deque per
  scope; `TurnScope` derives `Hash`+`Eq` at ironclaw_turns/src/scope.rs:4 — key
  is safe). Its own `stream_model_with_capabilities` returns a deterministic,
  LOUD "no gateway bound for scope" error (distinct error category, never
  confusable with `TraceLlm` exhaustion) so a routing miss fails legibly rather
  than re-masquerading as the bug we're fixing.
  Also update `tests/support/reborn/CLAUDE.md`: add `scope_gateway.rs` /
  `ScopeRegistryGateway` to the `## Files` registry (lines 72-103) and note that
  this dispatcher sits at the `HostManagedModelGateway` seam but routes to REAL
  `LlmProviderModelGateway` instances over the `ironclaw_llm` chain — the
  single-fake-at-the-vendor-SDK-seam invariant (CLAUDE.md:5-8,28) is preserved.

**Rejected alternative — closure field (do NOT implement):** a
`scope_resolver: Option<Arc<dyn Fn(&TurnScope) -> Option<Arc<dyn HostManagedModelGateway>>>>`
on `DefaultPlannedRuntimeParts` (prod passes `None`) keeps the test concept off
the trait, but BOTH the thermo-nuclear and approach reviews independently
rejected it: it threads a new type/field through the whole composition stack
(`DefaultPlannedRuntimeParts` → `RebornLoopDriverHostFactory` → `create_host`)
for IDENTICAL behavior — strictly more surface than the default-None method.
The trait already has a default-method precedent (`stream_model_with_capabilities`,
loop_support/src/lib.rs:1471); a `{ None }` default is invisible to all 500+
implementors and reversible (delete method + 2 call sites = zero prod change).
S2 is the minimum prod surface that injects at the one moment scope is known
(host construction). Embedding `TurnScope` in `HostManagedModelRequest` was also
rejected — it moves dispatch to the hot model-call path and touches every
`stream_model_with_capabilities` impl.

### 2. Group owns one runtime
Build `build_default_planned_runtime` ONCE at group construction; store
`coordinator` + `scheduler_handle` on `GroupSharedStorage`/`RebornIntegrationGroup`.
`.thread(conv).script([...])` no longer builds a runtime — it registers conv's
replies under its scope in the group gateway, creates the binding + thread
scope, and returns a lightweight handle that submits through the group's shared
workflow/coordinator.

### 3. Single-shot path
Make `test_default()` / single-shot a **degenerate 1-thread group** so there is
exactly ONE assembly path. Do NOT fork `assemble_thread_runtime`.
**Explicit routing (no de-facto fork):** `RebornIntegrationHarnessBuilder::build()`
constructs an internal `RebornIntegrationGroup` from the builder's
capability/storage selections, then calls
`.thread(self.conversation_id).script(self.replies).build()`. DELETE the inline
`GroupSharedStorage` construction block at builder.rs:247-253 and the direct
`assemble_thread_runtime`/`build_default_planned_runtime` call at builder.rs:256
— no separate runtime construction may remain in builder.rs.

### 3a. Turn-state store — ONE shared store, isolation by run_id (not path)
Build ONE `FilesystemTurnStateStore` at group construction (replacing the
per-thread stores). NOTE for the implementer: `turns_scope_path`
(tests/support/reborn/filesystem.rs:26-48) yields
`/tenants/{tenant}/agents/{agent}/users/{owner}/turns` with **no `thread_id`** —
so all same-tenant/agent/user threads already share one underlying path today.
Isolation is by `run_id` UUID within the shared snapshot, NOT by `TurnScope`
path namespacing. Do NOT hunt for per-thread path differentiation — there is
none and none is needed. One shared store is strictly better than today's N
stores fighting the same file under optimistic CAS (one in-memory snapshot
cache instead of N).

### 4. De-mask (independent, keep regardless)
Wire `.with_checkpoint_state_store(checkpoint_state_store.clone())` on the (now
single, group-level) `ThreadCheckpointLoopExitEvidencePort`.

### 5. Delete
Per-thread `scheduler_handle` / `coordinator` / `scripted_trace_llm`
construction. No prod `TurnRunScheduler` scope_filter change (that was Option C).

## Behavior-preservation risks (must be covered)
- **Shared coordinator gate-resume** (highest risk): one coordinator now serves
  all threads. Resume keys by `run_id` so it should hold — but add a NEW
  concurrent **dual-gate** scenario: gate thread A and thread B simultaneously,
  resolve each, assert no cross-resume / no bleed. This is new coverage P
  uniquely enables.
- Confirm every existing `reborn_group_*` scenario only uses submit/assert +
  shared store (no per-thread runtime internals).
- `step_matches` per-scope deques are cleaner than today (no cross-scope
  interleave); within a scope behavior is unchanged.

## Verification
- New routing/gateway logic unit-tested AND mutation-checked (inject bug → RED
  for the right reason → revert).
- Parallel-contention repro (9 concurrent group bins) ≥30 consecutive rounds,
  zero failures (was 1.4–5%).
- Full reborn suite green under `--features libsql` + `--all-features`;
  `cargo clippy --all --tests --all-features -- -D warnings` clean.
- Prod byte-identical for the single-gateway path. Verify BOTH gateway
  consumers explicitly: the `ThreadResolvingLoopModelGateway` wrap
  (loop_driver_host.rs:1639) AND `build_compaction_ports` (loop_driver_host.rs:1062)
  — prod gateway's default `resolve_for_scope → None` → `unwrap_or_else` →
  same `Arc::clone` at both. Confirm composition/runtime.rs (build_production_model_gateway
  :3766/:3842) and local_dev.rs:700 unchanged in behavior.
- De-mask credibility: after wiring `with_checkpoint_state_store`, confirm a
  genuinely-`Failed` run now reports its TRUE category (e.g. `ModelError`) — NOT
  a new phantom violation kind. Mutation-check this: force a real failure, assert
  the surfaced category is correct, not just "no longer driver_protocol_violation".

## Process
Plan reviewed via /thermo-nuclear-code-quality-review AND the reviewers at
approach.md / local-patterns.md / maintainability.md, in loops until green,
BEFORE implementation. Then implement with parallel subagents. Post-impl review
loop, then squash PR.
