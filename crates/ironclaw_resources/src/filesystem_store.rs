//! Filesystem-backed governor stores under the `/resources` mount alias.
//!
//! This module hosts both filesystem-backed stores this crate exposes:
//!
//! - [`FilesystemResourceGovernorStore`] — the single resource-governor
//!   snapshot at `/resources/snapshot.json`.
//! - [`FilesystemBudgetGateStore`] — the budget-approval gate snapshot
//!   at `/resources/budget-gates.json`.
//!
//! Both persist a single JSON snapshot under the caller-supplied
//! [`MountView`](ironclaw_host_api::MountView) and route every
//! read-modify-write transaction through
//! [`crate::cas_snapshot::CasSnapshotStore`], which provides:
//!
//! - In-process per-path async lock so same-process writers serialize.
//! - `CasExpectation::Version` precondition for cross-process safety,
//!   with `CasExpectation::Any` fallback for byte-only backends.
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
use ironclaw_host_api::{ResourceScope, TenantId, UserId};
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
#[derive(Clone)]
pub struct FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem,
{
    store: CasSnapshotStore<F>,
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
        U: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        self.store
            .update::<ResourceGovernorSnapshot, T, ResourceError, _>(update)
    }
}

// ---------------------------------------------------------------------------
// Budget-gate snapshot store
// ---------------------------------------------------------------------------

/// Filesystem-backed budget gate store.
///
/// All gates persist into one tenant/user-scoped snapshot file at
/// `/resources/budget-gates.json`; per-tenant separation is structural
/// via the caller's [`MountView`](ironclaw_host_api::MountView) plus
/// the [`ResourceScope`] supplied at store construction (review
/// feedback High #1).
///
/// Two stores constructed with different `scope.tenant_id` see
/// entirely separate snapshots — cross-tenant access requires
/// constructing a fresh store with the target scope. A
/// [`Self::system_scoped`] convenience constructor wires the store to
/// [`ResourceScope::system`] for single-tenant local-dev runtimes.
#[derive(Clone)]
pub struct FilesystemBudgetGateStore<F>
where
    F: RootFilesystem,
{
    store: CasSnapshotStore<F>,
    tenant_id: TenantId,
    user_id: UserId,
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
            .field("tenant", &self.tenant_id)
            .field("user", &self.user_id)
            .finish()
    }
}

impl<F> FilesystemBudgetGateStore<F>
where
    F: RootFilesystem + 'static,
{
    /// Construct a tenant-scoped store. Every operation routes through
    /// `scope` so the `ScopedFilesystem` mount view writes the snapshot
    /// under that tenant's `/resources/budget-gates.json` path.
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, scope: ResourceScope) -> Self {
        let tenant_id = scope.tenant_id.clone();
        let user_id = scope.user_id.clone();
        Self {
            store: CasSnapshotStore::new(
                filesystem,
                GATES_SNAPSHOT_PATH,
                scope,
                "budget-gate-filesystem",
            ),
            tenant_id,
            user_id,
            // Default: 30-day retention for terminal gates. Production
            // can tune via `with_terminal_retention`; tests that need
            // to read older terminal gates set it to `None` or a
            // larger window.
            terminal_retention: Some(chrono::Duration::days(30)),
        }
    }

    /// Back-compat factory: wires the store to [`ResourceScope::system`].
    /// **Avoid in production** — every tenant lands in the system
    /// `/tenants/__SYSTEM__/...` directory and `list_pending` returns
    /// gates across all callers. Useful only for single-tenant local-
    /// dev runtimes that have no tenant-isolation requirements.
    pub fn system_scoped(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self::new(filesystem, ResourceScope::system())
    }

    /// Override the retention window for terminal gates. Set to `None`
    /// to retain every terminal gate forever (legacy behavior used by
    /// audit-replay tests). Operators tune this via composition.
    pub fn with_terminal_retention(mut self, retention: Option<chrono::Duration>) -> Self {
        self.terminal_retention = retention;
        self
    }

    fn with_snapshot<T, U>(&self, update: U) -> Result<T, BudgetGateError>
    where
        T: Send + 'static,
        U: FnOnce(&mut BudgetGateSnapshot) -> Result<T, BudgetGateError> + Send + 'static,
    {
        let retention = self.terminal_retention;
        // The outer caller's `update` runs first, then we apply
        // retention pruning so the result of the user's update is
        // never re-pruned (`get` should return what was just written).
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
            .update::<BudgetGateSnapshot, T, BudgetGateError, _>(wrapped)
    }
}

impl<F> BudgetGateStore for FilesystemBudgetGateStore<F>
where
    F: RootFilesystem + 'static,
{
    fn open(&self, gate: BudgetApprovalGate) -> Result<(), BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            snapshot.gates.insert(gate.id, gate);
            Ok(())
        })
    }

    fn resolve(
        &self,
        id: BudgetGateId,
        outcome: BudgetGateOutcome,
        at: DateTime<Utc>,
    ) -> Result<BudgetApprovalGate, BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            let gate = snapshot
                .gates
                .get_mut(&id)
                .ok_or(BudgetGateError::Unknown { id })?;
            if gate.status.is_terminal() {
                return Err(BudgetGateError::AlreadyResolved { id });
            }
            gate.status = match outcome {
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
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(move |snapshot| {
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

    fn get(&self, id: BudgetGateId) -> Result<Option<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(move |snapshot| Ok(snapshot.gates.get(&id).cloned()))
    }

    fn list_pending(&self) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            Ok(snapshot
                .gates
                .values()
                .filter(|gate| matches!(gate.status, BudgetGateStatus::Pending))
                .cloned()
                .collect())
        })
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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
        ResourceEstimate, VirtualPath,
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
                ResourceLimits {
                    max_usd: Some(dec!(1.00)),
                    max_concurrency_slots: Some(1),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let reservation = governor
            .reserve(
                scope.clone(),
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
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
            .reserve(
                scope,
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
            )
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
                ResourceLimits {
                    max_concurrency_slots: Some(1),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        governor_a
            .reserve(
                scope_a,
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
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
                ResourceLimits {
                    max_concurrency_slots: Some(1),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let reservation = governor_b
            .reserve(
                scope_b,
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
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
        let store = FilesystemBudgetGateStore::new(scoped, gate_scope("tenant-fs", "alice"));
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate.clone()).unwrap();
        let reloaded = store.get(id).unwrap().unwrap();
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
            let store = FilesystemBudgetGateStore::new(scoped, scope.clone());
            store.open(gate).unwrap();
        }
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped, scope);
        let reloaded = store.get(id).unwrap().unwrap();
        assert_eq!(reloaded.id, id);
        assert!(matches!(reloaded.status, BudgetGateStatus::Pending));
        let pending = store.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
    }

    #[test]
    fn resolve_updates_gate_status_after_reload() {
        let backend = Arc::new(InMemoryBackend::new());
        let scope = gate_scope("tenant-fs", "alice");
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped, scope.clone());
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate).unwrap();
        let resolved = store
            .resolve(
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
        let store2 = FilesystemBudgetGateStore::new(scoped2, scope);
        let reloaded = store2.get(id).unwrap().unwrap();
        assert!(matches!(
            reloaded.status,
            BudgetGateStatus::Cancelled { .. }
        ));
        assert!(store2.list_pending().unwrap().is_empty());
    }

    #[test]
    fn expire_pending_older_than_persists_terminal_state() {
        let backend = Arc::new(InMemoryBackend::new());
        let scope = gate_scope("tenant-fs", "alice");
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped, scope.clone());
        let mut gate = sample_gate();
        gate.expires_at = Utc::now() - chrono::Duration::hours(1);
        let id = gate.id;
        store.open(gate).unwrap();
        let expired = store.expire_pending_older_than(Utc::now()).unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, id);

        let scoped2 = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store2 = FilesystemBudgetGateStore::new(scoped2, scope);
        let reloaded = store2.get(id).unwrap().unwrap();
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
        let store = FilesystemBudgetGateStore::new(scoped, scope)
            .with_terminal_retention(Some(chrono::Duration::days(7)));

        let stale = sample_gate();
        let stale_id = stale.id;
        store.open(stale).unwrap();
        let old_at = Utc::now() - chrono::Duration::days(30);
        store
            .resolve(
                stale_id,
                BudgetGateOutcome::Cancel {
                    by: UserId::new("alice").unwrap(),
                },
                old_at,
            )
            .unwrap();

        let fresh = sample_gate();
        let fresh_id = fresh.id;
        store.open(fresh).unwrap();

        assert!(
            store.get(stale_id).unwrap().is_none(),
            "terminal gate older than the retention window must be pruned"
        );
        assert!(
            store.get(fresh_id).unwrap().is_some(),
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

        let scoped_a = scoped_resources_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store_a = FilesystemBudgetGateStore::new(scoped_a, gate_scope("tenant-a", "alice"));
        let gate_a = sample_gate();
        let id_a = gate_a.id;
        store_a.open(gate_a).unwrap();

        let scoped_b = scoped_resources_fs(Arc::clone(&backend), "tenant-b", "bob");
        let store_b = FilesystemBudgetGateStore::new(scoped_b, gate_scope("tenant-b", "bob"));
        let gate_b = sample_gate();
        let id_b = gate_b.id;
        store_b.open(gate_b).unwrap();

        let pending_a = store_a.list_pending().unwrap();
        assert_eq!(pending_a.len(), 1, "store_a must see only its own gate");
        assert_eq!(pending_a[0].id, id_a);

        let pending_b = store_b.list_pending().unwrap();
        assert_eq!(pending_b.len(), 1, "store_b must see only its own gate");
        assert_eq!(pending_b[0].id, id_b);

        assert!(
            store_a.get(id_b).unwrap().is_none(),
            "store_a must NOT see gates opened in tenant-b's scope"
        );
        assert!(
            store_b.get(id_a).unwrap().is_none(),
            "store_b must NOT see gates opened in tenant-a's scope"
        );
    }
}
