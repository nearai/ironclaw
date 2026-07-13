# ironclaw_reborn_migration

Transitional v1/engine-v2 migration bridge for Reborn. It ships as the
same-version `ironclaw-reborn-migration` companion and is invoked by
`ironclaw-reborn migrate v1`; do not link the v1 root crate into the normal
Reborn CLI/runtime.

The public lifecycle is:

```rust
plan_migration(&MigrationOptions)
apply_migration(MigrationOptions, &MigrationManifest, MigrationSecretInputs, ApplyAcknowledgements)
resume_migration(MigrationOptions, &MigrationManifest, MigrationSecretInputs, ApplyAcknowledgements)
verify_migration(&MigrationOptions, &MigrationManifest)
```

`run_migration` is a deprecated compatibility wrapper. New code must use the
explicit lifecycle.

## Safety contract

- Source readers are narrow, read-only adapters. Never use the v1 runtime
  connection constructor because it runs schema migrations.
- Planning must not open or create the Reborn target. It may write a manifest
  only at the explicit operator path.
- Apply requires a stopped v1 source and a consistent snapshot acknowledgement.
- The manifest contains redacted locator fingerprints, not database URLs,
  keys, tokens, or decrypted values.
- Source and target keys are independent. Source key input is used only for v1
  decryption; target key resolution follows production Reborn composition.
- The v1 home is an explicit sealed input independent of the database snapshot
  location. Omitting it records an apply blocker because home coverage is unknown.
- Unknown or incompatible executable artifacts are never enabled.
- `verify` performs a cold, read-only structural check of supported records in
  the production persistence tables before transitioning the manifest to
  `verified`. It does not boot a full Reborn runtime or prove every product
  service can consume the records. `applying`, `failed`, `applied`, and
  `verifying` targets remain quarantined.
- PostgreSQL lifecycle state is stored in the shared target as well as the local
  marker so every replica enforces the same quarantine.

## Source and target ownership

`source.rs` owns supported v1 schema reads for PostgreSQL and libSQL. Engine-v2
state is stored as JSON documents inside v1 persistence; `v2_model.rs` contains
the compatibility DTOs.

Target profile, store, tenant, agent, and encryption configuration come from
`ironclaw_reborn_composition::resolve_reborn_migration_target`. Mount aliases
must use composition's production resolver. Do not reintroduce migration-only
target paths or identity defaults.

The companion protocol is intentionally small:

```text
ironclaw-reborn-migration __handshake
ironclaw-reborn-migration v1 plan|apply|resume|verify|status
```

PostgreSQL source URLs and source keys come from
`MIGRATION_SOURCE_POSTGRES` and `MIGRATION_SOURCE_SECRET_MASTER_KEY`.
Production target URL/key names come from Reborn `config.toml`. They must never
be accepted as raw CLI values.

## Disposition and fidelity

The versioned inventory is the source of truth. Every known v1 table and
persistent home artifact must be classified as imported, semantically
converted, archive-only, re-auth/reinstall, intentionally reset,
operator-skipped, unsupported, or derived/rebuilt. A new v1 category without a
registry entry is a blocker, not an implicit skip.

Converters must implement deterministic compare-and-upsert semantics. Exact
replay is a no-op; a divergent target collision fails without overwriting.
Unsupported transcript payloads are retained in thread metadata where the
converter explicitly says so. `archive_only` operational categories currently
retain inventory counts/checksums and a disposition only; the source payload is
not copied into a Reborn archive. Do not describe those payloads as archived.

API-token hashes cannot become Reborn signed sessions. Unknown v1 WASM/channel
packages cannot become runnable Reborn installations. Both require an explicit
re-auth/reinstall disposition.

## Validation

Minimum focused gates:

```bash
cargo test -p ironclaw_reborn_migration --no-default-features --features libsql
cargo test -p ironclaw_reborn_migration --no-default-features --features postgres
cargo clippy -p ironclaw_reborn_migration --all-targets --all-features -- -D warnings
```

Acceptance coverage must prove source immutability, target absence after plan,
manifest redaction, stopped-snapshot enforcement, idempotent resume, collision
handling, full inventory disposition, libSQL/PostgreSQL parity, and cold
structural readback. Full runtime/service readback remains additional work; a
converter-only roundtrip does not prove that the normal Reborn runtime can see
all migrated data.

The operator runbook is `docs/reborn/v1-migration.md`.
