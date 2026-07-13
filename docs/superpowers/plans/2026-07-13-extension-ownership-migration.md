# Per-User Extension Ownership Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one migration command that assigns every installed extension to every existing tenant user, and ensure a non-final user removal cleans only that user's personal extension state.

**Architecture:** Reuse `ironclaw_reborn_migration` to open libSQL/PostgreSQL and the composition-owned tenant installation store. The migration lists users, builds one `InstallationOwner::Users` set, and upserts every installation whose owner differs. The existing lifecycle remover keeps package teardown on the final member but runs actor cleanup before every membership leave.

**Tech Stack:** Rust 2024, clap, serde, `ExtensionInstallationStore`, `RebornUserDirectory`, libSQL/PostgreSQL root filesystems, Tokio tests.

## Global Constraints

- Keep the migration explicit; do not run it automatically at startup.
- Default execution applies the rewrite; `--dry-run` is the only optional mode.
- The server must be stopped while applying the migration.
- Do not add a persistence schema, HTTP endpoint, rollback command, or general migration framework.
- Never print database URLs, secrets, credentials, or user profile data.

---

### Task 1: Extension Ownership Migration Command

**Files:**
- Create: `crates/ironclaw_reborn_migration/src/extension_ownership.rs`
- Create: `crates/ironclaw_reborn_migration/src/extension_ownership_main.rs`
- Modify: `crates/ironclaw_reborn_migration/src/lib.rs`
- Modify: `crates/ironclaw_reborn_migration/src/target.rs`
- Modify: `crates/ironclaw_reborn_migration/Cargo.toml`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/src/lib.rs`
- Test: `crates/ironclaw_reborn_migration/src/extension_ownership.rs`

**Interfaces:**
- Produces: `run_extension_ownership_migration(ExtensionOwnershipMigrationOptions) -> Result<ExtensionOwnershipMigrationReport, MigrationError>`.
- Consumes: `TargetStore`, `TenantId`, repeatable extra `UserId` values, `RebornUserDirectory::list_users`, and `ExtensionInstallationStore::{list_installations, upsert_installation}`.

- [ ] **Step 1: Write failing migration tests**

Add tests that seed two users plus an explicit bootstrap operator, one tenant-owned installation, and one partially user-owned installation. Assert dry-run writes nothing, apply gives both installations the complete three-user owner set, and a second apply reports zero changes.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p ironclaw_reborn_migration --features libsql extension_ownership
```

Expected: compilation/test failure because the migration module and command do not exist.

- [ ] **Step 3: Add the tenant-qualified migration store seam**

Add a `migration-support`-gated composition function that opens
`/tenants/<tenant>/system/extensions/.installations/state.json` over a supplied
`RootFilesystem`. Re-export only that function; do not expose the concrete
filesystem store.

- [ ] **Step 4: Implement the minimal migration loop**

Implement pagination over `RebornUserDirectory` with a bounded page size, union
the explicit users, reject an empty set, list installations, and for each row:

```rust
let desired = InstallationOwner::users(user_ids.clone())?;
if installation.owner() != &desired {
    if !options.dry_run {
        store
            .upsert_installation(installation.with_owner(desired.clone()))
            .await?;
    }
    changed.push(extension_id);
}
```

Return only tenant id, sorted user ids, sorted installed extension ids, sorted
changed extension ids, and `dry_run` in the serializable report.

- [ ] **Step 5: Add the thin CLI**

Add binary `ironclaw-reborn-extension-ownership-migration` with mutually
exclusive `--target-libsql` / `--target-postgres`, required `--tenant-id`,
repeatable `--include-user`, and optional `--dry-run`. Parse database URLs into
`SecretString` and print only the JSON report.

- [ ] **Step 6: Run focused tests and lint**

Run:

```bash
cargo test -p ironclaw_reborn_migration --features libsql extension_ownership
cargo clippy -p ironclaw_reborn_migration --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

### Task 2: Cleanup on Non-Final Member Removal

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Test: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`

**Interfaces:**
- Consumes: existing `ExtensionRemovalCleanupRegistry`, `revoke_exclusive_credentials`, and `RemoveDecision`.
- Produces: the existing `remove` response contract; no new public interface.

- [ ] **Step 1: Write the failing caller-level regression**

Seed a two-member installation with a declared external cleanup requirement and
personal credential provider. Remove as Alice and assert the cleanup adapter and
credential cleanup each receive Alice's scope once, Alice leaves the owner set,
Bob remains, and package/manifest/runtime state remains.

- [ ] **Step 2: Run the focused regression and verify RED**

Run:

```bash
cargo test -p ironclaw_reborn_composition --all-features non_final_member_remove
```

Expected: FAIL because the current `RemoveDecision::LeaveMembers` early return
records neither external nor credential cleanup.

- [ ] **Step 3: Remove only the premature return**

Compute the remove decision before cleanup, but route both `LeaveMembers` and
final teardown through the existing manifest/provider/actor cleanup. After
cleanup succeeds, call `remove_locked`; it already rewrites the member set for a
non-final leave and performs full teardown for the final member.

- [ ] **Step 4: Run lifecycle tests and lint**

Run:

```bash
cargo test -p ironclaw_reborn_composition --all-features non_final_member_remove
cargo test -p ironclaw_reborn_composition --all-features extension_remove
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

### Task 3: Local Before/After Acceptance QA

**Files:**
- No repository files.
- Backup/report artifacts stay under a mode-0700 directory under the operator's
  chosen temporary or scratch path.

**Interfaces:**
- Consumes: port 8745 hosted-volume stack, operator bearer, Account A/B bearers, and the new migration binary.
- Produces: restorable before snapshot plus endpoint evidence for shared-before/private-after behavior.

- [ ] **Step 1: Back up and seed the before-state**

Take an online SQLite backup plus copy the matching secrets master key. Through
the operator extension API install Slack, GitHub, Gmail, Google Calendar, Docs,
Drive, Sheets, Slides, NearAI, Notion, and Web Access. Verify A and B project
each as `install_scope: shared`.

- [ ] **Step 2: Preserve the shared fixture and stop the server**

Take a second database backup after seeding, then remove the launchctl service.
Verify port 8745 has no listener before migration.

- [ ] **Step 3: Run dry-run, apply, and idempotence checks**

Run the new binary against the local database with tenant `reborn-cli` and
`--include-user reborn-local-operator`. Save the dry-run report, apply once, then
run dry-run again and assert `changed_extension_ids` is empty.

- [ ] **Step 4: Restart and verify user isolation**

Build/restart the exact branch, verify A and B project every seeded extension as
private, remove GitHub as A, and assert A no longer lists it while B still does.
Do not remove the final user in the preserved stack.

- [ ] **Step 5: Run final repository checks**

Run:

```bash
cargo test -p ironclaw_reborn_migration --all-features
cargo test -p ironclaw_reborn_composition --all-features
cargo test -p ironclaw_architecture
cargo clippy --workspace --all-targets --all-features -- -D warnings
scripts/pre-commit-safety.sh
```

Expected: every command exits 0 with zero warnings.
