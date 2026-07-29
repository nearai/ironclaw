# ironclaw_dispatcher guardrails

- Own already-authorized runtime routing only.
- Do not import authorization, approvals, run-state, capabilities, processes, host-runtime, concrete runtime crates, product workflow, or caller-facing state.
- Event sink failures are best-effort and must not alter dispatch success/failure outcomes.
- Runtime errors crossing public dispatch surfaces must be redacted to stable kinds.
- Resolve capabilities through the snapshot-shaped `ToolResolver`; dispatch must not reselect packages or runtime kinds per invocation.
- Runtime execution is still a **closed** model (arch-simplification §4.2): host-runtime binds each package through one exhaustive `RuntimeLaneExecutor` match over WASM/MCP/process/first-party. Do not restore a production `HashMap<RuntimeKind, dyn RuntimeAdapter>` router or add concrete runtime dependencies here. `System` is host-internal and must fail closed rather than default to a lane.
