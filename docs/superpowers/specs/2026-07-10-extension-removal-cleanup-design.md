# Durable Extension Removal Cleanup - Design

**Date:** 2026-07-10
**Status:** Approved design, pending implementation
**Target architecture:** IronClaw Reborn (`crates/ironclaw_*`)
**Related issue:** [nearai/ironclaw#5953](https://github.com/nearai/ironclaw/issues/5953)

## 1. Purpose

Make extension removal mean one thing across every caller: the extension is
quiesced, every resource it owns is cleaned by the module that owns that
resource, its local lifecycle state and files are deleted, and the operation is
safe to retry after a partial failure or process restart.

The WebUI `remove_extension` action and the model-invoked
`builtin.extension_remove` capability continue to converge on one removal
interface. Neither caller discovers cleanup work, invokes provider-specific
cleanup, or deletes extension state directly.

## 2. Problem

The current removal path infers cleanup from presentation metadata:

- the package id equals `"slack"`;
- the extension exposes `LifecycleExtensionSurfaceKind::ExternalChannel`; or
- the extension declares credentials.

It then routes channel cleanup through the only production
`ChannelConnectionFacade`, which is Slack-specific. This creates two incorrect
outcomes for a generic external-channel extension:

- without a wired facade, removal fails before package deletion; and
- with the Slack facade, cleanup returns success without cleaning a non-Slack
  channel.

The current sequencing also permits partial removal:

- channel cleanup is irreversible but runs before a locally compensated delete;
  a later file/store failure can restore an extension whose connection is
  already gone; and
- credential cleanup is best-effort after package deletion, so removal can
  report success while extension-owned authentication state remains.

Surface kind describes how an extension communicates. It is not resource
ownership and must not select cleanup behavior.

## 3. Goals

1. One removal interface for WebUI, model capability, CLI, and future callers.
2. Explicit cleanup requirements; no package-id or surface-kind heuristics in
   lifecycle code.
3. Cleanup owned by domain adapters: product auth cleans auth state, a channel
   adapter cleans channel state, and extension lifecycle cleans local package
   state.
4. Durable, monotonic progress that resumes after failure or restart.
5. Idempotent cleanup and deletion steps.
6. Existing installed extensions remain removable without an operator-run
   migration.
7. A successful response means all required cleanup and local deletion finished.
8. Missing required cleanup wiring fails retryably rather than silently skipping.

## 4. Non-goals

- Removing the repository's v1 `src/` architecture.
- Treating arbitrary third-party uninstall code as host authority.
- Allowing a manifest to choose tenant, user, agent, or project cleanup scope.
- Making external cleanup and local filesystem deletion a distributed ACID
  transaction.
- Adding a force-delete path to normal WebUI or model-visible removal.

## 5. Ownership Model

An installed package resolves to an `ExtensionRemovalPlan` before the first
side effect. The plan contains typed `ExtensionCleanupRequirement` values. A
requirement names the cleanup adapter and carries only adapter-validated,
non-secret binding data.

```rust
pub struct ExtensionCleanupRequirement {
    pub task_id: ExtensionCleanupTaskId,
    pub adapter_id: ExtensionCleanupAdapterId,
    pub binding: ExtensionCleanupBinding,
}

pub enum ExtensionCleanupBinding {
    ProductAuth {
        extension_id: ExtensionId,
        providers: Vec<AuthProviderId>,
    },
    ChannelConnection {
        connection_kind: ChannelConnectionKind,
    },
    ProductAdapterInstallation {
        installation_id: ProductAdapterInstallationId,
    },
}
```

The exact Rust ownership of domain-specific fields may use opaque validated
bytes or narrower domain types to preserve crate dependency direction. The
contract is that lifecycle code treats the binding as typed plan data and never
reconstructs provider/channel rules itself.

Requirements come from trusted projections:

- core package cleanup is implicit in the removal coordinator;
- validated runtime credential declarations produce product-auth cleanup
  requirements;
- a host API contract may project a host-managed resource cleanup requirement;
- host-bundled package metadata may attach a trusted cleanup requirement; and
- a compatibility resolver may project the same typed requirement for an older
  host-bundled installation whose persisted manifest predates that declaration.

Manifest declarations remain untrusted. A registered host API contract must
validate and project them. The browser/model cannot supply cleanup requirements
or cleanup scope.

Absence of a requirement means the package owns no resource in that domain.
Presence means cleanup is required. There is no `Optional`, `IfSupported`, or
facade-probing cleanup mode.

## 6. Deep Module and Interface

The removal state machine belongs in `ironclaw_extensions`, the crate that owns
extension manifests, installations, and lifecycle state. Reborn composition
wires adapters and storage; it does not own removal policy.

The external interface is one operation:

```rust
pub struct ExtensionRemovalRequest {
    pub extension_id: ExtensionId,
    pub installation_id: ExtensionInstallationId,
    pub actor: AuthenticatedRemovalActor,
}

pub enum ExtensionRemovalOutcome {
    Removed,
    AlreadyRemoved,
}

impl ExtensionRemovalCoordinator {
    pub async fn remove(
        &self,
        request: ExtensionRemovalRequest,
    ) -> Result<ExtensionRemovalOutcome, ExtensionRemovalError>;
}
```

`AuthenticatedRemovalActor` is constructed from trusted caller/execution
context. It does not accept body-supplied authority. The coordinator derives
the resource scope passed to adapters.

The coordinator hides plan creation, journaling, adapter lookup, ordering,
retry behavior, local purge, and sanitized error mapping. Callers need not know
which extension uses Slack, OAuth, a product adapter, WASM, MCP, or no external
resources.

## 7. Cleanup Adapter Seam

Domain cleanup crosses a real dependency-inversion seam: the extension domain
must not depend on Slack or product-auth implementations. Adapters satisfy a
small internal interface:

```rust
#[async_trait]
pub trait ExtensionCleanupAdapter: Send + Sync {
    fn id(&self) -> &ExtensionCleanupAdapterId;

    async fn cleanup(
        &self,
        context: &ExtensionCleanupContext,
        binding: &ExtensionCleanupBinding,
    ) -> Result<ExtensionCleanupReceipt, ExtensionCleanupError>;
}
```

Each adapter must:

- accept only its binding variant;
- be idempotent;
- derive scope from trusted context;
- fence concurrent creation/reconnection before deleting owned state;
- return success only when its required cleanup is complete; and
- return a sanitized retryable error for transient backend failure.

Initial production adapters:

1. **Product auth:** cancel pending extension/provider flows, remove extension
   grants, revoke and purge extension-owned accounts and secrets, and preserve
   shared accounts. Product auth decides ownership from its durable account and
   grant records; lifecycle does not scan remaining manifests to guess sharing.
2. **Slack personal connection:** fence reconnects, remove owned Slack identity
   bindings and personal DM targets, and delegate credential revocation through
   product auth. It is selected by an explicit Slack personal-connection
   requirement, not by package id inside lifecycle code.
3. **Product adapter installation:** remove host-owned conversation/pairing or
   routing state keyed by the adapter installation where such state exists.

An ordinary extension with only lifecycle-owned files has no domain cleanup
requirements. A generic `ExternalChannel` package likewise has no channel
requirement unless its validated contract says the host owns a connection for
it.

`ChannelConnectionFacade` remains available for the WebUI connection-status and
explicit disconnect interface. The removal coordinator does not use its
connection map to discover cleanup work.

## 8. Durable Removal Journal

Do not add required fields to existing `ExtensionInstallation` records. Store
removal progress in a sidecar journal keyed by installation id. The journal is
created only when removal begins, so existing installations need no eager
rewrite.

```rust
pub struct ExtensionRemovalJournal {
    pub schema_version: ExtensionRemovalJournalVersion,
    pub installation_id: ExtensionInstallationId,
    pub extension_id: ExtensionId,
    pub phase: ExtensionRemovalPhase,
    pub plan: Vec<ExtensionCleanupRequirement>,
    pub completed_tasks: BTreeSet<ExtensionCleanupTaskId>,
}

pub enum ExtensionRemovalPhase {
    Planned,
    Quiesced,
    Cleaning,
    Purging,
}
```

The filesystem adapter stores journals outside the extension's materialized
directory so deleting package files cannot delete recovery state. Writes use
the repository's atomic file-replacement pattern. The in-memory adapter mirrors
the same state transitions in tests.

The plan is immutable once the journal exists. A retry uses the persisted plan
even if the catalog or manifest source later changes.

## 9. State Machine

Removal is monotonic:

1. **Validate:** authenticate the actor, load the installation and manifest,
   acquire the installation-scoped operation lock, and reject conflicting
   install/activate/update work.
2. **Plan:** resolve explicit cleanup requirements and atomically persist the
   journal before any side effect.
3. **Quiesce:** set the existing installation state to `Disabled`, remove it
   from active publication, and stop new runtime/channel work. Persist
   `Quiesced`.
4. **Clean:** run required adapters in deterministic order. After each adapter
   succeeds, atomically add its task id to `completed_tasks`. A crash after an
   adapter succeeds but before the journal update is safe because adapters are
   idempotent.
5. **Purge:** remove lifecycle registrations, hooks/processes owned by the
   package, materialized files, and the manifest. Delete the installation record
   last.
6. **Finish:** delete the sidecar journal and return `Removed`. If a retry sees
   the journal but the installation and package are already absent, it removes
   the stale journal and returns `AlreadyRemoved`.

Once quiescing or external cleanup begins, the coordinator never compensates by
reactivating the extension. External effects cannot be rolled back safely. A
failure leaves the extension disabled with its journal intact, and the next
removal request resumes.

## 10. Failure Semantics

- Missing required adapter: retryable `Unavailable`; journal retained.
- Adapter backend/transient failure: retryable `Unavailable`; completed tasks
  retained.
- Invalid/tampered cleanup binding: non-retryable internal/recovery-required
  failure; no local purge.
- Local purge failure: retryable `Unavailable`; do not reactivate or rerun
  completed cleanup unnecessarily.
- Concurrent removal: serialize by installation; the second caller observes or
  resumes the same journal.
- Install/activate/update during removal: conflict until removal finishes.

Normal callers never receive success while a required cleanup task is pending.
An operator-only force-purge, if added later, requires a separate audited design
and must not be exposed to the model.

## 11. Backward Compatibility and Migration

There is no SQL migration and no operator-run state migration.

Compatibility is still required because old runtime versions already wrote
installation and manifest records. The new code must read them and build a
correct first-removal plan:

- existing installation records remain byte-compatible and unchanged;
- the new sidecar journal is created lazily on first removal;
- runtime credential declarations already present in old manifests project
  product-auth cleanup requirements;
- the trusted first-party catalog/compatibility resolver projects the explicit
  Slack personal-connection requirement for older bundled Slack installations;
  and
- once persisted, the journal is the source of truth for retries.

Compatibility logic is isolated in plan projection. It must not preserve the
old runtime execution path or the old cleanup heuristics. New removals always
execute through the coordinator and typed adapters.

A downgrade while a new removal journal is in progress is unsafe because the
old binary does not understand the journal. Rollback documentation must require
finishing or inspecting pending removals before downgrading. A deployment that
has created no journals remains downgrade-compatible because existing
installation records are unchanged.

## 12. Legacy Removal Code to Delete

After the coordinator is production-wired and its caller tests pass, remove:

- `RemovableChannelCleanup`;
- `removable_channel_cleanup_for_summary`;
- `disconnect_channel_for_cleanup`;
- `cleanup_channel_before_remove`;
- the extension-management `channel_connection` `OnceLock` and its removal-only
  wiring;
- the lifecycle-local `ExtensionCredentialCleanup` best-effort post-removal
  path and provider-sharing scan;
- removal-specific use of `ChannelConnectionFacade` as cleanup discovery;
- tests that assert generic external-channel removal fails without Slack; and
- obsolete split cleanup in either WebUI or model capability callers.

Do not delete `ChannelConnectionFacade` while connection status and explicit
disconnect callers still consume it. Do not modify v1 `src/` for this Reborn
fix.

## 13. Security Invariants

- Cleanup scope comes from the authenticated caller and installation, never
  request JSON.
- Manifest cleanup declarations grant no authority by themselves.
- Unknown cleanup adapter ids fail closed.
- Raw credentials, tokens, provider bodies, host paths, and backend details are
  absent from journals, receipts, logs, and user-visible errors.
- Cleanup tasks may delete only resources whose durable owner/binding matches
  the extension and trusted scope.
- Shared credentials and shared resources are preserved by their owning domain,
  not guessed by lifecycle code.
- Extension code cannot execute an arbitrary uninstall hook with host trust.

## 14. Testing

### Domain/state-machine tests

- no cleanup requirements removes an ordinary extension;
- explicit generic channel requirement invokes its matching adapter;
- generic `ExternalChannel` without a host-owned connection does not invoke
  Slack cleanup;
- missing required adapter retains the package/journal and returns retryable
  failure;
- each phase resumes after an injected process-stop equivalent;
- completed tasks are not semantically duplicated on retry;
- local purge failure after external cleanup never reactivates the extension;
- concurrent removals converge on one journal; and
- old installation records lazily create the correct Slack/product-auth plan.

### Adapter tests

- Slack cleanup fences reconnect races and removes identities/DM targets;
- product-auth cleanup cancels pending flows, removes extension grants, revokes
  extension-owned secrets, and preserves shared accounts;
- adapter errors are sanitized and retryable; and
- wrong binding variants are rejected.

### Caller-level tests

- WebUI `remove_extension` and `builtin.extension_remove` drive the same
  coordinator and observable filesystem/store side effects;
- both callers remove a generic channel with no host-owned connection;
- both callers clean Slack before final local purge;
- both callers surface pending/retryable removal consistently; and
- production composition fails if a declared required cleanup adapter is not
  registered.

Targeted tests must assert package files/state and owned domain records, not
only a `Removed` response.

## 15. Documentation and Parity

Update the extension lifecycle/removal contract documentation and name its
caller-level test command. Check `FEATURE_PARITY.md`; this bug fix does not
change a feature's implementation status unless the current entry claims
cleanup guarantees that were previously absent. Add a changelog note if the
release process classifies removal-state recovery as user-visible behavior.
