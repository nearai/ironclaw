# Plan #8513: Decompose oversized migration-touched files

## Scope

Split two existing files that exceed the repository's 1,500-line architecture
budget without changing their public behavior:

- `crates/ironclaw_reborn_cli/tests/smoke.rs`
- `crates/ironclaw_secrets/src/lib.rs`

The v1 migration work must remain narrow: CLI migration assertions belong with
the command and deployment surfaces they exercise, while secret material
comparison remains part of the `SecretStore` contract owner.

## CLI smoke suite

Move tests into integration-test targets grouped by command or deployment
surface. Keep shared process and environment helpers in a small test-support
module. Preserve binary-level coverage, feature gates, and the rule that a new
command is exercised through `CARGO_BIN_EXE_ironclaw-reborn`.

Suggested order:

1. Extract Dockerfile and entrypoint checks.
2. Extract onboarding and migration-activation checks.
3. Extract remaining command families while keeping shared helpers single-copy.
4. Confirm each extracted target is included by the existing CLI lint and test
   commands before deleting its original smoke coverage.

## Secrets contract

Move related public contracts and their implementations into owner modules,
then re-export the existing API from `lib.rs`. Preserve type paths, trait
signatures, object safety, default method behavior, and all downstream
implementations and test doubles.

Suggested order:

1. Inventory every `SecretStore`, credential-store, and broker implementation.
2. Extract the `SecretStore` types and trait as a behavior-preserving module.
3. Extract credential account/session contracts along their existing ownership
   boundaries.
4. Verify downstream imports and implementations before removing declarations
   from `lib.rs`.

## Completion criteria

- Each file is below 1,500 lines or has a smaller follow-up with an explicit
  owner boundary.
- Public exports and CLI-visible behavior remain unchanged.
- Repository formatting, Clippy, architecture checks, and relevant contract
  tests pass during the decomposition change.
