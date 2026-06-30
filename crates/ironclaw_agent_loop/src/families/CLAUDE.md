# ironclaw_agent_loop::families

Owns built-in loop-family factories.

## Responsibility

- A family chooses a sealed planner composition and exposes an opaque
  `LoopFamily`.
- Family ids and versions must remain stable enough for profile resolution and
  checkpoint/resume validation.
- `default()` is the baseline family used by planned-driver wiring.

## Boundaries

- Families are built into `ironclaw_agent_loop`; external crates do not
  implement strategies or register arbitrary planner compositions.
- Product behavior, adapter-specific routing, runtime readiness, and driver
  registration do not belong here.
- Heavy implementation logic belongs in narrowly named strategy files or helper
  modules, not inline inside `mod.rs`.

## Adding code

- Add a new family file when a built-in loop family needs a distinct strategy
  composition.
- Add a new strategy only when no existing decision axis owns the behavior.
- Keep factory output opaque; do not expose strategy slots to callers.

## Common mistakes

- Do not use families as a plugin system.
- Do not add profile parsing or product-default selection here.
- Do not change the default family casually; it affects replay and resume
  expectations.
