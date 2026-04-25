# PR 2953 architecture feedback integration notes

**Status:** Integration notes / provenance
**Date:** 2026-04-25
**Source PR:** <https://github.com/nearai/ironclaw/pull/2953>
**Source feedback doc:** `docs/reborn/2026-04-24-os-like-architecture-feedback.md`

---

## 1. Why this note exists

PR #2953 adds a standalone feedback document for the Reborn OS-like architecture. The feedback is useful, but it should not replace the baseline architecture document.

This note records which ideas are being promoted into the current docs set, which ideas need Reborn-specific wording, and which ideas should stay deferred. It preserves provenance while preventing the architecture from silently drifting.

---

## 2. Adopt now

### Workflow-first ownership pass

Adopt the PR's strongest recommendation: document representative workflows before locking service boundaries.

For each workflow, record:

- services touched
- owner of each step
- scope carried through the step
- durable state touched
- ephemeral/live state touched
- interface contract invoked
- what each service must not own

Promoted into:

- `docs/reborn/contracts/runtime-workflows.md`

### Logical service contracts before crate proliferation

Adopt the service-boundary framing, but treat most names as logical services first, not immediate crates.

Useful logical contracts:

- scope resolution and view construction
- conversation/thread lifecycle
- instruction bundle assembly
- capability catalog and access checks
- approval gates
- auth gates
- run-state lifecycle
- event vocabulary and stream delivery
- projection/read-model reducers
- transport adapters

Promoted into:

- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/run-state.md`
- `docs/reborn/contracts/events-projections.md`
- `docs/reborn/contracts/capability-access.md`

### Run-state is first-class

Adopt these invariants:

- one active run per thread
- approval-blocked and auth-blocked are distinct states
- cancel, interrupt, resume, and terminal transitions are typed
- blocked states are not free-form chat text
- checkpoint/resume semantics are explicit

Promoted into:

- `docs/reborn/contracts/run-state.md`

### Event and projection discipline

Adopt these distinctions:

- transcript persistence is not live progress
- realtime events are not durable audit/history
- projections are derived read models, not state owners
- reconnect/resume is event-id based
- transport adapters translate; they do not own business policy

Promoted into:

- `docs/reborn/contracts/events-projections.md`

### Parent model protocol remains `Text | ToolCalls`

Adopt the PR's recommendation that CodeAct is not a third top-level parent-loop response mode.

The parent loop should stay:

```text
Text | ToolCalls
```

CodeAct can exist behind explicit capabilities:

```text
spawn_subagent(mode = "codeact")
create_job(mode = "codeact")
```

This aligns with the existing Reborn decision that CodeAct/Monty are optional and not foundational.

Promoted into:

- `docs/reborn/contracts/runtime-workflows.md`

---

## 3. Adopt with Reborn-specific wording

### "Outer engine is host-owned"

The PR says the outer engine should be host-owned and service-driven. That is directionally useful, but the wording is risky.

Use this wording instead:

```text
The host owns stable runtime service contracts and run-state invariants.
The default agent loop is a first-party userland component that uses those contracts.
```

Avoid wording that implies:

```text
ironclaw_kernel owns the agent loop
```

The kernel remains composition-heavy and logic-light.

### Conversation manager and pending gates

The PR suggests `ConversationManager` owns pending gates. Refine that boundary:

- `ConversationManager` owns durable thread/transcript references to gates.
- `ApprovalManager` owns approval request semantics and resolution.
- `AuthFlowManager` owns auth-required state and retry-after-auth.
- `RunStateManager` owns blocked/resume transitions.

This prevents conversation from becoming the new blob.

### Tool manager terminology

The PR uses `ToolManager` and `ToolAccessManager`. Reborn should prefer capability-oriented terms so we do not recreate the old universal `ToolRegistry`.

Recommended split:

- `CapabilityCatalog` — descriptors and provider mapping
- `CapabilityAccessManager` — visible surface and action-time authorization
- `RuntimeDispatcher` — runtime lane selection
- runtime crates — actual execution

Promoted into:

- `docs/reborn/contracts/capability-access.md`

---

## 4. Defer or reject for now

### Do not create all manager crates immediately

The feedback identifies useful ownership seams, but creating a crate for every seam now would add churn before implementation pressure proves the boundaries.

Start with docs and module boundaries. Extract crates only when the implementation creates real pressure.

### Do not make CodeAct a peer parent loop

CodeAct can be a worker mode, but it should not become a foundational execution lane or parent engine protocol.

### Do not promote `ironclaw_processes` before script/MCP pressure demands it

Process lifecycle is real, but in the current stack the public V1 lanes are:

```text
Host | WASM | Script Runner
+ MCP adapter
```

A process substrate can emerge behind Script Runner, MCP stdio, jobs, and workers. It does not need to be the next public crate unless those implementations require it.

---

## 5. Resulting docs added

This branch adds the source feedback doc and these integration docs:

```text
docs/reborn/2026-04-24-os-like-architecture-feedback.md
docs/reborn/2026-04-25-pr-2953-architecture-feedback-integration.md
docs/reborn/contracts/runtime-workflows.md
docs/reborn/contracts/run-state.md
docs/reborn/contracts/events-projections.md
docs/reborn/contracts/capability-access.md
```

The source feedback remains a provenance artifact. The contract docs are the Reborn-specific promoted content.
