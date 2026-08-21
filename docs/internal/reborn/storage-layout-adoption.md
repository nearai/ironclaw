# Reborn Storage Layout Migration Runbook

This is the operator procedure for moving one supported released Reborn home
to the profile-stable layout. Migration runs automatically at boot, is
rename-based (nothing is copied, nothing is deleted), and fails closed on
every ambiguity. It does not merge deployments, discover arbitrary
directories, or make profiles interchangeable.

## Target layout and boundary

`IRONCLAW_REBORN_HOME` is the one installation storage boundary. After a
successful migration, its durable filesystem layout is:

```text
<IRONCLAW_REBORN_HOME>/
├── config.toml and providers.json
├── layout.toml
├── state/
│   ├── reborn-local-dev.db and recognized libSQL sidecars
│   └── .reborn-local-dev-secrets-master-key
├── system/
│   ├── extensions/
│   ├── prompts/
│   └── skills/
├── workspaces/users/<tenant-user-digest>/
├── runtime/
│   ├── layout-migration.toml
│   └── layout-adoption/snapshot/<legacy-source>/   (staged legacy skills)
├── logs/
├── cache/
└── tmp/
```

`state/` is authoritative application state. `system/` contains host-managed
extensions, prompts, and skills. `workspaces/users/<tenant-user-digest>/` is a
persistent tenant-plus-user workspace leaf. `runtime/` holds provider/process
bookkeeping, the retained migration provenance record, and the staged legacy
skill trees the boot importer reads. `logs/`, `cache/`, and `tmp/` are
operational namespaces, never authoritative application state.

There is no deployment-id directory and no profile-named state directory. The
root `layout.toml` is published last and records the durable-state and
security envelope that startup admits. Until it exists and validates, normal
startup refuses to open stores or start traffic.

## How boot-time migration works

On startup without a valid `layout.toml`, the binary classifies the home:

- **Empty home** → fresh canonical initialization.
- **Exactly one populated legacy source** (`local-dev/`,
  `hosted-single-tenant/`, `hosted-single-tenant-volume/`, or the bare-home
  DB/key set left by a fixed historical bug) → automatic migration.
- **Several populated sources** → the most recently used one (by database and
  master-key mtimes) is migrated; every loser stays byte-for-byte in place and
  is named in `runtime/layout-migration.toml`. An unorderable tie between real
  profile directories fails closed. A bare-home artifact never wins a tie.
- **Unknown content, a populated `hosted-single-tenant-volume-sandboxed/`
  root, or an unsafe shape** (embedded database without its cached master
  key, symlinks, sidecars without the main database) → fail closed with the
  specific reason; nothing is selected or modified.

Before a manifest exists, the installation root admits only the released
profile directories, the canonical namespaces (empty until initialization or
migration), the bare legacy database/key unit, and IronClaw's known operator
files: `config.toml`, `providers.json`, `webui-token`, and
`.onboard-completed.json` (plus the migration lock while admission holds it).
Every other top-level file or directory fails closed, even when empty, so
fresh initialization and bare-home migration cannot strand unknown content
beside a newly published manifest.

The migration itself, guarded by an advisory lock on the home and a POSIX
record-lock probe against SQLite's locking ranges (which detects live readers,
writers, and idle open WAL connections of any other process):

1. Writes `runtime/layout-migration.toml` with `phase = "in-progress"`.
2. Renames the database unit, master key, and `system/` content into
   `state/` and `system/`; renames legacy skill trees into
   `runtime/layout-adoption/snapshot/<source>/` for the normal boot importer.
3. Marks the record `phase = "complete"`, then publishes `layout.toml` last.
   The manifest preserves the legacy external-memory namespace
   (`legacy_memory_provider_app_id`).

Renames are same-volume metadata operations: nothing is duplicated, nothing
destroyed. The in-progress record persists the exact target manifest before
the first rename. A crash while that record is still in progress refuses with
a restore-the-backup message rather than guessing. If every move completed and
the record was durably marked complete but the process crashed before the final
manifest publication, startup admits that recorded target against the current
request, validates every canonical namespace, and republishes only that exact
manifest.

## Operator controls

- `IRONCLAW_REBORN_STORAGE_MIGRATION=manual` — defer migration; startup
  reports the deferral and exits instead of migrating. Unset or `automatic`
  is the default.
- `IRONCLAW_REBORN_PROFILE=migration-dry-run` — validate-only admission with
  the production database/environment configuration; never initializes,
  migrates, or starts traffic.
- Stateful CLI commands never migrate; they report the `ironclaw serve`
  remedy and leave the home untouched.

## Before upgrading a populated home

1. Stop every old `ironclaw` process that could use the home. Platform
   recreate deploys (volume-backed services) already guarantee this; the
   live-writer probe is the mechanical backstop.
2. Take and retain a filesystem/volume snapshot or backup of the entire home.
   Migration renames rather than copies, so the backup is the rollback path.
3. Optionally run the validate-only profile first:

   ```bash
   export IRONCLAW_REBORN_HOME=/absolute/path/to/ironclaw-reborn
   IRONCLAW_REBORN_PROFILE=migration-dry-run ironclaw serve
   ```

## Profiles, workspaces, and credentials

A profile selects runtime policy and process backend only. It may be changed
only by an operator-controlled restart. Startup compares the requested profile
with the persisted `layout.toml` security envelope and rejects changes that
alter durable backend, tenancy, or weaken per-caller workspace isolation.

For Docker, a sandbox gets exactly one selected
`workspaces/users/<tenant-user-digest>` leaf as `/workspace`. It never
receives the Reborn home, `state/`, the cached master key, `system/`,
`runtime/`, a workspace parent, sibling workspaces, provider credentials,
Railway tokens, or a Docker socket.

## Rollback and retention

Rolling back to an old binary is one-way and operational: stop IronClaw,
preserve/archive the canonical target, then restore the pre-migration backup
before starting the old binary. An old binary cannot safely read the canonical
layout. Never run old and new binaries against diverging copies. The retained
`runtime/layout-migration.toml` record and any ignored legacy sources are
diagnostic state; do not delete them as cleanup.

## Recorded-QA credential source

`IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_ROOT`, used by recorded QA fixtures, is
the Reborn **installation root**—the directory that contains `config.toml` and
`state/`—not a legacy `local-dev/` profile directory and not `state/` itself.
The fixture derives the database and master-key paths from
`<source-root>/state/`; its tenant, user, and agent selectors are separately
controlled by the corresponding `..._TENANT`, `..._USER`, and `..._AGENT`
variables.

## Regression commands

The bounded layout admission and migration machinery is covered by
`crates/app/ironclaw_cli/src/runtime/storage_layout/` tests. The canonical
path and transition contract is covered by
`crates/app/ironclaw_config/tests/profile_contract.rs`; run:

```bash
cargo test -p ironclaw_config --test profile_contract
cargo test -p ironclaw storage_layout
cargo test -p ironclaw storage_boot
cargo test -p ironclaw_composition --test profile_acceptance
```
