# ironclaw_dispatcher guardrails

- Own already-authorized runtime routing only.
- Do not import authorization, approvals, run-state, capabilities, processes, host-runtime, concrete runtime crates, product workflow, or caller-facing state.
- Event sink failures are best-effort and must not alter dispatch success/failure outcomes.
- Runtime errors crossing public dispatch surfaces must be redacted to stable kinds.
- Routing is a **closed** model (arch-simplification §4.2): the dispatcher holds one monomorphized `RuntimeExecutor` (`E`) resolved once at composition and routes each capability by matching the resolved `RuntimeLane` — not a `HashMap<RuntimeKind, dyn RuntimeAdapter>` trait-object registry. `RuntimeLane::from_runtime_kind` maps a descriptor's `RuntimeKind` to a lane; `None` (host-internal `System`) fails closed with `MissingRuntimeBackend`, never a default lane.
- The concrete executor set (the `RuntimeLaneExecutor` enum over WASM/Script/MCP/first-party) lives host-runtime-side, behind the `RuntimeExecutor` port. `RuntimeAdapter` remains the per-lane execution shape but is no longer used as a trait object here. Do not add direct WASM/Script/MCP dependencies back to this crate.
