# v1 Retirement Readiness Gate

Tracking issue: #6077

## Purpose

This document defines the first deletion gate for retiring the legacy root
`ironclaw` runtime. It is intentionally an inventory and decision record, not a
request to delete `src/` immediately.

## Discovery Notes

The codebase graph artifact was missing locally on 2026-07-15, so this pass used
the Reborn orientation skill plus live `cargo metadata` and targeted `rg`.
Re-run the commands below before any deletion PR because the counts are a point
in time.

## Current Legacy Surface

Known legacy package layers from `cargo metadata`:

| Package | Status |
| --- | --- |
| `ironclaw` | Root legacy package and v1 runtime. |
| `ironclaw_gateway` | v1 gateway frontend assets; Reborn WebChat v2 uses separate crates. |
| `ironclaw_tui` | v1-only TUI surface consumed by the root package. |

Known near-legacy package:

| Package | Status |
| --- | --- |
| `ironclaw_embeddings` | Documented as v1-only today, but metadata currently says `substrates`; decide whether to delete it or promote it to a real Reborn-owned substrate before final cleanup. |

Current dependency blockers:

| Dependency | Why it blocks deletion |
| --- | --- |
| `ironclaw_reborn_migration -> ironclaw` | Migration reads v1 database/state through root-crate types and DB helpers. |
| `ironclaw -> ironclaw_gateway` | Root v1 gateway asset dependency. |
| `ironclaw -> ironclaw_tui` | Root v1 TUI dependency. |
| `ironclaw -> ironclaw_embeddings` | Root v1 embedding provider dependency. |

Current scale snapshot:

| Surface | Count |
| --- | ---: |
| `src/` + `ironclaw_gateway` + `ironclaw_tui` + `ironclaw_embeddings` files | 506 |
| Test / migration files containing `ironclaw::` references | 83 |

## Deletion Gate

Do not delete the root package or `src/` until all of these are true:

- `ironclaw_reborn_migration` no longer depends on the root `ironclaw` crate, or
  migration support is explicitly retired with operator rollback guidance.
- CI and release packaging no longer build or ship the legacy `ironclaw` binary.
- Legacy tests that protect still-required behavior have Reborn-side coverage.
- `ironclaw_embeddings` has an explicit decision: delete, or reclassify with live
  Reborn consumers and ownership docs.
- Docs and agent guidance describe Reborn as the supported runtime path.
- Architecture tests can reject a reintroduced legacy package layer or root
  runtime dependency after the deletion lands.

## Migration Strategy Decision

Pick one before deletion:

1. Extract minimal v1 read models into a migration-only crate.
2. Vendor read-only v1 database schemas directly into `ironclaw_reborn_migration`.
3. Retire migration support and document the operator cutoff.

The preferred low-risk path is option 1 or 2 if users still need state migration.
Option 3 is only acceptable with explicit product approval because it turns this
cleanup into a compatibility break.

## Verification Commands

```bash
bash scripts/codebase-graph.sh status
cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.metadata?.ironclaw?.layer == "legacy") | .name'
cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] as $p | $p.dependencies[]? | select(.name=="ironclaw" or .name=="ironclaw_gateway" or .name=="ironclaw_tui" or .name=="ironclaw_embeddings") | $p.name + " -> " + .name'
rg -l '(^use ironclaw::|\bironclaw::)' tests crates/ironclaw_reborn_migration
rg -n 'layer = "legacy"|ironclaw_gateway|ironclaw_tui|ironclaw_embeddings|src/main.rs' Cargo.toml crates .github scripts Dockerfile* README.md FEATURE_PARITY.md
```

## Risks

Risk level: high.

The cleanup removes a large runtime surface and changes release/CI defaults. The
highest-risk hidden dependency is migration: deleting v1 before resolving the
read path either breaks state migration or forces a rushed compatibility shim.
