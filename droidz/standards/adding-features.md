# Adding Features Standard (AI Agents)

## Architecture-first rule

Prefer generic/extensible designs over hardcoded single-integration implementations.

## Required design checks

- Does this belong behind an existing trait boundary (`Tool`, `Channel`, `LlmProvider`, `Database`, etc.)?
- Does it preserve current module ownership and separation of concerns?
- Are security/sanitization implications addressed?

## Implementation discipline

- Follow existing patterns/libraries already used in the repo.
- Avoid introducing unnecessary dependencies.
- Keep changes minimal but complete (including tests and config wiring when needed).

## Delivery

- Verify behavior end-to-end for the touched path.
- Update parity tracking when feature status changes.
