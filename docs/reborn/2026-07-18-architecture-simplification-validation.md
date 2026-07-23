# Architecture-Simplification: Doc vs. Implementation Validation

**Date:** 2026-07-18 · **Against:** `main` @ commit `c3a9ecd36` (post #6245)
**Method:** four parallel read-only passes over `crates/` on HEAD, verifying each
claim of `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md` at
the definition/call site (not from the doc's prose). Companion to the interactive
board `2026-07-17-architecture-simplification-explorer.html`.

**One-line finding:** the target *vocabulary* has largely landed, but **additively
and mostly unwired**; the *collapses and deletions* the doc proposes have barely
begun; two areas **diverged by deliberate design**; and the #6170 security fix is
**real but implemented via a different mechanism** than the doc sketched. Treat the
doc's own "done/landed" language and its ratchet claims as unverified — a green
ratchet proves names, not wiring.

## Verdict board

| § | Claim | Verdict | Evidence (HEAD) |
| --- | --- | --- | --- |
| 3/4.1 | Target vocabulary in `host_api` | **DONE but ADDITIVE** | `Invocation` `invocation.rs:106`, `Authorized` `authorized.rs:81`, `Outcome`/`Blocked`/`Resolution` `resolution.rs:214/53/232`, `HostFailure` `failure.rs:44`. `failure.rs:14` — "nothing returns it yet." |
| 1.1/3.1 | 5 mirror DTOs collapse to 3 states | **NOT STARTED** | all 5 LIVE + frozen in `reborn_capability_dto_collapse_ratchet.rs:51` (which freezes ~10 names incl. resume siblings + `CapabilityOutcome`). |
| 3 | `Authorized` sealed & invocation-bound | **STRUCTURAL, guarantee vacuous** | private fields + `seal(grant)` witness (`authorized.rs:64`), but the one prod seal (`capabilities/src/host.rs:810`) fills 4/5 fields with `PROVISIONAL (Slice C)` placeholders; "does not yet gate dispatch." |
| 1.2 | `authorize()` centralizes policy | **DELEGATING SCAFFOLD** | `host.rs:426` takes the *old* `CapabilityInvocationRequest`, delegates trust to a separate `Arc<dyn>` authorizer (`host.rs:483`), does not reserve. No `dispatch(Authorized)` exists. Four-layer smear persists. |
| 4.2 | `RuntimeAdapter` dyn → closed enum | **DONE** (1 of 3 moves) | `enum RuntimeLane` `host_api/src/lane.rs:46`; dispatch routes via `from_runtime_kind` `dispatcher/src/lib.rs:344`; dyn registry gone. Trait retained as static-dispatch execution shape. |
| 4.2 | Delete `HostRuntime`/`CapabilityDispatcher` traits | **NOT DONE** | both still `pub trait` + `Arc<dyn>` on the hot path (`host_runtime/src/lib.rs:930`, `host_api/src/dispatch.rs:510`). Test doubles *grew*: HostRuntime 6→~10, dispatcher →~19 across test files. |
| 4.2 | `LlmProvider` kept | **HOLDS** | ~38 `impl LlmProvider` — genuine polymorphism, correctly a keep. |
| 4.3 | Delete `InMemory*Store` | **PARTIAL** | 12 frozen (turn store 4,258 LOC). Consolidated domains done; **turns deferred** pending a livelock stress test; ~2 are permanent bounded-cache keeps → floor ~2, not 0. Backends confirmed (`DiskFilesystem`, not `LocalFilesystem`). |
| 4.4 | `DeploymentConfig` = §5.6 backend-as-data | **DIVERGED (by design)** | `deployment.rs:31` is `{deployment, requested_profile, yolo_ack, org_policy}` → resolves to `EffectiveRuntimePolicy`. Team kept `ironclaw_runtime_policy` as the single resolver; no `{filesystem, process, network, approval}` fields. |
| 4.4 | Per-mode wiring removed | **DRIFTED** | 56 `RuntimeProfile::` refs, 6 `match profile`, 8+ `build_local_*` fns in composition. |
| 4.4 | `LocalDev*` shadow runtime → config | **RENAMED, not configified** | store/policy family genuinely deleted (~66 ids → enum variants + residuals); behavior de-prefixed to `Synthetic*/Staged*` families; mode enum survives on purpose. |
| 4.4.1/13.3 | Synthetic caps → first-party | **NOT STARTED** | `project_create`/`skill_activate`/`result_read`/`outbound_delivery_*` still synthetic port decorators, production-wired unconditionally (`wrap_synthetic_capabilities`); none in `first_party_tools/`. |
| 4.4 | `local_trigger_access` → `owner_seed` config | **DRIFTED** | still a module (`ironclaw_runner::local_trigger_access`) re-exported from composition; no `owner_seed` field on `DeploymentConfig`. |
| 4.5 | `host_api` shrunk & frozen | **DIVERGED / not started** | ~138 public types (grew from ~124); `runtime_policy.rs` still defines `DeploymentMode`/`RuntimeProfile` in `host_api`; **no freeze test** pins the type set. |
| 5.8 | Products out of composition | **NOT STARTED** | 184K LOC / 224 files; `product_auth` 32K, `slack` 31K, `runtime` 21K resident. WebUI *crate* consolidated (composition/webui ~2.3K) but no product left. `reborn_composition_boundaries.rs` freezes facade/deps, does **not** ban product identifiers. |
| 6 | #6170 shell cross-tenant escape | **FIXED (diff. mechanism)** | resolver fail-closed (`resolver.rs` — "hosted multi-tenant must never produce LocalHost"); fail-open `HostedSingleTenant→LocalSingleUser→LocalHost` mapping gone; `HostProcessPort` renamed w/ #6170 comment. But via `ProcessSandboxBackend` + executor-config host roots, **not** the doc's `SandboxMount::for_scope` (that API doesn't exist). |

## What the implementations taught us (deltas from the proposal)

1. **Vocabulary-first is genuinely additive — "type exists" ≠ "on the return path."**
   The new `host_api` types land, get frozen by a ratchet, and get unit tests
   *before* anything returns them. `Authorized` is minted then discarded; nothing
   returns `Resolution`/`HostFailure`/`Outcome`. A progress read that stops at
   "the type is defined" over-reports by a full migration.

2. **A green ratchet / rename proves NAMES, not WIRING.** The `LocalDev*`
   type-name ratchet hit 0 while composition kept 56 `RuntimeProfile::` refs and
   `DeploymentConfig` only *resolved a mode to policy*. Same shape as the identity-
   store miss in `.claude/rules/discovery-claims.md`. Before marking an axis done,
   grep the old call sites/branches the change was meant to remove.

3. **Two deliberate divergences the team chose over the doc:**
   - **`DeploymentConfig` is a mode-request → single resolver**, not backend-as-data
     fields. `ironclaw_runtime_policy` stays the one policy engine; the mode enum
     survives on purpose (contra "the kernel never names a mode").
   - **`LocalDev*` behavior was de-prefixed/renamed** to mode-neutral families,
     not turned into config data. §4.4.1 Bucket 2 ("de-prefix, don't configify")
     won over §4.4's "collapse to a value."

4. **`dyn → enum` was not free.** The `RuntimeLaneExecutor::dispatch_json` is
   hand-desugared (no `async fn`) to avoid stacking a second boxed future on the
   adapter's `#[async_trait]` future — the extra layer overflowed the 2 MiB
   test-thread stack in a trace-suite CI test. A real cost the clean framing missed.

5. **The test-double tax got worse before it got better.** The doc's rationale for
   deleting `HostRuntime`/`CapabilityDispatcher` was "the trait exists to inject
   test doubles"; during migration the double populations *grew* (HostRuntime
   6→~10, dispatcher →~19). The collapse that would pay this back hasn't landed.

6. **A security fix can be real while the doc's named API is aspirational.** #6170
   is genuinely closed (resolver fails closed), but through executor-config host
   roots + `SandboxMount { container_path, writable }`, not the doc's
   `for_scope`-only mount. Rule: do not mark a security item DONE against the doc's
   literal types — verify the *protection*, and note when the mechanism differs.

7. **Store consolidation has a real gate, not just a checklist.** The turns cluster
   is deferred until a concurrency/livelock stress test proves
   `FilesystemTurnStateStore<InMemoryBackend>` keeps the no-livelock property; some
   `InMemory*Store`s are permanent justified bounded-cache keeps. The allowlist
   floor is ~2, not 0 — "empty allowlist" was the wrong definition of done for A.

## Doc-drift flags (fix in the source note when convenient)

- §1.1 table lists `trust_decision` on `RuntimeCapabilityRequest`; it has since been
  removed there (`host_runtime/src/lib.rs:352`, "do not re-add"). Still a live field
  on the capabilities-crate DTOs.
- §1.1 line numbers drift (`CapabilityInvocation` 1722→1692) — the doc warns of this.
- §1.3 double counts are lower than reality (HostRuntime "6" → ~10; dispatcher "2"
  counted only in-`src` doubles, ~19 across test files).
- §4.5 "~124 types" is now ~138 and growing — the boundary is not frozen.
