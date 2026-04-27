# Reborn Contract — Turns and First-Party Agent Loop

**Status:** Contract-freeze draft
**Date:** 2026-04-25
**Depends on:** [`host-api.md`](host-api.md), [`capabilities.md`](capabilities.md), [`memory.md`](memory.md), [`events-projections.md`](events-projections.md), [`processes.md`](processes.md)

---

## 1. Purpose

The default agent loop is a trusted first-party service, not a generic runtime lane and not dispatcher behavior.

Recommended service names:

```text
TurnService
AgentLoopService
```

Responsibilities:

- normalize channel submissions into thread turns;
- enforce one-active-run-per-thread;
- build prompt context through `MemoryPromptContextService`;
- call LLM/provider layer;
- execute tools/capabilities through `CapabilityHost`;
- manage approval-blocked and resumable turns;
- persist turn/thread state;
- emit replies and durable progress events.

Non-responsibilities:

- direct runtime dispatch bypassing `CapabilityHost`;
- direct authorization/grant evaluation;
- low-level network/secrets policy;
- memory backend storage internals;
- extension registry mutation except through extension services.

---

## 2. Ownership model

```text
Channel adapter
  -> normalized incoming message
  -> TurnService
      -> thread/run ownership check
      -> MemoryPromptContextService
      -> LLM/provider call
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

The agent loop does not assemble prompt documents directly. It calls:

```text
MemoryPromptContextService::build(context, mode)
```

Prompt context modes include:

```text
direct/main session
group chat
project session
admin/system run
```

Rules:

- identity/system-prompt files are primary-scope only;
- group chat excludes personal memory/profile context;
- prompt-injected file writes are guarded by the memory prompt service;
- assembled prompts are not emitted in events/audit by default;
- prompt build failures are explicit turn failures unless a contract marks a missing optional doc as ignorable.

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

- turn service starts work through `CapabilityHost::spawn_json` or a first-party process API;
- process status/result/output are read through `ProcessHost`;
- streaming output/progress reaches clients through durable event stream;
- binary/large output is referenced by artifact refs, not embedded in turn state.

---

## 9. Channel boundary

Channel adapters own transport normalization only:

```text
Telegram/Slack/Web/CLI/etc.
  -> IncomingMessage-like normalized record
  -> TurnService
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

- one-active-run-per-thread blocks concurrent turns;
- turn scope propagates into `ExecutionContext.resource_scope`;
- tool calls go through `CapabilityHost` only;
- approval-blocked turn resumes with exact invocation fingerprint;
- group chat prompt excludes personal memory/profile docs;
- primary identity docs are not read from secondary scopes;
- cancellation propagates to process/capability work;
- durable event cursor can replay turn progress after reconnect;
- raw secrets/host paths/tool raw input do not leak in turn events.
