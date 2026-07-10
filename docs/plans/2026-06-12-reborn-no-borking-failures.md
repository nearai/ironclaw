# Reborn: No Run-Borking Failures — Failure Explanation + Retryable Failed Runs

**Status:** approved for implementation (single PR, multi-agent parallel)
**Date:** 2026-06-12
**Owner:** firat

## 1. Problem

Users report opaque "run borking" failures from the reborn binary: runs end in
`HostUnavailable`, "operation failed", or "run failed before producing a reply"
with no usable explanation and no recovery affordance. Target end state:

> Every run-terminal error is either (a) recovered deterministically, (b)
> explained to the user — by the model when reachable, by a deterministic
> template otherwise — with partial work preserved, and (c) retryable from the
> last good checkpoint when one exists.

## 2. What already exists (build on it, do not duplicate)

| Machinery | Where | Notes |
|---|---|---|
| Recovery strategy w/ exponential backoff | `crates/ironclaw_agent_loop/src/strategies/recovery.rs` (`DefaultRecoveryStrategy`, max 2 attempts/class, `RetryAlteration::Backoff`) | Capability `OperationFailed`/`InputInvalid`/`PolicyDenied` already become model-visible tool errors. Model `Transient/Unavailable/Internal` retried then **Abort → Failed**. |
| Retry loops | `crates/ironclaw_agent_loop/src/executor/model.rs:92`, `executor/capabilities.rs:618` (outer bound 8) | Backoff applied via `apply_model_retry_alteration`. |
| Failure taxonomy | `crates/ironclaw_turns/src/loop_exit.rs:424` (`LoopFailureKind`, 13 variants, all terminal) | Wire-stable snake_case via `as_str()`. |
| Failed exit construction | `crates/ironclaw_agent_loop/src/executor/exit_helpers.rs:33` (`failed_exit`) | **Drops `state` entirely; `diagnostic_ref: None` always.** |
| Exit validation w/ evidence | `crates/ironclaw_turns/src/loop_exit.rs:97-247` (`LoopExitApplier`, `LoopExitEvidencePort::verify_failure_evidence`) | Driver claims are validated against durable evidence before transitions. |
| Failed outcome | `crates/ironclaw_turns/src/runner.rs:116` (`TurnRunnerOutcome::Failed { failure: SanitizedFailure }`) | Constructed by `validate_failed_exit` (`loop_exit.rs:805-824`). |
| Driver error mapping | `crates/ironclaw_runner/src/planned_driver.rs:222-268` (`map_executor_error`) | `HostUnavailable` → `AgentLoopDriverError::Unavailable`; model-stage diagnostics → categories. |
| Failure categories | `crates/ironclaw_runner/src/failure_categories.rs`, `model_failure_mapping.rs` | e.g. `model_credits_exhausted`, `model_credentials_unavailable`. |
| User-facing failure sentences | `crates/ironclaw_reborn_composition/src/projection/` (`FailureExplanationProvider`; tests in `projection/tests/failure_explanation.rs`) | Maps category strings → sentences ("The run failed because its runner lease expired."). |
| Blocked-run resume (template for retry) | `LoopBlocked.checkpoint_id` → `TurnCoordinator::resume_turn` (`coordinator.rs:305`) → `PlannedDriver::resume` (`planned_driver.rs:121`) | Failed runs have **no** equivalent; `resumable_checkpoint_kind_from_host` (`planned_driver.rs:313`) rejects `Final`/`BeforeSideEffect`. |
| Transcript finalize port | `crates/ironclaw_turns/src/run_profile/host.rs:1814-1840` (`LoopTranscriptPort::finalize_assistant_message` → `LoopMessageRef`) | The mechanism WS-2 uses to write the explanation message. |
| Test fixtures | `crates/ironclaw_agent_loop/src/test_support/` (`MockAgentLoopDriverHost`, `MockHostCall`, `finalized_assistant_messages()`), `crates/ironclaw_runner/tests/` | Failure injection: `fail_model_with: Option<AgentLoopHostErrorKind>`. |

## 3. Gaps this PR closes

1. **No explain-turn.** Abort paths construct `LoopExit::Failed` directly; the
   model never gets a chance to explain the failure or summarize partial work.
2. **Failure exits discard context.** `failed_exit` ignores
   `state.assistant_refs` and sets `diagnostic_ref: None`, even when
   diagnostics exist.
3. **Failed runs are dead ends.** Checkpoints are written every iteration and
   `LoopFailed.checkpoint_id` is populated, but nothing can retry from it.
4. **`HostUnavailable` bypasses everything.** Channel-send failures become
   `AgentLoopDriverError::Unavailable` with no retryable signal and no
   user-facing story beyond a stage name.
5. **Explanation coverage is partial.** `FailureExplanationProvider` covers a
   few categories; most `LoopFailureKind` values render as raw codes.

## 4. Design

### 4.1 Failure explanation (two tiers)

- **Tier 1 — model explanation (WS-2).** When the loop is about to fail with a
  kind where the model is still reachable (`CapabilityProtocolError`,
  `IterationLimit`, `PolicyDenied`, `NoProgressDetected`,
  `CompactionUnavailable`, `InvalidModelOutput`), the executor makes ONE
  best-effort constrained model call (no capability view; prompt: explain to
  the user what failed, what was completed, what they can do), finalizes the
  reply via `LoopTranscriptPort::finalize_assistant_message`, and attaches the
  resulting `LoopMessageRef` to the failed exit. Strictly best-effort: any
  error in the explanation path degrades to the current behavior and must
  never change the original `reason_kind`.
- **Tier 2 — deterministic template (WS-4).** When the model itself is the
  failure (`ModelError`, `ContextBuildFailed`) or Tier 1 degraded, the
  existing projection-layer `FailureExplanationProvider` renders the sentence.
  WS-4 extends coverage to every `LoopFailureKind` and every reborn failure
  category with an actionable sentence (what happened + what the user can do).

### 4.2 Failed exits carry evidence

`LoopFailed` gains optional, `#[serde(default)]` fields (wire-compatible with
old payloads; add legacy round-trip tests per `.claude/rules/types.md`):

```rust
pub struct LoopFailed {
    pub reason_kind: LoopFailureKind,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub usage_summary_ref: Option<LoopUsageSummaryRef>,
    pub diagnostic_ref: Option<LoopDiagnosticRef>,
    pub exit_id: LoopExitId,
    // NEW (all #[serde(default)], skip_serializing_if where empty/None):
    pub explanation_message_refs: Vec<LoopMessageRef>, // Tier-1 explanation + partial-work reply refs
    pub safe_summary: Option<SanitizedFailure>,        // finer-grained category, e.g. model_credits_exhausted
}
```

Evidence rule: `explanation_message_refs` are validated through
`LoopExitEvidencePort` exactly like completion refs (bounded, unique,
verified durable). Unverified refs are dropped (fail-closed), not fatal.
`TurnRunnerOutcome::Failed` carries the verified refs + the resume
checkpoint id so the transition port can persist retryability.

### 4.3 Retry-from-failed

Mirror the blocked-resume path. A failed run with a durable resumable
checkpoint (`BeforeModel` / `BeforeBlock` — the same set
`resumable_checkpoint_kind_from_host` accepts) is **retryable**:

- The store records `resume_checkpoint_id` on the failed run when present.
- New `TurnCoordinator::retry_turn(RetryTurnRequest)` (shape mirrors
  `ResumeTurnRequest`: scope, actor, run_id, binding refs, scoped
  idempotency key). Semantics: only valid on a `Failed` run with a recorded
  resume checkpoint; spawns a **new run** for the same turn seeded from that
  checkpoint (never mutate the failed run — LLM data is never deleted);
  re-acquires the thread active lock; idempotent per key.
- Runner claim path passes the checkpoint to `PlannedDriver::resume`.
- `AgentLoopDriverError::Unavailable` (HostUnavailable cases) handled by the
  runner worker as a failed run that is retryable when a resumable checkpoint
  exists, with `safe_summary` carrying the stage category.

### 4.4 Surfacing

- Run-failed projection frames include: failure category, explanation
  sentence (Tier 2), and `retryable: bool`.
- Tier-1 explanation messages are ordinary finalized transcript messages —
  they appear in the timeline with no special casing.
- `webui_v2` exposes a retry endpoint (`POST .../runs/{run_id}/retry`) that
  routes through the product workflow facade to `retry_turn` (per
  "everything goes through tools"/facade rules — no direct store access).

## 5. Workstreams (parallelization plan)

Dependency graph: **WS-1 → {WS-2, WS-3, WS-4} (parallel) → WS-5**.
Crate ownership is disjoint in the parallel phase; agents must not edit
outside their listed crates.

### WS-1 — Wire contracts (crates/ironclaw_turns) [blocking, small]
1. Extend `LoopFailed` per 4.2 (+ `LoopExit::failed` constructor stays
   source-compatible; add `LoopFailed::with_*` builders or a struct-literal
   update at call sites).
2. Extend `FailureEvidenceRequest`/`verify_failure_evidence` contract docs to
   cover explanation refs; extend `validate_failed_exit` to admit verified
   explanation refs into `TurnRunnerOutcome::Failed` and to carry
   `resume_checkpoint_id` (only when checkpoint verified or
   `require_final_checkpoint` is off — match existing checkpoint policy).
3. `TurnRunnerOutcome::Failed` gains `explanation_message_refs`,
   `resume_checkpoint_id`, `safe_summary` (all optional/default).
4. Add `RetryTurnRequest` DTO + `TurnCoordinator::retry_turn` trait method
   (contract only; default coordinator implementation may delegate to a new
   store-trait method — WS-3 implements stores/runner).
5. Tests first: serde legacy round-trip (old `LoopFailed` JSON without new
   fields parses; new payload with empty vec serializes identically to old),
   `validate_failed_exit` fail-closed on unverified refs, deserialization
   cannot mint verified evidence.

### WS-2 — Explain-before-fail (crates/ironclaw_agent_loop only)
1. New executor module `executor/failure_explanation.rs` owning a
   best-effort `explain_failure(ctx, &state, reason_kind) ->
   Option<LoopMessageRef>`: one `stream_model` call with an
   explanation-only prompt assembled from sanitized data already in state
   (recent failure kinds, safe summaries, iteration count — NEVER raw
   provider errors, paths, secrets per crate boundary rules), then
   `finalize_assistant_message`. Hard rules: at most one model call, no
   retries, no capability view, any `Err` → `None` + `tracing::debug!`.
   Prompt text: if a multi-line template is needed, follow the existing
   pattern for prompt files in this crate; otherwise keep it a short single
   format string.
2. Route the explainable abort paths through it: `failed_exit` gains the
   refs; populate `LoopFailed.explanation_message_refs` with the explanation
   ref + existing `state.assistant_refs` (partial work), and stop discarding
   diagnostics — thread the `LoopDiagnosticRef`/safe category from the abort
   site into `diagnostic_ref`/`safe_summary`. Model-unreachable kinds
   (`ModelError`, `ContextBuildFailed`) and host-channel failures skip Tier 1.
3. Cancellation: check `cancel_if_requested` before the explanation call;
   explanation must not delay cancellation.
4. Tests first (TDD, `MockAgentLoopDriverHost`): capability abort produces a
   finalized explanation message and a `Failed` exit whose
   `explanation_message_refs` contain it; explanation model error degrades to
   plain `Failed` with original `reason_kind`; `ModelError` abort makes no
   extra model call; cancellation pre-empts explanation; partial
   `assistant_refs` always carried.

### WS-3 — Retry-from-failed (crates/ironclaw_turns store/runner impls + crates/ironclaw_runner + concrete turn-store adapters)
1. Find the concrete `TurnRunTransitionPort`/turn store implementations
   (search workspace for implementors; both PostgreSQL and libSQL backends
   must be updated if both exist — see `.claude/rules/database.md`).
2. Persist `resume_checkpoint_id` + `safe_summary` + retryable derivation on
   failed-run transition; implement `retry_turn` (new run, same turn, seeded
   checkpoint, idempotency, thread-lock re-acquisition, only-latest-failed-run
   retryable).
3. Runner worker: driver `Err(AgentLoopDriverError::Unavailable)` →
   failed-with-retryable when a resumable checkpoint exists; map stage to a
   `safe_summary` category (extend `failure_categories.rs`, e.g.
   `host_stage_unavailable:<stage>` — keep wire-stable snake_case).
4. `PlannedDriver` unchanged except: resume request validation must accept the
   retry-spawned run context (new run_id, same turn).
5. Tests first: store contract tests for fail-with-checkpoint + retry
   (parity across backends), runner test for Unavailable → retryable failed,
   idempotent double-retry, retry without checkpoint rejected with typed error.

### WS-4 — Surfacing (crates/ironclaw_reborn_composition + crates/ironclaw_product_workflow + crates/ironclaw_webui_v2)
1. `FailureExplanationProvider`: full coverage table — every
   `LoopFailureKind::as_str()` value + every `failure_categories.rs` constant
   + `host_stage_unavailable:*` → actionable sentence (what happened, what to
   do). Unknown categories get a safe generic sentence (never echo raw codes
   alone).
2. Failed-run projection frame carries `retryable` + explanation sentence;
   wire through product workflow facade types
   (`RebornServicesErrorKind`-style stable enums, snake_case).
3. `webui_v2`: retry endpoint routed through the product workflow facade
   (follow the existing gate-resolve handler as the template); error body
   includes `retryable`. Frontend: minimal — only if an existing failed-run
   status component exists, add the retry action; otherwise leave UI for a
   follow-up and keep this API-only.
4. Tests first: projection test in the style of
   `projection/tests/failure_explanation.rs` asserting category → sentence +
   retryable; webui_v2 handler test for retry endpoint (happy, non-retryable,
   idempotent-replay).

### WS-5 — Integration tests + gate (crates/ironclaw_runner/tests + workspace)
End-to-end scenarios through the runner/driver/composition stack:
1. Transient model error → invisible recovery → `Completed` (no failure
   artifacts).
2. Hard model failure → `Failed` with category, retryable, Tier-2 sentence in
   projection; retry endpoint spawns a new run that resumes from checkpoint
   and completes.
3. Capability protocol failure → Tier-1 explanation message finalized and
   visible in transcript refs; run failed with explanation refs verified.
4. HostUnavailable (capability stage) → retryable failed with
   `host_stage_unavailable:capability` category.
Quality gate: `cargo fmt`, `cargo clippy --all --benches --tests --examples
--all-features` (zero warnings), `cargo test`. Pre-commit safety script must
pass.

## 6. Invariants (all workstreams)

- No `.unwrap()`/`.expect()` in production code; `thiserror` for new errors;
  no silent `unwrap_or_default()` on store/IO results.
- State/exit types store refs + sanitized summaries only — never raw model
  output, provider errors, or paths (agent_loop crate boundary).
- Wire enums snake_case + `#[serde(default)]` for new fields on
  `deny_unknown_fields` structs; legacy round-trip tests mandatory.
- LLM data is never deleted: retry creates a new run; failed runs and their
  transcripts are retained untouched.
- Explanation is best-effort everywhere: no new failure mode may be
  introduced by the explanation/retry machinery itself.
- Crate dependency directions unchanged (agent_loop must not depend on
  reborn/product crates; turns stays neutral contracts).

## 7. Acceptance criteria

1. A run failing with `CapabilityProtocolError` after retries shows a
   model-written explanation message in the timeline and is retryable.
2. A run failing with `ModelError`/credits-exhausted shows an actionable
   deterministic sentence (not a bare code) and is retryable when a
   checkpoint exists.
3. `HostUnavailable` no longer surfaces as an opaque stage name: it is a
   categorized, retryable failure with a sentence.
4. Every `LoopFailureKind` has a Tier-2 sentence (exhaustive-match test so new
   variants can't ship without one).
5. Retry of a failed run resumes from the last resumable checkpoint in a new
   run; double-retry with the same idempotency key is a no-op.
6. Zero clippy warnings; all existing tests still pass; new tests cover each
   bullet above.
