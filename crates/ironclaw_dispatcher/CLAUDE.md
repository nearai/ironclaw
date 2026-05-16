# ironclaw_dispatcher guardrails

- Own already-authorized runtime routing only.
- Do not import authorization, approvals, run-state, capabilities, processes, host-runtime, concrete runtime crates, product workflow, or caller-facing state.
- Event sink failures are best-effort and must not alter dispatch success/failure outcomes.
- Runtime errors crossing public dispatch surfaces must be redacted to stable kinds.
- Runtime lanes must be registered through `RuntimeAdapter`; do not add direct WASM/Script/MCP dependencies back to this crate.
