# Unified Extension Runtime — End-to-End Implementation Plan

> **Execution note:** Follow this plan against the NEA-25 tip described below.
> Read the design and normative contract before editing. Implement in vertical,
> production-wired slices and keep the verification ledger current in every PR.

**Goal:** Complete the unified extension architecture so one resolved extension
contract is bound exactly to narrow tool/channel/auth implementations and no
generic IronClaw production code contains concrete product behavior.

**Baseline:** `900d435ee4d8496fb0d711fcf2f52807f1d414d3`
(`origin/nea25/08-audit-fixes`).

**Design:**
`docs/superpowers/specs/2026-07-09-unified-extension-runtime-design.md`

**Normative contract:** `docs/reborn/contracts/extension-runtime.md`

**Verification ledger:** `docs/reborn/extension-runtime-verification.md`

**Implementation language/stack:** Rust 2024, Tokio, Axum, serde/TOML, existing
`RootFilesystem`, libSQL/PostgreSQL stores, existing Reborn integration harness,
React/TypeScript WebUI v2.

---

## 1. Execution rules

1. Use one branch stack or independently reviewable PR stack in the order below.
2. Start each task with the named failing test. Do not add a new registry or
   interface without a real production caller in the same or immediately
   dependent slice.
3. Keep old behavior working until its production caller moves, then delete the
   old path in that slice. Do not leave two indefinite runtime paths.
4. Keep `ironclaw_reborn_composition` assembly-only.
5. Do not edit generated `openwiki/` files.
6. Preserve unrelated worktree changes.
7. Update `docs/reborn/extension-runtime-verification.md` evidence columns as
   requirements become real. An unchecked box is not silently waived.
8. Every persistent state change must have libSQL and PostgreSQL coverage.
9. Every helper that gates a side effect needs a caller-level test through the
   actual dispatcher/route/manager.
10. Run `cargo test -p ironclaw_architecture` in every slice that changes
    boundaries, manifests, concrete-name allowlists, or crate dependencies.
11. A cutover slice must add/migrate/forward its old durable and route inputs
    before it deletes the old production reader. S11 audits/completes these
    migrations; it must not be the first point at which an already-cut-over
    path becomes readable.

## 2. Slice graph

```text
S0 durable gates
  └─ S1 low-level contracts + immutable package compiler
      └─ S2 package blob store + digests + durable records
          └─ S4 binding contracts/loaders + concrete runtime skeletons
              └─ S3 migrate all first-party manifests/auth declarations
                  └─ S5 ExtensionHost atomic active set
                      └─ S6 tool dispatch cutover
                          └─ S7 generic auth host/provider cutover
                              └─ S8 generic channel ingress cutover
                                  └─ S9 generic outbound/target/action cutover
                                      └─ S11 legacy state/config migration
                                          └─ S10 Slack extraction + generic UI deletion
                                              └─ S12 zero-specificity proof
```

Slice labels are stable traceability IDs, not numeric execution order; follow
the graph exactly. There is no parallel merge path in the critical chain. S4
must provide loaders and concrete Slack/Telegram/provider entrypoint skeletons
before S3 changes roots to those services. S7 depends on the bound tool path in
S6. S11 must stage/migrate every old record and compatibility alias before S10
deletes the old readers/construction paths.

---

## 3. Slice S0 — Permanent architecture gates and test fixtures

**Purpose:** Make the target measurable before behavior moves.

### Task S0.1: Add path-scoped concrete-product source gate

**Files:**

- Modify `crates/ironclaw_architecture/tests/reborn_retired_taxonomy.rs`
- Add `crates/ironclaw_architecture/tests/reborn_extension_specificity.rs`
- Modify `crates/ironclaw_architecture/Cargo.toml` only if a parser dependency is
  actually required

**Red test:**

Add a test that scans all production `crates/**/src`, WebUI TypeScript, and
production Cargo/config TOML by default. Exempt only concrete extension/provider
crates, generated first-party catalog output, versioned migration paths, tests,
fixtures, and docs. Derive concrete extension/provider IDs, vendor hosts, route
tokens, and crate names from the package/catalog inventory so a future Discord
branch is caught without first adding `discord` to the scanner. Keep host-login
SSO in an explicitly annotated non-extension scope.

Use exact token/pattern matching that distinguishes tests/comments where
reasonable. Add a negative scanner fixture with a newly invented product ID,
vendor host, route, feature, and concrete import and prove all are detected.
Allow only path-scoped entries in a temporary
`LEGACY_EXTENSION_SPECIFICITY_ALLOWLIST`, with one entry per current production
file and a reason/removal slice.

**Implementation:**

- Do not make the gate green by skipping whole directories.
- Test/fixture/docs/migration paths are sanctioned by explicit predicate.
- Fail if an allowlisted path no longer contains a match; this forces allowlist
  shrinkage.
- Fail on a newly seen path.
- Give every entry `remove_by_slice: S6..S12`.

**Verification:**

```bash
cargo test -p ironclaw_architecture --test reborn_extension_specificity
```

Expected at S0: green only because every existing violation is individually
enumerated. Expected at S12: allowlist empty.

### Task S0.2: Add inverse dependency rules

**Files:**

- Modify
  `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`

**Red tests:**

- Generic composition/workflow/auth/config/CLI/WebUI/dispatcher crates cannot
  depend on concrete extension crates.
- `ironclaw_extension_host` (once added) cannot depend on concrete extensions.
- Concrete extension crates may depend on generic adapter/host-api contracts,
  never composition/auth-host/ingress implementations.
- Generated first-party catalog is the sole sanctioned generic-to-concrete
  aggregation edge and must live in `ironclaw_first_party_extensions`.

At S0, express future crate rules as a table that ignores a crate only if its
directory does not yet exist. Once created, rules become active automatically.

### Task S0.3: Add arbitrary extension fixtures

**Files:**

- Add `tests/fixtures/extensions/acme-messenger/manifest.toml`
- Add `tests/fixtures/extensions/acme-messenger/manifests/tools/ping.toml`
- Add `tests/fixtures/extensions/acme-messenger/manifests/channels/messages.toml`
- Add `tests/fixtures/extensions/acme-messenger/manifests/auth/account.toml`
- Add a second channel fixture under
  `tests/fixtures/extensions/second-messenger/`

Fixtures must use no real provider name and eventually drive backend/frontend
genericity tests. Until v3 compiler exists, store them without adding a caller.

### Task S0.4: Add machine-verifiable evidence ledger

**Files:**

- Add `docs/reborn/extension-runtime-evidence.toml`
- Add `scripts/ci/check-extension-runtime-verification.sh`
- Add checker tests/fixtures under `scripts/ci/tests/`

The checker extracts every atomic `CATEGORY-NNN` ID from
`docs/reborn/extension-runtime-verification.md`, rejects duplicates, and
requires exactly one TOML entry containing `status`, `evidence_type`,
`test_or_command`, `commit_sha`, `backend_or_feature`, `result_date`, and
`reviewer`. It rejects `[x]` without passing evidence and rejects stale evidence
for a changed requirement. Aggregate REL/SIGN rows reference atomic IDs rather
than duplicate test claims. Final release mode rejects pending, blocked, and
superseded rows; an ADR must update/remove the old requirement, not waive it.

### Task S0.5: Commit slice

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_architecture
bash scripts/ci/check-extension-runtime-verification.sh --allow-pending
git diff --check
```

Commit: `test(architecture): define generic extension specificity gate`

---

## 4. Slice S1 — Low-level contracts and immutable-package manifest compiler

**Purpose:** Produce one pure, immutable resolved contract from root plus typed
leaves without lifecycle side effects.

### Task S1.0: Create the dependency-cycle-safe contract crate

**Files:**

- Add workspace member `crates/ironclaw_extension_contracts`
- Add `Cargo.toml`, `AGENTS.md`, `CLAUDE.md`
- Add `src/{lib,ids,surface,binding_scope,channel,auth}.rs`
- Modify `ironclaw_host_api` to make `AuthProviderId` canonical and expose the
  old `RuntimeCredentialAccountProviderId` as a temporary deprecation alias
- Move `ExtensionInstallationId` into the new crate and reexport temporarily
  from `ironclaw_extensions`
- Add architecture dependency rules

The crate owns neutral resolved surface DTOs and identity/scope types only. It
does not parse TOML, load runtimes, use network/stores, or depend on operational
adapter crates. Add compile-time/serde/strong-ID tests and conversion tests for
retiring `ProductAdapterId`/`AdapterInstallationId`.

### Task S1.1: Create compiler modules and types

**Files:**

- Add `crates/ironclaw_extensions/src/manifest/mod.rs`
- Add `crates/ironclaw_extensions/src/manifest/fragment.rs`
- Add `crates/ironclaw_extensions/src/manifest/resolver.rs`
- Add `crates/ironclaw_extensions/src/manifest/source_map.rs`
- Add `crates/ironclaw_extensions/src/manifest/digest.rs`
- Add `crates/ironclaw_extensions/src/manifest/package_index.rs`
- Add `crates/ironclaw_extensions/src/resolved.rs`
- Modify `crates/ironclaw_extensions/src/lib.rs`
- Make only narrow compatibility changes in
  `crates/ironclaw_extensions/src/v2.rs`
- Add `crates/ironclaw_extensions/tests/manifest_fragment_contract.rs`

**First failing tests:**

1. v3 root plus one tool fragment resolves to the same semantic capability as
   an equivalent v2 inline manifest.
2. Root order is preserved.
3. Fragment cannot be parsed/installed alone.
4. Root with inline section plus fragments fails.
5. Duplicate normalized fragment path fails.
6. Missing/non-UTF-8/non-table/wrong-schema/wrong-kind/empty fragment fails with
   package-relative line/column.
7. Absolute, URL, drive, backslash, NUL/control, empty, `.`, `..`, glob, nested
   import, and symlink/mount escape fail.
8. Root >256 KiB, leaf >64 KiB, >512 leaves, closure >2 MiB fail before
   unbounded materialization.
9. Channel/auth root section local name differing from body `id` fails.
10. Full package file/path/per-file/runtime/aggregate/archive/ratio limits fail
    during bounded streaming snapshot construction.
11. Asset refs and package digest are evaluated against the same immutable
    `InstalledPackageSnapshot`; mutating the source directory afterwards cannot
    change compilation.

**Implementation:**

- Introduce `InstalledPackageSnapshot`, `PackageIndexV1`,
  `ManifestClosureSnapshot`, `ManifestFragmentInput`,
  `HostApiManifestInput`, `ResolvedExtensionManifest`, and source-map types
  exactly as specified in the design.
- `compile_package` is pure over one immutable package snapshot.
- `snapshot_from_filesystem` uses existing `RootFilesystem` and bounded reads;
  do not invent another filesystem trait.
- Normalize paths using existing `VirtualPath`/extension asset types.
- Depth is exactly one.
- Unknown fragment envelope fields fail via `deny_unknown_fields`.
- Never expose host absolute paths in errors.

### Task S1.2: Change host-API contract input

**Files:**

- Modify contract traits/registry in
  `crates/ironclaw_extensions/src/v2.rs` or extract them to
  `crates/ironclaw_extensions/src/host_api/mod.rs`
- Modify
  `crates/ironclaw_extensions/src/host_api/capability_provider.rs`
- Modify `crates/ironclaw_product_adapter_registry/src/lib.rs`
- Add/modify:
  - `crates/ironclaw_product_adapter_registry/tests/manifest_ingestion.rs`
  - `crates/ironclaw_product_adapter_registry/tests/registry_contract.rs`

**First failing tests:**

- Capability-provider v2 aggregates ordered capability leaves.
- Duplicate capability IDs across leaves fail globally.
- Channel contract accepts exactly one channel leaf per host-API instance.
- Auth-provider contract accepts exactly one auth leaf.
- Contract gets typed body/provenance, not a deep-merged root table.
- Current validation for credentials, ingress route policy, egress, and unknown
  fields remains effective on a fragmented channel.

**Implementation:**

- Add new host API IDs:
  - `ironclaw.capability_provider/v2`
  - `ironclaw.channel/v1`
  - `ironclaw.auth_provider/v1`
  - `ironclaw.hooks/v1`
- Add typed channel/auth resolved declarations and surface IDs.
- Keep v1/v2 root compatibility mapping isolated from v3 domain types.
- For v3, resolve every product-auth credential reference to explicit auth
  surface or pinned dependency.
- Do not implement runtime binding in this slice.
- Parse hooks into typed `ironclaw_hooks::HookManifestEntry`, include them in
  resolved/canonical contract, and remove composition raw-TOML reprojection in
  the later activation cutover.
- Reject v3 `runtime.kind = system`; add an explicit historical v2 migration/
  rejection test.

### Task S1.3: Model dynamic tool provider

**Files:**

- Modify `crates/ironclaw_extensions/src/hosted_mcp_discovery.rs`
- Add dynamic declaration types to `resolved.rs`
- Extend `manifest_fragment_contract.rs`
- Extend existing hosted MCP tests in
  `crates/ironclaw_extensions/src/hosted_mcp_discovery.rs`

**First failing tests:**

- Dynamic provider declaration resolves as one expected provider binding.
- Discovered child exceeding namespace/count/schema/effect/credential/host-port
  ceiling is rejected.
- Read-only/destructive annotation behavior remains conservative.
- Static provider still requires static leaves.

For this slice, adapt current discovery to validate through the new ceiling;
active discovery generation publication lands with S5/S6.

### Task S1.4: Wire discovery caller

**Files:**

- Modify `ExtensionDiscovery::load_package_entry` in
  `crates/ironclaw_extensions/src/lib.rs`
- Modify `crates/ironclaw_extensions/tests/discovery_manifest_bound.rs`
- Modify `crates/ironclaw_extensions/tests/extension_contract.rs`

**Caller-level tests:**

- Real filesystem root plus leaves discovers one package.
- Bad package quarantines itself; sibling succeeds.
- Existing extension-count bound prevents surplus reads.
- Source paths/limits are honored through the real discovery caller.

### Task S1.5: Verify/commit

```bash
cargo test -p ironclaw_extension_contracts
cargo test -p ironclaw_extensions --test manifest_fragment_contract
cargo test -p ironclaw_extensions --test discovery_manifest_bound
cargo test -p ironclaw_extensions --test extension_contract
cargo test -p ironclaw_product_adapter_registry --test manifest_ingestion
cargo test -p ironclaw_product_adapter_registry --test registry_contract
cargo clippy -p ironclaw_extensions -p ironclaw_product_adapter_registry --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture
git diff --check
```

Commit: `feat(extensions): compile typed manifest fragments into one contract`

---

## 5. Slice S2 — Durable package store, resolved records, and activation CAS

**Purpose:** Make the compiled closure/digests the only durable source of
manifest authority.

### Task S2.1: Implement content-addressed package storage and authenticity

**Files:**

- Add `crates/ironclaw_extensions/src/package_store.rs`
- Add `crates/ironclaw_extensions/src/package_authenticity.rs`
- Add `crates/ironclaw_extensions/tests/manifest_digest_contract.rs`
- Add `crates/ironclaw_extensions/tests/package_store_contract.rs`
- Modify `crates/ironclaw_extensions/src/lib.rs`
- Modify `crates/ironclaw_trust` with `PackageSignerTrustStore` and
  Ed25519/catalog/local-unsigned policy inputs

**First failing tests:**

- Path/length framing prevents concatenation ambiguity.
- Leaf byte mutation changes closure/package digest.
- Runtime/schema/prompt asset mutation changes package digest.
- Whitespace/comment-only manifest edit does not change contract digest.
- Semantic reorder changes digest only when order is semantic.
- Authority field mutation changes contract digest.
- Golden canonical bytes are deterministic.
- Missing/duplicate/unlisted package payload files fail.
- Detached signature signs identity/version/package digest and is not included
  recursively in that digest.
- Ed25519 wire/message/key-ID/base64url validation plus unknown/revoked/expired/
  wrong-source signer rejection matches design section 8.13.
- Stage failure exposes no package/install record.
- Atomic commit/open returns exact immutable bytes.
- Corrupt/missing blob quarantines; no mutable-source refetch.
- Generation lease pins root/dependency blobs.
- GC skips installed/active/pending/rollback/leased blobs and is crash-idempotent.
- Quota/file/count/aggregate limits hold on libSQL and PostgreSQL.

**Implementation:**

- Use the `ManifestClosureDigest`, `PackageDigest`, and `ContractDigest` types
  created in S1.
- Use the design's domain-separated SHA-256 framing.
- Add versioned canonical DTO and compact sorted canonical JSON.
- Implement `PackageAuthenticityV1`: build-catalog attestation, detached
  Ed25519 registry signature, and sandboxed local unsigned policy. Do not claim
  current trust already verifies registry signatures.

### Task S2.2: Replace raw-root record authority

**Files:**

- Modify `crates/ironclaw_extensions/src/installations.rs`
- Modify `crates/ironclaw_extensions/tests/installations_contract.rs`
- Modify `crates/ironclaw_product_adapter_registry/src/lib.rs`
- Modify product-adapter registry tests

**First failing tests:**

- `ResolvedManifestRecord` round-trips root, ordered leaves, resolved contract,
  source map, and digests.
- v2 `{raw_toml, source, manifest_hash}` migrates to versioned root-only closure.
- Restart projection succeeds with source filesystem unavailable.
- Stored digest mismatch fails closed.
- Product-adapter/channel projection reads resolved host-API input and has no
  raw TOML reparse.

**Implementation:**

- Version the stored wire.
- Make record constructors compute authoritative digests; callers do not pass a
  duplicate arbitrary hash.
- Deprecate `raw_toml()` and remove production domain reprojection uses.
- Preserve a root-source accessor only for diagnostics/compat migration if
  necessary, with architecture gate preventing authority use.

### Task S2.3: Add CAS activation commit store contract

**Files:**

- Modify `crates/ironclaw_extensions/src/installations.rs`
- Extend installation store contract tests

**First failing tests:**

- `commit_activation(expected_revision, commit)` succeeds once.
- stale revision fails without state change.
- package/contract/generation mismatch fails.
- manifest record plus install/activation commit updates atomically at trait
  contract level.

Add revision/tenant/generation/digest fields now; the live active set lands in
S5.

### Task S2.4: Persist in composition store with DB parity

**Files:**

- Modify
  `crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store.rs`
  as a temporary owner; S5 moves it to the new host module
- Modify existing durable integration tests under `tests/integration/`
- Add migration fixtures under `tests/fixtures/extensions/state/`

**Caller-level tests:**

- install fragmented package, close store, remove/unmount source tree, reopen,
  and restore exact contract;
- fragment-only tamper is rejected;
- concurrent stale activation CAS is rejected;
- both libSQL and PostgreSQL preserve the same wire/behavior.
- fresh schema and exact NEA-25 baseline upgrade create/backfill equivalent
  package/manifest/install/revision/lease tables, constraints, and indexes;
- failure injection at every multi-record transition rolls back;
- concurrent migration lock/CAS behavior and supported old/new binary schema
  window are explicit;
- PostgreSQL cases run against a real testcontainer and do not silently skip.

### Task S2.5: Package inventory and trust inputs

**Files:**

- Modify `crates/ironclaw_reborn_composition/build.rs`
- Modify
  `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`
- Modify `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Modify
  `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle/active_publication.rs`
- Modify `crates/ironclaw_host_runtime/src/production.rs`
- Modify `crates/ironclaw_reborn_composition/src/factory.rs`

**First failing tests:**

- Generated inventory includes all allowed package files, sorted.
- Symlinks and source-only `wasm-src/**` are excluded/rejected as specified.
- Package identity/trust uses full package digest.
- Fragment-only authority widening is not silently migrated.
- Package-byte change with same contract reloads/revalidates code.

Keep lifecycle behavior stable; full authority-delta activation lands S5.

### Task S2.6: Verify/commit

```bash
cargo test -p ironclaw_extensions --test manifest_digest_contract
cargo test -p ironclaw_extensions --test installations_contract
cargo test -p ironclaw_product_adapter_registry
cargo test --test reborn_integration_durable extension_install_survives_independent_reopen
cargo test -p ironclaw_architecture
git grep -n 'raw_toml()' -- crates ':!crates/ironclaw_extensions'
git diff --check
```

The grep may report only explicitly allowlisted migration/diagnostic tests.

Commit: `feat(extensions): persist resolved contracts and package digests`

---

## 6. Slice S3 — Migrate all first-party manifests to v3 fragments

**Purpose:** Prove modular manifests are generic and preserve exact behavior.

### Task S3.1: Freeze semantic parity fixtures

**Files:**

- Add generated/checked-in semantic snapshots under
  `tests/fixtures/extensions/first_party_manifest_parity/`
- Extend tests in
  `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`

Snapshot all 11 roots and all 137 static capabilities, including every field
listed in the design, plus derived surfaces and Slack route/direction/auth/
credential/egress data.

The red test should fail after a naive split that drops or changes any field.

### Task S3.2: Split every package

**Files:**

- Rewrite each root under
  `crates/ironclaw_first_party_extensions/assets/*/manifest.toml`
- Add `manifests/tools/*.toml` under every static package
- Add Slack:
  - `assets/slack/manifests/channels/messages.toml`
  - `assets/slack/manifests/auth/user_account.toml`
- Add Telegram package:
  - `assets/telegram/manifest.toml`
  - `assets/telegram/manifests/channels/messages.toml`
  - `assets/telegram/manifests/auth/bot_credentials.toml`
- Add explicit auth fragments for every owning root:
  - Google OAuth references pin `google.oauth/v1`
  - Notion OAuth/DCR references pin `notion.oauth/v1`
  - GitHub manual-token references point to one host-managed manual auth surface
  - NEAR AI manual-token reference points to one host-managed manual auth surface
- Add dynamic-provider leaves for hosted MCP packages

Rules:

- one static tool per leaf;
- explicit stable root order;
- fully qualified public IDs unchanged;
- all asset refs package-root-relative;
- no inline v3 operational section;
- no orphan leaf and no missing import.

Assert the baseline migration matrix: 63 Google OAuth, 18 Notion OAuth, 5
Slack OAuth, 47 GitHub manual-token, and 1 NEAR AI manual-token references all
resolve to explicit v3 auth surfaces/dependencies.

### Task S3.3: Fix real-asset callers and CI filters

**Files include:**

- `tests/integration/support/github.rs`
- `tests/integration/support/harness_web_access.rs`
- `tests/integration/support/extension_surface.rs`
- `crates/ironclaw_reborn_composition/tests/gsuite.rs`
- `crates/ironclaw_host_runtime/tests/github_wasm_runtime_contract.rs`
- `crates/ironclaw_host_runtime/tests/extension_v2_lifecycle_e2e.rs`
- `.github/workflows/test.yml`
- `.github/workflows/platform-and-compat.yml`

Replace root-text-only parsing with the compiler/package snapshot. Expand path
filters to all relevant extension assets/fragments.

### Task S3.4: Verify/commit

```bash
cargo test -p ironclaw_extensions
cargo test -p ironclaw_product_adapter_registry
cargo test -p ironclaw_host_runtime --test extension_v2_lifecycle_e2e
cargo test -p ironclaw_reborn_composition bundled_first_party_manifest_asset_refs_are_packaged
cargo test --test reborn_group_extensions
cargo test -p ironclaw_architecture
git diff --check
```

Commit: `refactor(extensions): split first-party contracts into typed leaves`

---

## 7. Slice S4 — Binding contracts, narrow adapters, and runtime loaders

**Purpose:** Define and prove the exact manifest-to-implementation join without
changing all production dispatchers yet.

### Task S4.1: Create `ironclaw_extension_host` skeleton

**Files:**

- Add workspace member `crates/ironclaw_extension_host`
- Add:
  - `Cargo.toml`
  - `AGENTS.md`
  - `CLAUDE.md`
  - `src/lib.rs`
  - `src/entrypoint.rs`
  - `src/bindings.rs`
  - `src/bound_extension.rs`
  - `src/loaders/mod.rs`
  - `src/loaders/native.rs`
  - `src/loaders/wasm.rs`
  - `src/loaders/mcp.rs`
  - `src/loaders/script.rs`
  - `src/resolvers.rs`
  - `wit/extension-runtime.wit`
  - `tests/binding_contract.rs`
- Modify root `Cargo.toml`
- Activate architecture rules added in S0

**First failing tests:**

- exact binding happy path;
- missing/extra/duplicate/wrong-kind;
- inbound/outbound/action direction mismatch;
- dependency owner/digest/export mismatch;
- host-port/credential widening;
- duplicate capability/route/provider conflict;
- ABI mismatch;
- error redaction/source key.

### Task S4.2: Split channel operational interfaces

**Files:**

- Add under `crates/ironclaw_product_adapters/src/channel/`:
  - `mod.rs`
  - `ingress.rs`
  - `outbound.rs`
  - `connection.rs`
  - `target.rs`
  - `action.rs`
  - `types.rs`
- Modify `crates/ironclaw_product_adapters/src/lib.rs`
- Deprecate/split `src/adapter.rs`
- Update adapter contract tests

Remove metadata getters (`surface_kind`, capabilities, auth requirement,
declared egress) from the new operational traits. Keep a compatibility wrapper
for current callers only until S8/S9, with an allowlist deletion item.

### Task S4.3: Add tool and auth interfaces/resolvers

**Files:**

- Add `ToolAdapter`, `ToolBindingResolver`, normalized invocation/result, and
  minimal scoped tool-port contracts to `ironclaw_extension_contracts`
- Modify `crates/ironclaw_auth/src/provider.rs`
- Add auth resolver/bound-provider types under `crates/ironclaw_auth/src/`
- Add contract tests in owning crates

Do not yet delete `AuthProviderClient`; adapt it behind the new interface until
S7.

### Task S4.4: Implement all loader/catalog and concrete skeletons

**Files:**

- Complete native/WASM/MCP/script loaders and versioned WIT ABI
- Add app-layer workspace member `crates/ironclaw_first_party_extension_catalog`
  with generated catalog interface/data; do not make lower-layer
  `ironclaw_first_party_extensions` depend on concrete product crates
- Add skeleton workspace crates `ironclaw_slack_extension`,
  `ironclaw_telegram_extension`, `ironclaw_google_auth_provider`, and
  `ironclaw_notion_auth_provider`, each with boundary docs and a side-effect-free
  entrypoint/factory that can be referenced by S3 manifests
- Add a fake native entrypoint and minimal WASM component fixture under
  `crates/ironclaw_extension_host/tests/fixtures/`

**Tests:**

- unknown native service fails;
- installed/untrusted package cannot request native;
- native factory receives immutable package/contract, not product-specific
  composition state;
- WASM world/ABI mismatch fails;
- MCP/script compatibility entrypoints load existing package roots;
- exported duplicate/missing keys fail at bind;
- memory/deadline/concurrency bounds are enforced;
- no concrete extension dependency from host crate.
- only `ironclaw_first_party_extension_catalog` aggregates concrete native
  factories.

### Task S4.5: Verify/commit

```bash
cargo test -p ironclaw_extension_host --test binding_contract
cargo test -p ironclaw_product_adapters
cargo test -p ironclaw_auth
cargo test -p ironclaw_slack_extension
cargo test -p ironclaw_telegram_extension
cargo test -p ironclaw_google_auth_provider
cargo test -p ironclaw_notion_auth_provider
cargo clippy -p ironclaw_extension_host -p ironclaw_product_adapters -p ironclaw_auth --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture
git diff --check
```

Commit: `feat(extension-host): bind resolved surfaces to exact typed adapters`

---

## 8. Slice S5 — Atomic `ExtensionHost` and active resolver views

**Purpose:** Make one service the only active-set writer and publish complete
generations atomically.

### Task S5.1: Implement immutable active set and drain guards

**Files:**

- Add to `crates/ironclaw_extension_host/src/`:
  - `active_set.rs`
  - `activation.rs`
  - `drain.rs`
  - `health.rs`
- Add `tests/activation_contract.rs`
- Add `tests/performance_contract.rs`
- Extend `resolvers.rs`

**First failing tests:**

- active snapshot is immutable to readers;
- failed load/bind/readiness/conflict/store CAS leaves snapshot unchanged;
- successful activation increments generation once and publishes all surfaces;
- resolver handles retain generation `Arc` across upgrade;
- resolver lookup is scope/tenant aware and rejects ambiguous eligible
  installations unless an authorized explicit installation is selected;
- deactivation removes new lookup and drains in-flight work;
- bounded drain cancels/records overdue work safely;
- concurrent activate uses revision CAS; one wins;
- duplicate global capability/route/provider conflict prevents publication.
- instrumented lookup proves no request-time TOML parse/store read; staged
  publication performs one pointer swap; concurrent activate/resolve stress
  exposes no mixed generation.

### Task S5.2: Move installation persistence behavior

**Files:**

- Move generic store implementation from
  `crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store.rs`
  into `crates/ironclaw_extensions/src/filesystem_installation_store.rs`
- Update imports and architecture rules
- Keep the existing legacy reader behind a temporary migration adapter until
  S11 installs its versioned replacement; do not delete it in S5

Do a behavior-preserving move with existing tests first, then add CAS methods.

### Task S5.3: Implement install/activate/restore/upgrade/deactivate/remove

**Files:**

- Complete `activation.rs` and `lib.rs`
- Modify composition factory to construct one `ExtensionHostBuilder`
- Modify lifecycle facade to delegate generic lifecycle commands
- Add `tests/integration/extension_runtime.rs`
- Add root `Cargo.toml` test entry named
  `reborn_integration_extension_runtime`

**Caller-level tests:**

1. Install fragmented fixture -> activate -> all expected resolver views appear.
2. Inject failure at load/bind/readiness/persist -> no resolver view appears.
3. Crash simulation after durable CAS/before live swap -> restore publishes the
   committed generation once.
4. Restore with one invalid independent package quarantines it and publishes
   valid siblings.
5. Upgrade swaps complete generation; old in-flight call completes on old Arc.
6. Deactivate rejects new work and drains.
7. Remove cleans generic bindings/state and reports retryable partial cleanup.
8. libSQL and PostgreSQL behavior matches.
9. Typed hook declarations install/uninstall atomically with the generation and
   no composition TOML reparse remains.

### Task S5.4: Authority delta and rollback

**Files:**

- Add `authority_delta.rs` in `ironclaw_extension_host`
- Modify trust/approval integration at existing lifecycle caller
- Add contract and caller tests

Test byte-only, equivalent, narrowing, widening, dependency conflict, approval
denial, approved activation, and rollback to persisted prior snapshot.

### Task S5.5: Dynamic discovery generation

Integrate dynamic provider binding with active snapshot:

- discover through bound adapter;
- validate ceiling;
- publish child tool bindings as one immutable discovery generation;
- store discovery digest/health;
- refresh cannot partially replace children.

Add a caller test through the real hosted MCP discovery path.

### Task S5.6: Durable generation and serving leases

**Files:**

- Add `crates/ironclaw_extension_host/src/generation_lease.rs`
- Add `crates/ironclaw_extension_host/src/serving_lease.rs`
- Extend filesystem/DB store contracts and both backend implementations

Auth flows, ingress admissions, delivery/reconciliation, cleanup, durable
targets, and rollback records acquire exact generation/package/dependency/ABI
leases transactionally. Test restart rehydration, TTL, terminal release, GC
blocking, and never switching to latest.

Implement one extension-serving leader lease/fencing token per deployment or
tenant partition. Nonholders are not ready for extension-bound routes and
cannot publish/mutate. Test two PostgreSQL-backed hosts contending, fencing a
stale holder, failover restore, and single-process libSQL behavior. Do not imply
multi-active-replica snapshot consistency.

### Task S5.7: Verify/commit

```bash
cargo test -p ironclaw_extension_host
cargo test --test reborn_integration_durable
cargo test -p ironclaw_architecture
cargo clippy -p ironclaw_extension_host -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
git diff --check
```

Commit: `feat(extension-host): publish atomic active extension generations`

---

## 9. Slice S6 — Tool dispatch cutover

**Purpose:** Make every capability invocation resolve a prebound tool, while
preserving host policy behavior.

### Task S6.1: Adapt existing runtime lanes to `ToolAdapter`

**Files:**

- `crates/ironclaw_dispatcher/src/lib.rs`
- `crates/ironclaw_host_runtime/src/services/runtime_adapters.rs`
- `crates/ironclaw_host_runtime/src/services/wasm_execution.rs`
- `crates/ironclaw_host_runtime/src/first_party.rs`
- owning MCP/script runtime modules reached from those adapters
- `crates/ironclaw_host_runtime/src/wasm_credentials.rs`
- `crates/ironclaw_extension_host/src/resolvers.rs`

Create `WasmToolAdapter`, `McpToolAdapter`, `ScriptToolAdapter`, and
`NativeToolAdapter` wrappers around existing execution behavior. Keep secret
injection/egress/host-port enforcement in host code.

### Task S6.2: Red tests through dispatcher caller

Add/modify dispatcher and integration tests to prove:

- known capability resolves its exact bound generation;
- unknown capability fails before runtime work;
- authorization/approval/obligation/resource reservation still execute;
- missing credential still emits generic auth gate;
- credential injection uses manifest/auth binding and never adapter metadata;
- tool adapter cannot access undeclared host port/credential/egress;
- runtime result/event/audit semantics remain unchanged;
- dynamic MCP child invokes through its provider adapter after ceiling check;
- runtime-kind mismatch branch is no longer part of invocation path.

### Task S6.3: Cut production dispatcher

Replace the current registry/package/runtime-kind lookup in
`crates/ironclaw_dispatcher/src/lib.rs:232-316` with `ToolBindingResolver`.
Composition injects a resolver handle from `ExtensionHost`.

Delete per-invocation runtime adapter selection and any now-unused parallel
registry. Keep runtime technology selection only in the loader at activation.

### Task S6.4: Slack tool proof

For the first production cutover, the Slack entrypoint returns adapters backed
by the existing WASM tool artifact. Add an integration test that activates the real Slack package
and invokes each of its five capability IDs through the generic dispatcher with
recorded egress/credentials. Assert no Slack branch in dispatcher/host runtime.

### Task S6.5: Verify/commit

```bash
cargo test -p ironclaw_dispatcher
cargo test -p ironclaw_host_runtime
cargo test --test reborn_group_extensions
cargo test --test reborn_integration_extension_runtime tool
cargo test -p ironclaw_architecture
cargo clippy -p ironclaw_dispatcher -p ironclaw_host_runtime -p ironclaw_extension_host --all-targets --all-features -- -D warnings
git diff --check
```

Commit: `refactor(dispatcher): invoke prebound extension tool adapters`

---

## 10. Slice S7 — Generic auth host and provider cutover

**Purpose:** Remove provider-specific OAuth behavior and string multiplexing
from auth core/composition.

### Task S7.1: Create `ironclaw_auth_host`

**Files:**

- Add workspace member `crates/ironclaw_auth_host`
- Add `Cargo.toml`, `AGENTS.md`, `CLAUDE.md`
- Add:
  - `src/lib.rs`
  - `src/service.rs`
  - `src/routes.rs`
  - `src/state.rs`
  - `src/egress.rs`
  - `src/callback.rs`
  - `src/refresh.rs`
  - `src/revoke.rs`
  - `tests/auth_host_contract.rs`
- Add architecture rules

### Task S7.2: Write provider-agnostic red tests

Use the arbitrary auth fixture and fake adapter. Drive the actual generic Axum
routes/service and real durable auth stores:

- begin creates flow bound to caller/extension/install/surface/provider digest;
- scope intersection rejects widening;
- authorization plan stays within allowed host/redirect ceiling;
- callback validates state/TTL/replay/PKCE/caller/install/generation;
- malformed provider response is redacted;
- normalized grant encrypts/stores account and resumes continuation once;
- refresh rotates token safely;
- revoke is idempotent;
- cross-install/cross-tenant callback fails;
- implementation digest changed between begin/callback fails or deliberately
  resolves the pinned old generation, never silently switches;
- both DB backends.
- host-managed manual-secret auth stores encrypted fields without a runtime
  binding, and optional declared remote validation resolves exactly one
  `ManualAuthValidatorAdapter` through restricted injection;
- authorization plans cannot override/duplicate state, redirect_uri, PKCE,
  client_id, scope, or response_type, and issuer mix-up checks pass.

### Task S7.3: Move concrete provider behavior to adapters

**Files to refactor/remove behavior from:**

- `crates/ironclaw_auth/src/oauth.rs`
- `crates/ironclaw_reborn_composition/src/product_auth/oauth/oauth_gate.rs`
- `.../oauth/oauth_provider_client.rs`
- `.../credentials/product_auth_providers.rs`
- `.../serve/oauth.rs`
- `.../serve/mod.rs`
- `crates/ironclaw_reborn_composition/src/slack/slack_personal_oauth.rs`

Create concrete provider adapters in owning provider/extension crates. Slack
provider logic goes to `ironclaw_slack_extension::auth`. Complete
`ironclaw_google_auth_provider` and `ironclaw_notion_auth_provider` here,
including Google OAuth, Notion OAuth/DCR, manual-token host-managed paths, and
their dependency exports. S11 validates migration/refcount/coexistence; it does
not introduce providers after this cutover.

### Task S7.4: Cut production routes and gates

- Mount one generic start/status/callback/revoke route family.
- Inject `AuthProviderResolver` from active extension host.
- Remove `if provider == SLACK/GOOGLE/...` route branches.
- Remove `MultiplexAuthProviderClient` string map once all callers bind directly.
- Ensure missing-credential gate resolves the auth surface referenced by the
  tool contract and resumes via generic continuation.

Preserve old callback URLs only as explicit compatibility aliases that forward
into the generic service and carry a removal version.

In the same slice, migrate/forward existing product-auth flow/account records,
Slack personal bindings, and provider callback state into generation-pinned
new records before deleting a reader.

### Task S7.5: Real Slack auth proof

Activate real Slack package, start its auth surface, run callback through
scripted token egress, persist account, invoke blocked tool, and assert resume.
Test scope union/least privilege exactly against the migrated fragment.

### Task S7.6: Verify/commit

```bash
cargo test -p ironclaw_auth
cargo test -p ironclaw_auth_host
cargo test -p ironclaw_google_auth_provider
cargo test -p ironclaw_notion_auth_provider
cargo test --test reborn_integration_extension_runtime auth
cargo test --test reborn_integration_oauth_connect
cargo test -p ironclaw_architecture
cargo clippy -p ironclaw_auth -p ironclaw_auth_host -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
git diff --check
```

Commit: `refactor(auth): resolve OAuth through bound extension providers`

---

## 11. Slice S8 — Generic channel ingress

**Purpose:** Replace Slack-specific route mounting, installation selection, and
signature handling with one manifest-driven host.

### Task S8.1: Create `ironclaw_extension_ingress`

**Files:**

- Add workspace member `crates/ironclaw_extension_ingress`
- Add `Cargo.toml`, `AGENTS.md`, `CLAUDE.md`
- Add:
  - `src/lib.rs`
  - `src/router.rs`
  - `src/route_table.rs`
  - `src/request.rs`
  - `src/candidates.rs`
  - `src/verification.rs`
  - `src/immediate_response.rs`
  - `src/drain.rs`
  - `tests/ingress_contract.rs`
  - `tests/bounded_ingress_stress.rs`
- Add architecture rules

### Task S8.2: Implement declarative verifier

**Red unit/contract tests:**

- HMAC-SHA256 recipe exact bytes/prefix/encoding;
- max 16 segments, 256 literal bytes, one body segment, and declared-header
  restrictions;
- missing/duplicate header behavior;
- stale timestamp and replay rejection;
- constant-time compare path;
- secret remains in host verifier;
- unsupported recipe/algorithm fails at manifest compile/activation;
- no adapter can mint sealed evidence.
- shared-secret-header comparison rejects missing/duplicate headers, uses
  constant-time exact comparison, and never exposes secret bytes.

Use a sealed/private constructor for verified evidence and retain existing
`host-auth-mint` protections or improve them.

### Task S8.3: Drive generic route caller

With the arbitrary channel fixture, test the actual mounted Axum route:

- method/path match;
- body/rate/deadline/candidate limit before expensive adapter work;
- malformed/no/ambiguous/bad-signature/stale/replay cases;
- untrusted hint intersection and cross-tenant isolation;
- connection-produced host-indexed `IngressRoutingClaim`s, HMAC-at-rest lookup,
  parser compatibility grouping, no-hint ambiguity, 4-parser/10-ms/32-candidate
  total budget;
- challenge immediate response;
- immediate response 64 KiB/status/header/deadline ceiling;
- authenticated no-op;
- authenticated normal event -> real generic workflow ledger/thread/turn;
- drain waits/cancels correctly;
- identical route descriptors group installations while incompatible policies
  on the same method/path fail activation;
- signing secret bytes absent from adapter recorder.
- canonical `/webhooks/extensions/{extension}/{surface}/{suffix}` route grammar,
  protected-route collision/encoded-separator/wildcard rejection;
- 64-header/16-KiB header limits, duplicate security-header rejection, and
  adapter panic/deadline isolation;
- normal/no-op 2xx only after durable dedupe/admission commit; DB failure returns
  retryable 5xx; crash-before/after enqueue, concurrent replay, and restart
  replay pass on both DBs;
- opaque 4-KiB reply-target seed is host-signed/scoped/generation-leased and
  reaches only the compatible bound outbound adapter.

### Task S8.4: Move Slack protocol code

Create/fill in `crates/ironclaw_slack_extension/src/channel/ingress.rs` and
protocol modules. Move Slack envelope/selectors/challenge/error normalization
from:

- `slack/slack_serve.rs`
- `slack/slack_serve/installation.rs`
- relevant `ironclaw_slack_v2_adapter` payload code

Host-specific route policy and HMAC execution stay in generic ingress. Slack
adapter returns only untrusted hints and normalized verified outcomes.

### Task S8.5: Cut production mount

- Composition mounts the generic ingress router/table once.
- Active snapshot changes update the route table view; no Axum rebuild per
  extension install.
- Delete `slack_events_route_mount`, `slack_events_handler`, static bundled
  Slack descriptor parsing, and Slack installation resolver from composition.
- Preserve old URL as the manifest-declared route/compat alias, not Rust
  hardcoding.
- Migrate existing installation selectors/identity route data into host-indexed
  ingress routing claims before removing the old resolver; retain a versioned
  read-only fallback in migration code until S11 audit.

### Task S8.6: Telegram second implementation

Fold `ironclaw_telegram_v2_adapter` into `ironclaw_telegram_extension`. Activate
the real v3 package created in S3 with native service `telegram.extension/v1`,
manual bot-token/webhook-secret auth surface, canonical `updates` route,
shared-secret-header verification, group trigger policy, and Bot API egress.
Prove both Slack and Telegram enter the same generic route/workflow caller
without host branches. Delete the old adapter crate after S10 moves all tests.

### Task S8.7: Verify/commit

```bash
cargo test -p ironclaw_extension_ingress
cargo test -p ironclaw_slack_extension ingress
cargo test --test reborn_integration_extension_runtime ingress
cargo test --test reborn_group_extensions
cargo test -p ironclaw_architecture
cargo clippy -p ironclaw_extension_ingress -p ironclaw_slack_extension -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
git diff --check
```

Commit: `refactor(channels): route inbound protocols through bound adapters`

---

## 12. Slice S9 — Generic outbound, targets, connection, and actions

**Purpose:** Remove every direct product send/setup/target/connection path from
generic code.

### Task S9.1: Implement generic delivery coordinator

**Files:**

- Add `crates/ironclaw_product_workflow/src/run_delivery_coordinator.rs`
- Modify `crates/ironclaw_product_workflow/src/outbound_delivery.rs`
- Modify event/projection adapters that currently trigger Slack observers
- Add contract tests in `ironclaw_outbound`
- Add caller integration tests

**First failing tests:**

- final reply, progress, gate, auth, failure, working, busy, connect, cleanup,
  capability activity, and trigger intents enter the same coordinator;
- source-route and preferred-triggered target policy is preserved;
- privacy `DirectMessageRequired` is generic;
- attempt persists before egress;
- coordinator alone persists prepared/sending/delivered/retryable/permanent/
  deferred/unknown outcomes; adapter receives no store/sink;
- retry/backoff/dedupe/single-flight/stale-target/shutdown drain behavior;
- partial multipart failure rule;
- crash after vendor success/before report persistence becomes `Unknown`; retry
  occurs only with a tested vendor idempotency key, otherwise reconcile or
  terminate unknown without blind resend;
- no preference and unauthorized target fail closed;
- no-op sink is rejected in production construction.

### Task S9.2: Implement restricted channel egress

**Files:**

- Add workspace member `crates/ironclaw_extension_egress` and implement
  restricted channel/auth egress over lower network/secret/policy ports
- Remove Slack identity/request injection from `slack/slack_egress.rs`
- Add security tests

Test undeclared host, wrong credential, adapter authorization header, forbidden
method, oversized body, redirect escape, private IP/DNS rebound, cross-install/
tenant access, deadline, and zero network calls after rejection.

### Task S9.3: Move Slack outbound behavior

Create/fill:

- `crates/ironclaw_slack_extension/src/channel/outbound.rs`
- `.../channel/render.rs`
- `.../channel/target.rs`
- `.../channel/connection.rs`
- protocol-specific delivery helpers

Move rendering, API calls, multipart, delete/update, target decoding, DM rules,
`conversations.open`, vendor error mapping, and product copy from:

- `slack/slack_delivery.rs`
- `slack/slack_egress.rs`
- `slack/slack_outbound_targets.rs`
- `slack/slack_dm_open.rs`
- `slack/slack_channel_connection.rs`

Keep scheduling, attempt persistence, communication policy, target claim
validation, retry, single-flight, and drain generic.

Change `ChannelOutboundAdapter::deliver` to return a structured
`ChannelDeliveryReport`; remove `OutboundDeliverySink` from the adapter
interface. The coordinator is the only durable writer.

### Task S9.4: Generic target binding

**Files:**

- Modify `crates/ironclaw_outbound` target types/store
- Modify product workflow target facade
- Add signed opaque adapter metadata codec in generic host
- Put old Slack target decoder in `ironclaw_reborn_migration` or Slack
  compatibility module with removal marker

Test signature/version/size/scope, cross-tenant/install/surface, stale
generation, list/search/provision, and old-codec migration.

Migrate target refs/preferences/DM provisioning records before old target
readers are deleted.

### Task S9.5: Generic connection and surface actions

**Files:**

- Replace generic facades in
  `crates/ironclaw_product_workflow/src/reborn_services/extensions.rs`
- Add generic fixed routes in WebUI v2 backend/route owner
- Move/delete behavior from:
  - `slack/slack_channel_routes.rs`
  - `slack/slack_channel_routes/**`
  - `slack/slack_setup.rs`
- Add arbitrary-fixture route tests

Test status/begin/complete/disconnect, schema validation, secret persistence,
authorization/rate/body policy, remote validation action, idempotent cleanup,
retryable partial disconnect, and absence of arbitrary adapter routes.

Migrate Slack setup/secrets/routes/allowed-subject state into generic
installation/surface state in this slice; S11 runs the full restart/idempotency
audit before S10 deletes compatibility code.

### Task S9.6: Cut all Slack delivery observers/hooks

- Replace manual `SlackFinalReplyDeliveryObserver` construction with generic
  coordinator registration.
- Move triggered delivery to generic event flow.
- Delete every direct Slack send, including working/busy/error/connect/auth
  notices.
- Search vendor method names/imports in generic code and fail if any remain.
- Replace concrete channel formatting in `ironclaw_llm/src/reasoning.rs` with
  typed `CommunicationPresentationPolicy` derived from the channel contract.
- Replace concrete Slack/Telegram trace contribution variants with generic
  extension/surface origin IDs and bounded capability data.

### Task S9.7: Telegram outbound proof

Run real production-shaped Slack and Telegram adapters through the same
coordinator, target policy, and restricted egress. Assert no host branch.

### Task S9.8: Verify/commit

```bash
cargo test -p ironclaw_outbound
cargo test -p ironclaw_product_workflow
cargo test -p ironclaw_slack_extension outbound
cargo test --test reborn_integration_extension_runtime outbound
cargo test --test reborn_group_extensions
cargo test -p ironclaw_architecture
cargo clippy -p ironclaw_outbound -p ironclaw_product_workflow -p ironclaw_slack_extension --all-targets --all-features -- -D warnings
git diff --check
```

Commit: `refactor(channels): deliver and configure through generic surface adapters`

---

## 13. Slice S10 — Complete Slack extraction and generic frontend

**Purpose:** Make Slack an ordinary extension package and remove bespoke UI/
composition construction.

### Task S10.1: Complete `ironclaw_slack_extension`

**Files:**

- Finalize crate `crates/ironclaw_slack_extension`
- Move remaining concrete behavior from
  `crates/ironclaw_reborn_composition/src/slack/**`
- Fold remaining `crates/ironclaw_slack_v2_adapter/**`
- Register service `slack.extension/v1` through generated first-party catalog
- Update workspace and architecture rules

The entrypoint returns exactly 5 tool + 1 channel + 1 auth binding. Add a
binding snapshot test and an end-to-end activation test against the real
fragmented package.

Delete `crates/ironclaw_reborn_composition/src/slack/` once no production or
test import remains. Move protocol unit tests to the Slack crate and generic
caller tests to integration tier.

### Task S10.2: Remove composition/CLI/config/Cargo Slack branches

**Files include:**

- `crates/ironclaw_reborn_composition/src/factory.rs`
- `crates/ironclaw_reborn_composition/src/lib.rs`
- `crates/ironclaw_reborn_composition/Cargo.toml`
- `crates/ironclaw_reborn_config/src/config_file.rs`
- `crates/ironclaw_reborn_cli/src/commands/serve_slack.rs` (delete)
- `crates/ironclaw_reborn_cli/src/commands/serve.rs`
- `crates/ironclaw_reborn_cli/src/runtime/mod.rs`
- `crates/ironclaw_reborn_cli/Cargo.toml`

Replace product compile features/config with generic runtime-technology support
and extension installation/config. Legacy env/TOML import lives only in a
versioned deprecation loader/migration.

### Task S10.3: Extend surface wire

**Files:**

- `crates/ironclaw_product_workflow/src/reborn_services/types.rs`
- `.../reborn_services/extensions.rs`
- WebUI route/facade tests

Add full surface key, display/action descriptors, auth/provider view, target
capabilities, and multiple channel support. Remove package-ID-as-channel-key
assumptions and Slack fallback cleanup.

### Task S10.4: Replace bespoke frontend

**Delete after replacements are called:**

- `crates/ironclaw_webui_v2/frontend/src/components/slack-setup-panel.tsx`
- its test
- `components/slack-channel-picker.tsx` and test
- `lib/slack-setup-api.ts` and test
- `lib/slack-channels-api.ts` and test

**Add:**

- `components/extensions/surface-card.tsx`
- `components/extensions/surface-action-form.tsx`
- `components/extensions/channel-surface-card.tsx`
- `components/extensions/channel-target-picker.tsx`
- `lib/extension-surfaces-api.ts`
- focused tests for each

**Modify:**

- `pages/extensions/components/channels-tab.tsx`
- `pages/extensions/components/configure-modal.tsx`
- `pages/chat/components/auth-oauth-card.tsx`
- `lib/channel-connection-events.ts`
- automation delivery defaults panel
- package/localization loading
- static asset tests in
  `crates/ironclaw_webui_v2/src/static_assets/assets.rs`

**Frontend tests:**

- arbitrary fixture channel renders/connects/configures/selects target;
- second channel renders without source change;
- two channels in one extension use distinct surface keys;
- auth action and OAuth completion are generic;
- no Slack package-ID branch or import remains;
- safe fallback copy when package localization missing;
- automation copy derives from target capabilities;
- generic pairing either works through connection adapter or is removed.

### Task S10.5: Real UI/backend flow

Drive browser/API integration against the real Slack package and arbitrary
fixture:

- list extension surfaces;
- open channel surface;
- save generic configuration/secret fields;
- connect/disconnect;
- list/resolve target;
- complete auth;
- activate;
- see connection state update without bespoke API.

Add `@playwright/test`, `playwright.config.ts`, package script `e2e`, and
`frontend/e2e/extension-surfaces.spec.ts`. The test boots the real WebUI v2
backend with scripted provider HTTP and drives arbitrary + Slack surface flows
in a browser. No live vendor secrets/network are used.

### Task S10.6: Verify/commit

Use the repository's pinned Node 22/pnpm setup.

```bash
cargo test -p ironclaw_product_workflow
cargo test -p ironclaw_webui_v2 --all-features
corepack pnpm --dir crates/ironclaw_webui_v2/frontend test
corepack pnpm --dir crates/ironclaw_webui_v2/frontend lint
corepack pnpm --dir crates/ironclaw_webui_v2/frontend e2e
cargo test --test reborn_integration_extension_runtime
cargo test -p ironclaw_architecture
git diff --check
```

Commit: `refactor(slack): run unified extension through generic host and UI`

---

## 14. Slice S11 — Shared-provider coexistence and durable legacy migration

**Purpose:** Finish provider reuse, state/config migration, and database parity
without reintroducing provider products.

### Task S11.1: Finish shared Google/Notion provider coexistence

**Files:**

- Complete the `ironclaw_google_auth_provider` and
  `ironclaw_notion_auth_provider` crates introduced/cut over in S4/S7
- Register them only through the first-party dependency catalog
- Add exact dependency pins to Gmail/Drive/Calendar/Docs/Sheets/Slides roots
- Modify each auth fragment to reference the dependency export
- Add architecture rules

**Tests:**

- all consumers resolve the same `(export, version, digest)` implementation;
- grants/accounts remain owning-extension/caller scoped;
- exact `(export, version, digest, ABI)` versions coexist for rolling upgrades,
  old callback leases, and different tenants;
- incompatible explicitly declared singleton compatibility keys require
  coordinated activation and fail safely;
- removing one consumer preserves others/refcount;
- provider cannot widen a root's auth surface;
- callback remains bound to pinned generation.

### Task S11.2: Implement versioned Slack/state migrations

**Files:**

- Add modules under `crates/ironclaw_reborn_migration/src/` for:
  - `extension_manifest_records.rs`
  - `slack_extension_identity.rs`
  - `slack_configuration.rs`
  - `slack_identities.rs`
  - `slack_routes.rs`
  - `slack_targets.rs`
  - `slack_conversations.rs`
  - `slack_delivery_state.rs`
  - `legacy_config.rs`
- Add old-wire fixtures and migration integration tests
- Add `tests/integration/extension_migration.rs`
- Add root `Cargo.toml` test entry named
  `reborn_integration_extension_migration`

Migrate every item listed in design section 23. Each migration must have:

- dry-run report;
- idempotent second run;
- crash/restart checkpoint test;
- cross-tenant isolation;
- malformed record quarantine;
- rollback/compat read test;
- cleanup-version test.

### Task S11.3: Generic scoped extension state

Move remaining generic identity/conversation/idempotency/outbound state out of
Slack roots. Expose only tenant/extension/install/surface-scoped adapter KV for
protocol-owned data. Add traversal/cross-namespace/CAS/quota tests on both DBs.

### Task S11.4: Compatibility aliases and removal markers

- Route old webhook/callback URLs into generic services.
- Decode old target refs only in Slack compatibility code.
- Import old config/env only once into new installation config.
- Emit actionable deprecation/operator audit.
- Store compatibility version and define S12 removal assertion.

### Task S11.5: Verify/commit

```bash
cargo test -p ironclaw_reborn_migration
cargo test -p ironclaw_google_auth_provider
cargo test --test reborn_integration_extension_migration
cargo test --test reborn_integration_durable
cargo test -p ironclaw_architecture
cargo clippy -p ironclaw_reborn_migration -p ironclaw_google_auth_provider --all-targets --all-features -- -D warnings
git diff --check
```

Commit: `feat(extensions): migrate shared providers and legacy extension state`

---

## 15. Slice S12 — Delete legacy paths and prove zero specificity

**Purpose:** Cross the finish line: no temporary alternate runtime, no concrete
core branch, and every ledger item evidenced.

### Task S12.1: Empty all temporary allowlists

Remove every entry from `LEGACY_EXTENSION_SPECIFICITY_ALLOWLIST`. Delete the
allowlist constant entirely if the test can express zero directly.

Delete:

- retired raw-manifest projection APIs;
- `ProductAdapterRuntimeEntry` and unused registry projection;
- compatibility metadata-bearing ProductAdapter trait;
- runtime-kind-per-invocation selection;
- provider string multiplexor and concrete callback handlers;
- Slack composition module and channel-specific features/config/CLI command;
- direct Slack delivery hooks/observers;
- old target/state/config implementation readers outside the versioned
  forwarding/read-only shims in `ironclaw_reborn_migration`;
- permanent split-ID cleanup from steady-state lifecycle;
- bespoke frontend files/copy/branches;
- dead tests that enshrine the old architecture.

### Task S12.2: Compile without concrete Slack

Add `scripts/ci/check-generic-extension-host-without-concrete.sh`. It uses
`cargo tree` to assert the selected generic packages contain no
`ironclaw_slack_extension`, `ironclaw_telegram_extension`, or concrete provider
dependency, then runs their tests individually with generic features only:

```bash
bash scripts/ci/check-generic-extension-host-without-concrete.sh
```

The script covers extension contracts/compiler/host/ingress/egress, auth host,
dispatcher, product workflow, outbound, generic WebUI backend, composition, and
CLI. It must not edit source or rely on root dev-dependency feature unification.
Separately run the arbitrary fixture and real Telegram production proofs.

### Task S12.3: Source and dependency audit

Run and check in machine gates for:

- concrete IDs/imports/routes/features outside sanctioned paths;
- generic-to-concrete Cargo dependencies;
- raw manifest reparsing outside compiler/migration tests;
- composition implementations of extension/channel/tool/auth adapters;
- extension-specific Axum mounts;
- vendor API method strings in generic code;
- package-ID-as-channel-key assumptions;
- metadata getters duplicating manifest authority;
- active-set writes outside `ExtensionHost`.

### Task S12.4: Full requirement ledger review

For every checkbox in `docs/reborn/extension-runtime-verification.md`:

- record the test name/path or command evidence;
- record both database evidence where applicable;
- link migration fixtures;
- mark exceptions only if a new ADR changes the normative contract;
- do not mark an item complete based only on a helper unit test when the item
  describes caller-visible behavior.

### Task S12.5: Documentation/parity/changelog

Update:

- `docs/reborn/contracts/extensions.md`
- `docs/reborn/contracts/product-adapters.md`
- `docs/reborn/contracts/auth-product.md`
- `docs/reborn/contracts/host-api.md`
- `docs/reborn/contracts/kernel-boundary.md`
- `docs/reborn/contracts/migration-compatibility.md`
- `.claude/skills/reborn-extension-surfaces/SKILL.md`
- relevant crate `AGENTS.md`/`CLAUDE.md`
- `FEATURE_PARITY.md`
- `CHANGELOG.md`
- CI path filters and test lanes

Remove language describing Slack host-beta hardcoding. Document “one logical
manifest compilation unit,” exact binding, generic adapters, and zero-specific
core.

### Task S12.6: Final full verification

Set Node 22 on `PATH` first if the local environment requires it.

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_architecture
bash scripts/ci/check-generic-extension-host-without-concrete.sh
cargo test -p ironclaw_extension_contracts
cargo test -p ironclaw_extensions
cargo test -p ironclaw_extension_host
cargo test -p ironclaw_extension_ingress
cargo test -p ironclaw_extension_egress
cargo test -p ironclaw_auth_host
cargo test -p ironclaw_product_adapters
cargo test -p ironclaw_product_workflow
cargo test -p ironclaw_dispatcher
cargo test -p ironclaw_slack_extension
cargo test -p ironclaw_telegram_extension
cargo test -p ironclaw_google_auth_provider
cargo test -p ironclaw_notion_auth_provider
cargo test -p ironclaw_first_party_extension_catalog
cargo test --test reborn_integration_extension_runtime
cargo test --test reborn_integration_extension_migration
bash scripts/reborn-e2e-rust.sh
bash scripts/ci/check-reborn-qa-fixtures.sh
bash scripts/ci/check-extension-runtime-verification.sh --release
corepack pnpm --dir crates/ironclaw_webui_v2/frontend test
corepack pnpm --dir crates/ironclaw_webui_v2/frontend lint
corepack pnpm --dir crates/ironclaw_webui_v2/frontend e2e
cargo test --features postgres,libsql --test reborn_integration_extension_runtime -- database_ --nocapture
cargo test --features postgres,libsql --test reborn_integration_extension_migration -- database_ --nocapture
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git diff --check
git status --short
```

The two `database_` commands above must start a real PostgreSQL testcontainer;
a runtime skip is failure. `cargo test --workspace --all-features` alone is not
database evidence.

### Task S12.7: Final review questions

Reviewers must answer yes, with evidence:

1. Can Slack be deleted without changing generic source?
2. Can Discord be added without changing generic source?
3. Does one root own every static/dynamic surface and implementation pin?
4. Can runtime code add undeclared authority? (It must not.)
5. Does every inbound/outbound/auth/tool caller resolve a bound surface?
6. Is every concrete protocol behavior implemented only in its extension?
7. Can activation/upgrade/restart expose a partial generation? (It must not.)
8. Do both databases preserve identical lifecycle/migration behavior?
9. Does the frontend render arbitrary/multiple channels without product code?
10. Are all old implementation paths/allowlists gone, with any release-N
    forwarding/read-only shim isolated in migration code and tracked to the
    N+2 cleanup criteria?

Commit: `refactor(architecture): enforce zero-specific unified extension runtime`

---

## 16. Completion handoff

Do not declare the project complete because the final build passes once. The
project is complete when:

- every slice's production caller is cut over;
- old callers are deleted;
- every required verification-ledger checkbox has named evidence;
- architecture allowlists are empty;
- Slack/Telegram/arbitrary-fixture proofs pass;
- libSQL/PostgreSQL and restart/migration paths pass;
- documentation describes the implementation that actually shipped;
- the final diff contains no unrelated changes.

## 17. Cleanup release N+2 milestone

The cutover release intentionally retains only versioned forwarding/read-only
shims in `ironclaw_reborn_migration`. Do not delete them in the first deployment
that begins telemetry. Open a cleanup PR no earlier than release N+2 and 30
days, after 14 consecutive days of zero old-format observations, successful
migration audit, operator acknowledgement, and rollback-window expiry.

That cleanup PR deletes old webhook/callback aliases, legacy target/config/state
readers, old codecs/fixtures no longer needed for supported upgrades, and their
path-scoped migration exceptions. It runs the complete command/evidence matrix
again and records cleanup evidence under `COMPAT-CLEAN-*` entries. Until then,
the architecture is complete at cutover because the new runtime is the only
implementation path and shims are isolated from generic core.
