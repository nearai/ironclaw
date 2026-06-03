# Agent Rules

## Purpose

- This crate is retained only as a retired trusted-ingress boundary placeholder.
- It must not expose public authority tokens, parse product payloads, bind conversations, submit turns, evaluate triggers, or store state.
- Trusted trigger ingress is owned inside `ironclaw_conversations`, where the trusted request constructor stays private.

## Boundary Rules

- Keep dependencies empty.
- Do not depend on `ironclaw_conversations`, `ironclaw_reborn_composition`, `ironclaw_triggers`, product adapters/workflow, host runtime, turns, threads, or storage crates.
- No production crate should depend on this crate.
- Product adapters and capabilities must not depend on this crate or re-export authority types.
- Update the Reborn architecture boundary tests before changing the no-dependent rule.

## Testing

- Keep architecture tests covering that no production crate depends on this crate.
