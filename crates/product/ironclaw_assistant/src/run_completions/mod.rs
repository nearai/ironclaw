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
pub mod observer;
pub mod operations;
pub mod push;
pub mod records;
pub mod store;
pub mod stream;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use store::RunCompletionOwner;

/// Tracing target shared by every module of the run-completion tree.
pub(super) const TRACE_TARGET: &str = "ironclaw::reborn::run_completions";

/// Bounded scan when joining a thread's unread count onto its notice events
/// and push copy; the wire field is capped at two digits anyway.
pub(super) const UNREAD_COUNT_SCAN_LIMIT: usize = 99;

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
    active_owners: Mutex<HashSet<RunCompletionOwner>>,
    /// Stale-grant regressions (§5.4 observability): grants rejected as
    /// `stale_state`/`effect_failed` plus grants that expired unacknowledged.
    /// Operator-visible through logs/metrics only; carries no content.
    stale_grants: std::sync::atomic::AtomicU64,
    /// The durable notification Inbox. Read bridge only: evidence that
    /// settles a run-completion notice also marks the run's `run_completed`
    /// Inbox row read, so the bell list (inbox-owned for scheduled runs) and
    /// the live notice surfaces never disagree about read state. Always
    /// wired; minimal test assemblies pass the no-op store.
    inbox: Arc<dyn ironclaw_notifications::NotificationInboxStorePort>,
}

impl RunCompletionSurfaceServices {
    pub fn new(
        notices: Arc<dyn store::RunCompletionNotices>,
        hub: Arc<stream::RunCompletionStreamHub>,
        inbox: Arc<dyn ironclaw_notifications::NotificationInboxStorePort>,
    ) -> Self {
        Self {
            notices,
            hub,
            wake: Arc::new(tokio::sync::Notify::new()),
            active_owners: Mutex::new(HashSet::new()),
            stale_grants: std::sync::atomic::AtomicU64::new(0),
            inbox,
        }
    }

    /// Best-effort read bridge: read evidence for one completed run also
    /// settles the run's `run_completed` Inbox row (present only for
    /// scheduled-trigger runs — the Inbox excludes foreground runs, so a
    /// missing row is the ordinary case, not a failure).
    pub(crate) async fn settle_inbox_row(&self, owner: &store::RunCompletionOwner, run_id: &str) {
        let run_uuid = match uuid::Uuid::parse_str(run_id) {
            Ok(run_uuid) => run_uuid,
            Err(error) => {
                // silent-ok: a durable notice whose run id is not a UUID has
                // no Inbox row to bridge; logged so the malformed row is
                // traceable rather than silently skipped.
                tracing::debug!(
                    target: TRACE_TARGET,
                    %error,
                    "inbox read bridge skipped: notice run id is not a uuid",
                );
                return;
            }
        };
        let notification_id = match crate::run_outcome_observer::outcome_notification_id(
            ironclaw_host_api::turn::TurnRunId::from_uuid(run_uuid),
            ironclaw_notifications::NotificationKind::RunCompleted,
        ) {
            Ok(notification_id) => notification_id,
            Err(_) => return,
        };
        if let Err(error) = self
            .inbox
            .mark_read(ironclaw_notifications::NotificationMutationRequest {
                recipient: ironclaw_notifications::NotificationRecipient {
                    tenant_id: owner.tenant_id.clone(),
                    user_id: owner.user_id.clone(),
                },
                notification_id,
                occurred_at: chrono::Utc::now(),
            })
            .await
        {
            // Foreground runs have no Inbox row; every failure here is
            // best-effort bookkeeping, never notice-settlement authority.
            tracing::debug!(
                target: TRACE_TARGET,
                error = %error,
                "inbox read bridge skipped",
            );
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
        owners.insert(owner.clone());
        drop(owners);
        self.wake.notify_one();
    }

    pub(crate) fn tracked_owners(&self) -> Vec<RunCompletionOwner> {
        self.active_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub(crate) fn untrack_owner(&self, owner: &RunCompletionOwner) {
        let mut owners = self
            .active_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        owners.remove(owner);
    }
}
