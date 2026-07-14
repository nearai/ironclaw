# Per-User Extension Ownership Migration

**Date:** 2026-07-13
**Status:** Approved
**Related issue:** [nearai/ironclaw#5953](https://github.com/nearai/ironclaw/issues/5953)

## Goal

Add one small migration command that changes every currently installed
extension from tenant/shared ownership to user ownership for every existing
user. The extension package and runtime remain singleton tenant infrastructure;
only install membership becomes per-user.

## Migration Behavior

Add a dedicated binary under the existing `ironclaw_reborn_migration` crate.
It accepts the Reborn libSQL path or PostgreSQL URL, tenant id, and optional
extra user ids such as the bootstrap operator.

When run, it:

1. lists every persisted user for the tenant;
2. unions any explicitly supplied user ids;
3. loads every installed extension from the tenant-qualified installation
   store;
4. rewrites every installation owner to
   `Users { user_ids: all_discovered_users }`; and
5. prints the users and extension ids it changed.

If there are no users, the command fails without writing. Re-running it is
safe: rows already carrying the complete user set are unchanged. An optional
`--dry-run` prints the same plan without writing.

The server must be stopped while the migration runs so its in-memory extension
snapshot cannot overwrite the migrated state. The migration uses the existing
typed installation store rather than raw JSON/SQL mutation, so the same command
works for libSQL and PostgreSQL.

No automatic startup migration, new HTTP endpoint, general migration
framework, custom rollback command, or new persistence schema is added.

## Required Removal Fix

The current lifecycle returns early when one user leaves a multi-user
installation, skipping that user's external connection and credential cleanup.
After this migration nearly every removal is a non-final member leave, so that
early return must be removed.

Every user removal must first run actor-scoped Slack/extension cleanup and
exclusive personal credential cleanup. It then removes only that user from the
member set. Package/runtime teardown still occurs only when the final member
removes the extension.

## Tests

- Dry run reports users/extensions and writes nothing.
- Apply changes every installation to the complete user set.
- Explicit bootstrap users are included even when absent from the directory.
- Re-running is a no-op.
- A non-final member removal runs personal cleanup, removes only that member,
  and leaves the other user's extension operational.
- The final member removal still performs full teardown.

## Local Before/After Test

On the existing port-8745 hosted-volume stack:

1. Back up the local database and secrets key.
2. Install Slack, GitHub, Gmail, Google Calendar, Docs, Drive, Sheets, Slides,
   NearAI, Notion, and Web Access through the operator so they are tenant/shared.
3. Save that database as the reproducible **before** snapshot and verify Account
   A and Account B see the extensions as shared.
4. Stop the server, run the migration including `reborn-local-operator`, and
   restart the exact branch binary.
5. Verify Account A and Account B see the extensions as private/mine.
6. Remove a non-Slack extension as Account A and confirm Account B keeps it.
7. After connecting Slack for both users, remove it as Account A and confirm A's
   personal Slack state is cleaned while B remains installed and connected.

## Railway QA Packaging

The Reborn Railway runtime image must include the existing
`ironclaw-reborn-extension-ownership-migration` binary so an operator can run
the migration against a volume-backed libSQL database. The binary is built with
only the `libsql` backend enabled and without the optional legacy full-migration
read stack, then copied into the runtime image beside `ironclaw-reborn`. Default
migration crate builds still include the full legacy-to-Reborn migrator. The
normal image entrypoint and application startup behavior remain unchanged;
merely deploying the image never runs the migration.

The existing CLI Dockerfile smoke suite must verify both sides of this contract:

1. the builder compiles the exact migration binary from
   `ironclaw_reborn_migration` with `--no-default-features --features libsql`;
2. the runtime stage copies that exact binary into `/usr/local/bin`.

The alternatives are intentionally rejected for this one-time operation:

- a separate migration image would require moving or reattaching the one-volume
  Railway service and add avoidable operational risk;
- remote-libSQL URL/token support would expand the migration's storage contract
  even though QA already exposes the database as a mounted filesystem;
- a Railway pre-deploy command cannot be used because Railway does not mount
  persistent volumes into pre-deploy containers.

The QA run remains an explicit operator sequence: deploy the image, create a
manual Railway volume backup, run a read-only dry run, stop application writers,
apply once against the discovered `reborn-local-dev.db`, restart the normal
entrypoint, and run a final dry run that reports no changed extensions.
