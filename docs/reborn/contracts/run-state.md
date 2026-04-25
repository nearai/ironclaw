# IronClaw Reborn run-state contract

**Date:** 2026-04-25
**Status:** Draft contract
**Depends on:** `docs/reborn/contracts/runtime-workflows.md`, `docs/reborn/contracts/host-api.md`

---

## 1. Purpose

Run state is the live lifecycle of work on a thread. It is not the transcript, not the realtime event stream, not a projection, and not a runtime lane.

The contract exists to make blocked/resume/cancel behavior explicit before conversation, agent-loop, jobs, and subagent implementations grow around implicit state.

---

## 2. Core invariant

```text
one active run per thread
```

A thread may have durable history and many completed runs, but only one run may be active or blocked at a time.

If another request arrives while a run is active, the system must choose an explicit behavior:

- reject
- enqueue
- interrupt then replace
- attach as input to the active run, if the active run type supports it

It must not silently start a second active run on the same thread.

---

## 3. State model

Minimum V1 states:

| State | Meaning |
| --- | --- |
| `idle` | No active run on the thread |
| `running` | Run is actively executing or waiting on model/runtime work |
| `blocked_approval` | Run is paused on a structured approval request |
| `blocked_auth` | Run is paused on a structured auth flow |
| `interrupted` | Run is paused by operator/system interruption and can resume if checkpoint permits |
| `cancelling` | Cancellation requested; runtime cleanup is in progress |
| `completed` | Run ended successfully |
| `failed` | Run ended with an error |
| `cancelled` | Run ended by cancellation |

Terminal states:

```text
completed | failed | cancelled
```

Blocked states are active states. They still occupy the one-active-run slot.

---

## 4. Transition sketch

```text
idle -> running
running -> blocked_approval
running -> blocked_auth
running -> interrupted
running -> cancelling
running -> completed
running -> failed
blocked_approval -> running | failed | cancelling
blocked_auth -> running | failed | cancelling
interrupted -> running | cancelled
cancelling -> cancelled | failed
terminal -> idle for the next run
```

Invalid transitions should fail closed and emit diagnostic events.

---

## 5. Approval-blocked vs auth-blocked

Approval and auth both pause work, but they are different gates.

| Gate | Owner | Resume input | Durable record |
| --- | --- | --- | --- |
| Approval | `ApprovalManager` | approve / deny / always with explicit reusable scope | approval request/resolution + audit |
| Auth | `AuthFlowManager` | OAuth callback/token completion/credential availability | auth flow record + secret lease/audit |

Rules:

- approval prompts do not collect raw secrets
- auth prompts do not imply user approval for the blocked action
- both gates must resume from a stable checkpoint or retry descriptor
- both gates must be replay-safe after process restart

---

## 6. Ownership boundaries

| Component | Owns | Must not own |
| --- | --- | --- |
| `RunStateManager` | current run id, state, transition validation, cancel/interrupt/resume, checkpoint references | transcript text, approval semantics, auth callbacks, runtime execution |
| `ConversationManager` | durable thread records and transcript milestones that reference run ids/gates | live run transition authority |
| `ApprovalManager` | approval requests and decisions | thread lifecycle or auth flow completion |
| `AuthFlowManager` | auth-required state and retry-after-auth | approval decisions or raw secret storage |
| `RuntimeDispatcher` | dispatch handoff to runtime lanes | run-state ownership |
| `EventStreamManager` | publishing state changes | transition authority |

---

## 7. Checkpoints and resume

A resumable blocked run needs a checkpoint containing enough structured data to continue without guessing from chat text.

Minimum checkpoint fields:

- run id
- thread id
- invocation id
- blocked action or retry descriptor
- scope
- capability id or runtime operation
- correlation id
- gate id, if approval/auth blocked

Checkpoint records must not contain:

- raw secrets
- raw host paths
- unredacted request payloads that policy forbids storing
- model-visible free-form authority grants

---

## 8. Event requirements

Every transition emits a typed event:

```text
run_started
run_blocked_approval
run_blocked_auth
run_interrupted
run_resumed
run_cancelling
run_completed
run_failed
run_cancelled
```

Events are used for live streams and projections. Durable audit/history decides which events or derived records become permanent.

---

## 9. Non-goals

This contract does not define:

- prompt assembly
- LLM provider routing
- approval UI
- OAuth provider mechanics
- process lifecycle internals
- transcript storage format
- event transport protocol

Those belong to neighboring service contracts.
