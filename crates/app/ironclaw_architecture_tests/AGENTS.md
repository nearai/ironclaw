# Agent Map — ironclaw_architecture_tests

Working rules for the enforcement suite. Orientation lives in `README.md`;
family rules in `crates/app/AGENTS.md`.

## Start Here

- Read `README.md`, then `Cargo.toml` (zero production dependencies — two
  dev-only vocabulary imports, `ironclaw_host_api` and
  `ironclaw_product_contracts`, each with its own comment explaining why).
- Use these Reborn contracts as the source of truth before changing behavior:
  - `docs/internal/reborn/contracts/_contract-freeze-index.md`
  - `docs/internal/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Workspace architecture contract tests and Reborn dependency-boundary
  enforcement — the layer ladder, per-crate `BoundaryRule`s, allowlists,
  ratchets, and their shared scanners (`tests/ratchet_support/`).
- Crate-local fixtures needed to prove that ownership.

## Do Not Move In Here

- Production runtime code, production dependencies, or a type another crate
  imports for a non-test purpose — a production dependency would make the
  crate's own zero-production-dependency claim false.
- Soft-only boundaries when a mechanical test can enforce them.

## Guardrails

- Use `cargo metadata` or equivalent workspace-graph checks to enforce Reborn
  dependency direction — inspect declared structure and source text; never
  link the crates being policed.
- Boundary tests must fail **loudly** with the exact forbidden edge and crate
  name, and fail **closed** when a scanned root resolves to nothing —
  sabotage-test every new gate (an uninspected tree must never read as
  clean; `reborn_ratchet_support_scanners.rs` is the pattern).
- Resolve crate paths through the inventory (`ratchet_support::crate_path`),
  never a bare `root.join("crates/…")` literal — that is what lets gates
  survive family moves (`reborn_crate_inventory.rs` pins the resolver).
- Keep rules conservative and explicit; when an intentional architecture edge
  changes, update the design record (`docs/internal/reborn/target-architecture/`) with
  a dated amendment in the same PR.
- If the contract and code disagree, stop and treat the task as a
  contract-change request instead of silently changing ownership.

## Validation

- Fast local check: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and
  libSQL parity tests.
