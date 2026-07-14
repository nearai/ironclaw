# Train A Roll-Up Hardening Implementation Plan

> **Required workflow:** Execute this plan task by task with
> `superpowers:test-driven-development`, `superpowers:subagent-driven-development`,
> and `superpowers:verification-before-completion`. Every task receives a
> specification review and a code-quality review before it is considered done.

**Goal:** Make PR #6061 an upgrade-safe, self-contained Train A prerequisite
that establishes one extension model, one strict v2 manifest contract, one
unified Slack identity, and no live legacy rails without introducing Train B.

**Architecture:** Keep ownership with the existing manifest, filesystem,
extension-installation, product-auth, identity, product-workflow, and WebUI
modules. Reuse the canonical installation reducer and shared bounded-CAS loop.
Add only the smallest typed surface detail needed to eliminate raw-manifest
reparsing. Compatibility exists only in bounded persistence migrations and
legacy identity lookup; runtime continues to use the new model exclusively.

**Technology:** Rust, Tokio, serde/TOML/JSON, `ironclaw_filesystem`, libSQL,
PostgreSQL, React/TypeScript/Vitest, Python/Playwright E2E, GitHub CLI.

**Design:**
`docs/superpowers/specs/2026-07-13-train-a-rollup-hardening-design.md`

---

## Task 1: Add a shared root-filesystem CAS entrypoint

**Files:**

- Modify: `crates/ironclaw_filesystem/src/cas.rs`
- Modify: `crates/ironclaw_filesystem/src/lib.rs`
- Test: `crates/ironclaw_filesystem/src/cas/tests.rs`
- Test: `crates/ironclaw_filesystem/tests/concurrent_cas_storm.rs`

### Step 1: Write failing root-CAS unit tests

Add tests for:

- absent record creation with `CasExpectation::Absent`;
- version-mismatch reread and retry;
- equality/no-op write suppression;
- backend capability rejection without `Any` fallback; and
- apply/decode/encode error preservation.

Run:

```bash
cargo test -p ironclaw_filesystem cas::tests::root_
```

Expected: FAIL because the root entrypoint does not exist.

### Step 2: Implement the smallest shared root entrypoint

Add `cas_update_root` over `RootFilesystem + VirtualPath`. Refactor
`cas_update` and `cas_update_root` through one private operation adapter or one
generic retry loop. Do not copy the retry/backoff algorithm and do not change
the scoped API.

Preserve:

- 32 bounded retries;
- the 15-second timeout;
- capability gating;
- create-if-absent/versioned writes;
- equality fast path;
- `CasUpdateError` mapping; and
- fail-closed behavior on unsupported CAS.

### Step 3: Add real-backend concurrency coverage

Extend `concurrent_cas_storm.rs` with a root-path storm and cover in-memory,
libSQL, and PostgreSQL without changing the existing scoped storm.

Run:

```bash
cargo test -p ironclaw_filesystem cas::tests::root_
cargo test -p ironclaw_filesystem --features libsql --test concurrent_cas_storm
cargo test -p ironclaw_filesystem --features postgres --test concurrent_cas_storm
cargo clippy -p ironclaw_filesystem --all-targets --all-features -- -D warnings
```

### Step 4: Review and commit

Review for duplicated retry logic, unbounded waits, `Any` fallback, and public
API growth beyond the one root entrypoint.

Commit:

```bash
git add crates/ironclaw_filesystem
git commit -m "fix(filesystem): share bounded CAS with root stores"
```

---

## Task 2: Make installation-state upgrades atomic and lossless

**Files:**

- Modify:
  `crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store.rs`
- Test:
  `crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store/tests.rs`
- Reference: `crates/ironclaw_extensions/src/canonicalization.rs`

### Step 1: Write failing legacy-manifest transition tests

Add caller-level `load_at` tests proving:

- a persisted host-bundled v2 record with top-level `[[capabilities]]` is
  converted before strict parsing;
- the converted record is accepted by the strict parser and persisted on a
  CAS-capable backend;
- new public manifest ingestion still rejects the old shape;
- already-current records do not advance the record version;
- malformed legacy records fail without changing bytes; and
- a concurrent unrelated snapshot update forces a reread and survives.

Run:

```bash
cargo test -p ironclaw_reborn_composition --lib load_at_migrates_legacy_manifest
cargo test -p ironclaw_reborn_composition --lib load_at_retries_rewrite
```

Expected: FAIL against the current strict load/unconditional write path.

### Step 2: Write failing Slack-fold tests

Add tests for:

- multiple retired member-owned rows without an existing unified row;
- mixed retired/unified rows with tenant dominance and member preservation;
- explicit Train A enabled-wins behavior;
- agreeing credential union and conflicting credential rejection;
- newest health and maximum `updated_at`;
- exact existing unified manifest hash preservation;
- legacy-only host-bundled manifest seeding;
- feature-disabled behavior that never rewrites/deletes state;
- structural manifest-id recognition independent of TOML formatting;
- typed-error propagation; and
- byte-stable second load.

Run:

```bash
cargo test -p ironclaw_reborn_composition --lib slack_bot
```

Expected: FAIL because the current fold selects one owner/row, invents fields,
and drops data on a feature-disabled build.

### Step 3: Implement the bounded wire transition

Add a pure, rerunnable `normalize_wire_state` that:

1. structurally identifies persisted manifest IDs;
2. recognizes only the legacy-v2 capability shape made obsolete by Train A;
3. rewrites it into the registered capability-provider section;
4. transforms every `slack_bot` row into a persisted-parts `slack` row;
5. preserves fields and the surviving unified manifest reference/hash;
6. applies enabled-wins to the complete Slack group before canonicalization;
7. calls `canonicalize_installation_rows` exactly once; and
8. validates the complete candidate snapshot through a fresh in-memory store.

`migrate_retired_slack_bot_identity` becomes fallible. It must never delete
state when the unified host-bundled manifest is unavailable.

### Step 4: Use root CAS for the load-time rewrite

Use `cas_update_root` for decode, normalize, validate, encode, and conditional
commit. Load the real in-memory store from the final snapshot returned by CAS.
Map `Apply` back to the original installation error and sanitize backend,
timeout, and retry failures consistently.

For a legacy local backend that explicitly cannot CAS, do not fall back to a
blind write. Preserve compatibility by validating and using the normalized
snapshot in memory while leaving persisted bytes untouched and emitting a
specific warning. Add a test for this non-persisting compatibility path. CAS
backends must persist and become byte-stable.

The final concurrency review broadened this task only far enough to close a
confirmed lost-update defect in the same store. Every ordinary mutation on a
CAS-capable backend applies to the latest winning snapshot, and in-memory
publication happens only after persistence succeeds. Fresh install uses the
store's atomic manifest-plus-installation transition. Preserve the explicitly
opted-in non-CAS local-development lifecycle through a bounded per-store worker;
hosted/CAS-capable profiles never enter that compatibility path.

### Step 5: Verify and commit

Run:

```bash
cargo test -p ironclaw_reborn_composition --lib extension_installation_store
cargo test -p ironclaw_extensions --test manifest_v2_contract
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store.rs \
  crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store/tests.rs
git commit -m "fix(reborn): migrate extension state without data loss"
```

---

## Task 3: Wire a strict durable Slack credential migration

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/product_auth/durable/mod.rs`
- Test: `crates/ironclaw_reborn_composition/src/product_auth/durable/tests.rs`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/src/error.rs`
- Test: `crates/ironclaw_reborn_composition/tests/facade_factory.rs`

### Step 1: Write failing strict migration unit tests

Change the expected API to `Result<usize, AuthProductError>` and test:

- only `provider` changes;
- every identity, owner, scope, status, secret, and timestamp field survives;
- second run is a no-op and does not advance the record version;
- root enumeration, record decode, and unsupported-CAS errors are returned;
- a forced version conflict rereads and preserves concurrent status/secret
  changes;
- a partial failure returns an error; and
- restart converges without duplication or data loss.

Run:

```bash
cargo test -p ironclaw_reborn_composition --lib migrate_retired_slack_personal
```

Expected: FAIL because the current helper is best-effort, unwired, and uses
`CasExpectation::Any`.

### Step 2: Separate strict migration enumeration from keepalive

Extract shared owner discovery with explicit best-effort and strict modes.
Strict mode returns backend, traversal, bound, path-consistency, decode, and
read failures and filters `slack_personal` during traversal. Keepalive retains
its current best-effort semantics but filters Google/configured/refresh-secret
records before collecting/sorting.

Do not add a general migration framework or change public product-auth APIs.

### Step 3: Migrate each record with bounded CAS

Use `ironclaw_filesystem::cas_update` at the exact located scope/path. The
rerunnable apply closure changes only the provider when it is still
`slack_personal`; current `slack`, other providers, or authoritative deletion
are no-ops. Map retries to a typed conflict and other failures to a sanitized
backend error. Never resurrect a concurrently deleted account.

Bound the complete strict traversal as well as each directory: charge every
retained owner, every base/session scope (including empty roots), and every
account-directory entry before filename/provider filtering. Production limits
are 8,192 owners, 65,536 scopes, and 65,536 entries. Test exact-limit success
and fail-before-write behavior for owner overflow, empty session roots, and
nonmatching entries.

### Step 4: Wire every composition-owned durable profile

- Construct local/hosted durable services with `new_with_root`.
- Await migration before creating/publishing product-auth ports.
- Await the same migration in production and migration-dry-run construction.
- Add a typed `RebornBuildError` variant.
- Leave caller-supplied auth ports untouched because their persistence is not
  composition-owned or enumerable.

### Step 5: Add real libSQL/PostgreSQL factory tests

Add production rebuild tests that seed `slack_personal`, rebuild on the same
database, and assert `slack` is visible immediately with all other fields
unchanged. Rebuild a third time for idempotency. Add a malformed-record libSQL
factory test proving startup fails with the migration error.

Run:

```bash
cargo test -p ironclaw_reborn_composition --lib migrate_retired_slack_personal
cargo test -p ironclaw_reborn_composition --features libsql --test facade_factory production_libsql_migrates_slack_personal
cargo test -p ironclaw_reborn_composition --features postgres --test facade_factory production_postgres_migrates_slack_personal
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
```

### Step 6: Review and commit

Review for best-effort behavior leaking into migration, whole-record blind
writes, secret/status loss, profiles skipped, and runtime aliases.

Commit:

```bash
git add crates/ironclaw_reborn_composition/src/product_auth \
  crates/ironclaw_reborn_composition/src/factory.rs \
  crates/ironclaw_reborn_composition/src/error.rs \
  crates/ironclaw_reborn_composition/tests/facade_factory.rs
git commit -m "fix(reborn): migrate unified Slack credentials at startup"
```

---

## Task 4: Make typed manifest projection authoritative

**Files:**

- Modify: `crates/ironclaw_extensions/src/v2.rs`
- Modify: `crates/ironclaw_extensions/src/lib.rs`
- Test: `crates/ironclaw_extensions/tests/manifest_v2_contract.rs`
- Modify: `crates/ironclaw_product_adapter_registry/src/lib.rs`
- Test: `crates/ironclaw_product_adapter_registry/tests/manifest_ingestion.rs`
- Modify:
  `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`
- Modify:
  `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Test: adjacent module tests and relevant Reborn integration tests

### Step 1: Write failing typed-projection tests

Cover:

- channel inbound/outbound direction preservation;
- inbound-only and outbound-only manifests;
- coarse `CapabilitySurfaceKind::Channel` projection;
- surface-derived OAuth versus proof-code connection strategy independent of
  package ID;
- bundled Slack tool+channel+auth projection; and
- serialized `runtime + surfaces` with no `kind`.

Run:

```bash
cargo test -p ironclaw_extensions --test manifest_v2_contract
cargo test -p ironclaw_product_adapter_registry --test manifest_ingestion
cargo test -p ironclaw_reborn_composition bundled_slack_package_declares_product_adapter_channel_surface
```

Expected: FAIL where direction detail is lost and composition reparses raw
TOML.

### Step 2: Extend the smallest owning projection

Represent typed surface declarations in the v2 host-API projection, including
channel direction flags, while preserving the public coarse `kind()` view.
Project directions once from `ProductAdapterCapabilities` in the registry.

### Step 3: Remove parallel taxonomy truth

Delete cached `surface_kinds` and `channel_directions` from
`AvailableExtensionPackage`. Derive summaries and lifecycle decisions from the
typed manifest projection. Remove the `product_adapter_sections(record)` raw
TOML reparse from the summary path. Derive credential/connection behavior from
typed surfaces and auth declarations, not Slack/package identifiers.

Do not add a resolved manifest, generic host, or Train B adapter traits.

### Step 4: Verify and commit

Run:

```bash
cargo test -p ironclaw_extensions
cargo test -p ironclaw_product_adapter_registry
cargo test -p ironclaw_reborn_composition available_extensions
cargo test -p ironclaw_reborn_composition extension_lifecycle
cargo clippy -p ironclaw_extensions --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_product_adapter_registry --all-targets --all-features -- -D warnings
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/ironclaw_extensions crates/ironclaw_product_adapter_registry \
  crates/ironclaw_reborn_composition/src/extension_host
git commit -m "fix(reborn): make extension surfaces the typed source of truth"
```

---

## Task 5: Make provider identity keys injective and compatible

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/provider_identity.rs`
- Modify: `crates/ironclaw_reborn_composition/src/lib.rs`
- Modify: `crates/ironclaw_reborn_composition/src/slack/slack_host_state.rs`
- Modify:
  `crates/ironclaw_reborn_composition/src/slack/slack_channel_connection.rs`
- Modify:
  `crates/ironclaw_reborn_composition/src/slack/slack_personal_oauth.rs`
- Modify:
  `crates/ironclaw_reborn_composition/src/slack/slack_personal_binding.rs`
- Test: adjacent provider-identity and Slack lifecycle tests

### Step 1: Write failing key-contract tests

Cover:

- the reported delimiter-collision pair produces distinct keys;
- Unicode installation IDs and actors round-trip safely;
- malformed lengths, separators, UTF-8 boundaries, and empty actors reject;
- lookup tries the new key first;
- unambiguous legacy keys remain readable;
- ambiguous legacy inputs never fall back;
- writes use only the new encoding; and
- epoch/revocation/disconnect cleanup works across both generations.

Run:

```bash
cargo test -p ironclaw_reborn_composition --lib provider_identity
cargo test -p ironclaw_reborn_composition --lib slack_personal_binding
cargo test -p ironclaw_reborn_composition --lib slack_channel_connection
```

Expected: FAIL because writes currently concatenate with `:` and lookups know
only one encoding.

### Step 2: Implement the compatibility boundary

Use
`ic1.<installation-byte-length>.<base64url-no-pad(installation)>.<base64url-no-pad(actor)>`
for new writes. Parse with checked decoded length, canonical base64url segments,
strict bounds, a nonempty actor, and typed installation-ID validation. Add
explicit new and safe-legacy exact-key and prefix helpers.

Lookup order is new first, then legacy only when neither component can make the
old delimiter representation ambiguous. Prefix scans and cleanup recognize
both generations. No normal write emits the legacy form.

### Step 3: Keep internals private and docs exact

Remove unused crate-root provider-identity reexports and make internal helpers
`pub(crate)`. Keep the generic product-blind resolver required by Train A, but
describe the current explicit composition wiring accurately.

### Step 4: Verify and commit

Run:

```bash
cargo test -p ironclaw_reborn_composition --lib provider_identity
cargo test -p ironclaw_reborn_composition --lib slack
cargo clippy -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/ironclaw_reborn_composition/src/provider_identity.rs \
  crates/ironclaw_reborn_composition/src/lib.rs \
  crates/ironclaw_reborn_composition/src/slack
git commit -m "fix(reborn): make provider identity keys collision-safe"
```

---

## Task 6: Finish Train A cleanup and acceptance tests

**Files:**

- Modify: residual composition and product-workflow files identified by scoped
  consumer searches
- Modify:
  `crates/ironclaw_webui_v2/frontend/src/pages/extensions/components/configure-modal.tsx`
- Test:
  `crates/ironclaw_webui_v2/frontend/src/pages/extensions/components/configure-modal.test.ts`
- Test: `crates/ironclaw_webui_v2/tests/webui_v2_handlers_contract.rs`
- Test: `tests/e2e/scenarios/test_reborn_webui_v2_extensions_api.py`
- Test: `tests/e2e/scenarios/test_reborn_webui_v2_legacy_extensions.py`
- Modify: `crates/ironclaw_architecture/tests/reborn_retired_taxonomy.rs`
- Modify: `scripts/reborn-e2e-rust.sh`

### Step 1: Freeze the cleanup inventory

Run scoped consumer searches for:

- `SlackHostBetaLegacySetup`;
- `SlackHostBetaActorUserResolver`;
- `is_internal_extension_package_ref`;
- `is_webui_v2_llm_config_route_id`;
- `RebornChannelConnectAction` and its strategy variants;
- stale gateway channel-status queries;
- `SLACK_TOOLS_EXTENSION_ID` and Model-B comments;
- `/v2/extensions/mcp` and `/channels/connectable`; and
- retired `kind` fixtures in Reborn WebUI v2 scenarios.

Classify every occurrence as production consumer, bounded migration/test, or
retired shim before deleting it. Do not broadly rename valid
`slack_personal_oauth` implementation terminology or alter #5957 fencing,
rollback, cleanup, and exact-conversation behavior.

### Step 2: Write failing source/UI/E2E contract tests

Add or update tests proving:

- the handler JSON has `runtime` and `surfaces` and omits `kind`;
- the unified Slack fixture has tool, channel, and auth surfaces;
- OAuth channel setup activates after OAuth, while non-OAuth channel setup uses
  pairing;
- Gmail/tools-only behavior is unchanged;
- channels and tools pages render current fixtures;
- retired MCP/connectable routes are absent; and
- the retired-taxonomy gate scans the relevant `.py` Reborn scenarios and
  residual shim names without scanning legacy v1 enclaves.

Run the focused frontend and architecture tests and confirm RED.

### Step 3: Remove only proven-dead shims

Delete or privatize the frozen inventory items with no production consumer.
Wire the generic provider resolver directly at its two current sites. Update
stale module ownership comments and unified-Slack frontend naming. Keep the
host-side Slack OAuth scope allowlist until a trusted manifest-to-provider
projection owns it.

### Step 4: Add missing Train A tests to full Reborn E2E

Extend `scripts/reborn-e2e-rust.sh` to run:

- `ironclaw_extensions::manifest_v2_contract`;
- `ironclaw_product_adapter_registry::manifest_ingestion`; and
- `ironclaw_architecture::reborn_retired_taxonomy`.

### Step 5: Verify and commit

Run:

```bash
cargo test -p ironclaw_webui_v2
cargo test -p ironclaw_architecture --test reborn_retired_taxonomy
cargo test -p ironclaw_reborn_composition --all-features
npm test --prefix crates/ironclaw_webui_v2/frontend -- --run
npm run typecheck --prefix crates/ironclaw_webui_v2/frontend
npm run lint --prefix crates/ironclaw_webui_v2/frontend
python -m pytest tests/e2e/scenarios/test_reborn_webui_v2_extensions_api.py
python -m pytest tests/e2e/scenarios/test_reborn_webui_v2_legacy_extensions.py
```

Use the repository's actual frontend/Python commands if package scripts or the
E2E harness expose different entrypoints.

Commit:

```bash
git add crates tests/e2e scripts/reborn-e2e-rust.sh
git commit -m "refactor(reborn): finish unified extension cleanup"
```

---

## Task 7: Align docs, perform final acceptance, and update the PR

**Files:**

- Modify: `CHANGELOG.md`
- Modify: `FEATURE_PARITY.md`
- Modify: `.claude/skills/reborn-extension-surfaces/SKILL.md`
- Modify: directly relevant contracts/comments
- Modify:
  `docs/superpowers/specs/2026-07-13-train-a-rollup-hardening-design.md`
- Modify: this plan checklist as evidence is completed
- External: PR #6061 body and review threads

### Step 1: Align documentation with verified behavior

Document:

- breaking `runtime + surfaces` wire shape;
- strict new-input manifest grammar and bounded persisted-state compatibility;
- unified Slack installation and credential transitions;
- CAS-backend versus legacy local-backend rewrite behavior;
- rollback implications; and
- the mandatory one-time quiesced cutover (no pre-Train-A writer overlap) and
  restore-or-roll-forward rollback rule; and
- explicit Train B exclusions.

Remove claims of generic manifest-driven identity wiring or migration coverage
unless a caller-level test proves them.

### Step 2: Run the complete verification matrix

At minimum:

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_filesystem --all-features
cargo test -p ironclaw_extensions --all-features
cargo test -p ironclaw_product_adapter_registry --all-features
cargo test -p ironclaw_reborn_composition --all-features
cargo test -p ironclaw_product_workflow --all-features
cargo test -p ironclaw_webui_v2 --all-features
cargo test -p ironclaw_architecture
cargo clippy --workspace --all-targets --all-features -- -D warnings
bash scripts/reborn-e2e-rust.sh
scripts/pre-commit-safety.sh
```

Also run the real PostgreSQL migration lane and relevant served/browser E2E.
Record exact commands, pass counts, skips, and environmental limitations.

### Step 3: Perform the final acceptance review

Dispatch fresh reviewers for security, bugs, performance/concurrency, tests,
conventions, local patterns, maintainability, and approach. Fix every accepted
finding and rerun affected tests. Confirm:

- no production `.unwrap()`/`.expect()` in changed files;
- no suspicious string slicing or hardcoded temp path;
- no retired live taxonomy outside bounded migrations/tests;
- no Train B symbol or architecture leak;
- no unresolved review thread; and
- exact final diff matches the design checklist.

### Step 4: Commit documentation and push

Commit:

```bash
git add CHANGELOG.md FEATURE_PARITY.md .claude/skills/reborn-extension-surfaces \
  docs/superpowers
git commit -m "docs(reborn): record unified extension compatibility"
```

Update the PR body with exact final SHA, scope, migrations, compatibility,
rollback, tests, skipped lanes, and Train B exclusions. Resolve only review
threads whose underlying issue is fixed.

Push the verified branch:

```bash
git push origin HEAD:nea25-rollup
```

Wait for all required GitHub checks on the pushed SHA. If any check fails,
diagnose, fix, rerun the affected local lane, repush, and wait again. Completion
requires green required checks, zero unresolved actionable threads, and an
accurate final PR body.
