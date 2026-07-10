# Migrate IronClaw v1 data to Reborn

The Reborn Docker image ships `ironclaw-reborn-migration` beside
`ironclaw-reborn`. Source builds must build both executables into the same
target directory. Native `cargo-dist` installers do not yet package the pair.
Operators use the companion through the primary binary:

```text
ironclaw-reborn migrate v1 plan
ironclaw-reborn migrate v1 apply
ironclaw-reborn migrate v1 resume
ironclaw-reborn migrate v1 verify
ironclaw-reborn migrate v1 status
```

The launcher never searches `PATH`. It resolves the companion beside its own
executable, rejects symlinks or writable helpers on Unix, and requires the same
release version and migration protocol before forwarding a command. Database
URLs and master keys stay in environment variables and are never forwarded in
argv.

Migration is an explicit, offline cutover workflow. Neither normal Reborn
startup nor container startup imports v1 automatically. Keep the v1 database,
home, and backups until the migrated target has verified and been accepted.

## Before planning

Install a release artifact containing both executables. Select the Reborn home
and profile exactly as they will be used after cutover. The migrator resolves
the target profile, database, tenant, agent, and target encryption key through
the same Reborn configuration rules as the runtime.

For local and volume-backed profiles:

```bash
export IRONCLAW_REBORN_HOME="$HOME/.ironclaw/reborn"
```

For PostgreSQL-backed profiles, set `[storage].url_env` and
`[storage].secret_master_key_env` in Reborn `config.toml`, then export the
values under those names. The defaults are `IRONCLAW_REBORN_POSTGRES_URL` and
`IRONCLAW_REBORN_SECRET_MASTER_KEY`.

The v1 PostgreSQL source uses a separate variable:

```bash
export MIGRATION_SOURCE_POSTGRES='postgresql://...'
```

If encrypted v1 secrets should be converted, also export the old key:

```bash
export MIGRATION_SOURCE_SECRET_MASTER_KEY='...'
```

Do not put either value in a command argument, shell history, manifest, or log.

## 1. Rehearse and review the plan

For libSQL, create a WAL-consistent backup rather than copying a live database
file. For PostgreSQL, restore a fresh `pg_dump` into a migration-only database
or use an equivalent consistent snapshot. A rehearsal against a running source
is advisory only; final apply must use a stopped-source snapshot.

```bash
ironclaw-reborn migrate v1 plan \
  --source-libsql /backups/ironclaw-v1.db \
  --manifest /secure/migration-v1.json
```

For PostgreSQL:

```bash
ironclaw-reborn migrate v1 plan \
  --source-postgres \
  --manifest /secure/migration-v1.json
```

`plan` does not open or create target storage. The manifest is written with
owner-only permissions and contains hashed store locators, counts,
dispositions, warnings, and blockers—not raw database URLs, tokens, or keys.
Use `--strict` when archive-only, re-auth, reinstall, unsupported, or blocked
categories should make planning return failure after writing the reviewable
manifest.

Review every category before scheduling downtime:

- `imported`: represented directly in Reborn;
- `semantically_converted`: carried into a different Reborn concept;
- `archive_only`: inventoried in the manifest but not exported or made live;
- `requires_reauth` or `requires_reinstall`: cannot be reused safely;
- `intentionally_reset`: transient runtime state that starts clean;
- `unsupported`: no safe target representation in this release.

No unknown v1 table or persistent home artifact is treated as implicitly
successful. Unknown categories block a strict plan and remain visible in the
manifest.

## 2. Stop v1 and take the final snapshot

1. Stop every v1 process, worker, and external writer.
2. Confirm that no process still has the database open for writes.
3. Create the final WAL-aware libSQL or PostgreSQL snapshot.
4. Back up relevant v1 home files and the current Reborn target, if any.
5. Re-run `plan` against the final snapshot if its fingerprint differs from the
   rehearsal.

The first release migrates only into a fresh staged Reborn target. It does not
support live dual-write, delta capture, or merging into an active target.

## 3. Apply or resume

Pass the same source snapshot again because its location is deliberately not
recoverable from the redacted manifest:

```bash
ironclaw-reborn migrate v1 apply \
  --source-libsql /backups/ironclaw-v1.db \
  --plan /secure/migration-v1.json \
  --confirm-v1-stopped \
  --confirm-source-snapshot
```

For PostgreSQL, use `--source-postgres` and keep
`MIGRATION_SOURCE_POSTGRES` pointed at the restored snapshot database.

If apply is interrupted, use the same manifest and source:

```bash
ironclaw-reborn migrate v1 resume \
  --source-libsql /backups/ironclaw-v1.db \
  --manifest /secure/migration-v1.json \
  --confirm-v1-stopped \
  --confirm-source-snapshot
```

Source and target fingerprints must still match the sealed plan. A mismatch or
divergent target collision fails closed instead of silently overwriting data.
Apply, resume, and verify also update the target-owned
`$IRONCLAW_REBORN_HOME/.v1-migration-state.json` marker atomically. Runtime
startup consults this canonical marker rather than assuming the manifest is at
a default path, so an interrupted operation remains quarantined even when the
operator selected a manifest elsewhere.

## 4. Verify before startup

```bash
ironclaw-reborn migrate v1 verify \
  --source-libsql /backups/ironclaw-v1.db \
  --manifest /secure/migration-v1.json

ironclaw-reborn migrate v1 status \
  --manifest /secure/migration-v1.json
```

Do not start Reborn unless status is `verified`. Current verification closes
migration-owned handles and checks structural counts in production durable
tables/paths; it is not yet a full production cold-boot/readback test.
`applying`, `failed`, `applied`, and `verifying` are quarantined states;
workers, triggers, adapters, and ingress must remain stopped. After `verified`,
perform a production canary and inspect representative migrated data before
accepting cutover.

After verification, start Reborn and issue new API/session credentials. v1
token hashes cannot be converted into Reborn signed sessions because their
plaintext tokens are unavailable.

## Rollback boundary

Before Reborn accepts new writes, rollback is straightforward: stop Reborn,
quarantine or discard the staged target, and restart v1 from its untouched
database and home. After Reborn accepts new writes, rollback is not lossless;
there is no reverse migrator. Retain the stopped v1 installation and backups
until users have accepted the Reborn result.

## Data compatibility summary

The manifest is the authority for a particular release and source. In broad
terms:

- users, engine-v2 projects, conversations, supported identity links, memory,
  secrets, and supported schedules are current migration candidates, with
  per-record losses reported;
- typed settings, unsupported message payloads, unsupported schedule sources,
  unsupported executable artifacts, and operational histories are currently
  inventoried/reported rather than converted;
- API/session credentials require re-authentication;
- unknown or incompatible executable extensions require reinstall and are
  never enabled as placeholders;
- in-flight jobs, approvals, leases, pairing requests, rate-limit counters,
  logs, lock files, and derived indexes are reset or rebuilt.

Run `status --json` to inspect the complete redacted manifest and the current
target-fingerprint match programmatically.
