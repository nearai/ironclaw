//! Run-completion notifications: durable notice records, the owner-scoped
//! store, completion ingest, the user-completion stream hub, arbitration
//! coordination, and the HTTP operation handlers (2026-08-13 design §5).
//!
//! Product-notification state owned by this crate, not push-transport state:
//! in-app notices exist even when Web Push is disabled. Composition
//! constructs the store over the `/run-notices` per-user mount and wires the
//! journal-commit observer adapter; the engine knows nothing about
//! notification routing or browser presence.

pub mod coordinator;
pub mod ingest;
pub mod operations;
pub mod push;
pub mod records;
pub mod store;
pub mod stream;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use store::RunCompletionOwner;

/// The bundle composition wires into `RebornServices::with_run_completions`:
/// the durable notice port, the per-owner stream hub, and the coordinator
/// wake channel. Product code never reaches around these to a backend.
pub struct RunCompletionSurfaceServices {
    pub notices: Arc<dyn store::RunCompletionNotices>,
    pub hub: Arc<stream::RunCompletionStreamHub>,
    /// Wakes the arbitration coordinator's due-work loop after intent,
    /// acknowledgement, read, and notice writes.
    pub wake: Arc<tokio::sync::Notify>,
    /// Owners with potentially due coordinator work. Bounded by active
    /// owners; settled owners fall out as their scans come back empty.
    active_owners: Mutex<HashSet<(String, String)>>,
    /// Stale-grant regressions (§5.4 observability): grants rejected as
    /// `stale_state`/`effect_failed` plus grants that expired unacknowledged.
    /// Operator-visible through logs/metrics only; carries no content.
    stale_grants: std::sync::atomic::AtomicU64,
}

impl RunCompletionSurfaceServices {
    pub fn new(
        notices: Arc<dyn store::RunCompletionNotices>,
        hub: Arc<stream::RunCompletionStreamHub>,
    ) -> Self {
        Self {
            notices,
            hub,
            wake: Arc::new(tokio::sync::Notify::new()),
            active_owners: Mutex::new(HashSet::new()),
            stale_grants: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Count one stale/expired/failed grant (§5.4 observability).
    pub fn record_stale_grant(&self) {
        self.stale_grants
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Total stale-grant regressions since boot.
    pub fn stale_grant_count(&self) -> u64 {
        self.stale_grants.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Mark an owner as having due work and wake the coordinator.
    pub fn wake_owner(&self, owner: &RunCompletionOwner) {
        let mut owners = self
            .active_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        owners.insert((
            owner.tenant_id.as_str().to_string(),
            owner.user_id.as_str().to_string(),
        ));
        drop(owners);
        self.wake.notify_one();
    }

    pub(crate) fn tracked_owners(&self) -> Vec<RunCompletionOwner> {
        let owners = self
            .active_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        owners
            .iter()
            .filter_map(|(tenant, user)| {
                Some(RunCompletionOwner {
                    tenant_id: ironclaw_host_api::ids::TenantId::new(tenant.clone()).ok()?,
                    user_id: ironclaw_host_api::ids::UserId::new(user.clone()).ok()?,
                })
            })
            .collect()
    }

    pub(crate) fn untrack_owner(&self, owner: &RunCompletionOwner) {
        let mut owners = self
            .active_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        owners.remove(&(
            owner.tenant_id.as_str().to_string(),
            owner.user_id.as_str().to_string(),
        ));
    }
}
