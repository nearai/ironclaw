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
  -> TurnStateStore transition
```

`LoopExit` contains references only. It must not carry raw prompts, assistant text, tool inputs, approval payloads, secrets, host paths, provider errors, stack traces, or raw runtime output. Loop-owned refs use tight host-minted opaque prefixes (`exit:`, `msg:`, `result:`, `gate:`, `usage:`, `diag:`) to avoid accepting free-form payload text as evidence.

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
- `Failed` uses stable sanitized failure kinds such as `iteration_limit`, `model_error`, `context_build_failed`, or `driver_bug`, and the host/runner must verify the failure evidence is safe to terminalize before mapping to a trusted failed outcome.
- Ref lists are bounded and duplicate-free so a driver cannot force unbounded evidence verification work.
- Usage/cost truth remains in host accounting/projection stores; `LoopExit` may carry only usage-summary refs.

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

`ironclaw_turns` currently provides contract types, a crate-private validator policy, and a trusted runner-side applicator:

- `LoopExit`, `LoopCompleted`, `LoopBlocked`, `LoopCancelled`, `LoopFailed`;
- bounded durable reference types for loop exit/message/result/usage/diagnostic refs;
- `LoopExitEvidencePort` and evidence request DTOs for host-owned validation inputs;
- crate-private `LoopExitValidationPolicy` construction plus public `LoopExitValidationDecision`;
- one-way mapping to `TurnRunnerOutcome` (invalid exits always map to Failed; `LoopExitMapping::RecoveryRequired` is a backward-compat shim);
- `LoopExitApplier`, which derives validation policy from host-owned evidence and invokes the trusted `TurnRunTransitionPort` with an already-validated `LoopExitMapping`.

`ApplyValidatedLoopExitRequest` remains the transition-port DTO for already-validated mappings. Driver-facing code must not be able to supply `LoopExitValidationPolicy` directly.

This slice deliberately does not wire durable exit-id idempotency storage, transcript draft validation, or product service-graph integration.
