# Static Kernel Process Journal Proof

**Date:** 2026-07-24
**Status:** Proved and implemented

## Result

The durable lifecycle formerly implemented as turn-run state is now a neutral
process journal. Queueing, claiming, leasing, suspension, cancellation,
recovery, terminal transitions, process trees, gates, and ordered lifecycle
facts are owned by `ironclaw_processes`.

`ProcessJournalStore` is the only production lifecycle authority. It implements
the process submission, transition, control, checkpoint, tree, gate-query,
lifecycle-lookup, and journal-source ports. Composition constructs one process
runtime and passes narrow views of that system to its consumers.

There is no turn row engine, reverse process-to-turn transition adapter, or
parallel turn transition authority.

## Agent Turn Projection

Agent turns are one process kind. `ironclaw_turns::AgentTurnProcessRuntime`
projects the product-facing turn API over process ports:

- `TurnRunRecord` and `TurnRunState` are views of
  `JournaledProcessSnapshot`;
- `TurnLifecycleEvent` is a view of `ProcessJournalEntry`;
- `TurnStatus` is a view of `ProcessLifecycleStatus`;
- blocked turn gates are `ProcessSuspension` values;
- runner claims and outcomes are converted only at the agent-loop boundary;
- turn-specific profile, actor, binding, and usage data is bounded process
  metadata owned by the agent-turn projection.

`ProcessJournalStoreTurnAdapter` only translates the process store's neutral
error type into `TurnError`. It does not contain lifecycle state or implement a
second state machine.

## Kernel Boundary

The static kernel owns the generic mechanisms required by every extension:

- filesystem and durable process journal;
- network mediation;
- secrets mediation;
- extension discovery and runtime lanes;
- host APIs, authorization, approvals, obligations, and resource ownership;
- generic process spawn, monitor, suspend, resume, stop, kill, recovery, and
  journal queries.

Agent-loop scheduling and loop-exit validation are not kernel policy.
`TurnRunScheduler` remains in `ironclaw_runner` and consumes
`ProcessTransitionPort`. `LoopExitApplier` validates agent-loop evidence and
then maps a validated exit to a neutral process transition. The kernel sees a
process outcome, not an agent turn.

## Deleted Compatibility

The transition removed:

- `TurnStateRowStore` and its row, cache, materializer, lease-sidecar, and
  migration machinery;
- `TurnRunTransitionPort` and its turn-specific mutation request DTOs;
- `AgentTurnProcessTransitionAdapter`;
- duplicate composition handles for transition, submission, journal, lifecycle,
  and gate sources;
- row-backed runner, composition, product, integration, and stress fixtures;
- disabled tests that depended on the deleted row authority.

`AgentTurnRuntimePort` is the query and product coordination projection
implemented by `AgentTurnProcessRuntime`. It owns no persistence.

## Design Proof

The abstraction reduces code because process lifecycle is implemented once and
agent turns only supply metadata plus projections. It would add code again if a
new extension introduced its own durable scheduler state machine, gate store,
or lifecycle event log instead of using the process ports.

New executable extension kinds should therefore:

1. submit a typed `ProcessKind` with bounded extension metadata;
2. execute outside the kernel through a runtime lane;
3. use process suspension and checkpoints for durable waits;
4. project domain-specific status and events from the process journal;
5. keep domain validation and scheduling strategy in the extension.

This is the boundary that preserves a small static kernel without leaking
agent-turn semantics into it.
