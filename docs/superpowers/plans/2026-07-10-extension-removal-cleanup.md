# Durable Extension Removal Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to execute this plan task-by-task, with task-compliance review before moving to the next task.

**Goal:** Replace extension-removal surface/package heuristics with an explicit, durable, retryable cleanup coordinator so generic external-channel extensions remove normally and every extension cleans only the resources declared by its trusted removal plan.

**Architecture:** `ironclaw_extensions` owns the removal state machine, journal contracts, adapter seam, and in-memory test store. Reborn composition projects trusted cleanup requirements, persists journals beside installation state, supplies product-auth and Slack-personal adapters, and implements idempotent quiesce/purge operations over the existing lifecycle, registry, filesystem, and installation store. WebUI and `builtin.extension_remove` remain thin callers of the same management-port method.

**Tech Stack:** Rust 2024, Tokio, `async-trait`, Serde/JSON, `RootFilesystem`, existing Reborn product-auth/channel facades, Cargo tests and Clippy.

## Global Constraints

- Work only in the Reborn `crates/` architecture. Do not modify v1 `src/`.
- Keep existing `ExtensionInstallation` JSON byte-compatible; no SQL or operator-run migration.
- Create removal journals lazily on first removal and keep them outside `/system/extensions/<extension-id>`.
- Never infer cleanup from `LifecycleExtensionSurfaceKind`, package id inside lifecycle removal, connection-status maps, or facade support probing.
- A persisted plan is immutable. Required adapter absence or failure retains the journal and returns a retryable error.
- After quiescing starts, never compensate by re-enabling, republishing, or restoring an extension.
- Product-auth cleanup uses extension ownership (`provider: None`); Slack personal credential revocation remains owned by the explicit Slack connection adapter.
- Test the caller that gates side effects, not only pure helpers. Both WebUI and model capability paths must be covered.
- Each task ends with focused tests, review, and a commit before the next task.

---

## Task 1: Add the removal domain and state machine

**Files:**

- Create: `crates/ironclaw_extensions/src/removal.rs`
- Modify: `crates/ironclaw_extensions/src/lib.rs`

### Steps

- [ ] Add a failing domain test named `removal_without_cleanup_requirements_quiesces_and_purges` that constructs an in-memory journal store and fake lifecycle, calls the coordinator, and expects the order `plan -> quiesce -> purge`, a `Removed` outcome, and no remaining journal.
- [ ] Run `cargo test -p ironclaw_extensions removal_without_cleanup_requirements_quiesces_and_purges -- --nocapture` and confirm the test initially fails because the removal API does not exist.
- [ ] Define validated, serializable newtypes `ExtensionCleanupAdapterId`, `ExtensionCleanupTaskId`, and `ExtensionCleanupChannelId`.
- [ ] Define `ExtensionCleanupBinding` with typed `ProductAuth { extension_id }`, `ChannelConnection { channel }`, and `ProductAdapterInstallation { installation_id }` variants. The binding must contain no secret data or caller-selectable scope.
- [ ] Define `ExtensionCleanupRequirement`, `AuthenticatedRemovalActor`, `ExtensionRemovalRequest`, `ExtensionRemovalOutcome`, `ExtensionRemovalPhase`, and versioned `ExtensionRemovalJournal`.
- [ ] Define object-safe async traits:

  ```rust
  #[async_trait]
  pub trait ExtensionRemovalLifecycle: Send + Sync {
      async fn plan(
          &self,
          request: &ExtensionRemovalRequest,
      ) -> Result<Option<Vec<ExtensionCleanupRequirement>>, ExtensionRemovalError>;
      async fn quiesce(
          &self,
          request: &ExtensionRemovalRequest,
      ) -> Result<(), ExtensionRemovalError>;
      async fn purge(
          &self,
          request: &ExtensionRemovalRequest,
      ) -> Result<(), ExtensionRemovalError>;
  }

  #[async_trait]
  pub trait ExtensionCleanupAdapter: Send + Sync {
      fn id(&self) -> &ExtensionCleanupAdapterId;
      async fn cleanup(
          &self,
          context: &ExtensionCleanupContext,
          binding: &ExtensionCleanupBinding,
      ) -> Result<ExtensionCleanupReceipt, ExtensionRemovalError>;
  }

  #[async_trait]
  pub trait ExtensionRemovalJournalStore: Send + Sync {
      async fn get(
          &self,
          installation_id: &ExtensionInstallationId,
      ) -> Result<Option<ExtensionRemovalJournal>, ExtensionRemovalError>;
      async fn create_if_absent(
          &self,
          journal: ExtensionRemovalJournal,
      ) -> Result<ExtensionRemovalJournal, ExtensionRemovalError>;
      async fn save(
          &self,
          journal: ExtensionRemovalJournal,
      ) -> Result<(), ExtensionRemovalError>;
      async fn delete(
          &self,
          installation_id: &ExtensionInstallationId,
      ) -> Result<(), ExtensionRemovalError>;
  }
  ```

- [ ] Implement `InMemoryExtensionRemovalJournalStore`. Its `save` path must reject schema/id/plan changes, phase regression, or removal of completed task ids.
- [ ] Implement `ExtensionRemovalCoordinator::new` with duplicate-adapter rejection and `remove` with the exact monotonic sequence `Planned -> Quiesced -> Cleaning -> Purging -> journal deleted`.
- [ ] Persist the journal before quiescing, persist each phase before moving on, and persist each completed task immediately after adapter success.
- [ ] Make retry behavior use the persisted plan and completed-task set; never call `lifecycle.plan` when a journal already exists.
- [ ] Add domain tests for:

  - missing required adapter leaves a `Cleaning` journal and does not purge;
  - a transient adapter failure resumes and completes on a second call;
  - completed tasks are skipped on retry;
  - a purge failure leaves `Purging` state and never calls quiesce again;
  - a journal/request identity mismatch fails closed; and
  - `plan == None` returns `AlreadyRemoved` without creating a journal.

- [ ] Run `cargo test -p ironclaw_extensions removal -- --nocapture`.
- [ ] Run `cargo fmt --check`.
- [ ] Review Task 1 against the approved design, then commit as `feat(extensions): add durable removal coordinator`.

## Task 2: Add atomic installation purge and durable journal storage

**Files:**

- Modify: `crates/ironclaw_extensions/src/installations.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_installation_store.rs`
- Create: `crates/ironclaw_reborn_composition/src/extension_host/extension_removal_journal_store.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/mod.rs`

### Steps

- [ ] Add a failing `ironclaw_extensions` store test named `purge_installation_and_manifest_is_atomic_and_idempotent` that expects one operation to remove the matching installation and manifest together, succeed on retry, and reject a mismatched installation/extension pair.
- [ ] Extend `ExtensionInstallationStore` with `purge_installation_and_manifest(&ExtensionInstallationId, &ExtensionId)`. Implement it for `Arc<T>` and `InMemoryExtensionInstallationStore` under one write lock.
- [ ] Preserve the invariant that a manifest cannot be purged while another installation still references the extension.
- [ ] Implement the filesystem wrapper method under `save_lock`; always persist a snapshot even when an idempotent retry finds both records absent.
- [ ] Add a failing composition test named `removal_journal_store_reopens_persisted_progress` using `InMemoryBackend`: create a journal through one store instance, construct a fresh store over the same filesystem/path, and assert the exact phase, immutable plan, and completed tasks reload.
- [ ] Implement `FilesystemExtensionRemovalJournalStore` as a separate JSON sidecar at the installation-state sibling path `removals.json` (for example `/system/extensions/.installations/removals.json` and the equivalent tenant-scoped path).
- [ ] Serialize journal state as a schema-owned map keyed by `ExtensionInstallationId`. Read the current durable snapshot while holding the store lock for every mutation, validate journal transitions with the domain contract, write the new snapshot atomically through `RootFilesystem::write_file`, and only then return success.
- [ ] Treat a missing journal file as empty state, sanitize filesystem/JSON errors, and make journal deletion idempotent.
- [ ] Add tests for invalid persisted JSON, immutable-plan rejection, tenant-scoped sibling-path derivation, and idempotent delete.
- [ ] Update the test-only `DeleteInstallationFailingStore` implementation in `extension_lifecycle.rs` to implement the new trait method; preserve its failure-injection controls for later monotonic-purge tests.
- [ ] Run:

  ```text
  cargo test -p ironclaw_extensions installations -- --nocapture
  cargo test -p ironclaw_reborn_composition --lib extension_removal_journal_store -- --nocapture
  ```

- [ ] Run `cargo fmt --check`.
- [ ] Review Task 2 and commit as `feat(reborn): persist extension removal journals`.

## Task 3: Project explicit ownership and implement cleanup adapters

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_credential_requirements.rs`
- Create: `crates/ironclaw_reborn_composition/src/extension_host/extension_removal.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/mod.rs`

### Steps

- [ ] Add a failing catalog test named `slack_personal_cleanup_is_explicit_catalog_metadata` that expects the bundled user-facing `slack` package to carry exactly one trusted `ChannelConnection(slack)` requirement and expects `slack_bot` plus a generic external-channel fixture to carry none.
- [ ] Add `cleanup_requirements: Vec<ExtensionCleanupRequirement>` to `AvailableExtensionPackage`; initialize it to empty for filesystem packages, ordinary bundled packages, and fixtures.
- [ ] Attach the Slack personal requirement only in `slack_package()`. This is the compatibility projection for already-installed bundled Slack records because removal resolves the current trusted catalog package before first journal creation.
- [ ] Add `package_declares_product_auth_credentials(&ExtensionPackage) -> bool` next to existing credential projection logic. It must inspect typed `RuntimeCredentialRequirementSource::ProductAuthAccount` values and must not consult lifecycle summaries.
- [ ] Add `extension_removal.rs` with stable adapter/task ids and a pure `removal_requirements_for_package` projection that combines:

  - trusted package `cleanup_requirements`; and
  - one product-auth requirement when the manifest declares product-auth credentials.

  Sort requirements by task id and reject duplicate task ids.
- [ ] Implement `ProductAuthExtensionCleanupAdapter` over `RebornProductAuthServices`. It must accept only `ProductAuth`, call `cleanup_credentials_for_lifecycle` with `SecretCleanupAction::Uninstall` and `provider: None`, and convert backend failures to sanitized retryable errors.
- [ ] Implement `SlackPersonalExtensionCleanupAdapter` over the existing late-bound `ChannelConnectionFacade` slot. It must accept only the explicit `ChannelConnection(slack)` binding, build `WebUiAuthenticatedCaller` from the trusted removal context, and call `disconnect_channel_for_caller` unconditionally. It must never call `caller_channel_connections`.
- [ ] Add adapter tests for wrong-binding rejection, missing Slack facade, actor-scoped Slack disconnect, product-auth request shape, and sanitized backend failure.
- [ ] Add the generic-channel regression test at the plan-projection seam: a package with `ExternalChannel` surface and no explicit cleanup metadata produces no channel cleanup requirement.
- [ ] Run `cargo test -p ironclaw_reborn_composition --lib extension_removal -- --nocapture` and the catalog tests.
- [ ] Run `cargo fmt --check`.
- [ ] Review Task 3 and commit as `feat(reborn): declare extension cleanup ownership`.

## Task 4: Rewire the management port and delete legacy removal code

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`
- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle/active_publication.rs` only if an idempotent unpublish helper is required
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/src/product_auth/durable/tests.rs`

### Steps

- [ ] Replace the old failing test `extension_remove_fails_required_cleanup_when_channel_facade_is_unset` with `extension_remove_generic_external_channel_without_owned_connection_succeeds`. Assert the package directory, installation, and manifest are all gone and no channel adapter was invoked.
- [ ] Run that test before implementation and confirm it fails on the current unconditional external-channel cleanup.
- [ ] Add `ExtensionRemovalCoordinator` to `RebornLocalExtensionManagementPort`; keep the existing operation lock as the serialization boundary for install/activate/remove.
- [ ] Implement `ExtensionRemovalLifecycle` for the management port:

  - `plan` validates installation ownership, resolves the trusted catalog package, and delegates to `removal_requirements_for_package`;
  - `quiesce` persists `Disabled`, disables the lifecycle package if still enabled, and unpublishes it from the active registry/trust policy idempotently;
  - `purge` removes the lifecycle registration if present, deletes materialized files idempotently, then calls `purge_installation_and_manifest`.

- [ ] Rewrite `RebornLocalExtensionManagementPort::remove` to construct the trusted actor scope, acquire the operation lock, call the coordinator, and map `Removed`/`AlreadyRemoved` to the existing lifecycle response.
- [ ] On any failure after quiescing, leave the installation disabled and journal intact. Delete all compensation/reactivation branches from the removal path.
- [ ] Wire `FilesystemExtensionRemovalJournalStore`, `ProductAuthExtensionCleanupAdapter`, and `SlackPersonalExtensionCleanupAdapter` in `factory.rs`. Reuse the runtime's existing facade slot for the Slack adapter without storing that slot on the management port.
- [ ] Update every management-port test fixture to supply an in-memory removal journal store and the exact fake adapters it needs.
- [ ] Delete the obsolete removal-specific code:

  - `ExtensionCredentialCleanup` and its impl;
  - `RemovableChannelCleanup`;
  - `removable_channel_cleanup_for_summary`;
  - `disconnect_channel_for_cleanup`;
  - `cleanup_channel_before_remove`;
  - `removed_extension_providers`;
  - `revoke_exclusive_credentials`;
  - `providers_still_in_use`;
  - the management-port `channel_connection` field and `with_channel_connection_facade_slot` method;
  - removal-only `OnceLock` imports and old credential cleanup fakes/tests; and
  - removal compensation helpers/tests that assert restoration after irreversible cleanup.

- [ ] Update the durable product-auth test that referenced `ExtensionCredentialCleanup` to exercise `ProductAuthExtensionCleanupAdapter` or the production coordinator seam instead.
- [ ] Add monotonic behavior tests showing:

  - adapter failure leaves the installation disabled and package files present;
  - lifecycle/file/store purge failure leaves `Purging` state and never republishes;
  - retry after failure finishes removal; and
  - a pre-existing installation with no journal gets a lazy journal, proving no migration is required.

- [ ] Run:

  ```text
  cargo test -p ironclaw_reborn_composition --lib extension_remove -- --nocapture
  cargo test -p ironclaw_reborn_composition --lib removal -- --nocapture
  ```

- [ ] Run `rg -n "RemovableChannelCleanup|removable_channel_cleanup_for_summary|disconnect_channel_for_cleanup|cleanup_channel_before_remove|ExtensionCredentialCleanup|revoke_exclusive_credentials|providers_still_in_use" crates/ironclaw_reborn_composition crates/ironclaw_extensions` and require no production hits.
- [ ] Run `cargo fmt --check`.
- [ ] Review Task 4 and commit as `fix(reborn): coordinate extension removal cleanup`.

## Task 5: Prove both callers and document the runtime contract

**Files:**

- Modify: `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle_capabilities.rs`
- Modify: relevant extension lifecycle documentation discovered under `docs/` or crate docs
- Modify: `FEATURE_PARITY.md` only if its current status/notes become inaccurate
- Modify: `CHANGELOG.md` only if the repository's current release convention requires user-visible bug-fix entries

### Steps

- [ ] Extend the production-shaped shared WebUI/tool removal test so both callers install and remove a generic external-channel fixture with no host-owned connection and observe the same store/filesystem result without a Slack facade.
- [ ] Keep or update the Slack shared-caller test so both callers invoke exactly one actor-scoped Slack disconnect before local purge. Assert package presence during cleanup and assert the status method was never consulted.
- [ ] Add a shared-caller retry test: inject one cleanup failure, verify both callers receive the same transient classification and retained disabled state/journal, then retry through the other caller and observe complete cleanup.
- [ ] Document:

  - explicit cleanup ownership;
  - journal location and lazy creation;
  - retry/monotonic semantics;
  - no SQL/operator migration; and
  - downgrade caution while a journal is active.

- [ ] Check `FEATURE_PARITY.md` and `CHANGELOG.md`; edit only when existing project conventions require it, and record the decision in the task report.
- [ ] Run the focused caller tests with `--features "webui-v2-beta slack-v2-host-beta test-support"`.
- [ ] Run final verification:

  ```text
  cargo test -p ironclaw_extensions
  cargo test -p ironclaw_reborn_composition --lib
  cargo clippy -p ironclaw_extensions -p ironclaw_reborn_composition --all-targets --all-features -- -D warnings
  cargo fmt --check
  git diff --check
  ```

- [ ] If all-feature Clippy exposes unrelated baseline failures, rerun the narrow feature set used by the changed code and report both exact commands/results; do not hide failures.
- [ ] Run a final code review focused on security, journal transition correctness, adapter ownership, retry behavior, caller convergence, legacy-code deletion, and compatibility.
- [ ] Commit as `test(reborn): cover coordinated extension removal`.

## Completion Criteria

- Issue #5953's generic external-channel removal succeeds without a Slack facade.
- Slack personal removal still disconnects exactly once per successful cleanup task and does so before local purge.
- Product-auth cleanup is ownership-aware and no longer scans remaining extension manifests.
- A crash/failure at any phase leaves a durable journal and monotonic disabled state that a later remove call resumes.
- Existing installation JSON is unchanged; journals are lazy sidecars.
- Both WebUI and model capability paths use the same coordinator and have caller-level coverage.
- All named legacy removal helpers, fields, split cleanup paths, and compensation logic are gone.
- Focused tests, formatting, Clippy, and final review are complete with results recorded.
