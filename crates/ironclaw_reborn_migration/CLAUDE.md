# ironclaw_reborn_migration

Transitional v1/engine-v2 migration bridge for Reborn. It ships as the
same-version `ironclaw-reborn-migration` companion and is invoked by
`ironclaw-reborn migrate v1`; do not link the v1 root crate into the normal
Reborn CLI/runtime.

The public lifecycle is:

```rust
plan_migration(&MigrationOptions)
preflight_apply_migration(&MigrationOptions, &MigrationManifest, &MigrationSecretInputs, ApplyAcknowledgements)
apply_migration(MigrationOptions, &MigrationManifest, MigrationSecretInputs, ApplyAcknowledgements)
preflight_resume_migration(&MigrationOptions, &MigrationManifest, &MigrationSecretInputs, ApplyAcknowledgements)
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
  decryption; target key resolution follows production Reborn composition. A
  non-empty source `secrets` table makes the source key an apply/resume preflight
  requirement.
- The v1 home is an explicit sealed input independent of the database snapshot
  location. Omitting it records an apply blocker because home coverage is unknown.
- Unknown or incompatible executable artifacts are never enabled.
- `verify` performs read-only target-data checks for users, projects, threads,
  messages, triggers, memory documents, secrets, and identity records in the
  production persistence tables before transitioning the manifest to
  `verified`. It writes lifecycle/quarantine state, does not independently
  read back other manifest domains, and does not boot a full Reborn runtime or
  prove every product service can consume the records. `applying`, `failed`,
  `applied`, and `verifying` targets remain quarantined.
- Apply/resume preflight finishes before the CLI persists `applying`. Apply
  accepts only `planned`; resume accepts `applying`, `failed`, or `applied` for
  the same sealed run.
- libSQL and PostgreSQL targets keep an atomic, run-bound lifecycle claim as
  well as the local marker. The claim binds release/protocol, profile, backend,
  locator fingerprint, tenant, and agent. PostgreSQL makes it visible to every
  replica; startup requires local and durable records to agree when both exist.

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

Converters must claim deterministic slots atomically and use absent-or-exact or
compare-and-create semantics. Exact replay is a no-op; a divergent target
collision fails without overwriting. Exact duplicate engine documents with the
same durable id are deduplicated, while divergent source duplicates fail the
migration. Non-engine memory retains its optional source-agent scope. Supported
automations import paused when their owner is not active, their next-fire time
is missing, or a mission references a project that was not imported; the latter
also omits the invalid project scope.
Unsupported transcript payloads are retained in thread metadata where the
converter explicitly says so. `archive_only` operational categories currently
retain inventory counts/checksums and a disposition only; the source payload is
not copied into a Reborn archive. Do not describe those payloads as archived.

The persisted manifest carries inventory and lifecycle checkpoints. Converter
`LossyItem` details exist only in the `MigrationReport` JSON emitted by apply or
resume, so operator guidance must tell users to retain that output securely and
must not claim `status --json` contains per-record losses.

API-token hashes cannot become Reborn signed sessions. Unknown v1 WASM/channel
packages cannot become runnable Reborn installations. Both require an explicit
re-auth/reinstall disposition.

`heartbeat_state` is unsupported: only actual source rows produce a loss, no
durable heartbeat row is written, and operators must recreate the cadence.

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
