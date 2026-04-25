# Feedback On IronClaw Reborn OS-Like Architecture Design

**Status:** Review feedback  
**Date:** 2026-04-24  
**Reviewed doc:** `docs/reborn/2026-04-24-os-like-architecture-design.md`

This feedback is scoped to the main OS-like architecture design document. The workflow analysis, harness-interface notes, engine-v2 review, and comparisons with other agent systems are supporting evidence only. They should not be treated as replacement architecture docs.

---

## 1. What The Main Doc Gets Right

The main direction is strong and should stay intact.

- The OS/service framing is the right level of abstraction. The host should behave like a small runtime that composes system services, not like a product-specific agent application.
- The kernel boundary is directionally correct. `ironclaw_kernel` should wire services, boot the host, and coordinate startup/shutdown, but not own product behavior.
- The extension/userland split is important. Agent loops, gateway, TUI, missions, and integrations should be first-party or third-party userland above stable host APIs.
- The current core service crates are mostly the right set: filesystem, resources, auth, network, processes, extensions, runtime lanes, and host API.
- The design already emphasizes boundary enforcement, dependency direction, and security-sensitive services. That should stay central.

The main gap is not the macro architecture. The gap is that several workflow-shaped service contracts are still implicit.

---

## 2. Crate Boundary Feedback

Do not split the crate graph aggressively up front. The better first move is to make service ownership explicit inside or above the existing crates.

| Area | Recommendation | Feedback |
| --- | --- | --- |
| `ironclaw_kernel` | Keep, narrow | Limit to composition, boot, dependency wiring, event bus wiring, and lifecycle. It should not own prompt assembly, run-state, tool policy, or product workflows. |
| `ironclaw_host_api` | Keep, tighten | Keep as the shared contract layer for ids, scopes, events, action/decision types, resources, audit envelopes, and views. Avoid runtime policy or helper logic creeping into this crate. |
| `ironclaw_filesystem` | Keep, narrow | Keep as mount, namespace, and durable path substrate. Conversation state, projections, widgets, customization policy, and job lifecycle should live above it. |
| `ironclaw_processes` | Keep, narrow | Keep as process lifecycle, sandbox, process table, and execution substrate. Do not let it own run-state, approvals, auth retry, UI progress, or projections. |
| `ironclaw_extensions` | Keep, split internally | Discovery, setup, activation, onboarding, pairing, registry projection, and uninstall cleanup are separate responsibilities. Keep one crate for now, but name the internal services. |
| `ironclaw_auth` | Keep, split internally | Secret leases, interactive auth flows, OAuth callbacks, credential injection, and retry-after-auth should be separate internal services. |
| Conversation/runtime area | Possible future crate | If `ConversationManager`, `RunStateManager`, events, and projections become crowded, this is the best candidate for a future crate. Do not create it until implementation pressure justifies it. |
| Transport adapters | No new crate yet | Define a shared contract, but keep browser, channel, webhook, and IDE-style implementations in their owning surfaces. |
| CodeAct | No parent-loop crate | Treat CodeAct as a worker mode behind subagent/job tools, not as the parent engine protocol. |

The document should be explicit that service completeness comes before crate proliferation.

---

## 3. Missing Or Under-Specified Service Contracts

The main doc lists core service crates, but it does not yet define the interfaces that the workflows require. These should be named in the architecture so implementation does not recreate a large hidden agent runtime.

| Service | Ownership |
| --- | --- |
| `ScopeManager` | Resolves instance, user, project, thread, and invocation scopes. Produces typed filesystem, tool, resource, audit, and prompt views. |
| `ConversationManager` | Owns durable thread lifecycle, transcript state, pending gates, and thread history reads. It should not own run-state, live streams, or projections. |
| `InstructionAssembler` | Builds deterministic instruction bundles from identity, context, capabilities, skills, attachments, and runtime metadata. |
| `ToolManager` | Owns tool existence, descriptors, registry plumbing, execution, and normalized tool results. |
| `ToolAccessManager` | Owns visible tools, callable tools, grants, scope filtering, policy checks, and action-time authorization. |
| `ApprovalManager` | Owns pending approval requests, stable request ids, approve/deny/always decisions, and replay-safe resolution. |
| `AuthFlowManager` | Owns auth-required state, OAuth/token prompting, callback completion, and retry-after-auth behavior. |
| `RunStateManager` | Owns one-active-run-per-thread, blocked states, cancel, interrupt, resume, terminal transitions, and checkpoints. |
| `SessionEvent` / `ThreadEvent` | Defines the append-only event vocabulary for replay, projections, harnesses, live streams, and debugging. |
| `ProjectionReducer` | Derives sidebar, activity, progress, job, project, and harness read models from events and durable state. |
| `EventStreamManager` | Owns live stream delivery, keepalives, event ids, reconnect semantics, and fanout. |
| `TransportAdapter` | Normalizes browser, channel, webhook, and IDE-style ingress into shared runtime requests and adapts runtime events back out. |

These do not all need new crates. They do need explicit contracts and "must not own" boundaries.

---

## 4. Engine Loop Feedback

The main doc's agent loop placement is directionally right: product behavior should not live in the kernel. The part that needs sharpening is the engine contract itself.

The outer engine should be host-owned and service-driven:

```text
TransportAdapter
-> ConversationManager
-> ScopeManager
-> RunStateManager
-> build ScopeSnapshot / InstructionBundleSnapshot / VisibleToolSnapshot
-> LLM call
-> Text | ToolCalls
-> ToolAccessManager.authorize
-> ToolManager.execute
-> emit events, persist milestones, refresh projections
-> continue | pause | complete
```

The top-level LLM contract should stay:

```text
Text | ToolCalls
```

CodeAct should not be a third top-level response mode in the parent loop. It is better modeled as a worker mode behind explicit tool calls:

```text
spawn_subagent(mode = "codeact") -> child thread owned by the parent run
create_job(mode = "codeact") -> standalone thread with its own output sink
```

This keeps the model boundary simple: the parent model can speak or call tools. It cannot switch the whole engine protocol into another execution mode.

The architecture can still keep `Thread` as the shared execution primitive:

```text
conversation = user-facing thread
subagent = child thread
job = standalone/background thread
routine = scheduled thread trigger
```

The difference is ownership and lifecycle, not the underlying primitive.

---

## 5. Workflows The Main Doc Should Cover

The main design would be stronger if it walked through representative workflows and identified which services are called.

### Interactive chat turn

```text
TransportAdapter
-> ConversationManager
-> ScopeManager
-> RunStateManager
-> InstructionAssembler
-> ToolAccessManager.visible_tools
-> LLM
-> ToolAccessManager.authorize
-> ToolManager.execute
-> ConversationManager persist
-> EventStreamManager publish
-> ProjectionReducer update
```

Key point: scope, instructions, and visible tools are warm-path snapshots. Tool authorization is an action-time check.

### Approval-blocked tool

```text
Tool call
-> ToolAccessManager.authorize
-> ApprovalManager.open_pending_gate
-> RunStateManager.blocked(approval)
-> EventStreamManager publish approval_needed
-> user approves or denies
-> RunStateManager.resume
```

Key point: approval should not be generic chat text. It is a structured run-state transition.

### Auth-blocked tool

```text
Tool call
-> auth required
-> AuthFlowManager.begin
-> RunStateManager.blocked(auth)
-> OAuth/token completion
-> SecretLeaseManager records lease
-> retry or resume original action
```

Key point: auth-blocked and approval-blocked are distinct. They can both resume work, but they are not the same workflow.

### Extension activation changing visible tools

```text
ExtensionManager activates extension
-> ToolManager updates catalog
-> ToolAccessManager invalidates visible tool snapshot
-> InstructionAssembler rebuilds only if model-visible capability text changed
```

Key point: tool visibility refresh is not the same as action-time authorization.

### Reconnect and live stream resume

```text
client reconnects
-> EventStreamManager resumes from event id
-> ProjectionReducer rebuilds current read model
-> ConversationManager remains source of durable transcript
```

Key point: live progress and durable transcript are different products.

### Long-running job

```text
create_job(...)
-> creates standalone thread
-> JobExecutionSession starts
-> EventStreamManager emits progress
-> ConversationManager or job store persists milestones
-> JobOutputSink receives final output
```

Key point: jobs need output sinks and progress policy. They should not smear background progress into normal chat history.

### Subagent delegation

```text
spawn_subagent(...)
-> creates child thread
-> child thread runs independently
-> parent consumes child result as tool output
```

Key point: subagent and job can both use threads. A subagent is parent-owned; a job is standalone.

### Transport ingress

```text
browser / channel / webhook / IDE
-> TransportAdapter
-> shared RuntimeRequest
-> shared runtime services
```

Key point: transport adapters translate. They do not own business policy.

---

## 6. Operational Invariants To Add

These should be stated explicitly in the main architecture discussion or a direct companion section.

- One active run per thread.
- Transcript persistence and live progress are different products.
- Scope, instruction, and visible-tool snapshots are built on warm paths and refreshed only on typed invalidation.
- Tool authorization is checked at action time.
- Visible tool surface is not the same as action authorization.
- Approval-blocked and auth-blocked resumes are distinct.
- Transport adapters do not own business policy.
- Projections are derived read models, not durable state ownership.
- Kernel wires services; it does not coordinate product behavior.
- CodeAct is a worker mode behind tools, not the parent loop protocol.

---

## 7. Borrowed Lessons From Other Agent Systems

The comparison work was useful, but the main architecture should borrow only specific practices.

| System | Borrow | Do not copy |
| --- | --- | --- |
| `opencode` | typed session events and reducer/projector discipline | a large session loop that owns prompt assembly, execution, and projection together |
| `claude-code` | separation between persisted transcript and ephemeral progress; careful deferred tool discovery | exposing deferred capability discovery without strict access control |
| `hermes-agent` | transport adapter/source-context discipline across CLI, gateway, ACP, and web | one large agent core plus hooks/registries owning most behavior |

The main doc's OS/service model is stronger than these systems at the macro level. The useful lesson is to make the missing runtime contracts concrete.

---

## 8. Recommended Follow-On Docs

The main architecture doc should stay readable. The deeper mechanics can live in focused follow-on docs:

- engine loop contract
- run-state lifecycle and transition table
- event vocabulary
- transport adapter contract
- projection/read-model contract
- job and subagent lifecycle contract

These should refine the baseline rather than replace it.

---

## 9. Bottom Line

The main architecture doc has the right high-level direction. The recommended feedback is to make ownership more explicit before implementation starts.

The highest-value changes are:

- keep the crate graph mostly intact
- narrow overloaded crates
- add missing service contracts
- make event/projection/run-state first-class
- keep the top-level LLM protocol to `Text | ToolCalls`
- put CodeAct behind `spawn_subagent` and `create_job`

That gives the reboot a cleaner path from architecture to implementation without recreating the current blob under newer names.
