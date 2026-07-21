# LoopFailureKind fault matrix — coverage, injection seams, and branch-vs-main delta

**Status:** synthesized from 4 parallel research passes (taxonomy/origins, coverage audit,
main delta, injection seams) · **Date:** 2026-07-03
**Scope:** every `LoopFailureKind` variant (`crates/ironclaw_turns/src/loop_exit.rs` ~440),
the Part-1 "no-borking failures" contract, and what `origin/main` (f92c658c9) does instead.

## 0. Contract properties (per failing run)

- **P1** sanitized failure category surfaces (`TurnRunState.failure` / `sanitized_reason` / projection)
- **P2** correct `retryable` (resume checkpoint preserved ⇒ retryable; else refused)
- **P3** no fabricated final assistant reply
- **P4** `retry_run` resumes to completion when retryable; refused (`RunNotRetryable`) when not

## 1. Structural facts the matrix hinges on

1. **Retryability is variant-agnostic.** No production code branches retry on the variant.
   `retryable` = "does a `BeforeModel`/`BeforeBlock` checkpoint exist for the run"
   (`memory.rs::fail_claimed_record` → `latest_resumable_loop_checkpoint`). The applier's
   `resume_checkpoint_id` is always `None` in production (test-only setter), so the store
   fallback scan is THE mechanism. The only `BeforeModel` writer is `prompt.rs:535`
   (once per iteration, before each model call).
2. **Two assertion vocabularies.** Executor tier asserts `LoopExit::Failed(f).reason_kind`
   (the enum); binary/driver tier asserts the sanitized **string** after
   `map_executor_error`/`map_host_error` (lossy: several host kinds collapse to
   `model_error`/`driver_bug`). Each matrix row declares its layer.
3. **Two variants have no production constructor** (`ContextBuildFailed`,
   `InterruptedUnexpectedly` at the enum level) — enum + name-table entries only.
   `interrupted_unexpectedly` *as a string* is reachable via `map_executor_error` on an
   in-flight `Cancelled` host error. Honest coverage = wire/category-table locks +
   documented unreachability; do NOT fabricate the enum in tests.
4. **Legacy-only origins.** `TranscriptWriteFailed` and `CheckpointRejected` originate in
   the legacy `text_loop_driver` (`CheckpointRejected` also via host run-not-active
   mapping / `fail_checkpoint` → `map_executor_error`).
5. **Explainable set (in-loop model explanation attached, branch only):**
   `CapabilityProtocolError, IterationLimit, PolicyDenied, NoProgressDetected,
   CompactionUnavailable, InvalidModelOutput` (`failure_explanation.rs:232-237`).

## 2. Branch vs main (per-run user experience)

Identical on both: the 13-variant taxonomy, `as_str()` categories, `sanitized_reason` on
events, `RecoveryRequired` modeling. Main also has a **lazy, category-generic** explanation
blurb at projection time (`FailureExplanationProvider`, input = category + fallback only).

Branch-only (#4841): **in-loop run-context-aware explanation** persisted as a real
transcript message (6 explainable kinds; bounded 10s model call, before Final checkpoint);
`LoopFailed.explanation_message_refs` + `safe_summary`; and the **entire retry path**
(`RebornServices::retry_run` → webui_v2 `WEBUI_V2_ROUTE_RETRY_RUN` → coordinator
`retry_turn` → resume from latest resumable checkpoint). Main has **no retry-from-failed
at all** — every failure is terminal for the user.

Stack note: this branch now sits on top of #5389 and #5390. #5389 changes
model-fixable capability failures into model-visible tool errors that continue
the loop instead of terminating it; the matrix asserts those recovered outcomes
directly. #5390 adds the `FailureLane` / `RetryDisposition` classifiers keyed on
the sanitized category string plus the run's `retryable` signal; this matrix PR
does not own that classifier, but proves real failure paths emit the inputs that
feed it.

## 3. The matrix

Layers: **E** = executor (`ironclaw_agent_loop` tests, MockHost), **B** = binary/driver
(`tests/support/reborn` harness or planned_driver tests). "≡" = covered by the shared
variant-agnostic mechanism (checkpoint-presence retry contracts + store/runner tests).

| Variant | Origin (stage) | Trigger seam | Layer | P1 today | P3 today | Explained (branch) | Main behavior | Gap → action |
|---|---|---|---|---|---|---|---|---|
| ModelError | model stage; recovery Abort | `with_model_errors([...])` | E+B | tables + e2e | e2e + executor | no | category only, terminal | none (reference row) |
| ContextBuildFailed | **no producer (dead)** | — | — | tables only | NONE | no | same (dead) | unreachable-row: category-table lock + doc; no fabricated test |
| CapabilityProtocolError | capability stage; recovery | `fail_batch_with(protocol kind)` | E | tables | 8× executor | **yes** | category only, terminal | matrix row (consolidate) |
| IterationLimit | budget stage | `family_with_iteration_limit(n)` | E | tables + unit | executor | **yes** | category only, terminal | matrix row |
| InvalidModelOutput | stop strategy | rejected-reply threshold / `RejectAlways` | E | tables | executor | **yes** | category only, terminal | matrix row |
| CheckpointRejected | checkpoint stage err / legacy | `fail_checkpoint(kind)` → `checkpoint_rejected` string | E(kind)+B(string) | tables | NONE | no | category only, terminal | **new row + P3 assert** |
| CheckpointUnavailable | PlannedDriver resume decode | corrupt/missing resume payload | **B only** | comp table only | NONE | no | category only, terminal | **new row + add to turns table** |
| TranscriptWriteFailed | legacy driver map_host_error | **new knob:** `fail_transcript_with` (S) | E | tables | NONE | no | category only, terminal | **new seam + row + P3 assert** |
| DriverBug | invariant guards (6+ origins) | canonical: `family_with_gate_outcome(SkipAndContinue)` on Approval gate | E | tables + evidence tests | via mapping | no | category only, terminal | matrix row (one canonical origin) |
| InterruptedUnexpectedly | enum: no producer; string via in-flight Cancelled | `with_model_errors([kind: Cancelled])` → `map_executor_error` | B(string) | tables + violation path | NONE | no | same mapping exists | **string-level row**; enum documented dead |
| NoProgressDetected | stop strategy repetition | repeated identical calls/errors | E | tables + projection | executor + safety_nets | **yes** | category only, terminal | matrix row |
| PolicyDenied | capability denials; gate abort | `with_batch_outcomes(Denied)` | E | tables + unit | executor | **yes** | category only, terminal | matrix row |
| CompactionUnavailable | prompt/compaction stage | `set_compaction_outcome(Err)` | E | comp table only | executor happy-path | **yes** | category only, terminal | **add to turns table** + matrix row |

Additional #5389 recovery rows live in the executor matrix: capability
`Failed(InvalidInput)` (planned terminal kind `ModelError`), `Failed(InvalidOutput)`
(planned `CapabilityProtocolError`), and `Failed(PolicyDenied)` (planned
`PolicyDenied`) now complete after the model receives a tool-error result.

P2/P4 (all variants): ≡ shared — `retry_failed_turn_store_contract.rs` (both directions,
incl. explicit-nonresumable refusal) + `turn_runner` retryable-mapping tests + the ModelError
E2E (`reborn_failure_retry_resume_e2e.rs`). The matrix adds **one more binary-level P4 row**
(capability-stage failure → retry resumes) so P4 isn't proven only through the model stage.

## 4. Known one-off gaps/status

- RESOLVED in the matrix: `all_failure_kinds_produce_stable_sanitized_category_strings`
  now includes `CheckpointUnavailable` + `CompactionUnavailable` and has an
  exhaustive-match guard so a new `LoopFailureKind` variant fails compilation.
- STILL OPEN: legacy `text_loop_driver` private name map omits
  `CompactionUnavailable` (drift risk).
- Prefer scripted-outcome injection over enum construction everywhere: the mapping logic
  (`executor/mapping.rs`, `capability_failure_kind`, recovery) is itself under test.
- Cancellation: cooperative cancel (`request_cancellation`) yields `LoopExit::Cancelled`
  (not a failure) — matrix must use in-flight `Cancelled` errors, never the cancel knobs.

## 5a. Divergences found by the executor matrix (asserted as actual behavior)

The table-driven test (`executor/tests/failure_matrix.rs`) asserts what the code
DOES; these divergences from the original expected rows are documented, not
silently fixed:

1. **CLOSED (2026-07-21) — Approval gate + `SkipAndContinue` now fails as
   `DriverBug`.**
   `GateStage` enforces `GateOutcome::validate_for_gate_kind`
   (`enforce_gate_outcome_contract`, executor `gates.rs`): an invalid
   `SkipAndContinue` on an Approval/ExternalTool gate is downgraded to
   `Abort { DriverBug }` and the run fails through the standard abort path
   (matrix row asserts the `Failed(DriverBug)` terminal). REMAINING for owner
   review: `AwaitDependentRunGateStage` is deliberately NOT enforced — the
   executor test
   `await_dependent_run_gate_skip_and_continue_accumulates_byte_len` pins its
   skip arm as reachable for custom gate resolvers deriving outcomes from
   external policy, contradicting the validator's declaration for that kind.
2. **CLOSED (2026-07-21) — `NoProgressDetected` now gets its failure
   explanation.**
   The `StopKind::NoProgressDetected` failed branch calls
   `attach_failure_explanation` (same path as other explainable kinds) after
   the final-answer nudge declines and before the Final checkpoint; the
   explanation ref rides `LoopFailed.explanation_message_refs`. The
   PinchBench-load-bearing nudge is untouched: a successful nudge still
   completes with no explanation call, and the explanation is best-effort
   (fails soft, bounded 10 s).
3. **RESOLVED BY STACK #5389 — model-fixable capability failures recover and
   complete.** A single `Denied` outcome is fed back to the model as a tool
   error, and the stack now does the same for capability
   `Failed(InvalidInput)`, `Failed(InvalidOutput)`, and
   `Failed(PolicyDenied)`. Terminal `ModelError` / `CapabilityProtocolError` /
   `PolicyDenied` still exist for true run-ending paths or exhausted recovery,
   but these model-fixable rows are intentionally recovered. The matrix labels
   those rows "stack #5389 makes this recoverable; matrix asserts recovery".
4. **STILL OPEN / legacy-only — `TranscriptWriteFailed` / `CheckpointRejected`
   never surface as
   `LoopExit::Failed` in the planned executor** — they surface as executor
   host-stage errors (`HostUnavailable{Transcript}` / `CheckpointFailed`) which
   the runner maps to retryable host-stage failures. The enum origins are
   legacy-`text_loop_driver`-only, confirming §1.4.

5. **CLOSED (2026-07-21) — `interrupted_unexpectedly` is preserved at the
   runner boundary.**
   `sanitized_driver_failure` (`ironclaw_runner::turn_runner`) now preserves
   the driver-mapped `interrupted_unexpectedly` category instead of
   overwriting it with `driver_failed`; the binary E2E
   (`reborn_inflight_model_cancelled_preserves_interrupted_unexpectedly`)
   locks the durable run failure end-to-end. Note: other driver reason kinds
   (`driver_bug`, `checkpoint_rejected`) are still collapsed to
   `driver_failed` by the same allowlist — unchanged here, candidate
   follow-up.

6. **CLOSED (2026-07-21) — loop-exit violation kinds now survive on the
   durable failure record.** Rejected `LoopExit` claims used to collapse all
   eight `LoopExitViolationKind`s into the bare `driver_protocol_violation` /
   `interrupted_unexpectedly` categories. `invalid_exit_decision`
   (`ironclaw_turns::loop_exit`) now persists the specific kind as the
   sanitized failure `detail` (`"loop exit violation: <kind>"`), so the run
   record, `TurnLifecycleEvent.detail`, and the failure explainer keep WHICH
   protocol rule was broken while the wire-stable category set is unchanged.

## 6. Relationship to #5390 FailureLane

#5390 owns the pure classifier unit tests: `failure_lane(category, retryable)`
and `retry_disposition(category, retryable)` are tested exhaustively against the
canonical category list in `ironclaw_reborn_composition`.

This matrix is complementary. It drives real executor/binary paths and proves
the emitted sanitized categories and retryable checkpoint signal are the inputs
the classifier expects. The actual `failure_lane()` / `retry_disposition()`
calls live in the binary E2E test because `ironclaw_agent_loop` has no dependency
on `ironclaw_reborn_composition`, and adding that dependency would be outside
this test/doc-only change. Executor-tier rows still assert their real terminal
or recovered loop outcome; binary rows assert the classifier alignment for the
observed `(category, retryable)` pair.

## 7. Definition of 100% coverage (acceptance)

Every variant has exactly one of:
(a) a real-origin injection row asserting P1+P3 at its declared layer (+ shared P2/P4), or
(b) an unreachable-variant row: category-table lock + `#[non_exhaustive]`-safe exhaustive
    guard + doc note that no production constructor exists (ContextBuildFailed;
    InterruptedUnexpectedly at enum level — string covered by (a) at binary layer).
Plus: the turns-crate category table made exhaustive+guarded, and one non-model P4 E2E.
