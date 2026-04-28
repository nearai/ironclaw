# IronClaw Reborn runtime workflow ownership contract

**Date:** 2026-04-25
**Status:** Draft contract
**Depends on:** `docs/reborn/2026-04-24-os-like-architecture-design.md`, `docs/reborn/2026-04-24-os-like-architecture-feedback.md`, `docs/reborn/contracts/agent-loop-protocol.md`
**Default loop mechanics:** `docs/reborn/contracts/lightweight-agent-loop.md`

---

## 1. Purpose

The OS-like architecture should be validated by workflows, not only by crate names.

For every core runtime workflow, the architecture must identify:

- services touched
- owner of each step
- scope carried through the step
- durable state touched
- ephemeral/live state touched
- interface contract invoked
- what the service must not own

This prevents a new hidden agent runtime from forming inside `ironclaw_kernel`, `ironclaw_extensions`, or a future conversation crate.

---

## 2. Logical services

These are logical service contracts. They do not all need to become crates immediately.

| Service | Owns | Must not own |
| --- | --- | --- |
| `TransportAdapter` | Normalizing browser/channel/webhook/IDE ingress and adapting events back to transport-specific output | Business policy, authorization, prompt assembly, durable transcript semantics |
| `ConversationManager` | Durable thread lifecycle, transcript/history reads, references to pending gates and run ids | Live stream fanout, approval semantics, auth semantics, runtime execution |
| `ScopeManager` | Resolving tenant/user/project/agent/thread/invocation scope and producing typed views | Policy decisions, prompt text, filesystem implementation |
| `InstructionBundleAssembler` | Deterministic prompt/instruction bundles from identity, context, skills, visible capabilities, attachments, and runtime metadata | Tool execution, authorization, long-lived mutable state |
| `CapabilityCatalog` | Capability descriptors, provider mapping, schema metadata, model-visible names/descriptions | Action-time authorization, runtime execution, resource accounting |
| `CapabilityAccessManager` | Visible capability surface, grants, scope filtering, action-time authorization inputs | Capability execution, manifest parsing, model prompting |
| `ApprovalManager` | Approval requests, stable request ids, approve/deny/always decisions, replay-safe resolution | Chat transcript ownership, auth flows, runtime execution |
| `AuthFlowManager` | Auth-required state, OAuth/token prompting, callback completion, retry-after-auth | Approval semantics, raw secret display, runtime dispatch |
| `RunStateManager` | One-active-run-per-thread, blocked states, cancel/interrupt/resume, terminal transitions, checkpoints | Transcript storage, stream delivery, capability execution |
| `RuntimeDispatcher` | Runtime lane selection and fail-closed handoff to configured backends | Manifest discovery, runtime-specific implementation, product workflows |
| `EventStreamManager` | Realtime delivery, event ids, reconnect semantics, keepalives, fanout | Durable audit/history ownership, business policy |
| `ProjectionReducer` | Derived read models for sidebar/activity/progress/job/project/harness views | Durable source-of-truth state, external side effects |

---

## 3. Parent model protocol

The default parent agent loop protocol is:

```text
Reply | CapabilityCalls
```

Where:

- `Reply` = user-visible assistant output for the active thread. Loop implementations may internally classify a reply as `FinalReply` vs `AskUser`, but that classification stays loop-owned metadata rather than becoming a new host-wide protocol branch.
- `CapabilityCalls` = one or more explicit capability invocations against the visible capability surface. Provider-native tool calls are normalized into this host contract before action-time authorization and dispatch.

The parent model can reply or request explicit capabilities. It cannot switch the entire engine protocol into an ad hoc execution mode.

CodeAct, Monty, scripting, delegation, or similar modes are worker paths behind explicit capabilities:

```text
spawn_subagent(mode = "codeact") -> child thread owned by parent run
create_job(mode = "codeact")     -> standalone/background thread with output sink
script.run(...)                   -> explicit capability call
```

This keeps the parent boundary simple, preserves `RuntimeDispatcher` as the capability execution boundary, and still allows loop implementations to keep stronger internal typing.

---

## 4. Interactive chat turn

```text
TransportAdapter
-> ConversationManager
-> ScopeManager
-> RunStateManager.begin
-> InstructionBundleAssembler
-> CapabilityAccessManager.visible_capabilities
-> LLM(Reply | CapabilityCalls)
-> CapabilityAccessManager.authorize(action-time)
-> RuntimeDispatcher.dispatch_json
-> ConversationManager persist transcript milestones
-> EventStreamManager publish live events
-> ProjectionReducer update read models
-> RunStateManager.complete | blocked | failed
```

Key rules:

- scope, instruction, and visible-capability snapshots are warm-path snapshots
- action-time authorization is still required for every capability call
- live progress is not the durable transcript
- kernel wires these services but does not own the chat workflow

---

## 5. Approval-blocked capability

```text
CapabilityCall
-> CapabilityAccessManager.authorize
-> Decision::RequireApproval
-> ApprovalManager.open_pending_gate
-> RunStateManager.blocked(approval)
-> EventStreamManager publish approval_needed
-> user approve/deny/always
-> ApprovalManager.resolve
-> RunStateManager.resume | fail
```

Key rules:

- approval is a structured run-state transition, not generic chat text
- reusable approval scopes must be explicit
- approval resolution is auditable
- approval-blocked is distinct from auth-blocked

---

## 6. Auth-blocked capability

```text
CapabilityCall
-> runtime/service reports auth required
-> AuthFlowManager.begin
-> RunStateManager.blocked(auth)
-> TransportAdapter presents auth URL/token prompt
-> callback/token completion
-> SecretLeaseManager records scoped lease
-> RunStateManager.resume
-> retry original action or continue from checkpoint
```

Key rules:

- auth-required state is not approval-required state
- raw secrets are never written to model-visible output or transcript text
- retry-after-auth must be replay-safe
- secret leases are scoped and auditable

---

## 7. Extension activation changing visible capabilities

```text
Extension activation/change
-> ExtensionRegistry updates package/capability catalog
-> CapabilityCatalog refreshes descriptors
-> CapabilityAccessManager invalidates visible-capability snapshot
-> InstructionBundleAssembler rebuilds only if model-visible capability text changed
-> EventStreamManager publishes capability_surface_changed
```

Key rules:

- visible capability surface is not action authorization
- action-time authorization still checks grants/policy/scope
- manifest parsing/discovery must not execute runtime code

---

## 8. Reconnect and live stream resume

```text
client reconnects with last_event_id
-> EventStreamManager resumes from event id when available
-> ProjectionReducer rebuilds current read model from durable state/events
-> ConversationManager remains transcript source of truth
-> TransportAdapter emits catch-up snapshot + live tail
```

Key rules:

- realtime stream loss must not corrupt durable transcript state
- projections are rebuildable
- transport adapters translate stream semantics but do not own policy

---

## 9. Long-running job

```text
create_job(...)
-> creates standalone thread/job record
-> RunStateManager.begin(job thread)
-> runtime lane starts work
-> EventStreamManager emits progress
-> ConversationManager/job store persists milestones
-> JobOutputSink receives final output/artifacts
-> RunStateManager terminal state
```

Key rules:

- jobs require explicit output sinks
- background progress should not smear into unrelated chat history
- jobs and conversations can both use threads, but their ownership differs

---

## 10. Subagent delegation

```text
spawn_subagent(...)
-> creates child thread linked to parent run
-> child RunStateManager begins independent run
-> child uses same capability/resource/audit services
-> parent consumes child terminal result as capability output
```

Key rules:

- subagent is parent-owned; job is standalone
- child scope must be derived from parent scope and explicit delegation
- parent run should record the child result, not the full child live stream unless requested

---

## 11. Transport ingress

```text
browser / channel / webhook / IDE
-> TransportAdapter
-> shared RuntimeRequest
-> shared runtime services
-> RuntimeEvent / ProjectionUpdate
-> transport-specific response
```

Key rules:

- transport adapters translate
- transport adapters do not own business policy
- webhook/channel auth is checked before runtime request creation
- runtime services do not depend on browser-specific state

---

## 12. Contract tests to add later

When these services exist, add caller-level tests for:

- one active run per thread
- approval-blocked resume
- auth-blocked resume and retry
- visible capability invalidation after extension activation
- action-time authorization even for visible capabilities
- reconnect from event id
- job output sink separation
- subagent child result propagation
- transport adapter policy-free normalization
