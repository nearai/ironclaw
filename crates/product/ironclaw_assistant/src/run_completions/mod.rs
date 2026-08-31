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
    /// The durable notification Inbox, when composition wires it. Read
    /// bridge only: evidence that settles a run-completion notice also
    /// marks the run's `run_completed` Inbox row read, so the bell list
    /// (inbox-owned for scheduled runs) and the live notice surfaces never
    /// disagree about read state. Absent in minimal test assemblies.
    inbox: Option<Arc<dyn ironclaw_notifications::NotificationInboxStorePort>>,
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
            inbox: None,
        }
    }

    /// Wire the durable notification Inbox for the read bridge.
    pub fn with_inbox(
        mut self,
        inbox: Arc<dyn ironclaw_notifications::NotificationInboxStorePort>,
    ) -> Self {
        self.inbox = Some(inbox);
        self
    }

    /// Best-effort read bridge: read evidence for one completed run also
    /// settles the run's `run_completed` Inbox row (present only for
    /// scheduled-trigger runs — the Inbox excludes foreground runs, so a
    /// missing row is the ordinary case, not a failure).
    pub(crate) async fn settle_inbox_row(&self, owner: &store::RunCompletionOwner, run_id: &str) {
        let Some(inbox) = self.inbox.as_ref() else {
            return;
        };
        let Ok(run_uuid) = uuid::Uuid::parse_str(run_id) else {
            return;
        };
        let notification_id = match crate::run_outcome_observer::outcome_notification_id(
            ironclaw_host_api::turn::TurnRunId::from_uuid(run_uuid),
            ironclaw_notifications::NotificationKind::RunCompleted,
        ) {
            Ok(notification_id) => notification_id,
            Err(_) => return,
        };
        if let Err(error) = inbox
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
                target: "ironclaw::reborn::run_completions",
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
                let tenant_id = match ironclaw_host_api::ids::TenantId::new(tenant.clone()) {
                    Ok(tenant_id) => tenant_id,
                    Err(error) => {
                        // Unreachable by construction: every entry was
                        // inserted FROM typed ids in `wake_owner`. Logged
                        // rather than silently skipped so a future
                        // constructor change cannot hide dropped owners.
                        tracing::debug!(
                            target: "ironclaw::reborn::run_completions",
                            %error,
                            "tracked owner tenant id failed re-validation; skipping",
                        );
                        return None;
                    }
                };
                let user_id = match ironclaw_host_api::ids::UserId::new(user.clone()) {
                    Ok(user_id) => user_id,
                    Err(error) => {
                        tracing::debug!(
                            target: "ironclaw::reborn::run_completions",
                            %error,
                            "tracked owner user id failed re-validation; skipping",
                        );
                        return None;
                    }
                };
                Some(RunCompletionOwner { tenant_id, user_id })
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
