//! Event-driven delivery for replies to live external-channel turns.
//!
//! A turn lifecycle event is the durable fact that makes a notification
//! actionable. Delivery never waits for a run or owns an auth/approval wait:
//! each committed transition independently re-opens the run's sealed reply
//! target, revalidates current membership/pairing, and sends through the one
//! generic outbound coordinator.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

use crate::{
    AuthPromptChallengeKind, OutboundPart, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload,
};
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    OutboundError, OutboundPolicyService, OutboundStateStore, PrepareCommunicationDeliveryRequest,
    ProjectionUpdateRef, RunDeliveryCleanupRecord, RunDeliveryCleanupRequest,
    RunFinalReplyDestination, RunFinalReplyHandoffRecord, RunFinalReplyTargetRequest,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, ThreadScope};
use ironclaw_turns::{
    EventCursor, GetRunStateRequest, ReplyTargetBindingRef, RunOriginAdapter, TurnActor, TurnError,
    TurnEventKind, TurnEventProjectionSource, TurnEventSink, TurnLifecycleEvent, TurnOriginKind,
    TurnRunId, TurnRunState, TurnStateStore, TurnStatus, TurnSurfaceType,
};
use tokio::sync::{Notify, Semaphore};

use super::gate_routes::record_gate_route_if_needed;
use super::prompts;
use super::{
    BlockedAuthPromptRequest, CurrentDeliveryTargetResolver, DeliveredChannelMessage,
    RunDeliveryError, RunDeliveryServices, cancel_auth_blocked_run,
    delivered_messages_from_outcome,
};
use crate::delivery_coordinator::{
    CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest, DeliveryIntent,
};
use crate::{
    ProductWorkflowError, ResolveStoredProductReplyTargetRequest, ResolvedStoredProductReplyTarget,
    StoredProductReplyTargetAccess,
};

mod reply_target_authority;
pub(crate) use reply_target_authority::AllowNoProjectionAccess;
use reply_target_authority::{LiveReplyTargetAuthority, StoredReplyTargetAuthority};

/// A dynamic fan-out installed once on the canonical turn lifecycle bus.
/// Channel graphs register weak handlers, so removing/replacing a graph also
/// revokes its ability to receive future lifecycle events without a second
/// unregister protocol.
const MAX_CONCURRENT_RUN_DELIVERIES: usize = 16;
const DURABLE_REPLAY_PAGE_SIZE: usize = 256;

#[derive(Default)]
struct PendingRunDeliveries {
    /// Only the newest committed fact for a run is useful. The consumer reads
    /// canonical run state before acting, so replacing an older queued event
    /// cannot hide a state transition.
    latest: HashMap<TurnRunId, TurnLifecycleEvent>,
    active: HashSet<TurnRunId>,
}

struct RunDeliveryEventRouterInner {
    handlers: RwLock<HashMap<String, Weak<RunDeliveryEventHandler>>>,
    triggered_handlers:
        RwLock<HashMap<TurnRunId, Arc<super::triggered::TriggeredRunDeliveryEventHandler>>>,
    pending: Mutex<PendingRunDeliveries>,
    idle: Notify,
    permits: Arc<Semaphore>,
    durable_replay: Option<DurableRunDeliveryReplay>,
}

struct DurableRunDeliveryReplay {
    source: Arc<dyn TurnEventProjectionSource>,
    /// Host-owned run-state lookup used only to classify a completed run's
    /// origin/destination at materialization time. Materializing a handoff for
    /// a run no channel handler will ever own (a WebApp-destined WebUI answer,
    /// a scheduled trigger the volatile driver owns, or a context-less run)
    /// leaks a permanent pending row that every later drain re-scans.
    run_state: Arc<dyn TurnStateStore>,
    outbound_state: Arc<dyn OutboundStateStore>,
    active: AtomicBool,
    dirty: AtomicBool,
    idle: Notify,
}

#[derive(Debug, thiserror::Error)]
enum DurableRunDeliveryReplayError {
    #[error("turn lifecycle replay failed: {0}")]
    Turn(#[from] TurnError),
    #[error("final-reply handoff store failed: {0}")]
    Outbound(#[from] OutboundError),
    #[error("turn lifecycle replay requires rebase")]
    RebaseRequired,
    #[error("turn lifecycle replay returned a malformed cursor page")]
    MalformedPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryEventOutcome {
    /// This handler does not own the event's sealed route.
    Irrelevant,
    /// Authority or message state may become available after another commit.
    Deferred,
    /// No retry is needed: delivery reached a durable terminal outcome, or
    /// current authority definitively rejected the sealed route.
    Settled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliveryEventDisposition {
    outcome: DeliveryEventOutcome,
    required_source_adapter: Option<String>,
    source_cleanup_settled: bool,
}

impl DeliveryEventDisposition {
    fn without_source(outcome: DeliveryEventOutcome) -> Self {
        Self {
            outcome,
            required_source_adapter: None,
            source_cleanup_settled: false,
        }
    }

    fn for_source(
        outcome: DeliveryEventOutcome,
        required_source_adapter: Option<String>,
        source_cleanup_settled: bool,
    ) -> Self {
        Self {
            outcome,
            required_source_adapter,
            source_cleanup_settled,
        }
    }
}

/// Enqueue-only lifecycle fan-out for external delivery.
///
/// The canonical lifecycle publisher must never wait on a provider. Runs are
/// serialized independently so a slow auth prompt cannot race and arrive
/// after that same run's final reply, while unrelated runs may still deliver
/// concurrently up to a bounded limit.
#[derive(Clone)]
pub struct RunDeliveryEventRouter {
    inner: Arc<RunDeliveryEventRouterInner>,
}

impl RunDeliveryEventRouter {
    /// Build the production router with crash-safe replay of the authoritative
    /// lifecycle log. Durability is mandatory in the production constructor;
    /// channel/provider policy remains in registered handlers and the outbound
    /// coordinator.
    pub fn new(
        source: Arc<dyn TurnEventProjectionSource>,
        run_state: Arc<dyn TurnStateStore>,
        outbound_state: Arc<dyn OutboundStateStore>,
    ) -> Self {
        let router = Self::build(Some(DurableRunDeliveryReplay {
            source,
            run_state,
            outbound_state,
            active: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            idle: Notify::new(),
        }));
        // Startup catch-up is not gated on a future lifecycle wake. It may
        // materialize rows before channel handlers register; each registration
        // wakes the same drain so those rows remain recoverable.
        router.wake_durable_replay();
        router
    }

    /// Construct an in-memory enqueue-only router for tests that exercise
    /// event fan-out independently of crash recovery.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn new_ephemeral_for_test() -> Self {
        Self::build(None)
    }

    fn build(durable_replay: Option<DurableRunDeliveryReplay>) -> Self {
        Self {
            inner: Arc::new(RunDeliveryEventRouterInner {
                handlers: RwLock::new(HashMap::new()),
                triggered_handlers: RwLock::new(HashMap::new()),
                pending: Mutex::new(PendingRunDeliveries::default()),
                idle: Notify::new(),
                permits: Arc::new(Semaphore::new(MAX_CONCURRENT_RUN_DELIVERIES)),
                durable_replay,
            }),
        }
    }

    pub fn register(&self, adapter_id: impl Into<String>, handler: &Arc<RunDeliveryEventHandler>) {
        self.inner
            .handlers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(adapter_id.into(), Arc::downgrade(handler));
        self.wake_durable_replay();
    }

    pub(crate) fn register_triggered(
        &self,
        run_id: TurnRunId,
        handler: Arc<super::triggered::TriggeredRunDeliveryEventHandler>,
    ) {
        self.inner
            .triggered_handlers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(run_id, handler);
    }

    pub(crate) fn remove_triggered(&self, run_id: TurnRunId) {
        self.inner
            .triggered_handlers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&run_id);
    }

    fn triggered_handler(
        &self,
        run_id: TurnRunId,
    ) -> Option<Arc<super::triggered::TriggeredRunDeliveryEventHandler>> {
        self.inner
            .triggered_handlers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&run_id)
            .cloned()
    }

    fn live_handlers(&self) -> Vec<Arc<RunDeliveryEventHandler>> {
        let mut handlers = self
            .inner
            .handlers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        handlers.retain(|_, handler| handler.strong_count() > 0);
        handlers.values().filter_map(Weak::upgrade).collect()
    }

    fn enqueue(&self, event: TurnLifecycleEvent) {
        let run_id = event.run_id;
        let should_start = {
            let mut pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.latest.insert(run_id, event);
            pending.active.insert(run_id)
        };
        if should_start {
            let router = self.clone();
            tokio::spawn(async move {
                router.drain_run(run_id).await;
            });
        }
    }

    fn wake_durable_replay(&self) {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return;
        };
        replay.dirty.store(true, Ordering::Release);
        if replay
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let router = self.clone();
            tokio::spawn(async move {
                router.drain_durable_replay().await;
            });
        }
    }

    async fn drain_durable_replay(&self) {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return;
        };
        loop {
            replay.dirty.store(false, Ordering::Release);
            if let Err(error) = self.replay_durable_once().await {
                tracing::error!(
                    target = "ironclaw::reborn::run_delivery",
                    %error,
                    "durable completed-run delivery replay stopped without advancing past the failing fact"
                );
            }
            if replay.dirty.swap(false, Ordering::AcqRel) {
                continue;
            }

            replay.active.store(false, Ordering::Release);
            // Close the wake-vs-idle race without holding a process lock over
            // backend I/O: a wake after the first dirty check either takes
            // ownership here or already started another worker.
            if replay.dirty.load(Ordering::Acquire)
                && replay
                    .active
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                continue;
            }
            replay.idle.notify_waiters();
            return;
        }
    }

    async fn replay_durable_once(&self) -> Result<(), DurableRunDeliveryReplayError> {
        self.materialize_completed_handoffs().await?;
        self.drain_completed_handoffs().await
    }

    async fn materialize_completed_handoffs(&self) -> Result<(), DurableRunDeliveryReplayError> {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return Ok(());
        };
        loop {
            let after = replay
                .outbound_state
                .load_run_final_reply_handoff_cursor()
                .await?;
            let page = replay
                .source
                .read_turn_event_log_after(Some(after), DURABLE_REPLAY_PAGE_SIZE)
                .await?;
            if page.rebase_required.is_some() {
                return Err(DurableRunDeliveryReplayError::RebaseRequired);
            }
            let truncated = page.truncated;
            let mut last_cursor = after;
            for event in page.entries {
                if event.cursor <= last_cursor {
                    return Err(DurableRunDeliveryReplayError::MalformedPage);
                }
                if event.kind == TurnEventKind::Completed
                    && self.completed_run_needs_channel_handoff(&event).await
                {
                    replay
                        .outbound_state
                        .put_run_final_reply_handoff(RunFinalReplyHandoffRecord {
                            event_cursor: event.cursor,
                            scope: event.scope,
                            run_id: event.run_id,
                        })
                        .await?;
                }
                last_cursor = event.cursor;
            }
            if last_cursor > after {
                // Every derived handoff in the page is durable before this
                // single checkpoint write. A crash before the checkpoint only
                // replays idempotent handoff puts.
                replay
                    .outbound_state
                    .advance_run_final_reply_handoff_cursor(last_cursor)
                    .await?;
            }
            if !truncated {
                return Ok(());
            }
            if last_cursor == after {
                return Err(DurableRunDeliveryReplayError::MalformedPage);
            }
        }
    }

    /// Decide whether a completed run needs a durable channel-delivery handoff.
    ///
    /// The handoff exists only to resume external channel delivery — the final
    /// reply and/or the source handler's progress-placeholder cleanup — across
    /// a crash. A run that no channel handler will ever own (a WebApp-destined
    /// WebUI answer, a scheduled trigger the volatile driver owns, or a run
    /// with no product context) has nothing to deliver, so a materialized
    /// handoff would leak a permanent pending row that every later drain
    /// re-scans. Skip those. On any lookup uncertainty the default is
    /// conservative — materialize — so a real channel delivery is never
    /// dropped; a channel-originated run that was blocked on OAuth and later
    /// completed is `Inbound` and always materializes.
    async fn completed_run_needs_channel_handoff(&self, event: &TurnLifecycleEvent) -> bool {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return true;
        };
        let state = match replay
            .run_state
            .get_run_state(GetRunStateRequest {
                scope: event.scope.clone(),
                run_id: event.run_id,
            })
            .await
        {
            Ok(state) => state,
            // The just-committed run's state should be readable; a leaked
            // WebApp handoff on a rare transient read error is strictly better
            // than a dropped channel reply, so fall back to materializing.
            Err(_) => return true,
        };
        let Some(context) = state.product_context.as_ref() else {
            // No product context means no channel adapter and no origin any
            // handler serves: nothing to deliver, so a handoff would only leak.
            return false;
        };
        match context.origin {
            // Channel-origin runs always have a source handler that either
            // delivers the final reply or retracts its progress placeholder and
            // settles, so their handoff is never orphaned.
            TurnOriginKind::Inbound => true,
            // Scheduled triggers are delivered by the separate volatile
            // triggered driver, never by this durable channel path.
            TurnOriginKind::ScheduledTrigger => false,
            // A WebUI answer lives in the web app unless the run sealed an
            // explicit external channel target (the cross-channel "send my
            // answer to X" case). No target, or a WebApp target, needs no
            // channel handoff.
            TurnOriginKind::WebUi => {
                let Some(actor) = state.actor.clone() else {
                    return true;
                };
                match replay
                    .outbound_state
                    .load_run_final_reply_target(RunFinalReplyTargetRequest {
                        run_id: state.run_id,
                        scope: state.scope.clone(),
                        actor,
                    })
                    .await
                {
                    Ok(Some(record)) => {
                        matches!(
                            record.destination,
                            RunFinalReplyDestination::External { .. }
                        )
                    }
                    Ok(None) => false,
                    Err(_) => true,
                }
            }
        }
    }

    async fn drain_completed_handoffs(&self) -> Result<(), DurableRunDeliveryReplayError> {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return Ok(());
        };
        let mut after: Option<RunFinalReplyHandoffRecord> = None;
        loop {
            let handoffs = replay
                .outbound_state
                .list_pending_run_final_reply_handoffs_after(
                    after.as_ref(),
                    DURABLE_REPLAY_PAGE_SIZE,
                )
                .await?;
            if handoffs.is_empty() {
                return Ok(());
            }
            let page_len = handoffs.len();
            for handoff in handoffs {
                let event = self.load_handoff_event(&handoff).await?;
                let mut saw_deferred = false;
                let mut delivery_settled = false;
                let mut required_source_adapter: Option<String> = None;
                let mut source_cleanup_settled = false;
                // A cross-channel final reply has two independent owners:
                // the selected target handler sends the result, while the
                // source handler retracts its last working/gate placeholder.
                // Fan out to every live handler even after one settles; an
                // early break leaves whichever responsibility comes later in
                // the registry's unordered iteration permanently unfinished.
                for handler in self.live_handlers() {
                    match handler.handle_event(&event).await {
                        Ok(disposition) => {
                            if let Some(source_adapter) = disposition.required_source_adapter {
                                match required_source_adapter.as_ref() {
                                    Some(existing) if existing != &source_adapter => {
                                        saw_deferred = true;
                                        tracing::warn!(
                                            target = "ironclaw::reborn::run_delivery",
                                            run_id = %event.run_id,
                                            expected_source_adapter = %existing,
                                            observed_source_adapter = %source_adapter,
                                            "durable completed-run delivery observed conflicting source owners"
                                        );
                                    }
                                    Some(_) => {}
                                    None => required_source_adapter = Some(source_adapter),
                                }
                            }
                            source_cleanup_settled |= disposition.source_cleanup_settled;
                            match disposition.outcome {
                                DeliveryEventOutcome::Settled => delivery_settled = true,
                                DeliveryEventOutcome::Deferred => saw_deferred = true,
                                DeliveryEventOutcome::Irrelevant => {}
                            }
                        }
                        Err(error) => {
                            saw_deferred = true;
                            tracing::warn!(
                                target = "ironclaw::reborn::run_delivery",
                                run_id = %event.run_id,
                                event_kind = ?event.kind,
                                %error,
                                "durable completed-run delivery remains pending after retryable failure"
                            );
                        }
                    }
                }
                let required_source_settled =
                    required_source_adapter.is_none() || source_cleanup_settled;
                if delivery_settled && required_source_settled && !saw_deferred {
                    replay
                        .outbound_state
                        .complete_run_final_reply_handoff(&handoff)
                        .await?;
                } else if !saw_deferred {
                    // No registered handler proved ownership yet. This is
                    // expected while channel graphs register during startup;
                    // registration supplies the next wake.
                    tracing::debug!(
                        target = "ironclaw::reborn::run_delivery",
                        run_id = %event.run_id,
                        "completed-run delivery awaits its route handler"
                    );
                }
                after = Some(handoff);
            }
            if page_len < DURABLE_REPLAY_PAGE_SIZE {
                return Ok(());
            }
        }
    }

    async fn load_handoff_event(
        &self,
        handoff: &RunFinalReplyHandoffRecord,
    ) -> Result<TurnLifecycleEvent, DurableRunDeliveryReplayError> {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return Err(DurableRunDeliveryReplayError::MalformedPage);
        };
        let Some(previous_cursor) = handoff.event_cursor.0.checked_sub(1).map(EventCursor) else {
            return Err(DurableRunDeliveryReplayError::MalformedPage);
        };
        let page = replay
            .source
            .read_turn_event_log_after(Some(previous_cursor), 1)
            .await?;
        if page.rebase_required.is_some() {
            return Err(DurableRunDeliveryReplayError::RebaseRequired);
        }
        let Some(event) = page.entries.into_iter().next() else {
            return Err(DurableRunDeliveryReplayError::MalformedPage);
        };
        if event.cursor != handoff.event_cursor
            || event.scope != handoff.scope
            || event.run_id != handoff.run_id
            || event.kind != TurnEventKind::Completed
        {
            return Err(DurableRunDeliveryReplayError::MalformedPage);
        }
        Ok(event)
    }

    async fn drain_run(&self, run_id: TurnRunId) {
        loop {
            let event = {
                let mut pending = self
                    .inner
                    .pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match pending.latest.remove(&run_id) {
                    Some(event) => event,
                    None => {
                        pending.active.remove(&run_id);
                        self.inner.idle.notify_waiters();
                        return;
                    }
                }
            };
            let permit = match Arc::clone(&self.inner.permits).acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    tracing::error!(
                        target = "ironclaw::reborn::run_delivery",
                        run_id = %run_id,
                        "run-delivery dispatcher closed unexpectedly"
                    );
                    let mut pending = self
                        .inner
                        .pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    pending.active.remove(&run_id);
                    self.inner.idle.notify_waiters();
                    return;
                }
            };
            self.dispatch_event(event).await;
            drop(permit);
        }
    }

    async fn dispatch_event(&self, event: TurnLifecycleEvent) {
        // In production, Completed delivery is owned by the durable handoff
        // projection. Keeping a second volatile live path would race the
        // durable worker and make an in-memory claim completion responsible
        // for waking it. Non-durable test/embedding routers retain the legacy
        // enqueue path, and triggered-run handlers remain independently live.
        if event.kind != TurnEventKind::Completed || self.inner.durable_replay.is_none() {
            for handler in self.live_handlers() {
                if let Err(error) = handler.handle_event(&event).await {
                    tracing::warn!(
                        target = "ironclaw::reborn::run_delivery",
                        run_id = %event.run_id,
                        event_kind = ?event.kind,
                        error = %error,
                        "event-driven external-channel delivery failed"
                    );
                }
            }
        }
        if let Some(handler) = self.triggered_handler(event.run_id) {
            match handler.handle_event(&event).await {
                Ok(true) => self.remove_triggered(event.run_id),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        target = "ironclaw::reborn::run_delivery",
                        run_id = %event.run_id,
                        event_kind = ?event.kind,
                        error = %error,
                        "event-driven triggered delivery failed"
                    );
                }
            }
        }
    }

    /// Waits until the named run has no queued or executing delivery work.
    /// This is useful for deterministic callers such as shutdown and contract
    /// tests; ordinary lifecycle publication remains enqueue-only.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn wait_until_run_idle(&self, run_id: TurnRunId) {
        loop {
            let notified = self.inner.idle.notified();
            tokio::pin!(notified);
            // Register before checking state so a drain that becomes idle
            // between the check and await cannot lose its notification.
            notified.as_mut().enable();
            let is_idle = {
                let pending = self
                    .inner
                    .pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                !pending.active.contains(&run_id) && !pending.latest.contains_key(&run_id)
            };
            if is_idle {
                return;
            }
            notified.await;
        }
    }

    /// Wait until the one-shot durable replay worker has no active work.
    /// Pending handoffs without a registered owner do not make this wait spin;
    /// a later handler registration supplies the next wake.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn wait_until_durable_replay_idle(&self) {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return;
        };
        loop {
            let notified = replay.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !replay.active.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

#[async_trait]
impl TurnEventSink for RunDeliveryEventRouter {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        if !matches!(
            event.kind,
            TurnEventKind::Submitted
                | TurnEventKind::Resumed
                | TurnEventKind::Blocked
                | TurnEventKind::Completed
                | TurnEventKind::Failed
                | TurnEventKind::Cancelled
        ) {
            return Ok(());
        }
        self.enqueue(event);
        self.wake_durable_replay();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DeliveryStage {
    Working(WorkingCycle),
    Approval(String),
    Auth(String),
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum WorkingCycle {
    Initial,
    Resumed(u64),
}

impl DeliveryStage {
    fn projection_epoch(&self, event_cursor: u64) -> String {
        match self {
            Self::Working(WorkingCycle::Initial) => "initial".to_string(),
            Self::Working(WorkingCycle::Resumed(cursor)) => format!("resumed-{cursor}"),
            _ => event_cursor.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DeliveryEventKey {
    run_id: TurnRunId,
    stage: DeliveryStage,
}

#[derive(Default)]
struct DeliveryLedger {
    active: HashSet<DeliveryEventKey>,
    delivered: HashSet<DeliveryEventKey>,
}

struct DeliveryClaim<'a> {
    ledger: &'a Mutex<DeliveryLedger>,
    key: DeliveryEventKey,
    complete: bool,
}

impl DeliveryClaim<'_> {
    fn complete(mut self, terminal: bool) {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ledger.active.remove(&self.key);
        ledger
            .delivered
            .retain(|delivered| delivered.run_id != self.key.run_id);
        if !terminal {
            ledger.delivered.insert(self.key.clone());
        }
        self.complete = true;
    }
}

impl Drop for DeliveryClaim<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.ledger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .active
                .remove(&self.key);
        }
    }
}

struct EventNotification {
    event_kind: RunNotificationEventKind,
    intent: DeliveryIntent,
    access: StoredProductReplyTargetAccess,
    part: OutboundPart,
    gate_ref: Option<String>,
    require_direct_message_target: bool,
}

/// One live channel graph's lifecycle consumer. It contains no vendor
/// behavior; the adapter identity is only a routing key and every send goes
/// through [`crate::DeliveryCoordinator`].
pub struct RunDeliveryEventHandler {
    services: RunDeliveryServices,
    adapter_id: String,
    installation_id: String,
    current_target_resolver: Option<Arc<dyn CurrentDeliveryTargetResolver>>,
    ledger: Mutex<DeliveryLedger>,
}

mod handler;

fn accepted_user_message_run_id(
    envelope: &ProductInboundEnvelope,
    ack: &ProductInboundAck,
) -> Option<TurnRunId> {
    if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_)) {
        return None;
    }
    let mut current = ack;
    loop {
        match current {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => return Some(*submitted_run_id),
            ProductInboundAck::Duplicate { prior } => current = prior,
            _ => return None,
        }
    }
}

fn notification_claim(
    event: &TurnLifecycleEvent,
    state: &TurnRunState,
) -> Option<(DeliveryStage, StoredProductReplyTargetAccess)> {
    match state.status {
        TurnStatus::Completed => Some((
            DeliveryStage::Final,
            StoredProductReplyTargetAccess::OrdinaryReply,
        )),
        TurnStatus::BlockedApproval => state.gate_ref.as_ref().map(|gate_ref| {
            (
                DeliveryStage::Approval(gate_ref.as_str().to_string()),
                StoredProductReplyTargetAccess::AuthorityBearingPrompt,
            )
        }),
        TurnStatus::BlockedAuth => state.gate_ref.as_ref().map(|gate_ref| {
            (
                DeliveryStage::Auth(gate_ref.as_str().to_string()),
                auth_prompt_target_access(source_surface_is_direct(state)),
            )
        }),
        _ if event.kind == TurnEventKind::Submitted => Some((
            DeliveryStage::Working(WorkingCycle::Initial),
            StoredProductReplyTargetAccess::OrdinaryReply,
        )),
        _ if event.kind == TurnEventKind::Resumed => Some((
            DeliveryStage::Working(WorkingCycle::Resumed(state.event_cursor.0)),
            StoredProductReplyTargetAccess::OrdinaryReply,
        )),
        _ => None,
    }
}

fn is_retractable_notification(intent: DeliveryIntent) -> bool {
    matches!(
        intent,
        DeliveryIntent::RunProgress | DeliveryIntent::GatePrompt | DeliveryIntent::AuthPrompt
    )
}

fn reconciliation_event_kind(status: TurnStatus) -> TurnEventKind {
    match status {
        TurnStatus::Queued | TurnStatus::Running => TurnEventKind::Submitted,
        TurnStatus::BlockedApproval
        | TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::BlockedExternalTool => TurnEventKind::Blocked,
        TurnStatus::CancelRequested => TurnEventKind::CancelRequested,
        TurnStatus::Cancelled => TurnEventKind::Cancelled,
        TurnStatus::Completed => TurnEventKind::Completed,
        TurnStatus::Failed => TurnEventKind::Failed,
        TurnStatus::RecoveryRequired => TurnEventKind::RecoveryRequired,
    }
}

fn source_surface_is_direct(state: &TurnRunState) -> bool {
    matches!(
        state
            .product_context
            .as_ref()
            .and_then(|context| context.surface_type),
        Some(TurnSurfaceType::Direct)
    )
}

fn auth_prompt_target_access(direct: bool) -> StoredProductReplyTargetAccess {
    if direct {
        StoredProductReplyTargetAccess::AuthorityBearingPrompt
    } else {
        // Shared-surface auth notifications have their challenge material
        // removed before delivery. The remaining generic notice carries no
        // authority, so it may use the same sealed shared route as an ordinary
        // reply while direct prompts retain exact-origin-actor enforcement.
        StoredProductReplyTargetAccess::OrdinaryReply
    }
}
