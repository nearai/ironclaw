# Agent Map — ironclaw_composition

**Read [`CLAUDE.md`](./CLAUDE.md) — it is this crate's module spec** (named in
the root `CLAUDE.md` Module Specs table): the guardrails, the WebUI v2
surface, product-auth wiring rules, security invariants, and tests. Code
follows the spec; the spec is the tiebreaker. Consolidated per
`docs/reborn/guidance-conventions.md`, this file is a pointer, not a second
copy.

- Orientation (what this crate is, entry points, measured deps/consumers, the
  mass ratchet): [`README.md`](./README.md).
- Family boundary and the armed gates:
  [`crates/app/AGENTS.md`](../AGENTS.md) — the family charter is
  "composition wires owners, never becomes one".
- Neighboring contracts to read before changing behavior:
  `crates/loop/ironclaw_turn_runner/AGENTS.md`,
  `crates/app/ironclaw_config/AGENTS.md`,
  `crates/kernel/ironclaw_host_runtime/AGENTS.md`,
  `crates/kernel/ironclaw_turns/AGENTS.md`.
- Validation: `cargo test -p ironclaw_composition`;
  `cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries`;
  `bash scripts/ci/check-composition-budget.sh`;
  `scripts/reborn-e2e-rust.sh` for production wiring changes.
