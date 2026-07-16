# Reborn Error Recoverability — Audit + Remediation Plan

**Date:** 2026-06-28
**Status:** discovery complete (core); two backend sweeps outstanding (see §6.4)
**Builds on:** [`docs/plans/2026-06-12-reborn-no-borking-failures.md`](2026-06-12-reborn-no-borking-failures.md) and PR #4841 (`reborn: no run-borking failures`, OPEN).

## Goal (decided with user)

Every reborn run error must end in one of two user-visible outcomes:

1. **Security-related → stop the run** (deliberate, clean halt).
2. **Everything else → user-explainable OR retriable.**

The "agent cannot cover / opaque dead-end" category must be **eliminated** by routing every failure into one of three terminal lanes.

Two decisions locked in:

- **Bucket 1 (SecurityStop) is minimal** — *only* injection/jailbreak detection and real secret/credential leaks halt a run. Authorization, policy, and egress denials are **not** stops; they stay recoverable or explainable.
- **Hybrid retry** — infra/lease/transient faults auto-retry silently; model/provider faults surface to the user with a retry affordance.

---

## 1. The error spine (how every error is classified today)

The agent-loop executor (`crates/ironclaw_agent_loop/src/executor.rs:99`) returns `Result<LoopExit, AgentLoopExecutorError>`.

- `LoopExit::Completed` = success
- `LoopExit::Blocked(LoopBlockedKind)` = **parked / resumable** (Approval / Auth / Resource / AwaitDependentRun / ExternalTool) — not a failure
- `LoopExit::Cancelled` = cancel / interrupt
- `LoopExit::Failed(LoopFailureKind)` = **graceful run-bork** (`crates/ironclaw_turns/src/loop_exit.rs:251,432`)
- `AgentLoopExecutorError` = **hard run-bork** — no trustworthy exit (`HostUnavailable`, `HostUnavailableWithDiagnostics`, `PlannerContract`(=driver bug), `CheckpointFailed`, `Cancelled`)

### Two decision layers govern a capability (tool) failure

**Layer 1 — host_runtime disposition** (`crates/ironclaw_host_runtime/src/lib.rs:811` `capability_failure_disposition`). Only two outcomes exist:

- `ModelVisibleToolError` — return a tool error to the model in the same loop
- `RetrySameCall` — retry first; the loop recovery strategy owns the budget and post-exhaustion fallback

**There is no "abort" disposition. By design, no runtime capability failure should ever end the run.** Default for any kind that is not retryable-infra and not `InvalidInput` is `ModelVisibleToolError`. Retryable-infra set = `Backend / Network / Transient / Unavailable / Internal`.

**Layer 2 — recovery strategy class** (`crates/ironclaw_agent_loop/src/strategies/recovery.rs` `DefaultRecoveryStrategy`, max 2 retries/class; classification in `crates/ironclaw_agent_loop/src/executor/mapping.rs:131` `capability_error_class`):

| `CapabilityFailureKind` | class | fate |
|---|---|---|
| Network, Transient | Transient | retry → **recoverable** |
| Backend, Unavailable | Unavailable | retry → **recoverable** |
| InvalidInput | InputInvalid | **recoverable** |
| MissingRuntime, OperationFailed, OutputTooLarge, Process, Resource | OperationFailed | **recoverable** |
| Authorization, GateDeclined, PolicyDenied | PolicyDenied | **recoverable** (denied) |
| Internal | Internal | retry → **recoverable** |
| **Dispatcher, Cancelled, InvalidOutput, Permanent, Unknown(_), + future** | Permanent | **ABORT → `LoopExit::Failed{CapabilityProtocolError}`** |

### Model-call failures are *always* run-borking

`model_error_class` (`mapping.rs:94`): transient/unavailable/internal/context-overflow → retry ×2 then Abort (`ModelError`); content-filtered → Abort; budget-approval → parked; cancelled → cancel; **Unauthorized / PolicyDenied / ScopeMismatch / StaleSurface / InvalidInvocation / Invalid / CheckpointRejected / TranscriptWriteFailed → None → hard bork** (`HostUnavailableWithDiagnostics{Model}`), invisible to the model. A model failure can never become a recoverable tool error.

---

## 2. Complete classification map

**Recoverable (loop continues, agent adapts):** all tool/capability failures by design — bad args, policy/authorization denials, WASM fuel/trap/memory/timeout, MCP timeout/protocol/session-loss, process non-zero-exit/timeout/OOM, filesystem permission/size/path, network DNS/TLS/denied-domain, transient model errors (retried in-loop to `Completed`). The tool backends (`mcp`, `wasm*`, `processes`, `process_sandbox`, `filesystem`, `network`, `outbound`, `secrets`, `resources`) are **clean** — no `panic`/`unwrap`/`expect` on runtime input; every recoverable trigger is a structured `Result::Err` → `Failed` outcome.

**Run-bork (graceful, `LoopExit::Failed`):** capability `Permanent` class or 8-retry ceiling → `CapabilityProtocolError`; all model errors after retry → `ModelError`; iteration limit (32) → `IterationLimit`; no-progress/stuck → `NoProgressDetected`; ≥3 rejected replies → `InvalidModelOutput`; compaction failure → `CompactionUnavailable`; `SpawnedProcess` unsupported → `CapabilityProtocolError`.

**Run-bork (hard, `AgentLoopExecutorError`):** any host-port call fails; model class=None; planner/driver contract violations; checkpoint serialize/write; safe-summary validation reject; malformed sandbox-plan; the 4 `unreachable!` index-key constants (regression-only); Postgres `row.get` schema drift.

**Parked / cancel (not failures):** gate outcomes (resume_turn); host cancel/interrupt.

`ContextBuildFailed` is **dead** (verified — no construction site); prompt-build failures hard-bork as `HostUnavailable{Prompt}` instead.

---

## 3. PR #4841 coverage (OPEN, "Part 1")

#4841 delivers the **explainable + retryable backbone** — in the two-bucket model, that *is* bucket 2 for the graceful-failure surface.

- **Tier-1 model explanation** for model-reachable failures (CapabilityProtocolError, IterationLimit, PolicyDenied, NoProgressDetected, CompactionUnavailable, InvalidModelOutput) — one best-effort no-capability model call before failing.
- **Tier-2 deterministic templates** — `FailureExplanationProvider` covers *every* `LoopFailureKind` + reborn failure category + `host_stage_unavailable:*` (exhaustive-match test).
- **Failed exits carry evidence** (explanation refs, partial assistant refs, diagnostics, `safe_summary` category).
- **Retry-from-failed** — `retry_turn` in both store backends, new run from last resumable checkpoint, idempotent, `webui_v2` retry endpoint, `retryable` flag on the wire.
- **HostUnavailable → categorized retryable failure** (`host_stage_unavailable:<stage>`), no longer opaque.

It does **not** touch `ironclaw_llm`, `ironclaw_processes`, `outbound_delivery`, `process_sandbox`, `ironclaw_safety`, or `turn_scheduler.rs`.

### Per-case status

| §4 cannot-cover case | #4841 |
|---|---|
| HostUnavailable infra, lease expiry, checkpoint/transcript, scope/surface, driver-bug, model PolicyDenied | **Covered (explainable; retryable if checkpoint)** |
| model bad key / ModelNotAvailable | **Partial** — surfaced+retryable, but root mis-classification in `ironclaw_llm` remains |
| safe-summary kill | **Partial** — explainable+retryable, not prevented |
| resumed-checkpoint-gone, projection split-brain | **Partial — verify** |
| Codex truncated-stream, detached bg-process | **Not covered** (`ironclaw_llm` / `ironclaw_processes` untouched) |
| evidence stores unwired | **Softened, not fixed** |

---

## 4. The "retriable" gap

#4841 is **not** fully retriable per the hybrid decision:

1. **Auto half missing** — all retry is *user-initiated* (the endpoint / `retry_turn`). No silent auto re-drive; `turn_scheduler.rs` is untouched. Everything we marked Auto-retriable (infra `HostUnavailable`, checkpoint/transcript, lease loss, scope/surface) is only manually retryable.
2. **Checkpoint-gated** — retryable *only* when a `BeforeModel`/`BeforeBlock` checkpoint exists (`crates/ironclaw_runner/src/planned_driver.rs:393`). `BeforeSideEffect` and `Final` are not resumable. Failures before the first `BeforeModel` checkpoint (`crates/ironclaw_agent_loop/src/executor/canonical.rs:111`) — input drain, context/prompt build, surface build, compaction — are explainable but **not retriable at all**.
3. **Codex truncation** — not even detected as a failure, so nothing to retry.

### No-checkpoint user journey + the from-input gap

The original input **is durable** — `SubmitTurnRequest.accepted_message_ref` (`crates/ironclaw_turns/src/request.rs:58`). So a from-scratch retry is feasible without asking the user to re-type. Two populations:

- **(a) Pre-first-checkpoint failures** — nothing durable happened. A from-input re-drive (seed a new run from `accepted_message_ref` when no resumable checkpoint exists) is **safe and cheap** and makes essentially every early failure retryable via the same affordance. **This is a near-free win.**
- **(b) Side-effect-only / final-only failures** — a side effect may have run; blind re-drive double-executes it. This is the genuine open decision: (i) make side-effect dispatch idempotent and add `BeforeSideEffect` to the resumable set; or (ii) explicit user-confirmed "resend (may repeat actions)"; or (iii) explainable-only.

Today's no-retry journey is a dead end: explanation shown, `retryable: false`, no retry button (copy-mismatch risk: Tier-2 says "Retry the run" while the flag is false), user must manually re-send → new turn, lost continuity.

---

## 5. Residue after #4841 + Part-2 (still neither retriable nor explainable)

1. **Pre-run failures** — boot/config (operator-explainable via logs only) and API-ingress/submission rejections (HTTP-explainable to the client; the agent never sees them; no run to retry).
2. **SecurityStop** — explainable but deliberately not retriable (correct by the minimal-bucket-1 decision).
3. **Explainable-but-futile** — driver bugs (generic sentence + telemetry; retry re-hits the bug), genuinely-permanent capability failures, and the §6 recoverable→bork defects (retry re-fails).
4. **Out-of-run-loop surfaces** — proactive work (triggers, heartbeat, routines) and projection/SSE/UI-convergence have their own failure handling, not the run-retry/explain machinery.
5. **Persistence floor** — a sustained store outage means even *recording* the explainable/retryable failure can't land.

---

## 6. The recoverable→bork defect hunt (the high-leverage fix)

**Premise:** turn model-fixable failures into agent-recoverable tool errors so ~90% of failures keep the run alive. There are three shapes.

### 6.1 Keystone — the table contradicts the disposition layer (verified)

Layer 1 (host_runtime) dispositions `Dispatcher`, `InvalidOutput`, and `Unknown` as `ModelVisibleToolError` (intended recoverable). Layer 2 (`capability_error_class`, `mapping.rs:155-162`) maps the same three to `Permanent` → **Abort**. `RuntimeFailureKind` has **no `Permanent` variant** (`crates/ironclaw_host_runtime/src/lib.rs:698`), so the abort class is reached *only* through these three (plus `Cancelled`, which is legitimately a cancel).

Notably, `DispatchFailureKind::UnknownCapability` / `UnknownProvider` → `RuntimeFailureKind::InvalidOutput` (`crates/ironclaw_host_runtime/src/production.rs:2113`), so **"model called a nonexistent tool" → InvalidOutput → Permanent → run dies**, when it should be a model-visible `Failed{InvalidInput}` ("no such tool, choose another").

There's also an internal tell: `SameCallRetryConstraint` (`crates/ironclaw_agent_loop/src/executor/capability_helpers.rs:388`) marks `Dispatcher` = `Allowed` and `InvalidOutput` = `RequiresChangedInput` (i.e. retry/adapt makes sense) while the recovery class aborts — the two layers disagree.

**Fix (single, high-leverage):** in `capability_error_class`, move `Dispatcher`, `InvalidOutput`, and `Unknown(_)` out of `Permanent` into a recoverable class (`OperationFailed` → `ToolErrorResult`), aligning Layer 2 with Layer 1's `ModelVisibleToolError` intent. Keep only genuinely-terminal sources (none from the runtime path) and `Cancelled` (handled as cancel) as abort. This converts the entire "nonexistent tool / malformed output / unknown failure" population from run-ending to recoverable in one change.

### 6.2 Handler-level Err-instead-of-Ok(Failed) — Invariant 1 (confirmed)

A handler `invoke` returning `Err(AgentLoopHostError)` is mapped by `capability_host_error` (`mapping.rs:117`) to terminal `HostUnavailable{Capability}` — every non-`Cancelled` kind kills the run. Per `.claude/rules/agent-loop-capabilities.md`, model-fixable conditions must be `Ok(CapabilityOutcome::Failed/Denied)`.

**Confirmed defects:**

| Site | Condition | Current | Fix |
|---|---|---|---|
| `crates/ironclaw_reborn_composition/src/runtime/local_dev/outbound_delivery.rs:108,213` (via `outbound_delivery_host_error`, :575) | model picks bad/nonexistent `target_id` (InvalidRequest/NotFound), forbidden, conflict, rate-limited, transient-unavailable | maps **all** `RebornServicesErrorCode` → `Err` → terminal | mirror `project_service_outcome`: InvalidRequest/NotFound→`Failed{InvalidInput}`; Unauthenticated/Forbidden→`Denied`; Conflict→`Failed{OperationFailed}`; RateLimited→`Failed{Resource}`; Unavailable→`Failed{Unavailable}`; only Internal→`Err` |
| `outbound_delivery.rs:223` | model-supplied `target_id` interpolated into `safe_summary` | `format!("set delivery target to {target_id}")` — a delimiter in the id trips validation → terminal | fixed host-authored string; id travels in `output` |
| `outbound_delivery.rs:208,340-351,392-394` (via `approval_lease_error`) | expired/lost approval lease, not-yet-approved gate | → `Unauthorized` `Err` (terminal) | route lease-state arms → `Ok(Denied)` (re-request); keep Persistence/CAS as `Err` |
| `crates/ironclaw_host_runtime/src/production.rs:1990,1995` (`host_runtime_spawn_input_for_capability`) | malformed model-supplied `SandboxProcessPlan` | `HostRuntimeError::invalid_request` → `InvalidInvocation` → terminal | `Ok(CapabilityOutcome::Failed{InvalidInput})` (locked in by a test at `loop_support/.../capability_port.rs:6655` — update it) |

The exemplars to copy: `skill_activation.rs` (`skill_activation_selection_outcome`) and `project_create.rs` (`project_service_outcome`) — recoverable codes → `Ok(Failed)`, only `Internal` → `Err`.

### 6.3 Provider fidelity (model path) — accurate explainability

| Site | Condition | Current | Fix |
|---|---|---|---|
| `crates/ironclaw_llm/src/rig_adapter.rs:1019` (`map_rig_error`) | 401/403 auth on a turn (OpenAI/Anthropic/Ollama/Tinfoil/openai_compatible) | only context-length special-cased; everything else → `RequestFailed` → generic `Unavailable` → retried then bork as "model unavailable" | detect auth → `AuthFailed`/credentials category so the user is told to fix the key |
| `crates/ironclaw_llm/src/bedrock.rs:700` | AccessDenied / Throttling / ValidationException(overflow) | all → `RequestFailed` | map to AuthFailed / RateLimited / ContextLengthExceeded |
| Codex (`openai_codex_provider.rs:830`, `codex_chatgpt.rs:673`) | truncated SSE stream / dropped `error`/`response.failed` events | silent partial success labeled `Stop` | detect incomplete stream → `InvalidResponse`/`Unavailable` (retryable); map SSE error events |
| Codex/Bedrock/Copilot/Anthropic-OAuth | context overflow | no 413 detection → no `ShrinkContext` | detect 413/context-overflow → `ContextLengthExceeded` |

### 6.4 Outstanding sweeps (not completed — finish these)

Two parallel read-only sweeps were started and interrupted; finish them with the §6.1 rubric:

- **host_runtime + dispatcher variant-by-variant**: audit `failure_kind_from` (`production.rs:2068`), `From<DispatchFailureKind>` (`production.rs:2113`), `RuntimeDispatchErrorKind` / `DispatchFailureKind` / `CapabilityInvocationError` variant lists. For each model-fixable variant, confirm it doesn't land in the abort set or in an infra-retry kind it shouldn't. (`MethodMissing`, `UndeclaredCapability`, `OutputDecode`, `InvalidResult`, `InputEncode` are the suspects.)
- **tool backends + extensions**: `mcp`, `wasm*`, `processes`, `process_sandbox`, `filesystem`, `network`, `outbound`, `first_party_extensions`, `product_adapters`, `skills` — any model-fixable backend failure (unknown method, bad args, malformed output, denied target) that surfaces as a hard `Err` or lands in `InvalidOutput`/`Dispatcher`/`Unknown` (abort set).

---

## 7. Remediation plan — target architecture

Collapse every terminal failure into three lanes via one classifier:

| Lane | Retry policy | Membership |
|---|---|---|
| **SecurityStop** | None (clean halt) | *only* injection/jailbreak + real leak (safety layer / leak detector) |
| **Retriable** | `AutoBounded` silent re-drive; exhaustion → Explainable | infra host-port faults, lease loss/runner death, checkpoint/transcript, projection, scope/surface |
| **Explainable** | `UserInitiated` (retry affordance) or `None+restart` | model/provider faults, config/credentials, driver bugs, lost checkpoint |

### Building blocks (dependency order)

1. **`RunFailureReason` taxonomy** — wire-stable, user-facing, distinct from internal `LoopFailureKind`; carries `{lane, retry_policy, user_message, correlation_id}`. (#4841's `FailureExplanationProvider` + `safe_summary` category is most of this.)
2. **One exhaustive-match classifier** at the run boundary (`crates/ironclaw_runner/src/planned_driver.rs` + `turn_run_executor.rs`). Every failure passes through it; a new kind without classification fails to compile.
3. **Re-bucket the `Permanent` class** (§6.1) — the highest-leverage single change.
4. **Fix handler Err sites** (§6.2) + provider fidelity (§6.3).
5. **Scheduler auto re-drive** for the Retriable lane (the missing "Auto" half) — consumes #4841's `retry_turn`, bounded, exhaustion → Explainable.
6. **From-input retry** (§4 population a) — seed a new run from `accepted_message_ref` when no resumable checkpoint exists.
7. **Defense-in-depth degradation** — safe-summary/validation failures fall back to a fixed summary (recoverable), never bork.
8. **Reconcilers** — stuck-`Running` bg-process; lease reclaim → requeue.
9. **Fail-fast composition** — required evidence stores; loud startup failure if unwired.

### Sequencing

1. **#6.1 table re-bucket** (small, high-leverage, makes most capability failures recoverable).
2. **#6.2 handler fixes + #6.3 provider fidelity** (accurate recoverable/explainable).
3. **Block 1+2 classifier keystone** (enforce the three-lane invariant).
4. **Retriable lane**: #5 auto re-drive + #6 from-input retry + lease requeue + projection self-heal.
5. **Prevention/security**: #7 degradation, #9 fail-fast, the SecurityStop set, the enforcement test.

### Enforcement

A test asserting: every `RunFailureReason` resolves to `SecurityStop` **only if** sourced from the safety/leak layer; everything else is `Retriable` or `Explainable` with a non-empty `user_message`. A new variant without classification fails to compile; a new `SecurityStop` outside the safety layer fails the test.

---

## 8. Open decisions

1. **Population (b) side-effect retry** (§4) — idempotent dispatch + resumable `BeforeSideEffect`, vs user-confirmed resend, vs explainable-only.
2. **Pre-run / ingress failures** (§5.1) — surface into the same user-facing taxonomy, or accept the HTTP/log boundary for those.

---

## Appendix — key code locations

| Concern | Location |
|---|---|
| Disposition layer (no-abort intent) | `crates/ironclaw_host_runtime/src/lib.rs:811` (`capability_failure_disposition`), enum `:745`, `RuntimeFailureKind` `:698` |
| Recovery classes | `crates/ironclaw_agent_loop/src/executor/mapping.rs:131` (`capability_error_class`), `:94` (`model_error_class`), `:166` |
| Recovery strategy | `crates/ironclaw_agent_loop/src/strategies/recovery.rs` (`DefaultRecoveryStrategy`) |
| Same-call retry hint | `crates/ironclaw_agent_loop/src/executor/capability_helpers.rs:388` |
| Terminal mapper (Err→HostUnavailable) | `crates/ironclaw_agent_loop/src/executor/mapping.rs:117` (`capability_host_error`) |
| LoopExit / LoopFailureKind | `crates/ironclaw_turns/src/loop_exit.rs:251,432` |
| AgentLoopExecutorError | `crates/ironclaw_agent_loop/src/executor.rs:99` |
| Runtime→loop kind map | `crates/ironclaw_loop_host/src/capability_port.rs:2570` (`runtime_failure_kind_to_loop`) |
| Dispatch kind map | `crates/ironclaw_host_runtime/src/production.rs:2068` (`failure_kind_from`), `:2113` (`From<DispatchFailureKind>`) |
| outbound_delivery defect | `crates/ironclaw_reborn_composition/src/runtime/local_dev/outbound_delivery.rs:108,213,223,575` |
| sandbox-plan defect | `crates/ironclaw_host_runtime/src/production.rs:1990` |
| Provider fidelity | `crates/ironclaw_llm/src/rig_adapter.rs:1019`, `bedrock.rs:700`, `openai_codex_provider.rs:830`, `codex_chatgpt.rs:673` |
| Model-error path | `crates/ironclaw_agent_loop/src/executor/model.rs:125` |
| Resumable checkpoint kinds | `crates/ironclaw_runner/src/planned_driver.rs:393`; first checkpoint `canonical.rs:111` |
| Durable turn input | `crates/ironclaw_turns/src/request.rs:58` (`accepted_message_ref`) |
| Lease expiry | `crates/ironclaw_turns/src/memory.rs` (`recover_expired_leases`) |
| Exemplar recoverable handlers | `crates/ironclaw_reborn_composition/src/runtime/local_dev/{skill_activation,project_create}.rs` |

🤖 Generated with [Claude Code](https://claude.com/claude-code)
