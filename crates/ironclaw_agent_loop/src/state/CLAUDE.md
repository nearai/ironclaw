# ironclaw_agent_loop::state

Owns the typed, resumable state carried by the canonical loop executor.

## Files

- `state.rs` defines `LoopExecutionState`, checkpoint payload constants, and
  constructors.
- `slots.rs` defines per-strategy state slots.
- `bounded_ring.rs` defines fixed-window observation history.
- `signature.rs` defines repeat-detection signatures for capability calls.

## Boundaries

- Store only loop-safe data: refs, cursors, counters, versions, digests,
  compact safe summaries, and typed strategy slots.
- Do not store raw prompt text, raw model output, tool arguments, secrets,
  provider errors, host paths, filesystem contents, or backend stack traces.
- Do not put family-domain durable state here. Mission progress, routine
  cursors, plan trees, and product state belong behind host/workspace context
  sources and are surfaced through prompt/context ports.
- State types may depend on neutral `ironclaw_turns` refs and request types;
  they must not depend on Reborn runtime, product, DB, or capability-host
  implementations.

## Adding code

- Add a slot type when one strategy needs resumable private state.
- Add a helper type when it is part of checkpointed executor state or repeat
  detection.
- Create a new file when a helper grows its own invariants or tests.

## Common mistakes

- Do not use a shared control slot for unrelated strategies.
- Do not add generic maps for future strategy state.
- Do not make state mutation implicit through interior mutability.
- Do not change checkpoint wire shape without updating constructor,
  validation, and resume tests together.
