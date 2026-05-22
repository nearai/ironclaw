# Agent Map - ironclaw_first_party_extensions

## Start Here

- Read `Cargo.toml` for dependencies and feature shape.
- Use neighboring contracts before changing behavior:
  - `crates/ironclaw_loop_support/AGENTS.md`
  - `crates/ironclaw_skills/AGENTS.md`
  - `crates/ironclaw_reborn_composition/AGENTS.md`
  - `.claude/rules/skills.md`

## What This Crate Owns

- First-party in-process extension ports shipped with IronClaw but composed as userland capabilities.
- Skill activation selection and scoped skill bundle handles for Reborn first-party extensions.
- Narrow loop-facing adapters that expose selected skill context without ambient runtime authority.

## Do Not Move In Here

- Host runtime handles, dispatcher internals, network/secrets/resource authority, or product workflow orchestration.
- General skill parser/registry/scoring behavior owned by `ironclaw_skills`.
- Reborn runtime assembly owned by `ironclaw_reborn_composition`.

## Validation

- Fast local check: `cargo test -p ironclaw_first_party_extensions`
- Lint check: `cargo clippy -p ironclaw_first_party_extensions --all-targets -- -D warnings`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`

## Agent Notes

- Keep extension handles explicit and scoped.
- Preserve prompt-context bounding: only selected skills should become model-visible context.
- Fail closed on ambiguous explicit same-name activation across sources.
- Do not collapse first-party extension ports into runtime composition.
