# IronClaw Reborn agent loop protocol contract

**Date:** 2026-04-25
**Status:** Draft contract
**Depends on:** `docs/reborn/2026-04-24-os-like-architecture-design.md`, `docs/reborn/contracts/runtime-workflows.md`, `docs/reborn/contracts/capability-access.md`, `docs/reborn/contracts/run-state.md`
**Reference loop mechanics:** `docs/reborn/contracts/lightweight-agent-loop.md`

---

## 1. Purpose

Define the parent loop protocol boundary for Reborn and make the layering explicit:

- the host-facing parent-loop envelope stays small and stable
- loop implementations may keep stronger internal typing
- effectful work stays behind kernel-mediated capability authorization and dispatch
- the protocol describes a userland loop surface, not kernel authority

This prevents two common failure modes:

1. a vague `Text` branch that hides multiple semantics with no typed meaning
2. a growing list of top-level parent response modes (`Script`, `Delegate`, `CodeAct`, `Job`, ...) that bypasses the capability/runtime boundary

---

## 2. External parent-loop envelope

The default host-facing parent-loop protocol is:

```text
Reply | CapabilityCalls
```

Conceptually:

```rust
pub enum ParentLoopOutput {
    Reply(AssistantReply),
    CapabilityCalls(Vec<CapabilityCall>),
}
```

Where:

- `Reply` is user-visible assistant output for the active thread.
- `CapabilityCalls` is one or more explicit requests against the visible capability surface.

Provider-native tool calling can be one encoding of `CapabilityCalls`, but the host contract is capability-oriented rather than provider- or product-specific tool-oriented.

---

## 3. Why `Reply`, not generic `Text`

`Text` is too ambiguous at the architecture boundary. It can blur together:

- final answer
- clarifying question
- status update
- accidental narration of intended actions
- hidden control text

`Reply` is narrower:

- it is the assistant's user-visible output for the current thread
- it is not a request to execute side effects
- it must not be used to smuggle execution intent that should have been expressed as a capability call

If a loop needs to perform effectful work, it must emit `CapabilityCalls` instead of describing the work in prose.

---

## 4. Internal loop typing is still encouraged

The external envelope is intentionally small. That does **not** mean loop internals should be mushy.

A loop implementation may normalize its next action into stronger internal types such as:

```rust
pub enum LoopDecision {
    FinalReply,
    AskUser,
    CallCapabilities,
}
```

Or, more explicitly:

```rust
pub enum ReplyKind {
    Final,
    AskUser,
}
```

The important boundary rule is:

- stronger reply typing is loop-owned metadata and logic
- stronger reply typing does not become a new host-wide parent protocol branch unless Reborn explicitly revisits the contract

This gives loop authors the semantic clarity of a typed loop without fragmenting the host/runtime boundary.

---

## 5. What stays behind capabilities

The following are **not** peer top-level parent-loop response modes:

- CodeAct
- scripting / code execution
- delegation to child threads
- background jobs
- project-specific workflows
- extension-defined product actions

These should appear as explicit capability calls, for example:

```text
spawn_subagent(mode = "codeact")
create_job(mode = "codeact")
script.run(...)
project.search(...)
```

This preserves the Reborn architecture law that effectful work flows through:

```text
CapabilityAccessManager -> RuntimeDispatcher -> runtime lane
```

That path remains the place for:

- visible surface filtering
- action-time authorization
- approvals
- auth-blocked transitions
- resource accounting
- audit/event hooks

---

## 6. Relationship to run state

`Reply | CapabilityCalls` is the model/loop envelope. It is not the full run-state model.

Host-managed run-state remains typed and separate:

- `Running`
- `BlockedApproval`
- `BlockedAuth`
- `Completed`
- `Failed`

A reply that asks the user a question does not by itself become `BlockedApproval` or `BlockedAuth`.
Those blocked states are host-managed transitions driven by approval or auth services.

Similarly:

- `Reply(kind = AskUser)` is loop metadata about user-visible output
- `BlockedApproval` / `BlockedAuth` is host control-plane state

These must not be collapsed into a single vague text outcome.

---

## 7. Persistence expectations

Loops should persist typed records even though the external envelope is small.

Recommended durable distinction:

- assistant reply message/artifact
- reply metadata (`final` vs `ask_user`) when the loop chooses to classify it
- capability call batch
- capability results
- approval/auth blocking milestones
- child-thread/job creation milestones

This keeps projections, debugging, and compaction grounded in typed state rather than inferences from transcript prose alone.

---

## 8. Non-goals

This contract does not:

- require every loop extension to use the exact same internal enum names
- require providers to expose native tool calling directly
- define child-thread or job result schemas in detail
- define prompt wording for a specific loop
- make CodeAct a foundational parent protocol

Those belong in loop-specific docs or adjacent contracts.

---

## 9. Contract tests to add later

When the loop and host services exist, add caller-level tests for:

- parent replies cannot trigger side effects without a capability call
- `spawn_subagent`, `create_job`, and `script.run` go through capability authorization/dispatch
- provider-native tool calls normalize into `CapabilityCalls`
- loop-specific reply classification (`final` vs `ask_user`) does not alter the host envelope
- approval/auth blocked transitions remain host-managed state, not free-form reply text
