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

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    OutboundError, OutboundPolicyService, OutboundStateStore, PrepareCommunicationDeliveryRequest,
    ProjectionUpdateRef, RunFinalReplyDestination, RunFinalReplyHandoffRecord,
    RunFinalReplyTargetRequest, RunNotificationContext, RunNotificationEventKind,
    RunNotificationOrigin, SourceRouteContext,
};
use ironclaw_product_adapters::{
    AuthPromptChallengeKind, OutboundPart, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload,
};
use ironclaw_threads::{FinalizedAssistantMessageByRunRequest, ThreadScope};
use ironclaw_turns::{
    EventCursor, GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnError, TurnEventKind,
    TurnEventProjectionSource, TurnEventSink, TurnLifecycleEvent, TurnOriginKind, TurnRunId,
    TurnRunState, TurnStatus, TurnSurfaceType,
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
enum DeliveryEventDisposition {
    /// This handler does not own the event's sealed route.
    Irrelevant,
    /// Authority or message state may become available after another commit.
    Deferred,
    /// No retry is needed: delivery reached a durable terminal outcome, or
    /// current authority definitively rejected the sealed route.
    Settled,
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
        outbound_state: Arc<dyn OutboundStateStore>,
    ) -> Self {
        let router = Self::build(Some(DurableRunDeliveryReplay {
            source,
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
            let mut progressed = false;
            for event in page.entries {
                if event.cursor <= after {
                    return Err(DurableRunDeliveryReplayError::MalformedPage);
                }
                if event.kind == TurnEventKind::Completed {
                    replay
                        .outbound_state
                        .put_run_final_reply_handoff(RunFinalReplyHandoffRecord {
                            event_cursor: event.cursor,
                            scope: event.scope,
                            run_id: event.run_id,
                        })
                        .await?;
                }
                // The cursor moves only after the derived handoff row is
                // durable. Replaying an identical put after a CAS race is safe.
                replay
                    .outbound_state
                    .advance_run_final_reply_handoff_cursor(event.cursor)
                    .await?;
                progressed = true;
            }
            if !truncated {
                return Ok(());
            }
            if !progressed {
                return Err(DurableRunDeliveryReplayError::MalformedPage);
            }
        }
    }

    async fn drain_completed_handoffs(&self) -> Result<(), DurableRunDeliveryReplayError> {
        let Some(replay) = self.inner.durable_replay.as_ref() else {
            return Ok(());
        };
        loop {
            let handoffs = replay
                .outbound_state
                .list_pending_run_final_reply_handoffs(DURABLE_REPLAY_PAGE_SIZE)
                .await?;
            if handoffs.is_empty() {
                return Ok(());
            }
            let mut progressed = false;
            for handoff in handoffs {
                let event = self.load_handoff_event(&handoff).await?;
                let mut saw_deferred = false;
                let mut settled = false;
                for handler in self.live_handlers() {
                    match handler.handle_event(&event).await {
                        Ok(DeliveryEventDisposition::Settled) => {
                            settled = true;
                            break;
                        }
                        Ok(DeliveryEventDisposition::Deferred) => saw_deferred = true,
                        Ok(DeliveryEventDisposition::Irrelevant) => {}
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
                if settled {
                    replay
                        .outbound_state
                        .complete_run_final_reply_handoff(&handoff)
                        .await?;
                    progressed = true;
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
            }
            if !progressed {
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
    #[doc(hidden)]
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
    cleanup: HashMap<TurnRunId, Vec<DeliveredChannelMessage>>,
}

struct DeliveryClaim<'a> {
    ledger: &'a Mutex<DeliveryLedger>,
    key: DeliveryEventKey,
    complete: bool,
}

impl DeliveryClaim<'_> {
    fn complete(mut self) {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ledger.active.remove(&self.key);
        ledger.delivered.insert(self.key.clone());
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

impl RunDeliveryEventHandler {
    pub fn new(
        services: RunDeliveryServices,
        adapter_id: impl Into<String>,
        installation_id: impl Into<String>,
    ) -> Self {
        Self {
            services,
            adapter_id: adapter_id.into(),
            installation_id: installation_id.into(),
            current_target_resolver: None,
            ledger: Mutex::new(DeliveryLedger::default()),
        }
    }

    pub fn with_current_target_resolver(
        mut self,
        resolver: Arc<dyn CurrentDeliveryTargetResolver>,
    ) -> Self {
        self.current_target_resolver = Some(resolver);
        self
    }

    /// Reconcile an accepted external user message after the inbound workflow
    /// has durably finished binding its source route.
    ///
    /// Lifecycle events may be published by the turn coordinator before the
    /// product workflow returns its admission acknowledgement. The router is
    /// intentionally enqueue-only, so that event can race the final binding
    /// commit. This post-admission replay re-opens canonical run state through
    /// the same event path; it never sends provider traffic inline.
    pub async fn reconcile_accepted_user_message(
        &self,
        router: &RunDeliveryEventRouter,
        envelope: &ProductInboundEnvelope,
        ack: &ProductInboundAck,
    ) -> Result<(), ProductWorkflowError> {
        let Some(submitted_run_id) = accepted_user_message_run_id(envelope, ack) else {
            return Ok(());
        };
        let binding = self
            .services
            .binding_service
            .lookup_binding(crate::ResolveBindingRequest::from_envelope(envelope))
            .await?;
        let scope = ironclaw_turns::TurnScope::new_with_owner(
            binding.tenant_id,
            binding.agent_id,
            binding.project_id,
            binding.thread_id,
            binding.subject_user_id,
        );
        let state = self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope,
                run_id: submitted_run_id,
            })
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("post-admission run state is unavailable: {error}"),
            })?;
        router
            .publish(TurnLifecycleEvent::from_run_state(
                &state,
                reconciliation_event_kind(state.status),
                None,
            ))
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to enqueue post-admission run delivery: {error}"),
            })
    }

    async fn handle_event(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<DeliveryEventDisposition, RunDeliveryError> {
        let state = self
            .services
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: event.scope.clone(),
                run_id: event.run_id,
            })
            .await?;
        let Some(context) = state.product_context.as_ref() else {
            return Ok(DeliveryEventDisposition::Irrelevant);
        };
        // External inbound runs use their sealed source route for every live
        // notification. A WebUI run has no channel adapter, but its completed
        // answer may still carry an explicit, host-sealed external target.
        // Let that one terminal case proceed to the durable target lookup;
        // without a run target it will fail closed below. Triggered runs remain
        // owned by the triggered-delivery driver.
        let handles_live_origin = context.origin == TurnOriginKind::Inbound
            || (context.origin == TurnOriginKind::WebUi && state.status == TurnStatus::Completed);
        if !handles_live_origin {
            return Ok(DeliveryEventDisposition::Irrelevant);
        }
        let is_source_handler = context
            .adapter
            .as_ref()
            .is_some_and(|source_adapter| source_adapter.as_str() == self.adapter_id);
        let Some(actor) = state.actor.clone() else {
            return Ok(DeliveryEventDisposition::Deferred);
        };
        if matches!(state.status, TurnStatus::Failed | TurnStatus::Cancelled) {
            if is_source_handler {
                self.retract_pending_messages(state.run_id, &state).await;
            }
            return Ok(if is_source_handler {
                DeliveryEventDisposition::Settled
            } else {
                DeliveryEventDisposition::Irrelevant
            });
        }
        if state.status != TurnStatus::Completed && !is_source_handler {
            return Ok(DeliveryEventDisposition::Irrelevant);
        }

        let delivery_target = if state.status == TurnStatus::Completed {
            match self
                .services
                .outbound_store
                .load_run_final_reply_target(RunFinalReplyTargetRequest {
                    run_id: state.run_id,
                    scope: state.scope.clone(),
                    actor: actor.clone(),
                })
                .await?
                .map(|record| record.destination)
            {
                Some(RunFinalReplyDestination::WebApp) => {
                    if is_source_handler {
                        self.retract_pending_messages(state.run_id, &state).await;
                    }
                    return Ok(DeliveryEventDisposition::Settled);
                }
                Some(RunFinalReplyDestination::External {
                    reply_target_binding_ref,
                }) => {
                    if is_source_handler {
                        self.retract_pending_messages(state.run_id, &state).await;
                    }
                    reply_target_binding_ref
                }
                None if is_source_handler => state.reply_target_binding_ref.clone(),
                None => return Ok(DeliveryEventDisposition::Irrelevant),
            }
        } else {
            state.reply_target_binding_ref.clone()
        };
        let Some((stage, access)) = notification_claim(event, &state) else {
            return Ok(DeliveryEventDisposition::Irrelevant);
        };
        let projection_epoch = stage.projection_epoch(state.event_cursor.0);
        let key = DeliveryEventKey {
            run_id: state.run_id,
            stage,
        };
        let Some(claim) = self.claim(key) else {
            return Ok(DeliveryEventDisposition::Settled);
        };

        // Establish the sealed source-route authority before constructing an
        // auth prompt. Prompt construction can mint or supersede a provider
        // OAuth flow, so it must not happen for an uncommitted or revoked
        // reply route.
        let uses_run_scoped_target = delivery_target != state.reply_target_binding_ref;
        let source_target = if uses_run_scoped_target {
            let Some(resolver) = self.current_target_resolver.as_ref() else {
                return Ok(DeliveryEventDisposition::Irrelevant);
            };
            match resolver
                .resolve_current_target(&state.scope, &actor, &delivery_target)
                .await
            {
                Ok(Some(target)) if target.extension_id == self.services.extension_id => None,
                Ok(Some(_)) => return Ok(DeliveryEventDisposition::Irrelevant),
                // Let the outbound policy persist its durable Rejected outcome
                // for a removed target. The same current-authority check runs
                // again immediately before egress.
                Ok(None) | Err(ProductWorkflowError::BindingAccessDenied) => None,
                Err(error) => return Err(RunDeliveryError::Workflow(error)),
            }
        } else if state.status == TurnStatus::Completed {
            // Completed replies have no auth-prompt construction side effect.
            // Defer current membership/pairing validation to the outbound
            // policy so denial is persisted as a terminal Rejected attempt.
            None
        } else {
            match self
                .resolve_target(&state, &actor, &delivery_target, access)
                .await
            {
                Ok(Some(target)) => Some(target),
                Ok(None) => return Ok(DeliveryEventDisposition::Irrelevant),
                Err(
                    error @ (ProductWorkflowError::BindingAccessDenied
                    | ProductWorkflowError::BindingRequired { .. }
                    | ProductWorkflowError::UnknownInstallation),
                ) => {
                    tracing::debug!(
                        target = "ironclaw::reborn::run_delivery",
                        run_id = %state.run_id,
                        %error,
                        "deferred lifecycle delivery because the stored source route is not currently authorized"
                    );
                    // Submitted/blocked events can race the inbound workflow's
                    // source-route commit. Their post-admission replay must be
                    // allowed to retry.
                    return Ok(DeliveryEventDisposition::Deferred);
                }
                Err(error) => return Err(RunDeliveryError::Workflow(error)),
            }
        };
        let Some(notification) = self.notification(event, &state, &actor).await? else {
            return Ok(DeliveryEventDisposition::Deferred);
        };

        let cleanup_intent = notification.intent;
        let delivered = match self
            .deliver_policy_notification(
                &state,
                &actor,
                &delivery_target,
                source_target.as_ref(),
                uses_run_scoped_target,
                notification,
                &projection_epoch,
            )
            .await
        {
            Ok(delivered) => delivered,
            Err(RunDeliveryError::DeliveryFailed { .. }) => {
                // The coordinator has already recorded a durable terminal
                // provider outcome. Retrying this handoff cannot improve it.
                claim.complete();
                return Ok(DeliveryEventDisposition::Settled);
            }
            Err(error) => return Err(error),
        };

        self.update_cleanup(state.run_id, cleanup_intent, &delivered, &state)
            .await;
        claim.complete();
        Ok(DeliveryEventDisposition::Settled)
    }

    fn claim(&self, key: DeliveryEventKey) -> Option<DeliveryClaim<'_>> {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if ledger.delivered.contains(&key) || !ledger.active.insert(key.clone()) {
            return None;
        }
        drop(ledger);
        Some(DeliveryClaim {
            ledger: &self.ledger,
            key,
            complete: false,
        })
    }

    async fn notification(
        &self,
        event: &TurnLifecycleEvent,
        state: &TurnRunState,
        actor: &TurnActor,
    ) -> Result<Option<EventNotification>, RunDeliveryError> {
        let notification = match state.status {
            TurnStatus::Completed => {
                let Some(text) = self.read_final_text(state).await? else {
                    tracing::warn!(run_id = %state.run_id, "completed run has no finalized assistant message");
                    return Ok(None);
                };
                EventNotification {
                    event_kind: RunNotificationEventKind::FinalReplyReady,
                    intent: DeliveryIntent::FinalReply,
                    access: StoredProductReplyTargetAccess::OrdinaryReply,
                    part: OutboundPart::Text(text),
                    gate_ref: None,
                    require_direct_message_target: false,
                }
            }
            TurnStatus::BlockedApproval => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    return Ok(None);
                };
                let context = match &self.services.approval_context {
                    Some(source) => {
                        source
                            .approval_prompt_context(gate_ref, &actor.user_id, &state.scope)
                            .await
                    }
                    None => None,
                };
                let direct = source_surface_is_direct(state);
                let view =
                    prompts::approval_gate_prompt_view(state.run_id, gate_ref, context.as_ref());
                EventNotification {
                    event_kind: RunNotificationEventKind::ApprovalNeeded,
                    intent: DeliveryIntent::GatePrompt,
                    access: StoredProductReplyTargetAccess::AuthorityBearingPrompt,
                    part: OutboundPart::Text(prompts::gate_prompt_text(&view, direct)),
                    gate_ref: Some(gate_ref.as_str().to_string()),
                    require_direct_message_target: false,
                }
            }
            TurnStatus::BlockedAuth => {
                let Some(gate_ref) = state.gate_ref.as_ref() else {
                    return Ok(None);
                };
                let direct = source_surface_is_direct(state);
                let access = auth_prompt_target_access(direct);
                let view = match &self.services.blocked_auth_prompts {
                    Some(source) => Some(
                        source
                            .auth_prompt_for_blocked_run(BlockedAuthPromptRequest {
                                fallback_owner_user_id: &actor.user_id,
                                scope: &state.scope,
                                run_id: state.run_id,
                                gate_ref: gate_ref.as_str(),
                                invocation_id: None,
                                body: "Authenticate to continue this run.".to_string(),
                                credential_requirements: &state.credential_requirements,
                            })
                            .await?,
                    ),
                    None => None,
                };
                let unavailable_message = prompts::unserviceable_auth_prompt_message(view.as_ref());
                let Some(mut view) = view.filter(prompts::auth_prompt_is_serviceable) else {
                    cancel_auth_blocked_run(
                        self.services.turn_coordinator.as_ref(),
                        self.services.auth_flow_cancel.as_deref(),
                        &state.scope,
                        actor.clone(),
                        state.run_id,
                        Some(gate_ref.as_str()),
                    )
                    .await?;
                    return Ok(Some(EventNotification {
                        event_kind: RunNotificationEventKind::AuthRequired,
                        intent: DeliveryIntent::RunFailureNotice,
                        access: StoredProductReplyTargetAccess::OrdinaryReply,
                        part: OutboundPart::Text(unavailable_message.to_string()),
                        gate_ref: None,
                        require_direct_message_target: false,
                    }));
                };
                view.body = prompts::actionable_auth_prompt_body(&view);
                if !direct {
                    view.authorization_url = None;
                    view.pairing = None;
                    if view.challenge_kind == Some(AuthPromptChallengeKind::Pairing) {
                        view.body = prompts::PAIRING_PRIVATE_SETUP_MESSAGE.to_string();
                    } else if view.challenge_kind == Some(AuthPromptChallengeKind::OAuthUrl) {
                        view.body = prompts::OAUTH_PRIVATE_SETUP_MESSAGE.to_string();
                    }
                }
                let require_direct_message_target =
                    view.authorization_url.is_some() || view.pairing.is_some();
                EventNotification {
                    event_kind: RunNotificationEventKind::AuthRequired,
                    intent: DeliveryIntent::AuthPrompt,
                    access,
                    part: OutboundPart::AuthPrompt {
                        view: Box::new(view),
                        direct_message: direct,
                    },
                    gate_ref: Some(gate_ref.as_str().to_string()),
                    require_direct_message_target,
                }
            }
            _ if matches!(
                event.kind,
                TurnEventKind::Submitted | TurnEventKind::Resumed
            ) =>
            {
                EventNotification {
                    event_kind: RunNotificationEventKind::ProgressUpdate,
                    intent: DeliveryIntent::RunProgress,
                    access: StoredProductReplyTargetAccess::OrdinaryReply,
                    part: OutboundPart::Text(prompts::WORKING_MESSAGE.to_string()),
                    gate_ref: None,
                    require_direct_message_target: false,
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(notification))
    }

    async fn resolve_target(
        &self,
        state: &TurnRunState,
        actor: &TurnActor,
        reply_target_binding_ref: &ReplyTargetBindingRef,
        access: StoredProductReplyTargetAccess,
    ) -> Result<Option<ResolvedStoredProductReplyTarget>, ProductWorkflowError> {
        let target = self
            .services
            .binding_service
            .resolve_stored_reply_target(ResolveStoredProductReplyTargetRequest {
                scope: state.scope.clone(),
                actor: actor.clone(),
                reply_target_binding_ref: reply_target_binding_ref.clone(),
                access,
            })
            .await?;
        if target.adapter_id.as_str() != self.adapter_id
            || target.installation_id.as_str() != self.installation_id
        {
            return Ok(None);
        }
        Ok(Some(target))
    }

    async fn deliver_policy_notification(
        &self,
        state: &TurnRunState,
        actor: &TurnActor,
        delivery_target: &ReplyTargetBindingRef,
        source_target: Option<&ResolvedStoredProductReplyTarget>,
        uses_run_scoped_target: bool,
        notification: EventNotification,
        projection_epoch: &str,
    ) -> Result<Vec<DeliveredChannelMessage>, RunDeliveryError> {
        let authority = if uses_run_scoped_target {
            let resolver =
                self.current_target_resolver
                    .as_ref()
                    .ok_or(RunDeliveryError::Workflow(
                        ProductWorkflowError::BindingAccessDenied,
                    ))?;
            LiveReplyTargetAuthority::Current {
                resolver: Arc::clone(resolver),
                scope: state.scope.clone(),
                actor: actor.clone(),
                expected_target: delivery_target.clone(),
                expected_extension_id: self.services.extension_id.clone(),
            }
        } else {
            LiveReplyTargetAuthority::Source(StoredReplyTargetAuthority {
                binding_service: Arc::clone(&self.services.binding_service),
                scope: state.scope.clone(),
                actor: actor.clone(),
                expected_target: delivery_target.clone(),
                expected_adapter_id: self.adapter_id.clone(),
                expected_installation_id: self.installation_id.clone(),
                access: notification.access,
            })
        };
        let projection_policy = AllowNoProjectionAccess;
        let outbound_policy = OutboundPolicyService::new(
            self.services.outbound_store.as_ref(),
            &projection_policy,
            &authority,
        );
        // The lifecycle cursor gives each committed notification epoch a
        // stable projection identity. The handler ledger suppresses replay
        // within this router lifetime; durable cross-process at-most-once
        // behavior remains the outbound coordinator/store's responsibility.
        let projection_ref = ProjectionUpdateRef::new(format!(
            "{}:{projection_epoch}",
            prompts::run_notification_projection_id(state.run_id, notification.event_kind),
        ))
        .map_err(|reason| RunDeliveryError::InvalidProjectionRef { reason })?;
        let outcome = self
            .services
            .coordinator
            .deliver(
                &outbound_policy,
                self.services.communication_preferences.as_ref(),
                &authority,
                CoordinatedDeliveryRequest {
                    intent: notification.intent,
                    delivery: PrepareCommunicationDeliveryRequest {
                        resolution_request: CommunicationDeliveryResolutionRequest {
                            scope: state.scope.clone(),
                            actor: actor.clone(),
                            modality: CommunicationModality::Text,
                            intent: CommunicationDeliveryIntent::RunNotification(
                                RunNotificationContext {
                                    event_kind: notification.event_kind,
                                    origin: if delivery_target == &state.reply_target_binding_ref {
                                        RunNotificationOrigin::LiveSourceRoute {
                                            source_route: SourceRouteContext {
                                                reply_target_binding_ref: delivery_target.clone(),
                                            },
                                        }
                                    } else {
                                        RunNotificationOrigin::RunScopedTarget {
                                            target: delivery_target.clone(),
                                        }
                                    },
                                },
                            ),
                        },
                        turn_run_id: Some(state.run_id),
                        projection_ref,
                        attempted_at: Utc::now(),
                    },
                    parts: vec![notification.part],
                    thread_anchor: None,
                    require_direct_message_target: notification.require_direct_message_target,
                    extension_id: &self.services.extension_id,
                },
            )
            .await?;
        let delivered = match outcome {
            CoordinatedDeliveryOutcome::Failed { failure_kind, .. } => {
                return Err(RunDeliveryError::DeliveryFailed { failure_kind });
            }
            outcome => delivered_messages_from_outcome(&outcome),
        };
        if let Some(gate_ref) = notification.gate_ref.as_deref() {
            record_gate_route_if_needed(
                self.services.route_store.as_ref(),
                state.run_id,
                &state.scope.tenant_id,
                &actor.user_id,
                gate_ref,
                &state.scope,
                &delivered,
                source_target.map(|target| &target.external_conversation_ref),
            )
            .await;
        }
        Ok(delivered)
    }

    async fn read_final_text(
        &self,
        state: &TurnRunState,
    ) -> Result<Option<String>, RunDeliveryError> {
        let Some(agent_id) = state.scope.agent_id.clone() else {
            return Ok(None);
        };
        let thread_scope = ThreadScope {
            tenant_id: state.scope.tenant_id.clone(),
            agent_id,
            project_id: state.scope.project_id.clone(),
            owner_user_id: state.scope.explicit_owner_user_id().cloned(),
            mission_id: None,
        };
        Ok(self
            .services
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: thread_scope,
                thread_id: state.scope.thread_id.clone(),
                turn_run_id: state.run_id.to_string(),
            })
            .await?
            .and_then(|message| message.content))
    }

    async fn update_cleanup(
        &self,
        run_id: TurnRunId,
        intent: DeliveryIntent,
        delivered: &[DeliveredChannelMessage],
        state: &TurnRunState,
    ) {
        let cleanup = {
            let mut ledger = self
                .ledger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = ledger.cleanup.remove(&run_id).unwrap_or_default();
            if is_retractable_notification(intent) {
                ledger.cleanup.insert(run_id, delivered.to_vec());
            }
            previous
        };
        for message in cleanup {
            self.retract_if_current(state, message).await;
        }
    }

    async fn retract_pending_messages(&self, run_id: TurnRunId, state: &TurnRunState) {
        let cleanup = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cleanup
            .remove(&run_id)
            .unwrap_or_default();
        for message in cleanup {
            self.retract_if_current(state, message).await;
        }
    }

    async fn retract_if_current(&self, state: &TurnRunState, message: DeliveredChannelMessage) {
        let Some(actor) = state.actor.as_ref() else {
            return;
        };
        if message.reply_target_binding_ref != state.reply_target_binding_ref {
            return;
        }
        let Ok(target) = self
            .resolve_target(
                state,
                actor,
                &state.reply_target_binding_ref,
                StoredProductReplyTargetAccess::OrdinaryReply,
            )
            .await
        else {
            return;
        };
        let Some(target) = target else {
            return;
        };
        if target.external_conversation_ref != message.conversation {
            return;
        }
        self.services
            .retract_message(state.scope.clone(), Some(state.run_id), message)
            .await;
    }
}

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
