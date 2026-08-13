# Reborn Contract — Loop Exit Handshake

**Status:** Contract-freeze draft  
**Date:** 2026-05-06  
**Depends on:** [`turn-runner.md`](turn-runner.md), [`turn-persistence.md`](turn-persistence.md), [`turns-agent-loop.md`](turns-agent-loop.md)

---

## 1. Purpose

`LoopExit` is the driver-facing claim returned by an agent-loop attempt after a runner has already claimed a turn run. It is not durable run state and it is not trusted by itself.

`TurnRunner` validates `LoopExit` evidence before translating it to a trusted `TurnRunnerOutcome`. Invalid exits that cannot be proven safe to terminalize are mapped to `TurnRunnerOutcome::Failed` with a stable sanitized category. `RecoveryRequired` is a legacy compat status; see §5. Syntactically valid refs are not evidence by themselves; the host/runner must verify referenced transcript, result, checkpoint, and gate records before trusting an exit.

This prevents unsafe state changes such as releasing the active-thread lock after a driver says `Completed` without durable transcript/result refs, or blocking a run without a durable checkpoint and gate reference.

---

## 2. Boundary

```text
AgentLoopDriver
  -> LoopExit claim
  -> TurnRunner validates evidence/policy
  -> TurnRunnerOutcome
  -> ProcessTransitionPort
```

`LoopExit` carries typed metadata, bounded host-minted references, and
sanitized summaries; it never carries raw payloads. For example, `LoopFailed`
contains a typed `reason_kind`, optional checkpoint and model-usage metadata,
bounded exit/explanation references, and an optional sanitized `safe_summary`.
It must not carry raw prompts, assistant text, tool inputs, approval payloads,
secrets, host paths, provider errors, stack traces, or raw runtime output.
Loop-owned refs use tight host-minted opaque prefixes (`exit:`, `msg:`,
`result:`, `gate:`, `usage:`) to avoid accepting free-form payload text as
evidence.

For rolling compatibility, `LoopFailed` still accepts and ignores the retired
`diagnostic_ref` JSON field (including the historical `null` form), but never
serializes it. Older readers treated that field as optional, so exits written
without it remain readable during rollback; no store migration is required.

`crates/kernel/ironclaw_turns/src/loop_exit/tests/mod.rs` test
`loop_exit::tests::loop_failed_accepts_retired_diagnostic_ref_but_does_not_serialize_it`
owns this rolling-compatibility assertion, including both the historical string
and `null` forms. Run:

```bash
cargo test -p ironclaw_turns --lib loop_exit::tests::loop_failed_accepts_retired_diagnostic_ref_but_does_not_serialize_it -- --exact
```

---

## 3. Exit variants

The driver-facing variants are fixed for the MVP:

| Variant | Meaning | Trusted mapping after validation |
| --- | --- | --- |
| `Completed` | Loop reached a terminal user-visible or result-producing boundary. | `TurnRunnerOutcome::Completed` |
| `Blocked` | Loop stopped at an approval/auth/resource gate with a safe resume checkpoint. | `TurnRunnerOutcome::Blocked` |
| `Cancelled` | Loop observed a host cancellation/interrupt and stopped safely. | `TurnRunnerOutcome::Cancelled` |
| `Failed` | Loop stopped because of a stable sanitized failure category. | `TurnRunnerOutcome::Failed` |

`RecoveryRequired` is intentionally not a normal driver return. It is a legacy compat status retained for backward-compat deserialization of persisted rows; new invalid-exit handling always maps to `TurnRunnerOutcome::Failed`.

---

## 4. Evidence requirements

- `Completed` requires at least one durable reply-message ref or result ref, and the host/runner must verify those refs exist before mapping to a trusted completed outcome. Raw reply text is rejected by the wire shape and by strict loop-ref grammar.
- `Completed.completion_kind` distinguishes the completion artifact: `FinalReply` is backed by reply-message refs, `ResultOnly` is backed by result refs without a finalized assistant reply, `DelegatedResult` is backed by delegated subtask result refs, and `NoReply` remains profile-gated for exits without durable reply/result refs.
- `Completed` requires `final_checkpoint_id` only when the resolved run profile/checkpoint policy requires a terminal checkpoint.
- `Blocked` requires all of: blocked kind, durable `gate_ref`, `checkpoint_id`, and opaque `state_ref`, and the host/runner must verify the gate/checkpoint evidence before mapping to a trusted blocked outcome. The blocked kind is limited to approval, auth, and resource for MVP.
- `Cancelled` is accepted only when the host cancellation/interrupt input was observed by the runner/host policy. Host-initiated cancellation may preempt the driver before a final checkpoint exists, so cancellation validation does not require a missing final checkpoint to become a protocol violation. During application, terminal cancellation is still gated by durable run state in one transition-port operation: if the run is already `CancelRequested`, it becomes `Cancelled`; if an interrupt is observed before that durable state exists, the exit maps to recovery instead of terminal cancellation.
- `Failed` uses stable sanitized failure kinds such as `iteration_limit`, `model_error`, `context_build_failed`, or `driver_bug`, and the host/runner must verify the failure evidence is safe to terminalize before mapping to a trusted failed outcome. Failed exits may also carry bounded `explanation_message_refs` and an optional sanitized `safe_summary`; validated failed outcomes surface only host-verified explanation refs, prefer `safe_summary` over the generic failure kind, and carry a resume checkpoint id only when the checkpoint policy admits it.
- Ref lists are bounded and duplicate-free so a driver cannot force unbounded evidence verification work.
- Usage/cost truth remains in host accounting/projection stores; `LoopExit` may carry only usage-summary refs.

### 4.1 Checkpoint rejection before a trustworthy exit

`CheckpointRejected` is the explicit exception to the model receiving a final
word. A rejection while writing a pre-model checkpoint means the driver cannot
produce a trustworthy `LoopExit` and must not run a model or capability from
the uncheckpointed state. The staged private payload is not a resume point;
only committed checkpoint metadata is authoritative.

The runner preserves the distinct `checkpoint_rejected` category and records a
bounded host-authored terminal explanation through the independent turn-state
row and `Failed` lifecycle event. Product projection revalidates that
host-authored envelope and never asks a failure-explainer model to paraphrase
it. No assistant transcript message is created, no partial success is emitted,
and the rejected run is not retryable. The explanation directs the user to
start a new run and the operator to inspect checkpoint storage and run-profile
compatibility.

This contract is pinned by
`executor_checkpoint_rejection_maps_to_host_authored_terminal_explanation` in
`crates/loop/ironclaw_turn_runner/src/planned_driver.rs` (a rejected
checkpoint maps to the bounded host-authored terminal explanation) and by
`retry_rejects_checkpoint_rejection_without_creating_a_process` in
`crates/kernel/ironclaw_turns/src/process_projection/tests.rs` (the rejection
stays terminal after projection into the process journal and cannot create a
retry process; that selector is also mapped into
`scripts/reborn-e2e-rust.sh`). The former `loop_driver_host`
integration-test target and its
`turn_runner_worker_persists_checkpoint_rejection_without_running_uncheckpointed_work`
test were deleted in #6696, not moved:

```bash
cargo test -p ironclaw_turn_runner --lib \
  executor_checkpoint_rejection_maps_to_host_authored_terminal_explanation
cargo test -p ironclaw_turns --lib \
  retry_rejects_checkpoint_rejection_without_creating_a_process
```

---

## 5. Invalid exit handling

Validation always produces a redacted decision:

- Invalid exits map to `TurnRunnerOutcome::Failed` with a stable sanitized category such as `driver_protocol_violation` or `interrupted_unexpectedly`. The active-thread lock is released on the Failed terminal transition.
- `LoopExitMapping::RecoveryRequired` is a compat shim retained for deserialization of legacy stored rows; it is treated as terminal Failed by the transition port and no longer keeps the active lock held.

Initial validation covers:

- completed exits missing durable completion refs;
- completed exits whose refs have not been verified by host evidence;
- terminal exits missing a required final checkpoint;
- blocked exits whose checkpoint/gate evidence has not been verified by host evidence;
- failed exits whose failure evidence has not been verified safe to terminalize by host evidence;
- cancelled exits without observed host cancellation/interrupt.

Later slices may add validation against transcript draft state, checkpoint freshness, event evidence, usage-summary refs, and idempotent exit replay storage.

---

## 6. Implemented slice

The claim vocabulary and the authority that validates it live in two crates,
and the split is the contract: a driver can only ever hold the claim half.

`ironclaw_loop_contracts` (contracts layer) provides the claim types:

- `LoopExit`, `LoopCompleted`, `LoopBlocked`, `LoopCancelled`, `LoopFailed`;
- bounded durable reference types for loop exit/message/result/usage refs.

`ironclaw_turns` (kernel) provides the validator policy and the trusted
runner-side applicator, and depends on the contracts crate — never the reverse:

- `LoopExitEvidencePort` and evidence request DTOs for host-owned validation inputs;
- crate-private `LoopExitValidationPolicy` construction plus public `LoopExitValidationDecision`;
- one-way mapping to `TurnRunnerOutcome` (invalid exits always map to Failed; valid failed outcomes may carry verified explanation refs and a retry checkpoint id; `LoopExitMapping::RecoveryRequired` is a backward-compat shim);
- `LoopExitApplier`, which derives validation policy from host-owned evidence
  and invokes `ProcessTransitionPort` with the neutral process outcome or
  suspension produced from the validated exit.

Driver-facing code must not be able to supply `LoopExitValidationPolicy`
directly. Agent-loop validation remains outside the process kernel; only its
validated lifecycle result crosses the process transition port.

This slice deliberately does not wire durable exit-id idempotency storage, transcript draft validation, or product service-graph integration.
