# v1 Retirement Deletion and Guardrails Runbook

Tracking issue: #6077

## Purpose

This runbook describes the final deletion phase for the legacy v1 runtime. It is
meant to be used after migration, CI, Docker, release, and test retargeting have
landed.

## Preconditions

Do not start the deletion PR until these are true:

- `ironclaw_reborn_migration` no longer depends on the root `ironclaw` package,
  or migration support has been explicitly retired.
- Docker and release flows already target the canonical Reborn `ironclaw`
  binary.
- Reborn tests cover behavior still needed from legacy v1 tests.
- Docs and common developer commands already point to the canonical Reborn
  `ironclaw` command.
- `ironclaw_embeddings` has a final decision: delete or keep with Reborn
  ownership and live Reborn consumers.

## Deletion Scope

The final deletion PR should remove or update:

- root `ironclaw` package metadata and dependencies in `Cargo.toml`
- legacy `src/`
- `crates/ironclaw_gateway`
- `crates/ironclaw_tui`
- `crates/ironclaw_embeddings`, unless it was promoted to a Reborn-owned
  substrate in an earlier PR
- root package tests and snapshots that import `ironclaw::`
- stale scripts, workflow scopes, docs, and fixtures that reference deleted
  paths

Avoid mixing final deletion with new Reborn feature work. If a behavior gap
blocks deletion, land the Reborn replacement separately first.

## Guardrails To Add In The Final PR

Add architecture tests that fail when any of the following returns:

- a package with `[package.metadata.ironclaw] layer = "legacy"`
- a workspace dependency on the root `ironclaw` package
- a root `src/main.rs` or `src/lib.rs` legacy runtime entrypoint
- a workspace member named `ironclaw_gateway` or `ironclaw_tui`
- a Reborn crate importing `ironclaw::` or `src/channels/web`

The existing architecture suite already enforces layer metadata and blocks
non-root crates from depending on legacy crates. After deletion, tighten that
suite from "legacy is quarantined" to "legacy is absent."

## Suggested Mechanical Order

1. Remove legacy crates from `workspace.members`.
2. Remove root package dependencies and feature forwards.
3. Delete legacy directories.
4. Run `cargo metadata --no-deps --format-version 1` and fix stale workspace
   members.
5. Run `rg` for deleted names and remove stale references.
6. Add architecture guardrails.
7. Run the validation suite below.

## Validation

```bash
cargo metadata --no-deps --format-version 1
cargo build -p ironclaw_reborn_cli --bin ironclaw
cargo test -p ironclaw_architecture
bash scripts/reborn-e2e-rust.sh
rg -n 'layer = "legacy"|ironclaw_gateway|ironclaw_tui|use ironclaw::|\bironclaw::|src/channels/web|src/main.rs' Cargo.toml crates tests scripts .github Dockerfile* README.md FEATURE_PARITY.md
git diff --check
```

For any retained `ironclaw_embeddings` path, also prove:

```bash
cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] as $p | $p.dependencies[]? | select(.name=="ironclaw_embeddings") | $p.name + " -> " + .name'
```

There should be at least one real Reborn consumer and no root `ironclaw`
consumer.

## Rollback

The final deletion PR should be easy to revert as one unit. If a hidden
production dependency appears after merge, revert the deletion PR first, then
restore only the missing Reborn replacement in a narrower follow-up.

## Risk

Risk level: high.

The final deletion removes a large amount of code and changes package topology.
The main risks are stale release scripts, lost migration capability, and tests
that were deleted before equivalent Reborn coverage existed.
