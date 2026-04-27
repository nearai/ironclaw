# Reborn Contract — Turns and Reference Agent Loops

**Status:** Contract-freeze draft
**Date:** 2026-04-25
**Depends on:** [`host-api.md`](host-api.md), [`capabilities.md`](capabilities.md), [`memory.md`](memory.md), [`events-projections.md`](events-projections.md), [`processes.md`](processes.md)

---

## 1. Purpose

This contract separates kernel-mediated turn coordination from replaceable agent-loop behavior.

The default shipped loop is a reference loop running on the kernel surface, not a privileged bypass and not dispatcher behavior. Other loop families such as lightweight loop, CodeAct loop, model-specific loops, provider-specific loops, or deployment/user-installed loops must be allowed by the architecture when trust-class policy and explicit grants permit them.

Recommended names for shipped reference services:

```text
TurnCoordinator
ReferenceAgentLoop
```

Kernel-mediated responsibilities:

- normalize accepted channel submissions into scoped turn/run records or require an adapter to do so before handoff;
- enforce one-active-run-per-thread before model/tool side effects;
- coordinate approval/auth/resource-blocked and resumable turns;
- persist scope-consistent turn/thread/run state needed for recovery;
- route capability/tool effects through `CapabilityHost`;
- route process status/output/cancellation through `ProcessHost`/process services;
- emit redacted durable progress/audit events through the event substrate;
- enforce prompt-injection write-safety policy for kernel-injected prompt files.

Loop/userland responsibilities:

- choose model/provider and loop heuristics;
- assemble prompt context from authorized memory reads;
- choose tools/capabilities to request;
- plan, retry, summarize, ask follow-up questions, and manage loop-local strategy state;
- provide reference behavior such as lightweight loop or CodeAct loop.

Non-responsibilities for any loop:

- direct runtime dispatch bypassing `CapabilityHost`;
- direct authorization/grant evaluation;
- low-level network/secrets policy;
- memory backend storage internals;
- extension registry mutation except through extension services;
- self-assigning `TrustClass::FirstParty` or `TrustClass::System`.

---

## 2. Ownership model

```text
Channel adapter
  -> normalized incoming message
  -> kernel turn coordination
      -> thread/run ownership check
      -> active reference/custom loop
          -> authorized memory reads + loop-owned prompt assembly
          -> LLM/provider call through policy-aware provider boundary
          -> CapabilityHost for capability/tool effects
      -> ProcessHost for process status/output where needed
      -> EventStream for progress/replies
```

`RuntimeDispatcher` is not in this flow except behind `CapabilityHost` or process executors.

---

## 3. Scope model

A turn carries:

```text
tenant_id
user_id
project_id: Option<ProjectId>
agent_id: Option<AgentId>
thread_id
turn_id or invocation_id
correlation_id
channel/session metadata
```

Rules:

- thread ownership is tenant/user/project/agent scoped;
- one active run per thread is enforced before LLM/tool side effects;
- every capability call receives an `ExecutionContext` with matching resource scope;
- memory prompt context uses the same tenant/user/project/agent scope;
- channel metadata does not grant authority by itself.

---

## 4. Turn lifecycle

Minimum states:

```text
accepted
queued
running
blocked_approval
blocked_auth
waiting_tool
waiting_process
completed
failed
cancelled
```

Transitions:

```text
accepted -> queued -> running
running -> blocked_approval -> running
running -> waiting_tool -> running
running -> waiting_process -> running
running -> completed|failed|cancelled
```

Rules:

- state transitions are persisted before externally visible side effects where needed for recovery;
- approval-blocked turns persist enough fingerprint metadata to resume without raw input leakage;
- cancellation requests propagate to running process/capability work when possible;
- turn failures use stable, redacted error categories.

---

## 5. Prompt context

Prompt assembly is loop/userland strategy over authorized memory reads. The kernel does not own a single prompt builder.

Reference loops may provide default prompt assemblers for modes such as:

```text
direct/main session
group chat
project session
admin/system run
```

Kernel-mediated prompt safety rules:

- identity/system-prompt files are primary-scope only;
- group chat must not receive personal memory/profile context unless explicit policy allows it;
- writes to prompt-injected files are guarded by prompt-injection write-safety policy;
- assembled prompts are not emitted in events/audit by default;
- prompt read/build failures are explicit turn failures unless a contract marks a missing optional doc as ignorable;
- custom loops may change assembly strategy, but not scope filtering, write-safety checks, or redaction requirements.

---

## 6. Capability/tool effects

All tool/capability effects go through `CapabilityHost`.

Rules:

- the agent loop never manually evaluates grants then calls dispatcher;
- exact-invocation approval leases are used for v1 resumes;
- all built-in obligations must be satisfied or fail closed before side effects;
- tool/capability raw input is not persisted in approval/audit records unless an owning contract explicitly allows redacted transcript storage.

---

## 7. Events and replies

The turn service emits durable redacted events and reply records.

Minimum event classes:

```text
turn.accepted
turn.started
turn.llm_started
turn.llm_completed
turn.tool_requested
turn.tool_completed
turn.blocked_approval
turn.resumed
turn.completed
turn.failed
turn.cancelled
reply.created
```

Rules:

- event stream uses durable append log + replay cursors;
- SSE/WebSocket clients may resume with last cursor;
- reply content is user-visible transcript state and follows transcript retention rules;
- progress/tool events are metadata/redacted unless explicitly user-facing;
- event sink delivery failure must not corrupt turn state.

---

## 8. Process integration

For spawned/background capability work:

- turn coordination starts work through `CapabilityHost::spawn_json` or a kernel-mediated process API;
- process status/result/output are read through `ProcessHost`;
- streaming output/progress reaches clients through durable event stream;
- binary/large output is referenced by artifact refs, not embedded in turn state.

---

## 9. Channel boundary

Channel adapters own transport normalization only:

```text
Telegram/Slack/Web/CLI/etc.
  -> IncomingMessage-like normalized record
  -> TurnCoordinator / active loop runner
```

They do not own:

- prompt assembly;
- tool authorization;
- approval semantics;
- memory write policy;
- durable thread source of truth.

Transport-specific auth/webhook checks happen before the turn is accepted.

---

## 10. Required acceptance tests

- one-active-run-per-thread blocks concurrent turns before model/tool side effects;
- turn scope propagates into `ExecutionContext.resource_scope`;
- shipped and custom loops both send privileged effects through `CapabilityHost` only;
- trust class alone does not let a loop bypass grants, mounts, leases, obligations, or resource policy;
- approval-blocked turn resumes with exact invocation fingerprint;
- group chat prompt excludes personal memory/profile docs;
- primary identity docs are not read from secondary scopes;
- prompt-injected file writes are scanned or fail closed regardless of loop implementation;
- cancellation propagates to process/capability work;
- durable event cursor can replay turn progress after reconnect;
- raw secrets/host paths/tool raw input do not leak in turn events.
