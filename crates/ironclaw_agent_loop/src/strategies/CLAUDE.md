# ironclaw_agent_loop::strategies

Owns sealed built-in strategy traits and their default implementations. Each
file is one loop decision axis.

## Files

- `context.rs` chooses prompt/context request shape, not prompt assembly.
- `capability.rs` narrows an already-resolved capability surface.
- `model.rs` chooses model preference.
- `batch.rs` chooses batch sequencing policy.
- `gate.rs` handles approval/auth/resource gates.
- `recovery.rs` handles model and capability failures.
- `stop.rs` decides whether the loop is done.
- `drain.rs` decides when steering/followup inputs are drained.
- `budget.rs` owns iteration and wall-clock limits.
- `mod.rs` declares strategy modules and crate-internal re-exports only.

## Boundaries

- Strategies are crate-private implementation details of built-in families.
- Strategies receive `&LoopExecutionState` and return typed decisions or typed
  slot updates. They do not own the canonical tick.
- Prompt file loading, skill loading, identity file reads, model message
  materialization, tool invocation, approvals, persistence, and product routing
  belong to host ports and downstream crates.
- Capability strategies can only narrow a resolved surface; profile grants and
  host authorization stay outside this crate.

## Adding code

- Add a new strategy only for a stable, independent decision axis.
- Put default behavior in the same file as the trait when it exactly matches
  that axis.
- Add new outcome variants only when the executor can handle them generically.
- If logic needs host/runtime/product details, add a host adapter or context
  source elsewhere instead of widening a strategy.

## Common mistakes

- Do not create a catch-all strategy for unrelated policy.
- Do not pass raw content into strategy decisions.
- Do not smuggle host calls through strategy types.
- Do not add public strategy exports to make an external family compile.
