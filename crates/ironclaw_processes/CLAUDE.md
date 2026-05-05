# ironclaw_processes guardrails

- Own process lifecycle records, process/result stores, cancellation tokens, host-facing process status/output helpers, and the background process manager.
- Keep runtime execution behind `ProcessExecutor`; this crate must not know how Script/MCP/WASM dispatch works beyond carrying typed process requests and results.
- Preserve the background ordering invariant: result store first, lifecycle terminal status second, so observing a terminal process means its result is already available.
- Carry all spawn-time handoffs explicitly: scoped mounts, resource estimates/reservations, cancellation, input, and identity fields must not be recomputed from global state.
- Keep resource-management wrappers honest: prepared reservations should be reconciled/released exactly once, and cleanup errors must remain visible where contracts require them.
- Persistence backends must preserve exact tenant/user/agent/project/mission/thread scoping and hide wrong-scope records as unknown.
- Do not leak backend paths, raw runtime errors, secret material, or transport details through process errors or result records.
