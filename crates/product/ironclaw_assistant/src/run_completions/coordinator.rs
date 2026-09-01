//! The durable notification coordinator (2026-08-13 design §5.4–§5.6).
//!
//! One bounded worker per process: at startup it performs one bounded
//! reconciliation over pending, expired-grant, and push-owned records, then
//! sleeps until the earliest `closes_at`/`expires_at` or an explicit wake
//! from a notice/intent/acknowledgement write. There is no steady interval
//! scan and no client polling.
//!
//! Decision order at a closed intent window (§6.1): a read notice settles;
//! the best-ranked intent wins one grant; with no eligible intent the
//! notice falls to the push facade (Phase 3) or settles `NoExternalTarget`.
//! Grants expire into exactly one re-arbitration before fallback.
//!
//! Logging is `debug!` only — background tasks never write `info!`/`warn!`
//! (REPL rule). No log field carries titles, payloads, endpoints, or
//! navigation.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ironclaw_product_contracts::run_completions::RunCompletionIntentKind;
use tokio::sync::{Notify, watch};

use super::RunCompletionSurfaceServices;
use super::ingest::ARBITRATION_WINDOW_MS;
use super::records::{
    CompletionDeliveryState, CompletionDeliveryStateKind, CompletionIntentRecord,
    CompletionSurface, RunCompletionNotice,
};
use super::store::{RunCompletionOwner, RunCompletionStoreError};

/// Presentation-grant acknowledgement timeout (§5.4).
pub const GRANT_ACK_TIMEOUT_MS: i64 = 2_000;

/// Bounded per-tick scan of each non-terminal state partition. Bounded by
/// the active workload, not history: terminal states are never scanned.
const DUE_SCAN_LIMIT: usize = 250;

/// Fallback poll when the due queue is empty. Purely a safety net for a
/// missed wake; ordinary operation is timer + wake driven.
const IDLE_FALLBACK: Duration = Duration::from_secs(60);

/// Push fallback decision port. Phase 1 wires the always-`NoTarget`
/// implementation; Phase 3 replaces it with the authorized web-app push
/// facade. Deciding here keeps the coordinator free of outbound vocabulary.
#[async_trait::async_trait]
pub trait CompletionPushFallback: Send + Sync {
    /// Attempt push ownership + delivery for one pending notice. Returns
    /// `true` when the notice was transitioned (PushOwned or settled) by
    /// this fallback; `false` when no push target exists (the coordinator
    /// settles `NoExternalTarget`).
    async fn attempt_push(
        &self,
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
    ) -> Result<bool, RunCompletionStoreError>;
}

/// Phase 1: external completion delivery is disabled by design (§14 Phase 1
/// step 6); every no-presenter notice settles in-app-unread.
pub struct NoPushFallback;

#[async_trait::async_trait]
impl CompletionPushFallback for NoPushFallback {
    async fn attempt_push(
        &self,
        _owner: &RunCompletionOwner,
        _notice: &RunCompletionNotice,
    ) -> Result<bool, RunCompletionStoreError> {
        Ok(false)
    }
}

/// Validation port for `local_os` intents (§7.8): the server checks the
/// caller's live web-app target selection and the browser instance's
/// host-owned enrollment; client claims cannot mint permission or a target.
/// Phase 1 rejects every `local_os` intent (OS presentation ships in
/// Phase 2 alongside enrollment correlation).
#[async_trait::async_trait]
pub trait LocalOsIntentPolicy: Send + Sync {
    async fn allows_local_os(&self, owner: &RunCompletionOwner, browser_instance_id: &str) -> bool;
}

pub struct DenyLocalOsIntents;

#[async_trait::async_trait]
impl LocalOsIntentPolicy for DenyLocalOsIntents {
    async fn allows_local_os(
        &self,
        _owner: &RunCompletionOwner,
        _browser_instance_id: &str,
    ) -> bool {
        false
    }
}

pub struct RunCompletionCoordinator {
    services: Arc<RunCompletionSurfaceServices>,
    push_fallback: Arc<dyn CompletionPushFallback>,
    local_os_policy: Arc<dyn LocalOsIntentPolicy>,
}

impl RunCompletionCoordinator {
    pub fn new(
        services: Arc<RunCompletionSurfaceServices>,
        push_fallback: Arc<dyn CompletionPushFallback>,
        local_os_policy: Arc<dyn LocalOsIntentPolicy>,
    ) -> Self {
        Self {
            services,
            push_fallback,
            local_os_policy,
        }
    }

    /// One coordinator pass over every tracked owner. Returns the earliest
    /// future deadline, for the timer. `now` is injected for deterministic
    /// tests.
    pub async fn tick_once(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut next_deadline: Option<DateTime<Utc>> = None;
        let mut observe = |candidate: DateTime<Utc>| {
            next_deadline = Some(match next_deadline {
                Some(current) if current <= candidate => current,
                _ => candidate,
            });
        };
        for owner in self.services.tracked_owners() {
            match self.tick_owner(&owner, now).await {
                Ok(Some(deadline)) => observe(deadline),
                Ok(None) => {
                    // No outstanding work: stop tracking until re-woken, and
                    // clear the durable due-registry entry (§5.4). A failed
                    // clear is harmless — one extra empty scan after the
                    // next boot.
                    self.services.untrack_owner(&owner);
                    if let Err(error) = self.services.notices.clear_owner_due(&owner).await {
                        tracing::debug!(
                            target: "ironclaw::reborn::run_completions",
                            error = %error,
                            "due-owner clear failed; boot reconciliation will rescan",
                        );
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        target: "ironclaw::reborn::run_completions",
                        error = %error,
                        "coordinator owner pass failed; will retry on next wake",
                    );
                    observe(now + ChronoDuration::milliseconds(ARBITRATION_WINDOW_MS));
                }
            }
        }
        next_deadline
    }

    async fn tick_owner(
        &self,
        owner: &RunCompletionOwner,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, RunCompletionStoreError> {
        let mut next: Option<DateTime<Utc>> = None;
        let mut observe = |candidate: DateTime<Utc>| {
            if candidate > now {
                next = Some(match next {
                    Some(current) if current <= candidate => current,
                    _ => candidate,
                });
            }
        };

        let mut saturated = false;
        let pending = self
            .services
            .notices
            .in_delivery_state(
                owner,
                CompletionDeliveryStateKind::PendingArbitration,
                DUE_SCAN_LIMIT,
            )
            .await?;
        saturated |= pending.len() >= DUE_SCAN_LIMIT;
        for notice in pending {
            let CompletionDeliveryState::PendingArbitration { closes_at, .. } = notice.delivery
            else {
                continue;
            };
            if notice.is_read() {
                continue;
            }
            if closes_at > now {
                observe(closes_at);
                continue;
            }
            self.arbitrate(owner, &notice, now).await?;
        }

        let granted = self
            .services
            .notices
            .in_delivery_state(owner, CompletionDeliveryStateKind::Granted, DUE_SCAN_LIMIT)
            .await?;
        saturated |= granted.len() >= DUE_SCAN_LIMIT;
        for notice in granted {
            let CompletionDeliveryState::Granted {
                grant_id,
                expires_at,
                grants_issued,
                ..
            } = &notice.delivery
            else {
                continue;
            };
            if *expires_at > now {
                observe(*expires_at);
                continue;
            }
            // Grant expiry: exactly one re-arbitration before fallback
            // (§5.4). A second expiry goes straight to fallback.
            if *grants_issued <= 1 {
                let closes_at = now + ChronoDuration::milliseconds(ARBITRATION_WINDOW_MS);
                match self
                    .services
                    .notices
                    .regress_expired_grant(owner, &notice.notice_id, grant_id, closes_at)
                    .await
                {
                    Ok(_) => {
                        self.services.record_stale_grant();
                        observe(closes_at)
                    }
                    Err(RunCompletionStoreError::Conflict { .. }) => {}
                    Err(error) => return Err(error),
                }
            } else {
                // §5.4: the one re-arbitration is spent. Regress the expired
                // grant to pending first — push claim and no-target
                // settlement are pending-only CAS transitions (§5.3) — then
                // fall back immediately from the regressed record.
                let regressed = match self
                    .services
                    .notices
                    .regress_expired_grant(owner, &notice.notice_id, grant_id, now)
                    .await
                {
                    Ok(regressed) => regressed,
                    Err(RunCompletionStoreError::Conflict { .. }) => continue,
                    Err(error) => return Err(error),
                };
                self.services.record_stale_grant();
                match self.fallback(owner, &regressed, now).await {
                    Ok(()) => {}
                    Err(RunCompletionStoreError::Conflict { .. }) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        if saturated {
            // A full page means the scan may not have seen every due record.
            // Report immediate residual work so the owner stays tracked (and
            // its durable due entry stays) and the next tick drains more —
            // otherwise a >DUE_SCAN_LIMIT backlog whose first page settled
            // without future deadlines would be untracked with the surplus
            // stranded until an unrelated wake.
            return Ok(Some(now));
        }
        Ok(next)
    }

    /// Rank the collected intents and issue at most one grant (§5.6).
    async fn arbitrate(
        &self,
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
        now: DateTime<Utc>,
    ) -> Result<(), RunCompletionStoreError> {
        let mut best: Option<&CompletionIntentRecord> = None;
        for intent in &notice.intents {
            if intent.intent == RunCompletionIntentKind::Unavailable {
                continue;
            }
            if intent.intent == RunCompletionIntentKind::LocalOs
                && !self
                    .local_os_policy
                    .allows_local_os(owner, &intent.browser_instance_id)
                    .await
            {
                continue;
            }
            best = Some(match best {
                None => intent,
                Some(current) => {
                    if intent_rank(intent.intent) < intent_rank(current.intent)
                        || (intent_rank(intent.intent) == intent_rank(current.intent)
                            && ranks_above(intent, current))
                    {
                        intent
                    } else {
                        current
                    }
                }
            });
        }
        let Some(winner) = best else {
            return match self.fallback(owner, notice, now).await {
                Ok(()) => Ok(()),
                Err(RunCompletionStoreError::Conflict { .. }) => Ok(()),
                Err(error) => Err(error),
            };
        };
        let surface = match winner.intent {
            RunCompletionIntentKind::WatchingThread => CompletionSurface::NoSurfaceWatchingThread,
            RunCompletionIntentKind::InApp => CompletionSurface::InApp,
            RunCompletionIntentKind::LocalOs => CompletionSurface::LocalOs,
            // ReplyObserved settles at intent time; Unavailable was skipped.
            RunCompletionIntentKind::ReplyObserved | RunCompletionIntentKind::Unavailable => {
                return Ok(());
            }
        };
        let grant_id = format!("rcg-{}", uuid::Uuid::new_v4().simple());
        let expires_at = now + ChronoDuration::milliseconds(GRANT_ACK_TIMEOUT_MS);
        match self
            .services
            .notices
            .issue_grant(
                owner,
                &notice.notice_id,
                super::store::NewGrant {
                    grant_id,
                    browser_instance_id: winner.browser_instance_id.clone(),
                    surface,
                    state_revision: winner.state_revision,
                    expires_at,
                },
            )
            .await
        {
            Ok(granted) => {
                self.services.hub.publish_grant(owner, &granted);
                Ok(())
            }
            // Read/raced elsewhere between scan and CAS: nothing to do.
            Err(RunCompletionStoreError::Conflict { .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// No eligible presenter: push fallback (Phase 3) or in-app-unread
    /// settlement.
    async fn fallback(
        &self,
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
        now: DateTime<Utc>,
    ) -> Result<(), RunCompletionStoreError> {
        if self.push_fallback.attempt_push(owner, notice).await? {
            return Ok(());
        }
        self.services
            .notices
            .settle_no_target(owner, &notice.notice_id, now)
            .await
            .map(|_| ())
    }
}

/// Priority rank, lowest number wins (§5.6).
fn intent_rank(intent: RunCompletionIntentKind) -> u8 {
    match intent {
        RunCompletionIntentKind::ReplyObserved => 0,
        RunCompletionIntentKind::WatchingThread => 1,
        RunCompletionIntentKind::InApp => 2,
        RunCompletionIntentKind::LocalOs => 3,
        RunCompletionIntentKind::Unavailable => 4,
    }
}

/// Equal-rank tie-break: greatest state revision, then newest focus epoch,
/// then lexicographically smallest opaque browser/tab IDs (§5.6).
fn ranks_above(candidate: &CompletionIntentRecord, current: &CompletionIntentRecord) -> bool {
    (
        std::cmp::Reverse(candidate.state_revision),
        std::cmp::Reverse(candidate.focus_epoch),
        &candidate.browser_instance_id,
        &candidate.tab_id,
    ) < (
        std::cmp::Reverse(current.state_revision),
        std::cmp::Reverse(current.focus_epoch),
        &current.browser_instance_id,
        &current.tab_id,
    )
}

/// Worker handle in the trigger-poller/keepalive shape: cancel, then join
/// with a bounded timeout.
pub struct RunCompletionCoordinatorHandle {
    cancel: watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
}

pub const COORDINATOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

impl RunCompletionCoordinatorHandle {
    pub async fn shutdown(self, timeout: Duration) {
        let _ = self.cancel.send(true);
        let handle = self.handle;
        if tokio::time::timeout(timeout, handle).await.is_err() {
            tracing::debug!(
                target: "ironclaw::reborn::run_completions",
                "coordinator did not stop within its shutdown budget",
            );
        }
    }
}

/// Spawn the coordinator loop: boot reconciliation, then timer/wake-driven
/// due work.
///
/// `boot_scope_owner` names the tenant whose durable due-owner registry the
/// one bounded startup reconciliation reads (§5.4); `None` skips the boot
/// pass (single-purpose test workers).
pub fn spawn_run_completion_coordinator(
    coordinator: Arc<RunCompletionCoordinator>,
    wake: Arc<Notify>,
    boot_scope_owner: Option<RunCompletionOwner>,
) -> RunCompletionCoordinatorHandle {
    let (cancel, mut cancelled) = watch::channel(false);
    let handle = tokio::spawn(async move {
        if let Some(scope_owner) = boot_scope_owner {
            // One bounded reconciliation over the owners the durable
            // registry says may hold pending, expired-grant, or push-owned
            // records (§5.4). Failure degrades to wake-driven operation;
            // notices written after boot re-mark their owners.
            match coordinator.services.notices.due_owners(&scope_owner).await {
                Ok(owners) => {
                    for owner in owners {
                        coordinator.services.wake_owner(&owner);
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        target: "ironclaw::reborn::run_completions",
                        error = %error,
                        "boot due-owner reconciliation failed; continuing wake-driven",
                    );
                }
            }
        }
        loop {
            let next_deadline = coordinator.tick_once(Utc::now()).await;
            let sleep_for = match next_deadline {
                Some(deadline) => (deadline - Utc::now())
                    .to_std()
                    .unwrap_or(Duration::from_millis(0)),
                None => IDLE_FALLBACK,
            };
            tokio::select! {
                _ = cancelled.changed() => return,
                _ = wake.notified() => {}
                _ = tokio::time::sleep(sleep_for) => {}
            }
        }
    });
    RunCompletionCoordinatorHandle { cancel, handle }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_completions::RunCompletionSurfaceServices;
    use crate::run_completions::records::CompletionReadEvidence;
    use crate::run_completions::store::{
        NewRunCompletionNotice, NoticeCreateOutcome, RunCompletionNoticeStore, RunCompletionNotices,
    };
    use crate::run_completions::stream::RunCompletionStreamHub;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::ids::{TenantId, UserId};
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};
    use ironclaw_host_api::resource::ResourceScope;
    use ironclaw_product_contracts::run_completions::RunCompletionIntentKind;

    fn services() -> Arc<RunCompletionSurfaceServices> {
        let store = Arc::new(RunCompletionNoticeStore::new(Arc::new(
            ScopedFilesystem::new(Arc::new(InMemoryBackend::new()), |scope: &ResourceScope| {
                MountView::new(vec![
                    MountGrant::new(
                        MountAlias::new(crate::run_completions::store::RUN_NOTICES_MOUNT_ALIAS)?,
                        VirtualPath::new(format!(
                            "/tenants/{}/users/{}/run-notices",
                            scope.tenant_id, scope.user_id
                        ))?,
                        MountPermissions::read_write_list_delete(),
                    ),
                    MountGrant::new(
                        MountAlias::new("/tenant-shared")?,
                        VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id))?,
                        MountPermissions::read_write(),
                    ),
                ])
            }),
        ))) as Arc<dyn RunCompletionNotices>;
        let hub = Arc::new(RunCompletionStreamHub::new(Arc::clone(&store)));
        Arc::new(RunCompletionSurfaceServices::new(store, hub))
    }

    fn owner() -> RunCompletionOwner {
        RunCompletionOwner {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("user-alpha").expect("user"),
        }
    }

    fn coordinator(services: &Arc<RunCompletionSurfaceServices>) -> RunCompletionCoordinator {
        RunCompletionCoordinator::new(
            Arc::clone(services),
            Arc::new(NoPushFallback),
            Arc::new(DenyLocalOsIntents),
        )
    }

    struct AlwaysPush;

    #[async_trait::async_trait]
    impl CompletionPushFallback for AlwaysPush {
        async fn attempt_push(
            &self,
            owner: &RunCompletionOwner,
            notice: &RunCompletionNotice,
        ) -> Result<bool, RunCompletionStoreError> {
            // Mirrors the production facade's transition without delivering.
            self_claim(owner, notice).await
        }
    }

    async fn self_claim(
        _owner: &RunCompletionOwner,
        _notice: &RunCompletionNotice,
    ) -> Result<bool, RunCompletionStoreError> {
        Ok(true)
    }

    struct AllowLocalOs;

    #[async_trait::async_trait]
    impl LocalOsIntentPolicy for AllowLocalOs {
        async fn allows_local_os(&self, _owner: &RunCompletionOwner, _id: &str) -> bool {
            true
        }
    }

    async fn seed_notice(
        services: &Arc<RunCompletionSurfaceServices>,
        suffix: &str,
        closes_at: DateTime<Utc>,
    ) -> RunCompletionNotice {
        // Mirror ingest's ordering contract (§5.4): the owner lands in the
        // durable due registry BEFORE the notice write.
        services
            .notices
            .mark_owner_due(&owner())
            .await
            .expect("mark due");
        let outcome = services
            .notices
            .create_notice(
                &owner(),
                NewRunCompletionNotice {
                    notice_id: format!("rcn-{suffix}"),
                    run_id: format!("11111111-2222-3333-4444-55555555555{}", suffix.len() % 10),
                    thread_id: format!("thread-{suffix}"),
                    agent_id: Some("agent-alpha".to_string()),
                    project_id: None,
                    thread_tag: format!("rct-{suffix}"),
                    terminal_projection_ref: format!("run-completion/rcn-{suffix}"),
                    completed_at: Utc::now(),
                    arbitration_closes_at: closes_at,
                },
            )
            .await
            .expect("create notice");
        services.wake_owner(&owner());
        match outcome {
            NoticeCreateOutcome::Created(notice) => notice,
            NoticeCreateOutcome::AlreadyRecorded(notice) => notice,
        }
    }

    fn intent(
        browser: &str,
        kind: RunCompletionIntentKind,
        revision: u64,
    ) -> CompletionIntentRecord {
        CompletionIntentRecord {
            browser_instance_id: browser.to_string(),
            tab_id: format!("tab-{browser}"),
            state_revision: revision,
            focus_epoch: 1,
            intent: kind,
            offered_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn best_ranked_intent_wins_one_grant_at_window_close() {
        let services = services();
        let now = Utc::now();
        let notice = seed_notice(&services, "grant", now - ChronoDuration::seconds(1)).await;
        for record in [
            intent("browser-b", RunCompletionIntentKind::LocalOs, 7),
            intent("browser-a", RunCompletionIntentKind::InApp, 5),
        ] {
            services
                .notices
                .record_intent(&owner(), &notice.notice_id, record)
                .await
                .expect("record intent");
        }
        let deadline = coordinator(&services).tick_once(now).await;
        let after = services
            .notices
            .get(&owner(), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        let CompletionDeliveryState::Granted {
            browser_instance_id,
            surface,
            expires_at,
            ..
        } = &after.delivery
        else {
            panic!("expected a grant, got {:?}", after.delivery);
        };
        // in_app outranks local_os (§5.6); local_os was also policy-denied.
        assert_eq!(browser_instance_id, "browser-a");
        assert_eq!(*surface, CompletionSurface::InApp);
        assert_eq!(
            *expires_at,
            now + ChronoDuration::milliseconds(GRANT_ACK_TIMEOUT_MS)
        );
        // The grant expiry becomes the next deadline the timer sleeps to.
        assert_eq!(deadline, Some(*expires_at));
    }

    #[tokio::test]
    async fn local_os_intent_needs_the_validation_policy() {
        let services = services();
        let now = Utc::now();
        let notice = seed_notice(&services, "localos", now - ChronoDuration::seconds(1)).await;
        services
            .notices
            .record_intent(
                &owner(),
                &notice.notice_id,
                intent("browser-os", RunCompletionIntentKind::LocalOs, 3),
            )
            .await
            .expect("record intent");
        // Denied policy: the only intent is ineligible, no push target ->
        // NoExternalTarget settle.
        coordinator(&services).tick_once(now).await;
        let denied = services
            .notices
            .get(&owner(), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(matches!(
            denied.delivery,
            CompletionDeliveryState::NoExternalTarget { .. }
        ));

        // Allowed policy on a fresh notice: the same intent wins a LocalOs
        // grant.
        let second = seed_notice(&services, "localos2", now - ChronoDuration::seconds(1)).await;
        services
            .notices
            .record_intent(
                &owner(),
                &second.notice_id,
                intent("browser-os", RunCompletionIntentKind::LocalOs, 3),
            )
            .await
            .expect("record intent");
        services.wake_owner(&owner());
        let permissive = RunCompletionCoordinator::new(
            Arc::clone(&services),
            Arc::new(NoPushFallback),
            Arc::new(AllowLocalOs),
        );
        permissive.tick_once(now).await;
        let granted = services
            .notices
            .get(&owner(), &second.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(matches!(
            granted.delivery,
            CompletionDeliveryState::Granted {
                surface: CompletionSurface::LocalOs,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn grant_expiry_re_arbitrates_once_then_falls_back() {
        let services = services();
        let start = Utc::now();
        let notice = seed_notice(&services, "expiry", start - ChronoDuration::seconds(2)).await;
        services
            .notices
            .record_intent(
                &owner(),
                &notice.notice_id,
                intent("browser-a", RunCompletionIntentKind::InApp, 5),
            )
            .await
            .expect("record intent");
        let coordinator = coordinator(&services);

        // Window closed -> grant issued.
        coordinator.tick_once(start).await;
        // Grant expires unacknowledged -> exactly one re-arbitration.
        let after_expiry = start + ChronoDuration::milliseconds(GRANT_ACK_TIMEOUT_MS + 100);
        coordinator.tick_once(after_expiry).await;
        let regressed = services
            .notices
            .get(&owner(), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(
            matches!(
                regressed.delivery,
                CompletionDeliveryState::PendingArbitration { .. }
            ),
            "first expiry re-arbitrates: {:?}",
            regressed.delivery
        );
        assert_eq!(services.stale_grant_count(), 1);

        // Second window closes with the stored intent still best -> second
        // grant; its expiry goes straight to fallback (NoPushFallback ->
        // in-app-unread settle).
        let second_close = after_expiry + ChronoDuration::milliseconds(ARBITRATION_WINDOW_MS + 50);
        coordinator.tick_once(second_close).await;
        let regranted = services
            .notices
            .get(&owner(), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(matches!(
            regranted.delivery,
            CompletionDeliveryState::Granted { .. }
        ));
        let final_tick = second_close + ChronoDuration::milliseconds(GRANT_ACK_TIMEOUT_MS + 100);
        coordinator.tick_once(final_tick).await;
        let settled = services
            .notices
            .get(&owner(), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        assert!(
            matches!(
                settled.delivery,
                CompletionDeliveryState::NoExternalTarget { .. }
            ),
            "second expiry falls back: {:?}",
            settled.delivery
        );
    }

    #[tokio::test]
    async fn read_notices_settle_without_grants_and_owner_untracks() {
        let services = services();
        let now = Utc::now();
        let notice = seed_notice(&services, "read", now - ChronoDuration::seconds(1)).await;
        services
            .notices
            .mark_read(
                &owner(),
                &notice.notice_id,
                CompletionReadEvidence::ReplyRendered {
                    browser_instance_id: "browser-a".to_string(),
                },
                now,
            )
            .await
            .expect("mark read");
        let deadline = coordinator(&services).tick_once(now).await;
        assert_eq!(deadline, None, "no outstanding work");
        assert!(
            services.tracked_owners().is_empty(),
            "settled owners fall out of tracking"
        );
        // The durable due registry cleared too (§5.4): a rebooted
        // coordinator would find nothing to reconcile.
        let due = services
            .notices
            .due_owners(&owner())
            .await
            .expect("due owners");
        assert!(due.is_empty(), "due registry cleared: {due:?}");
    }

    #[tokio::test]
    async fn boot_reconciliation_recovers_pending_work_from_the_due_registry() {
        let services = services();
        let now = Utc::now();
        seed_notice(&services, "boot", now - ChronoDuration::seconds(1)).await;
        // Simulate restart: in-memory tracking is gone, only durable state
        // remains.
        services.untrack_owner(&owner());
        assert!(services.tracked_owners().is_empty());
        let due = services
            .notices
            .due_owners(&owner())
            .await
            .expect("due owners");
        assert_eq!(due.len(), 1, "ingest marked the owner due before the write");
        // The boot pass (spawn_run_completion_coordinator) wakes each due
        // owner; equivalently, wake + tick drains the recovered work.
        for recovered in due {
            services.wake_owner(&recovered);
        }
        coordinator(&services).tick_once(now).await;
        let settled = services
            .notices
            .unread_snapshot(&owner())
            .await
            .expect("snapshot");
        assert_eq!(settled.len(), 1, "notice survives, now settled unread");
        assert!(matches!(
            settled[0].delivery,
            CompletionDeliveryState::NoExternalTarget { .. }
        ));
    }

    #[tokio::test]
    async fn push_fallback_owns_the_no_presenter_path() {
        let services = services();
        let now = Utc::now();
        let notice = seed_notice(&services, "push", now - ChronoDuration::seconds(1)).await;
        let coordinator = RunCompletionCoordinator::new(
            Arc::clone(&services),
            Arc::new(AlwaysPush),
            Arc::new(DenyLocalOsIntents),
        );
        coordinator.tick_once(now).await;
        let after = services
            .notices
            .get(&owner(), &notice.notice_id)
            .await
            .expect("get")
            .expect("exists");
        // AlwaysPush reported the transition handled; the coordinator must
        // NOT also settle NoExternalTarget over it.
        assert!(
            matches!(
                after.delivery,
                CompletionDeliveryState::PendingArbitration { .. }
            ),
            "fallback owned the transition (fake leaves state untouched): {:?}",
            after.delivery
        );
    }
}
