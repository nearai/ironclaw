# Agent Rules

## Purpose

- This crate defines host-owned trusted ingress authority tokens only.
- It must not parse product payloads, bind conversations, submit turns, evaluate triggers, or store state.

## Boundary Rules

- Keep dependencies empty unless a concrete authority-token type truly needs shared host vocabulary.
- Do not depend on `ironclaw_conversations`, `ironclaw_reborn_composition`, `ironclaw_triggers`, product adapters/workflow, host runtime, turns, threads, or storage crates.
- Only Reborn composition should construct runtime authority values. Conversations may require the token in trusted constructors.
- Product adapters and capabilities must not depend on this crate or re-export its authority types.
- Update the Reborn architecture boundary tests before changing the allowed dependent set.

## Testing

- Keep architecture tests covering which crates may depend on this crate.
- Add narrow unit tests for authority-token invariants when the token shape changes.
