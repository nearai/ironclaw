# ironclaw_agent_loop::state

Owns the typed, resumable state carried by the canonical loop executor.

## Files

Re-derive this list with `ls crates/loop/ironclaw_agent_loop/src/state/`
before trusting it.

- `state.rs` defines `LoopExecutionState`, checkpoint payload constants, and
  constructors.
- `slots.rs` retains only trivial marker and compatibility slots that do not
  have enough independent behavior to earn a file.
- `compaction.rs`, `recovery.rs`, `reply_admission.rs`, and `stop_control.rs`
  own the substantive typed slots for those decision axes.
- `tests.rs` routes checkpoint/wire and lifecycle state-contract test
  submodules, kept outside the wire-struct definition.
- `budget_ledger.rs` owns the three per-run budget counters that used to be
  bare public fields on `LoopExecutionState`.
- `bounded_ring.rs` defines fixed-window observation history.
- `signature.rs` defines repeat-detection signatures for capability calls.
- `model_recovery.rs` defines checkpointed, typed model-recovery controls.
- `terminal_warning.rs` defines checkpointed pre-termination warnings and
  their pending, active, and one-shot accounting.

## Boundaries

- Store only loop-safe data: refs, cursors, counters, versions, digests,
  compact safe summaries, and typed strategy slots.
- Do not store raw prompt text, raw model output, tool arguments, secrets,
  provider errors, host paths, filesystem contents, or backend stack traces.
- Do not put family-domain durable state here. Mission progress, routine
  cursors, plan trees, and product state belong behind host/workspace context
  sources and are surfaced through prompt/context ports.
- Before adding a field, name its producer, reader/owner, and fresh-run,
  rebase, and reset policy. Classify it as durable checkpoint state or
  ephemeral execution state first; durable additions must preserve the flat
  wire shape and root `crate::state::*` re-exports.
- An unread serialized field is a compatibility tombstone until an explicit
  wire migration removes it. Do not prune it as part of an ownership move.
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
- Keep state mutation with the executor stage that owns the transition; state
  defines the typed data and reset/rebase rules but does not become a second
  lifecycle authority.
- Keep terminal-warning observations and scheduling crate-private. Downstream
  tests should drive the executor or use `test_support` scenario helpers.
- A delivered warning remains active across capability gates until the stop
  stage evaluates that warning turn's result; do not clear it at model dispatch.
