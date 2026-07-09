//! Filesystem-backed governor stores under the `/resources` mount alias.
//!
//! This module hosts both filesystem-backed stores this crate exposes:
//!
//! - [`FilesystemResourceGovernorStore`] — the resource-governor compaction
//!   snapshot at `/resources/snapshot.json` (and the legacy transactional
//!   store used by snapshot-focused contract tests).
//! - [`FilesystemBudgetGateStore`] — the budget-approval gate snapshot
//!   at `/resources/budget-gates.json`.
//!
//! Both persist a single JSON snapshot under the caller-supplied
//! [`MountView`](ironclaw_host_api::MountView) and route every
//! read-modify-write transaction through
//! [`crate::cas_snapshot::CasSnapshotStore`], which provides:
//!
//! - The shared, lock-free
//!   [`cas_update`](ironclaw_filesystem::cas_update) helper: an optimistic
//!   CAS-retry loop (`CasExpectation::Version` precondition) with bounded
//!   retries, jittered backoff, and an overall timeout. No per-record
//!   `tokio::sync::Mutex` is held across the backend awaits, so cross-process
//!   contention on one scope's snapshot is resolved lock-free rather than
//!   convoyed. Same-process writers sharing a cloned store handle still
//!   serialize one job at a time on the dedicated `AsyncStorageWorker`
//!   below — #5470 tracks making the store async so they can overlap too.
//!   The helper fails closed on a non-CAS backend rather than
//!   blind-overwriting.
//! - A dedicated current-thread tokio worker bridging the sync trait
//!   surface to the async [`ScopedFilesystem`] API.
//!
//! Tenant/user identity comes from the
//! [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem) mount
//! view rewriting the alias-relative snapshot path to a tenant/user-
//! scoped target; this module never derives identity from
//! `ResourceScope` itself.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ResourceScope;
use serde::{Deserialize, Serialize};

use crate::cas_snapshot::{CasSnapshotStore, Snapshot};
use crate::gate::{
    BudgetApprovalGate, BudgetGateError, BudgetGateId, BudgetGateOutcome, BudgetGateStatus,
    BudgetGateStore,
};
use crate::{ResourceError, ResourceGovernorSnapshot, ResourceGovernorStore};

const GOVERNOR_SNAPSHOT_PATH: &str = "/resources/snapshot.json";
const GATES_SNAPSHOT_PATH: &str = "/resources/budget-gates.json";

// ---------------------------------------------------------------------------
// Resource-governor snapshot store
// ---------------------------------------------------------------------------

/// Filesystem-backed resource-governor snapshot store under the
/// `/resources` mount alias.
///
/// Construct with a [`ScopedFilesystem`] over any [`RootFilesystem`].
/// The [`ScopedFilesystem`] resolves the `/resources` alias to a
/// tenant/user-scoped [`VirtualPath`](ironclaw_host_api::VirtualPath)
/// per its [`MountView`](ironclaw_host_api::MountView) and enforces
/// per-op ACL before any backend dispatch — so tenant isolation is
/// structural rather than something this crate has to re-derive from
/// `ResourceScope`.
///
/// The whole governor state (limits, reservations, usage by account)
/// is serialized as one snapshot at `/resources/snapshot.json` under
/// the system scope. Resource quotas are process-global (operator-set
/// caps applied across all tenants), which is why the snapshot lives
/// under [`ResourceScope::system`] rather than a tenant scope —
/// tenant-scoped resource accounting is a future capability that would
/// change the [`ResourceGovernorStore`] trait surface.
pub struct FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem,
{
    store: CasSnapshotStore<F>,
}

impl<F> Clone for FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem,
{
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
        }
    }
}

impl<F> FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            store: CasSnapshotStore::new(
                filesystem,
                GOVERNOR_SNAPSHOT_PATH,
                ResourceScope::system(),
                "resource-governor-filesystem",
            ),
        }
    }
}

impl<F> ResourceGovernorStore for FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem + 'static,
{
    fn update<T, U>(&self, update: U) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        U: FnMut(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        self.store
            .update::<ResourceGovernorSnapshot, T, ResourceError, _>(update)
    }

    fn inspect<T, U>(&self, inspect: U) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        U: FnOnce(&ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        self.store
            .inspect::<ResourceGovernorSnapshot, T, ResourceError, _>(inspect)
    }
}

// ---------------------------------------------------------------------------
// Budget-gate snapshot store
// ---------------------------------------------------------------------------

/// Filesystem-backed budget gate store.
///
/// Each call routes through the caller-supplied [`ResourceScope`], so a
/// single shared instance can serve every tenant: the
/// `ScopedFilesystem` mount view rewrites
/// `/resources/budget-gates.json` under the caller's tenant root for
/// every read or write (review feedback Thermo-Nuclear #2: scope at the
/// store-operation boundary instead of at construction time).
#[derive(Clone)]
pub struct FilesystemBudgetGateStore<F>
where
    F: RootFilesystem,
{
    store: CasSnapshotStore<F>,
    /// Terminal gates older than this are dropped from the snapshot on
    /// every mutation. Bounds the snapshot size so `list_pending` /
    /// `open` / `resolve` stay roughly O(active pending) instead of
    /// O(total gates ever opened) (review feedback Medium #7).
    /// `None` retains every terminal gate forever (legacy behavior for
    /// tests that want to inspect resolved gates without time
    /// constraints).
    terminal_retention: Option<chrono::Duration>,
}

impl<F> std::fmt::Debug for FilesystemBudgetGateStore<F>
where
    F: RootFilesystem,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesystemBudgetGateStore")
            .field("retention", &self.terminal_retention)
            .finish()
    }
}

impl<F> FilesystemBudgetGateStore<F>
where
    F: RootFilesystem + 'static,
{
    /// Construct a shared store. Every operation supplies its own
    /// scope; the underlying `ScopedFilesystem` rewrites the snapshot
    /// path under the supplied scope's tenant/user mount view.
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            store: CasSnapshotStore::new(
                filesystem,
                GATES_SNAPSHOT_PATH,
                // The default scope is only used by callers that go
                // through `CasSnapshotStore::update` without supplying
                // a scope — the gate store always supplies one via
                // `update_with_scope`. Keep `system()` here so any
                // accidental fallback lands in a documented place.
                ResourceScope::system(),
                "budget-gate-filesystem",
            ),
            // Default: 30-day retention for terminal gates. Production
            // can tune via `with_terminal_retention`; tests that need
            // to read older terminal gates set it to `None` or a
            // larger window.
            terminal_retention: Some(chrono::Duration::days(30)),
        }
    }

    /// Override the retention window for terminal gates. Set to `None`
    /// to retain every terminal gate forever (legacy behavior used by
    /// audit-replay tests). Operators tune this via composition.
    pub fn with_terminal_retention(mut self, retention: Option<chrono::Duration>) -> Self {
        self.terminal_retention = retention;
        self
    }

    fn with_snapshot<T, U>(
        &self,
        scope: &ResourceScope,
        mut update: U,
    ) -> Result<T, BudgetGateError>
    where
        T: Send + 'static,
        U: FnMut(&mut BudgetGateSnapshot) -> Result<T, BudgetGateError> + Send + 'static,
    {
        let retention = self.terminal_retention;
        // The outer caller's `update` runs first, then we apply retention
        // pruning so the result of the user's update is never re-pruned
        // (`get` should return what was just written). The closure is
        // re-runnable: `cas_update` re-invokes it against a freshly read
        // snapshot on every CAS retry, so it must not consume captured
        // state by move (leaf closures clone any captured value per call).
        let wrapped = move |snapshot: &mut BudgetGateSnapshot| -> Result<T, BudgetGateError> {
            snapshot.ensure_current()?;
            let value = update(snapshot)?;
            snapshot.schema_version = BudgetGateSnapshot::CURRENT_SCHEMA;
            if let Some(retention) = retention {
                prune_terminal_gates(snapshot, Utc::now() - retention);
            }
            Ok(value)
        };
        self.store
            .update_with_scope::<BudgetGateSnapshot, T, BudgetGateError, _>(scope.clone(), wrapped)
    }
}

impl<F> BudgetGateStore for FilesystemBudgetGateStore<F>
where
    F: RootFilesystem + 'static,
{
    fn open(&self, scope: &ResourceScope, gate: BudgetApprovalGate) -> Result<(), BudgetGateError> {
        // Clone per invocation: `with_snapshot` may re-run this closure on a
        // CAS retry, so it must not move `gate` out of its capture.
        self.with_snapshot(scope, move |snapshot| {
            let gate = gate.clone();
            snapshot.gates.insert(gate.id, gate);
            Ok(())
        })
    }

    fn resolve(
        &self,
        scope: &ResourceScope,
        id: BudgetGateId,
        outcome: BudgetGateOutcome,
        at: DateTime<Utc>,
    ) -> Result<BudgetApprovalGate, BudgetGateError> {
        // Clone per invocation: this closure may be re-run on a CAS retry,
        // so match on a fresh clone of `outcome` rather than moving it out.
        self.with_snapshot(scope, move |snapshot| {
            let gate = snapshot
                .gates
                .get_mut(&id)
                .ok_or(BudgetGateError::Unknown { id })?;
            if gate.status.is_terminal() {
                return Err(BudgetGateError::AlreadyResolved { id });
            }
            gate.status = match outcome.clone() {
                BudgetGateOutcome::Approve {
                    increased_limit,
                    by,
                } => BudgetGateStatus::Approved {
                    increased_limit,
                    by,
                    at,
                },
                BudgetGateOutcome::Cancel { by } => BudgetGateStatus::Cancelled { by, at },
            };
            Ok(gate.clone())
        })
    }

    fn expire_pending_older_than(
        &self,
        scope: &ResourceScope,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(scope, move |snapshot| {
            let mut expired = Vec::new();
            for gate in snapshot.gates.values_mut() {
                if matches!(gate.status, BudgetGateStatus::Pending) && gate.expires_at <= cutoff {
                    gate.status = BudgetGateStatus::Expired { at: cutoff };
                    expired.push(gate.clone());
                }
            }
            Ok(expired)
        })
    }

    fn get(
        &self,
        scope: &ResourceScope,
        id: BudgetGateId,
    ) -> Result<Option<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(scope, move |snapshot| Ok(snapshot.gates.get(&id).cloned()))
    }

    fn list_pending(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(scope, move |snapshot| {
            Ok(snapshot
                .gates
                .values()
                .filter(|gate| matches!(gate.status, BudgetGateStatus::Pending))
                .cloned()
                .collect())
        })
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetGateSnapshot {
    /// Schema version. Bump when the on-disk shape changes; today
    /// there is only v1.
    schema_version: u32,
    /// All gates, keyed by id. Terminal-state gates persist so audit /
    /// `get(id)` lookups can still hydrate them after a restart.
    gates: HashMap<BudgetGateId, BudgetApprovalGate>,
}

impl BudgetGateSnapshot {
    const CURRENT_SCHEMA: u32 = 1;

    fn ensure_current(&mut self) -> Result<(), BudgetGateError> {
        if self.schema_version == 0 {
            // Default value (never persisted) — coerce to current schema.
            self.schema_version = Self::CURRENT_SCHEMA;
            return Ok(());
        }
        if self.schema_version != Self::CURRENT_SCHEMA {
            return Err(BudgetGateError::Storage {
                reason: format!(
                    "budget gate snapshot schema {} is not supported (expected {})",
                    self.schema_version,
                    Self::CURRENT_SCHEMA
                ),
            });
        }
        Ok(())
    }
}

impl Snapshot for BudgetGateSnapshot {
    const RECORD_KIND: &'static str = "budget_gate_snapshot";

    fn fresh() -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA,
            gates: HashMap::new(),
        }
    }
}

/// Drop terminal-status gates whose resolution timestamp is older
/// than `cutoff`. Pending gates are never pruned. Bounds the snapshot
/// size so hot path operations stay O(active pending).
fn prune_terminal_gates(snapshot: &mut BudgetGateSnapshot, cutoff: DateTime<Utc>) {
    snapshot.gates.retain(|_, gate| match &gate.status {
        BudgetGateStatus::Pending => true,
        BudgetGateStatus::Approved { at, .. }
        | BudgetGateStatus::Cancelled { at, .. }
        | BudgetGateStatus::Expired { at } => *at >= cutoff,
    });
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
        ResourceEstimate, TenantId, UserId, VirtualPath,
    };
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{
        PersistentResourceGovernor, ResourceAccount, ResourceApprovalNeeded, ResourceDimension,
        ResourceGovernor, ResourceLimits, ResourceValue,
    };

    fn scoped_resources_fs(
        backend: Arc<InMemoryBackend>,
        tenant: &str,
        user: &str,
    ) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let tenant_user_prefix = format!("/tenants/{tenant}/users/{user}");
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new(format!("{tenant_user_prefix}/resources")).expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn sample_scope(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: project.map(|value| ProjectId::new(value).unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    // -----------------------------------------------------------------
    // FilesystemResourceGovernorStore
    // -----------------------------------------------------------------

    #[test]
    fn snapshot_persists_and_reloads_through_scoped_filesystem() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-a", "alice");

        let store = FilesystemResourceGovernorStore::new(Arc::clone(&scoped));
        let scope = sample_scope("tenant-a", "alice", Some("p1"));
        let account = ResourceAccount::tenant(scope.tenant_id.clone());

        let governor = PersistentResourceGovernor::new(store);
        governor
            .try_set_limit(
                account.clone(),
                ResourceLimits::default()
                    .set_max_usd(dec!(1.00))
                    .set_max_concurrency_slots(1),
            )
            .unwrap();
        let reservation = governor
            .reserve(
                scope.clone(),
                ResourceEstimate::default().set_concurrency_slots(1),
            )
            .unwrap();

        // Reload from the same on-disk snapshot.
        let reloaded = PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(
            Arc::clone(&scoped),
        ));
        assert_eq!(
            reloaded.reserved_for(&account).unwrap().concurrency_slots,
            1
        );

        // Concurrency-slot budget is exhausted; a second reservation
        // must be denied even though it goes through a fresh store
        // handle.
        let denied = reloaded
            .reserve(scope, ResourceEstimate::default().set_concurrency_slots(1))
            .unwrap_err();
        assert!(matches!(denied, ResourceError::LimitExceeded { .. }));

        reloaded.release(reservation.id).unwrap();
    }

    /// Cross-tenant isolation regression — two `ScopedFilesystem`s
    /// over the same `RootFilesystem` with disjoint `MountView`
    /// targets must produce fully disjoint snapshots. Writing on
    /// tenant A must not be visible from tenant B, even when both
    /// scopes carry the same `user_id` and `project_id`.
    #[test]
    fn isolates_two_tenants_with_same_user_project_ids() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped_a = scoped_resources_fs(Arc::clone(&backend), "tenant-a", "alice");
        let scoped_b = scoped_resources_fs(Arc::clone(&backend), "tenant-b", "alice");

        let governor_a =
            PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(scoped_a));
        let governor_b =
            PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(scoped_b));

        let scope_a = sample_scope("tenant-a", "alice", Some("p1"));
        let scope_b = sample_scope("tenant-b", "alice", Some("p1"));
        let account_a = ResourceAccount::tenant(scope_a.tenant_id.clone());
        let account_b = ResourceAccount::tenant(scope_b.tenant_id.clone());

        governor_a
            .try_set_limit(
                account_a.clone(),
                ResourceLimits::default().set_max_concurrency_slots(1),
            )
            .unwrap();
        governor_a
            .reserve(
                scope_a,
                ResourceEstimate::default().set_concurrency_slots(1),
            )
            .unwrap();
        assert_eq!(
            governor_a
                .reserved_for(&account_a)
                .unwrap()
                .concurrency_slots,
            1
        );

        assert_eq!(
            governor_b
                .reserved_for(&account_b)
                .unwrap()
                .concurrency_slots,
            0
        );
        governor_b
            .try_set_limit(
                account_b.clone(),
                ResourceLimits::default().set_max_concurrency_slots(1),
            )
            .unwrap();
        let reservation = governor_b
            .reserve(
                scope_b,
                ResourceEstimate::default().set_concurrency_slots(1),
            )
            .unwrap();
        assert_eq!(
            governor_b
                .reserved_for(&account_b)
                .unwrap()
                .concurrency_slots,
            1
        );
        assert_eq!(
            governor_a
                .reserved_for(&account_a)
                .unwrap()
                .concurrency_slots,
            1
        );

        governor_b.release(reservation.id).unwrap();
    }

    // -----------------------------------------------------------------
    // FilesystemBudgetGateStore
    // -----------------------------------------------------------------

    fn gate_scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn sample_needed() -> ResourceApprovalNeeded {
        ResourceApprovalNeeded {
            account: ResourceAccount::tenant(TenantId::new("t").unwrap()),
            dimension: ResourceDimension::Usd,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            current_usage: ResourceValue::Decimal(Decimal::from(0)),
            active_reserved: ResourceValue::Decimal(Decimal::from(0)),
            requested: ResourceValue::Decimal(Decimal::from(9)),
            utilization: 0.91,
            period_end: None,
        }
    }

    fn sample_gate() -> BudgetApprovalGate {
        BudgetApprovalGate {
            id: BudgetGateId::new(),
            needed: sample_needed(),
            opened_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            status: BudgetGateStatus::Pending,
        }
    }

    #[test]
    fn open_and_get_round_trips_through_filesystem() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let scope = gate_scope("tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let gate = sample_gate();
        let id = gate.id;
        store.open(&scope, gate.clone()).unwrap();
        let reloaded = store.get(&scope, id).unwrap().unwrap();
        assert_eq!(reloaded.id, id);
        assert!(matches!(reloaded.status, BudgetGateStatus::Pending));
    }

    /// Regression for #3841 follow-up: pending gates must NOT be lost
    /// on process restart. A fresh `FilesystemBudgetGateStore` over
    /// the same backend filesystem must rehydrate the prior snapshot.
    #[test]
    fn pending_gate_survives_restart_via_fresh_handle() {
        let backend = Arc::new(InMemoryBackend::new());
        let gate = sample_gate();
        let id = gate.id;
        let scope = gate_scope("tenant-fs", "alice");
        {
            let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
            let store = FilesystemBudgetGateStore::new(scoped);
            store.open(&scope, gate).unwrap();
        }
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let reloaded = store.get(&scope, id).unwrap().unwrap();
        assert_eq!(reloaded.id, id);
        assert!(matches!(reloaded.status, BudgetGateStatus::Pending));
        let pending = store.list_pending(&scope).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
    }

    #[test]
    fn resolve_updates_gate_status_after_reload() {
        let backend = Arc::new(InMemoryBackend::new());
        let scope = gate_scope("tenant-fs", "alice");
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let gate = sample_gate();
        let id = gate.id;
        store.open(&scope, gate).unwrap();
        let resolved = store
            .resolve(
                &scope,
                id,
                BudgetGateOutcome::Cancel {
                    by: UserId::new("alice").unwrap(),
                },
                Utc::now(),
            )
            .unwrap();
        assert!(matches!(
            resolved.status,
            BudgetGateStatus::Cancelled { .. }
        ));

        let scoped2 = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store2 = FilesystemBudgetGateStore::new(scoped2);
        let reloaded = store2.get(&scope, id).unwrap().unwrap();
        assert!(matches!(
            reloaded.status,
            BudgetGateStatus::Cancelled { .. }
        ));
        assert!(store2.list_pending(&scope).unwrap().is_empty());
    }

    #[test]
    fn approved_gate_with_increased_decimal_limit_reloads() {
        let backend = Arc::new(InMemoryBackend::new());
        let scope = gate_scope("tenant-fs", "alice");
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let gate = sample_gate();
        let id = gate.id;
        let increased_limit = ResourceLimits::default().set_max_usd(dec!(1000.00));

        store.open(&scope, gate).unwrap();
        store
            .resolve(
                &scope,
                id,
                BudgetGateOutcome::Approve {
                    increased_limit: increased_limit.clone(),
                    by: UserId::new("alice").unwrap(),
                },
                Utc::now(),
            )
            .unwrap();

        let scoped2 = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store2 = FilesystemBudgetGateStore::new(scoped2);
        let reloaded = store2.get(&scope, id).unwrap().unwrap();
        assert!(matches!(
            reloaded.status,
            BudgetGateStatus::Approved {
                increased_limit: ref reloaded_limit,
                ..
            } if reloaded_limit == &increased_limit
        ));
    }

    #[test]
    fn expire_pending_older_than_persists_terminal_state() {
        let backend = Arc::new(InMemoryBackend::new());
        let scope = gate_scope("tenant-fs", "alice");
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let mut gate = sample_gate();
        gate.expires_at = Utc::now() - chrono::Duration::hours(1);
        let id = gate.id;
        store.open(&scope, gate).unwrap();
        let expired = store.expire_pending_older_than(&scope, Utc::now()).unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, id);

        let scoped2 = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store2 = FilesystemBudgetGateStore::new(scoped2);
        let reloaded = store2.get(&scope, id).unwrap().unwrap();
        assert!(matches!(reloaded.status, BudgetGateStatus::Expired { .. }));
    }

    /// Regression for review feedback Medium #7: terminal gates older
    /// than the retention window are pruned on the next mutation so
    /// the snapshot doesn't grow unbounded over the lifetime of a
    /// long-running deployment.
    #[test]
    fn terminal_gates_older_than_retention_are_pruned_on_next_write() {
        let backend = Arc::new(InMemoryBackend::new());
        let scope = gate_scope("tenant-retention", "alice");
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-retention", "alice");
        let store = FilesystemBudgetGateStore::new(scoped)
            .with_terminal_retention(Some(chrono::Duration::days(7)));

        let stale = sample_gate();
        let stale_id = stale.id;
        store.open(&scope, stale).unwrap();
        let old_at = Utc::now() - chrono::Duration::days(30);
        store
            .resolve(
                &scope,
                stale_id,
                BudgetGateOutcome::Cancel {
                    by: UserId::new("alice").unwrap(),
                },
                old_at,
            )
            .unwrap();

        let fresh = sample_gate();
        let fresh_id = fresh.id;
        store.open(&scope, fresh).unwrap();

        assert!(
            store.get(&scope, stale_id).unwrap().is_none(),
            "terminal gate older than the retention window must be pruned"
        );
        assert!(
            store.get(&scope, fresh_id).unwrap().is_some(),
            "fresh gate must survive pruning"
        );
    }

    /// Regression for review feedback High #1: two stores constructed
    /// with different tenant scopes must NOT see each other's gates.
    /// Without per-tenant scoping, `list_pending` on one store would
    /// surface gates from another tenant.
    #[test]
    fn list_pending_does_not_leak_across_tenants() {
        let backend = Arc::new(InMemoryBackend::new());

        let scope_a = gate_scope("tenant-a", "alice");
        let scoped_a = scoped_resources_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store_a = FilesystemBudgetGateStore::new(scoped_a);
        let gate_a = sample_gate();
        let id_a = gate_a.id;
        store_a.open(&scope_a, gate_a).unwrap();

        let scope_b = gate_scope("tenant-b", "bob");
        let scoped_b = scoped_resources_fs(Arc::clone(&backend), "tenant-b", "bob");
        let store_b = FilesystemBudgetGateStore::new(scoped_b);
        let gate_b = sample_gate();
        let id_b = gate_b.id;
        store_b.open(&scope_b, gate_b).unwrap();

        let pending_a = store_a.list_pending(&scope_a).unwrap();
        assert_eq!(pending_a.len(), 1, "store_a must see only its own gate");
        assert_eq!(pending_a[0].id, id_a);

        let pending_b = store_b.list_pending(&scope_b).unwrap();
        assert_eq!(pending_b.len(), 1, "store_b must see only its own gate");
        assert_eq!(pending_b[0].id, id_b);

        assert!(
            store_a.get(&scope_a, id_b).unwrap().is_none(),
            "store_a must NOT see gates opened in tenant-b's scope"
        );
        assert!(
            store_b.get(&scope_b, id_a).unwrap().is_none(),
            "store_b must NOT see gates opened in tenant-a's scope"
        );
    }
}
