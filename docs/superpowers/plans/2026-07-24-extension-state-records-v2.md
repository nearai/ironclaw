# Filesystem-Native Extension State V2 Implementation Plan

> **For Codex:** Execute this plan with `superpowers:executing-plans` and use
> `superpowers:test-driven-development` for every behavioral step.

**Goal:** Replace the extension store's aggregate filesystem row as the
canonical representation with typed, independently mutable manifest,
installation, membership, and credential-binding records, while
keeping package bytes file-shaped and preserving the existing typed store API.

**Architecture:** `ExtensionInstallationStorePort` remains the only public
domain repository. `ExtensionInstallationStore` reconstructs its existing
`ExtensionInstallation` aggregate from normalized records under
`/system/extensions/.installations/v2`. The previous manifest and installation
rows remain a temporary compatibility projection for transition inspection and
forward repair; v2 is authoritative after bootstrap. User removal soft-removes
membership and installation records instead of erasing their bodies. Package
assets remain under `/system/extensions/{extension_id}` and are not copied into
lifecycle records.

**Tech Stack:** Rust 2024, `RootFilesystem`/`ScopedFilesystem`, portable
`Entry` records and exact indexes, the shared bounded `cas_update` helper,
libSQL/PostgreSQL-backed filesystem implementations, Tokio tests.

## Storage Contract

The v2 domain-owned paths are:

```text
/system/extensions/.installations/v2/
  manifests/{extension_token}.json
  installations/{installation_token}.json
  memberships/{installation_token}/{user_token}.json
  credential-bindings/{installation_token}/{binding_token}.json
```

Each body carries `schema_version = "extension_state.v2"` and its canonical
typed IDs. Each queried ID/status is duplicated into `Entry::indexed`; paths
are locators and never independent identity.

- Manifest record: the validated `ExtensionManifestRecord` wire plus active or
  removed lifecycle status.
- Installation record: installation ID, extension ID, manifest reference,
  active or removed status, timestamps, and the legacy-tenant compatibility
  bit. It contains no member set, binding list, or package bytes.
- Membership record: one `(installation_id, user_id)` row with active or
  removed status and install/remove timestamps.
- Credential-binding record: one `(installation_id, credential_handle)` row
  with active or removed status and an opaque `SecretHandle`; never secret
  material.

Extension failure state is deliberately not persisted here. The host's
activation record already records it and surfaces it to callers, so a second
durable copy could only diverge.

The pre-v2 `manifests/` and `installations/` rows are compatibility projections,
not a second authority. Store startup imports IDs absent from v2, then repairs
the compatibility projection from v2. A deployment must stop old writers
before first v2 startup; rolling old/new writers are unsupported. The retained
projection supports inspection and forward repair, but does not make an
aggregate-only binary a safe rollback target after v2 mutations.

## Task 1: Lock the Normalized Filesystem Contract

**Files:**

- Modify: `crates/ironclaw_extension_registry/tests/installations_contract.rs`

**Step 1: Add failing public store contract tests**

Add tests that use a real `InMemoryBackend` and assert:

1. one install produces exactly one row in each applicable v2 collection;
2. installation core JSON contains no `owner` or `credential_bindings`;
3. membership and binding paths are nested under the hashed installation
   token and bodies repeat the canonical IDs;
4. row kinds and indexed projections are distinct and typed;
5. package bytes are not written under `.installations/v2`;
6. reopening the store reconstructs an aggregate equal to the one written.

Run:

```bash
cargo test -p ironclaw_extension_registry --test installations_contract normalized_v2
```

Expected: FAIL because the v2 collections and record kinds do not exist.

**Step 2: Add failing soft-removal and compatibility tests**

Add tests that assert:

- deleting an installation hides it from `get`/`list` but leaves a removed v2
  core, removed membership tombstones, and binding rows;
- re-upserting the captured aggregate reactivates the same record identities;
- deleting a manifest soft-removes the v2 manifest after no active
  installation remains;
- loading a root containing only legacy aggregate rows imports them into v2
  without deleting the legacy rows;
- reopening after a v2 mutation repairs the legacy compatibility aggregate.

Run the same filtered command and confirm the new assertions fail for the
missing behavior.

## Task 2: Implement V2 Records Behind the Existing Store

**Files:**

- Modify: `crates/ironclaw_extension_registry/src/installations.rs`

**Step 1: Add private wire types and path/index helpers**

Implement the five v2 record types, lifecycle-status enum, schema constant,
record-kind constants, collection roots, deterministic nested paths, parsers,
encoders, and exact index declarations. Validate that body IDs match the key
requested by every direct load.

**Step 2: Add aggregate reconstruction**

For one active installation core:

1. query active membership rows by installation ID;
2. query active credential-binding rows by installation ID;
3. rebuild `InstallationOwner::Users` or the legacy tenant owner;
4. calculate aggregate `updated_at` as the maximum durable component update;
5. call `ExtensionInstallation::from_persisted_parts`.

An active non-legacy core with no active memberships is corrupt and fails
closed. Removed cores are invisible to normal `get`/`list`.

**Step 3: Implement aggregate upsert and soft removal**

`upsert_installation` activates desired membership and binding rows, marks
previously active omitted rows removed, and commits the core
last. Existing active cores first enter a CAS-protected `removing` reservation,
which keeps partial child updates out of reads and makes the compatibility
aggregate a bounded rollback snapshot. It never hard-deletes v2 domain rows.
`delete_installation` idempotently marks active memberships and credential
bindings removed, commits the core tombstone, and then removes the legacy
compatibility projection. `delete_manifest` marks the v2 manifest removed
after proving no active installation references it.

Use `cas_update` through a fixed, store-private `ScopedFilesystem` view for
every record read-modify-write. Do not add a mutex or a local CAS retry loop.

**Step 4: Bootstrap and repair compatibility projections**

During `load_at`:

1. migrate the existing legacy manifest wire as today;
2. ensure all v2 indexes;
3. import legacy manifest/installation IDs absent from v2;
4. verify v2 readback;
5. regenerate active legacy aggregate rows from v2 and remove compatibility
   rows for soft-removed v2 installations/manifests.

V2 wins whenever both forms exist. Preserve legacy rows until v2 readback
succeeds. Surface partial/corrupt records as typed store errors.

**Step 5: Make the Task 1 tests green**

Run:

```bash
cargo test -p ironclaw_extension_registry --test installations_contract normalized_v2
cargo test -p ironclaw_extension_registry --test installations_contract
cargo test -p ironclaw_extension_registry
```

Expected: PASS.

## Task 3: Move User Joins and Leaves Onto Per-User CAS Rows

**Files:**

- Modify: `crates/ironclaw_extension_registry/src/installations.rs`
- Modify: `crates/ironclaw_extension_registry/tests/installations_contract.rs`
- Modify:
  `crates/ironclaw_composition/src/extension_host/extension_lifecycle.rs`

**Step 1: Add failing membership mutation tests**

Extend `ExtensionInstallationStorePort` with:

```rust
async fn activate_membership(
    &self,
    installation_id: &ExtensionInstallationId,
    user_id: &UserId,
) -> Result<ExtensionInstallation, ExtensionInstallationError>;

async fn deactivate_membership(
    &self,
    installation_id: &ExtensionInstallationId,
    user_id: &UserId,
) -> Result<ExtensionInstallation, ExtensionInstallationError>;
```

The trait requires explicit membership operations so a new implementation
cannot silently fall back to aggregate read-modify-write. The concrete
filesystem store performs one-row CAS operations, and every decorator/test
double delegates them explicitly.

Tests prove:

- activating different users does not replace existing memberships;
- repeated activation/deactivation is idempotent;
- deactivation preserves other users and tombstones only the requested row;
- deactivating the final active member fails with a typed error, because final
  package teardown must call `delete_installation`;
- concurrent activations for distinct users lose no membership.

Run:

```bash
cargo test -p ironclaw_extension_registry --test installations_contract membership_v2
```

Expected: FAIL before the methods exist.

**Step 2: Implement per-user CAS mutations**

Use the shared `cas_update` helper on the deterministic membership path.
Reconstruct the aggregate after the row commit and repair the legacy
compatibility projection before returning success. Do not rewrite sibling
membership rows.

**Step 3: Route production lifecycle calls through the new methods**

- Existing-install flow calls `activate_membership` instead of cloning and
  rewriting `InstallationOwner`.
- Non-final remove calls `deactivate_membership` instead of aggregate upsert.
- Final remove retains the existing runtime teardown ordering and calls
  `delete_installation`, which soft-removes the core and all memberships.
- Restore and manifest migration may continue using aggregate upsert.

Enumerate every `ExtensionInstallationStorePort` implementation. Each
production adapter, decorator, test double, and the `Arc<T>` delegating
implementation must forward the new methods.

**Step 4: Run crate and caller tests**

Run:

```bash
cargo test -p ironclaw_extension_registry --test installations_contract membership_v2
cargo test -p ironclaw_extension_registry
cargo test -p ironclaw_composition extension_lifecycle
```

Expected: PASS.

## Task 4: Prove Restart and User Isolation Through Production Composition

**Files:**

- Modify: `tests/integration/extension_user_lifecycle_isolation.rs`

**Step 1: Add a failing caller-path scenario**

Extend `users_install_and_remove_the_same_extension_independently` or add one
adjacent test that:

1. installs the same extension for Alice and Bob through the WebUI product
   surface;
2. removes Alice and shuts the runtime down;
3. independently reopens production composition on the same libSQL storage;
4. proves Alice remains absent and Bob remains active;
5. reinstalls Alice and proves the tombstoned membership reactivates without
   disturbing Bob;
6. removes both and proves neither user sees the soft-removed installation.

Run:

```bash
cargo test --test reborn_integration_extension_user_lifecycle_isolation \
  normalized_user_memberships_survive_runtime_restart_and_soft_removal
```

Expected: FAIL until the restart/reinstall contract is implemented.

**Step 2: Make only the necessary lifecycle/store corrections**

Do not add test-only production behavior. Correct the v2 store or lifecycle
caller until the scenario passes.

**Step 3: Run the full integration binary**

Run:

```bash
cargo test --test reborn_integration_extension_user_lifecycle_isolation
```

Expected: PASS.

## Task 5: Document the Authority, Compatibility, and Rollback Model

**Files:**

- Modify: `docs/reborn/contracts/extensions.md`
- Modify: `CHANGELOG.md`

**Step 1: Update the extension contract**

Replace the two-row layout with the five v2 collections. State explicitly:

- v2 records are canonical;
- legacy aggregate rows are temporary compatibility projections;
- package files remain file/blob data outside lifecycle records;
- membership/install removal is a status/timestamp transition;
- bindings contain handles only;
- old and new binaries must not write concurrently during cutover;
- rollback after v2 mutations requires a data backup or a v2-aware binary;
  stopping writers does not make an aggregate-only binary safe.

Name the store contract and integration test commands that enforce the
behavior.

**Step 2: Add a changelog entry**

Describe the user-visible durability/concurrency improvement without claiming
that package bytes or all user data moved to relational tables.

## Task 6: Verification and Publication

**Files:**

- Review all changed files.
- Complete `.github/pull_request_template.md` in the PR body.

**Step 1: Format and run targeted verification**

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_extension_registry --no-fail-fast
cargo clippy -p ironclaw_extension_registry --all-targets --all-features -- -D warnings
cargo test -p ironclaw_composition extension_lifecycle
cargo clippy -p ironclaw_composition --all-targets --all-features -- -D warnings
cargo test --test reborn_integration_extension_user_lifecycle_isolation
cargo test -p ironclaw_architecture_tests
```

**Step 2: Run repository safety checks**

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
scripts/pre-commit-safety.sh
```

If a broad check is too slow or fails outside the changed scope, retain the
complete output and disclose it precisely; do not mislabel targeted checks as
workspace-wide evidence.

**Step 3: Mechanical audit**

Search changed production Rust for `.unwrap()`, `.expect()`, suspicious byte
slicing, hardcoded temporary paths, leaked raw IDs/secret values, and hand-rolled
CAS loops. Re-read the final diff and confirm the old documentation branch is
unchanged.

**Step 4: Commit and publish**

Commit the implementation with a scoped message, push
`codex/extension-state-records-v2`, and open a draft Track C PR. The PR body
must include:

- the five-record layout and unchanged package-file layout;
- v2 authority and compatibility-projection rules;
- migration/rollback and old-writer exclusion;
- PostgreSQL/libSQL portability through `RootFilesystem`;
- every test tier with evidence or `Not applicable: <reason>`;
- known follow-up: remove the compatibility projection only after the rollback
  window is closed.
