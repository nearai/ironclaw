# ironclaw_processes guardrails

- Own the process journal, process lifecycle and dependency records,
  capability-invocation projections, process/result stores, cancellation
  tokens, host-facing process status/output helpers, and the background
  process manager.
- Child process creation, tree reservation, and dependency open are one journal
  command. Dependency consume/abandon and reservation release are one journal
  command. Never reintroduce compensating dual writes for either transition.
- Keep runtime execution behind `ProcessExecutor`; this crate must not know how Script/MCP/WASM dispatch works beyond carrying typed process requests and results.
- Preserve the background ordering invariant: result store first, lifecycle terminal status second, so observing a terminal process means its result is already available.
- Carry all spawn-time handoffs explicitly: scoped mounts, resource estimates/reservations, cancellation, input, and identity fields must not be recomputed from global state.
- Keep resource-management wrappers honest: prepared reservations should be reconciled/released exactly once, and cleanup errors must remain visible where contracts require them.
- Persistence backends must preserve exact tenant/user/agent/project/mission/thread scoping and hide wrong-scope records as unknown.
- Normal startup and process requests may use exact reads and bounded,
  partition-leading keyset queries only. Legacy replay and projection rebuilds
  are explicit offline methods; index declaration never backfills.
- Do not leak backend paths, raw runtime errors, secret material, or transport details through process errors or result records.
