# ironclaw_loop_support

Owns reusable host-side adapters for neutral loop ports.

## Main entry points

- `src/lib.rs` exposes loop support adapters and shared host-port helpers.
- `skill_context.rs` and `identity_context.rs` build prompt-safe instruction
  and identity context.
- `capability_port.rs`, `capability_surface_filter.rs`, and
  `capability_allow_set.rs` adapt host runtime capability surfaces and enforce
  profile-scoped narrowing.
- `input_queue.rs` and `input_port.rs` adapt steering/followup queues.
- `cancellation_port.rs` adapts run cancellation observations.

## Boundaries

- This crate implements neutral `ironclaw_turns` loop ports using host-owned
  services. It is adapter glue, not the executor, runner, product workflow, or
  low-level runtime.
- It may depend on host service contracts needed to adapt a port, but should not
  own provider clients, dispatcher internals, product binding, DB migrations, or
  driver registration.
- Prompt context builders may produce safe summaries and refs; full prompt
  materialization is still owned by the prompt port contract.

## Adding code

- Add one file per host adapter or context source.
- Add decorators, such as profile filters, as named types with a single policy
  responsibility.
- Put new capability-surface filtering policy in `capability_surface_filter.rs`.
  Put profile-to-allow-set construction and validation in
  `capability_allow_set.rs`.
- Add traits here only when they are host-owned inputs to an existing loop port.

## Common mistakes

- Do not bypass `CapabilityHost` or dispatcher authority paths.
- Do not make adapters perform runner state transitions.
- Do not fold unrelated ports into `lib.rs`.
- Do not use this crate as a dumping ground for Reborn runtime composition.
