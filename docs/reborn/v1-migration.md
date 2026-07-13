# Migrate IronClaw v1 data to Reborn

The Reborn Docker image ships `ironclaw-reborn-migration` beside
`ironclaw-reborn`. Source builds must build both executables into the same
target directory and compile the primary CLI with the intended target backend
(`--features libsql` or `--features postgres`). The companion enables both
backends by default. Native `cargo-dist` installers do not yet package the pair.
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
the target profile, database, tenant, and agent through the same Reborn
configuration rules as the runtime. PostgreSQL key configuration is resolved
without opening the database; local target-key creation is deferred until
apply preconditions pass.

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

Remote PostgreSQL sources must use TLS; a remote URL with
`sslmode=disable` is rejected. Local PostgreSQL snapshots may explicitly
disable TLS. Migration source and target URLs must omit the PostgreSQL
`options` connection parameter; locator fingerprinting rejects it rather than
risk incorporating opaque or secret-bearing session options. PostgreSQL source
sessions are forced read-only, and the sealed source fingerprint binds table
contents rather than credentials.

If the source `secrets` table contains any rows, export the old key before
apply or resume:

```bash
export MIGRATION_SOURCE_SECRET_MASTER_KEY='...'
```

Do not put either value in a command argument, shell history, manifest, or log.
Planning can inventory encrypted rows without the key, but apply/resume treats a
missing source key as a preflight blocker rather than silently skipping secrets.

## 1. Rehearse and review the plan

For libSQL, create a WAL-consistent backup rather than copying a live database
file. For PostgreSQL, restore a fresh `pg_dump` into a migration-only database
or use an equivalent consistent snapshot. A rehearsal against a running source
is advisory only; final apply must use a stopped-source snapshot.

```bash
ironclaw-reborn migrate v1 plan \
  --source-libsql /backups/ironclaw-v1.db \
  --source-home /srv/ironclaw-v1 \
  --manifest /secure/migration-v1.json
```

For PostgreSQL:

```bash
ironclaw-reborn migrate v1 plan \
  --source-postgres \
  --source-home /srv/ironclaw-v1 \
  --manifest /secure/migration-v1.json
```

`plan` does not open or create target storage. On Unix, the manifest is written
with owner-only permissions; on other platforms, protect the manifest with the
platform's filesystem ACLs. It contains hashed store locators, counts,
dispositions, warnings, and blockers—not raw database URLs, tokens, or keys.
Planning refuses to overwrite an existing manifest; choose a new path, or
deliberately remove the old manifest before replanning.

Use `--strict` when archive-only, re-auth, reinstall, unsupported, or blocked
categories should make planning return failure after writing the reviewable
manifest. Strict mode evaluates registered inventory entries by disposition,
but absent zero-count categories do not constitute data loss. Blockers remain
strict failures regardless of their recorded count.

`--source-home` must name the actual v1 home (or its matching rehearsal
snapshot), independently of where a database backup was placed. The final plan
must use the stopped-source home snapshot that apply will receive. Omitting it
leaves home-artifact coverage unproven and records an apply-blocking inventory
entry. If the configured Reborn target is nested under that home, inventory
excludes only the exact target-owned tree; adjacent v1 files and directories
remain part of the sealed source-home fingerprint.

Review every category before scheduling downtime:

- `imported`: represented directly in Reborn;
- `semantically_converted`: carried into a different Reborn concept;
- `archive_only`: inventoried in the manifest but not exported or made live;
- `requires_reauth` or `requires_reinstall`: cannot be reused safely;
- `intentionally_reset`: transient runtime state that starts clean;
- `skipped_by_operator`: explicitly excluded by an operator-selected scope;
- `unsupported`: no safe target representation in this release;
- `unsupported_unknown`: not recognized by this release and therefore a
  blocker;
- `derived_rebuilt`: derived state that Reborn recomputes.

No unknown v1 table or persistent home artifact is treated as implicitly
successful. Unknown or unreadable categories remain visible in the manifest,
make `--strict` planning fail after writing it, and block apply even when the
plan was created without `--strict`.

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
  --source-home /srv/ironclaw-v1 \
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
  --source-home /srv/ironclaw-v1 \
  --manifest /secure/migration-v1.json \
  --confirm-v1-stopped \
  --confirm-source-snapshot
```

The source inventory/content fingerprint and the target backend, locator,
profile, tenant, agent, and source-home seal must still match the plan.
Divergent deterministic target records fail closed instead of being
overwritten.
Apply and resume complete source, manifest, key, and acknowledgement preflight
before persisting an `applying` checkpoint. Apply additionally proves target
freshness and accepts only a `planned` manifest; resume accepts `applying`,
`failed`, or `applied` for the same run.
Apply, resume, and verify also update the target-owned
`$IRONCLAW_REBORN_HOME/.v1-migration-state.json` marker atomically. Runtime
startup consults this canonical marker rather than assuming the manifest is at
a default path, so an interrupted operation remains quarantined even when the
operator selected a manifest elsewhere.
Both libSQL and PostgreSQL targets also keep an atomic, run-bound lifecycle
claim in `reborn_migration_state`. The claim binds the release, protocol,
profile, backend, target fingerprint, tenant, and agent, and another run cannot
replace it. PostgreSQL stores this authority in the shared database so every
replica observes the quarantine even when replicas use different local homes.
Startup requires local and durable state to agree when both are present.

Successful apply and resume print a `MigrationReport` JSON document to stdout.
Preserve it in a secure operator-controlled location: its `lossy` entries
contain the per-record conversion losses that are not copied into the persisted
migration manifest and can include source identifiers. `status --json` reports
the redacted manifest and target-fingerprint match, not those apply-time losses.

## 4. Verify before startup

```bash
ironclaw-reborn migrate v1 verify \
  --source-libsql /backups/ironclaw-v1.db \
  --source-home /srv/ironclaw-v1 \
  --manifest /secure/migration-v1.json

ironclaw-reborn migrate v1 status \
  --manifest /secure/migration-v1.json
```

`status` does not open the source or target database, but it re-resolves the
current production target configuration to report `target_fingerprint_match`.
Run it with the intended Reborn home/profile and configured target env inputs,
including PostgreSQL URL and key variables when that profile requires them.

Do not start Reborn unless status is `verified`. Current verification closes
migration-owned handles and checks exact structural counts for users, projects,
threads, messages, triggers, memory documents, and secrets, plus a lower-bound
identity-record count, in production durable tables/paths. Other manifest
domains do not receive an independent durable readback. Verification updates
manifest/quarantine lifecycle state, but its target data reads are read-only;
it is not a full production cold-boot/readback test.
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

- canonical users, data owners synthesized for older schemas, supported
  engine-v2 project documents, conversations, supported identity links, memory,
  secrets, and supported schedules are current migration candidates. Canonical
  deactivated or unknown user states map fail-closed to suspended, unknown roles
  map to member. Synthesized owners are members with epoch timestamps: older
  schemas without a canonical users table produce active users, while owners
  missing from an existing canonical table are suspended;
- non-engine memory documents preserve their optional source-agent scope rather
  than being rebound to the configured target agent;
- exact duplicate engine project, mission, and mission-thread documents sharing
  a durable UUID are deduplicated; divergent duplicates fail the migration;
- supported cron routines and missions import paused when their migrated owner
  is absent or suspended. Missing `next_fire_at` also imports paused, retaining a
  deterministic historical timestamp for operator review. A mission that
  references an unimported project drops that project scope and imports paused;
- typed settings, engine-v2 plan/runtime documents without an explicit
  converter, unsupported schedule sources, unsupported executable artifacts,
  v1 home `projects/` content, and operational histories are currently
  inventoried/reported rather than converted. Unsupported transcript payloads
  are reported and retained in thread metadata only where the converter
  explicitly supports that fallback;
- API/session credentials require re-authentication;
- unknown or incompatible executable extensions require reinstall and are
  never enabled as placeholders;
- agent jobs/actions/events are archive-only inventory (their payloads are not
  copied), while approvals, leases, pairing requests, rate-limit counters,
  logs, lock files, and derived indexes are reset or rebuilt;
- `heartbeat_state` is unsupported in this release. Actual heartbeat rows are
  reported as requiring operator action, no durable heartbeat row is written,
  and the cadence must be recreated as a Reborn scheduled trigger.

Run `status --json` to inspect the complete redacted manifest and the current
target-fingerprint match programmatically. Use the securely retained apply or
resume report for per-record loss details.
