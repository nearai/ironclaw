use std::{
    collections::HashMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_turns::{
    GetRunStateRequest, TurnRunId, TurnRunState, TurnRunWake, TurnRunWakeNotifier, TurnScope,
    TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, LoopCancelReasonKind, LoopCancellationPort,
        LoopCancellationSignal,
    },
};
use parking_lot::RwLock;
use tokio::sync::Notify;

const DEFAULT_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone)]
struct RunCancellationRequester {
    fired: Arc<AtomicBool>,
    signal: Arc<RwLock<Option<LoopCancellationSignal>>>,
    notify: Arc<Notify>,
    owner: Weak<()>,
}

impl RunCancellationRequester {
    fn request(&self, reason_kind: LoopCancelReasonKind) {
        request_cancellation(&self.fired, &self.signal, &self.notify, reason_kind);
    }

    fn is_owner_alive(&self) -> bool {
        self.owner.upgrade().is_some()
    }
}

fn request_cancellation(
    fired: &AtomicBool,
    signal: &RwLock<Option<LoopCancellationSignal>>,
    notify: &Notify,
    reason_kind: LoopCancelReasonKind,
) {
    if fired.load(Ordering::Acquire) {
        return;
    }
    let mut signal_lock = signal.write();
    if signal_lock.is_some() {
        return;
    }
    let new_signal = LoopCancellationSignal {
        reason_kind,
        requested_at: Utc::now(),
    };
    *signal_lock = Some(new_signal);
    // Publish `fired` while the write guard is still held so any reader that
    // observes `fired == true` via Acquire is also guaranteed to see the
    // populated `signal_lock`, independent of the RwLock's own ordering.
    fired.store(true, Ordering::Release);
    drop(signal_lock);
    notify.notify_waiters();
}

/// Snapshot handle the host runtime owns and flips on cancellation.
#[derive(Clone, Default)]
pub struct RunCancellationHandle {
    fired: Arc<AtomicBool>,
    signal: Arc<RwLock<Option<LoopCancellationSignal>>>,
    notify: Arc<Notify>,
    owner: Arc<()>,
}

impl RunCancellationHandle {
    pub fn request(&self, reason_kind: LoopCancelReasonKind) {
        request_cancellation(&self.fired, &self.signal, &self.notify, reason_kind);
    }

    pub fn is_requested(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }

    fn observe(&self) -> Option<LoopCancellationSignal> {
        if !self.fired.load(Ordering::Acquire) {
            return None;
        }
        self.signal.read().clone()
    }

    async fn requested(&self) -> LoopCancellationSignal {
        loop {
            let notified = self.notify.notified();
            if let Some(signal) = self.observe() {
                return signal;
            }
            notified.await;
        }
    }

    fn requester(&self) -> RunCancellationRequester {
        RunCancellationRequester {
            fired: Arc::clone(&self.fired),
            signal: Arc::clone(&self.signal),
            notify: Arc::clone(&self.notify),
            owner: Arc::downgrade(&self.owner),
        }
    }
}

/// Cancellation port backed by a run-scoped snapshot handle.
pub struct RunStateLoopCancellationPort {
    handle: RunCancellationHandle,
}

impl RunStateLoopCancellationPort {
    pub fn new(handle: RunCancellationHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl LoopCancellationPort for RunStateLoopCancellationPort {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        self.handle.observe()
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        self.handle.requested().await
    }
}

/// Always reports "not cancelled".
pub struct AlwaysAliveLoopCancellationPort;

#[async_trait]
impl LoopCancellationPort for AlwaysAliveLoopCancellationPort {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        None
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        std::future::pending().await
    }
}

/// Produces one cancellation handle per claimed run.
#[async_trait]
pub trait RunCancellationFactory: Send + Sync {
    /// Describes whether handles from this factory can observe real host
    /// cancellation requests.
    fn observation_kind(&self) -> RunCancellationObservationKind {
        RunCancellationObservationKind::LiveCapable
    }

    async fn handle_for_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError>;

    /// Build a handle for an already-claimed run.
    ///
    /// Implementations that can trust the claimed state may use it as the
    /// initial cancellation seed to keep host construction off a durable
    /// turn-state read. The default preserves the strict durable-read path for
    /// implementations without a claimed-state fast path.
    async fn handle_for_claimed_run(
        &self,
        state: &TurnRunState,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        self.handle_for_run(&state.scope, state.run_id).await
    }

    /// Observe a `TurnRunWake` published by the turn coordinator.
    ///
    /// **Called synchronously on the wake publisher's thread** by
    /// `CompositeTurnRunWakeNotifier` when the runtime composition wires a
    /// cancellation factory into the coordinator's wake notifier. Implementations
    /// MUST be non-blocking: no I/O, no awaits, no locks held across awaits, no
    /// waiting on channels. Slow work here directly slows
    /// `TurnCoordinator::cancel_run` and `submit_turn`. Default is a no-op.
    fn notify_run_wake(&self, _wake: &TurnRunWake) {}

    fn product_live_cancellation_probe(&self) -> Option<Box<dyn ProductLiveCancellationProbe>> {
        None
    }

    fn is_product_cancellation_observed(
        &self,
        _run_id: TurnRunId,
    ) -> Result<bool, AgentLoopHostError> {
        tracing::debug!(
            "run cancellation factory does not observe product cancellation: default Ok(false) — factory is not product-live-capable"
        );
        Ok(false)
    }
}

/// Executable product-path cancellation probe used to gate product-live runtime
/// wiring. Implementations must exercise the same request/observe path product
/// code uses for a retained run handle.
///
/// Probes are short-lived and self-contained: implementations MUST NOT retain
/// probe handles in any shared map keyed by run id. The probe's lifetime ends
/// when the verifier drops the `Box<dyn ProductLiveCancellationProbe>`; any
/// state owned by the probe must be released by that point. This avoids growing
/// the factory's run-handle map on every readiness check.
pub trait ProductLiveCancellationProbe: Send + Sync {
    fn request_cancellation(
        &self,
        reason_kind: LoopCancelReasonKind,
    ) -> Result<(), AgentLoopHostError>;

    fn is_cancellation_observed(&self) -> Result<bool, AgentLoopHostError>;
}

/// Runtime liveness contract for run cancellation observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunCancellationObservationKind {
    /// Handles can be flipped by live host/runtime cancellation requests.
    LiveCapable,
    /// Handles are inert fallbacks for non-live or test-only runtimes.
    InertFallback,
}

impl RunCancellationObservationKind {
    pub fn is_live_capable(self) -> bool {
        matches!(self, Self::LiveCapable)
    }
}

/// Product-live readiness evidence for a run cancellation source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductLiveCancellationReadiness {
    /// The source cannot be cancelled or observed from the product path.
    Inert,
    /// Product code retains per-run handles and can request or observe cancellation.
    ExternallyControllable,
}

pub fn verify_product_live_cancellation_probe(
    factory: &dyn RunCancellationFactory,
) -> Result<ProductLiveCancellationReadiness, AgentLoopHostError> {
    let Some(probe) = factory.product_live_cancellation_probe() else {
        return Ok(ProductLiveCancellationReadiness::Inert);
    };
    if probe.is_cancellation_observed()? {
        return Ok(ProductLiveCancellationReadiness::Inert);
    }
    probe.request_cancellation(LoopCancelReasonKind::UserRequested)?;
    if probe.is_cancellation_observed()? {
        Ok(ProductLiveCancellationReadiness::ExternallyControllable)
    } else {
        Ok(ProductLiveCancellationReadiness::Inert)
    }
}

/// Run cancellation factory backed by durable turn state.
///
/// Handles are seeded from the current run state before being returned and are
/// registered for later wake-driven flips. A lightweight polling fallback
/// covers runtimes that have not yet wired the wake notifier into their cancel
/// path.
pub struct TurnStateRunCancellationFactory {
    store: Arc<dyn TurnStateStore>,
    handles: Arc<RwLock<HashMap<TurnRunId, Vec<RunCancellationRequester>>>>,
    poll_interval: Duration,
}

impl TurnStateRunCancellationFactory {
    pub fn new(store: Arc<dyn TurnStateStore>) -> Self {
        Self {
            store,
            handles: Arc::new(RwLock::new(HashMap::new())),
            poll_interval: DEFAULT_CANCEL_POLL_INTERVAL,
        }
    }

    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    fn notify_cancel_requested(&self, run_id: TurnRunId) {
        // Atomically drain the entry so any concurrent `register` that arrives
        // after this point starts a fresh vec instead of being silently dropped
        // by a follow-up `remove_run`.
        let requesters = self.handles.write().remove(&run_id);
        if let Some(requesters) = requesters {
            for requester in requesters {
                requester.request(LoopCancelReasonKind::UserRequested);
            }
        }
    }

    async fn read_run_status(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<TurnStatus, AgentLoopHostError> {
        self.store
            .get_run_state_for_cancellation(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .map(|state| state.status)
            .map_err(turn_state_error_to_host_error)
    }

    async fn seed_from_state(
        &self,
        scope: &TurnScope,
        requester: &RunCancellationRequester,
        run_id: TurnRunId,
    ) -> Result<TurnStatus, AgentLoopHostError> {
        let status = self.read_run_status(scope, run_id).await?;
        self.seed_requester_from_status(requester, status);
        Ok(status)
    }

    fn register(&self, run_id: TurnRunId, requester: RunCancellationRequester) {
        self.handles
            .write()
            .entry(run_id)
            .or_default()
            .push(requester);
    }

    fn seed_requester_from_status(&self, requester: &RunCancellationRequester, status: TurnStatus) {
        if status == TurnStatus::CancelRequested {
            requester.request(LoopCancelReasonKind::UserRequested);
        }
    }

    fn remove_run(&self, run_id: TurnRunId) {
        remove_run_handles(&self.handles, run_id);
    }

    #[cfg(test)]
    fn registered_run_count(&self) -> usize {
        self.handles.read().len()
    }

    fn spawn_polling_fallback(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
        requester: RunCancellationRequester,
    ) {
        let store = Arc::clone(&self.store);
        let handles = Arc::clone(&self.handles);
        let base_interval = self.poll_interval;
        tokio::spawn(async move {
            // Exponential backoff caps long-lived stuck runs
            // at one poll every `MAX_POLL_INTERVAL` instead of hammering the store at
            // `base_interval` for the full owner lifetime.
            const MAX_POLL_INTERVAL: Duration = Duration::from_secs(5);
            let mut interval = base_interval;
            while requester.is_owner_alive() && !requester.fired.load(Ordering::Acquire) {
                let status = store
                    .get_run_state(GetRunStateRequest {
                        scope: scope.clone(),
                        run_id,
                    })
                    .await
                    .map(|state| state.status);
                match status {
                    Ok(TurnStatus::CancelRequested) => {
                        requester.request(LoopCancelReasonKind::UserRequested);
                        break;
                    }
                    Ok(status) if status.is_terminal() => break,
                    Ok(_) | Err(_) => {
                        interval = (interval.saturating_mul(2)).min(MAX_POLL_INTERVAL);
                    }
                }
                tokio::time::sleep(interval).await;
            }
            remove_run_handles(&handles, run_id);
        });
    }
}

#[async_trait]
impl RunCancellationFactory for TurnStateRunCancellationFactory {
    async fn handle_for_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        let handle = RunCancellationHandle::default();
        let requester = handle.requester();
        self.register(run_id, requester.clone());
        // Register before the durable read so there is no missed-wake window:
        // a cancel that lands after registration flips this requester via the
        // wake notifier, while a cancel that landed earlier is observed by the
        // state read below. Any error/terminal path must drop the entry;
        // otherwise the caller never receives `handle`, the `Weak<()>` owner
        // dies, and the `RunCancellationRequester` leaks in `self.handles`
        // forever.
        match self.seed_from_state(scope, &requester, run_id).await {
            Ok(status) if status == TurnStatus::CancelRequested || status.is_terminal() => {
                self.remove_run(run_id);
            }
            Ok(_) => {
                self.spawn_polling_fallback(scope.clone(), run_id, requester);
            }
            Err(error) => {
                self.remove_run(run_id);
                return Err(error);
            }
        }
        Ok(handle)
    }

    async fn handle_for_claimed_run(
        &self,
        state: &TurnRunState,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        let handle = RunCancellationHandle::default();
        let requester = handle.requester();
        self.register(state.run_id, requester.clone());
        self.seed_requester_from_status(&requester, state.status);
        if state.status == TurnStatus::CancelRequested || state.status.is_terminal() {
            self.remove_run(state.run_id);
        } else {
            self.spawn_polling_fallback(state.scope.clone(), state.run_id, requester);
        }
        Ok(handle)
    }

    fn notify_run_wake(&self, wake: &TurnRunWake) {
        if wake.status == TurnStatus::CancelRequested {
            self.notify_cancel_requested(wake.run_id);
        } else if wake.status.is_terminal() {
            self.remove_run(wake.run_id);
        }
    }
}

impl TurnRunWakeNotifier for TurnStateRunCancellationFactory {
    fn notify_queued_run(
        &self,
        wake: TurnRunWake,
    ) -> Result<(), ironclaw_turns::TurnRunWakeNotifyError> {
        self.notify_run_wake(&wake);
        Ok(())
    }
}

fn turn_state_error_to_host_error(error: ironclaw_turns::TurnError) -> AgentLoopHostError {
    crate::raw_agent_loop_host_error(
        "turn_state_cancellation",
        "build_cancellation_handle",
        AgentLoopHostErrorKind::Unavailable,
        "turn state was unavailable while building cancellation handle",
        error,
    )
}

/// Fan-out `TurnRunWakeNotifier` that delivers each wake to a worker-side
/// notifier (e.g. the runner wake sender) AND a `RunCancellationFactory`'s
/// `notify_run_wake` observer so retained product run handles flip in lockstep
/// with the worker wake.
///
/// This is the wiring required to drive end-to-end cancellation observation
/// from `TurnCoordinator::cancel_run` alone: the coordinator publishes a single
/// `TurnRunWake`, and both consumers see it.
pub struct CompositeTurnRunWakeNotifier {
    worker: Arc<dyn TurnRunWakeNotifier>,
    cancellation_factory: Arc<dyn RunCancellationFactory>,
}

impl CompositeTurnRunWakeNotifier {
    pub fn new(
        worker: Arc<dyn TurnRunWakeNotifier>,
        cancellation_factory: Arc<dyn RunCancellationFactory>,
    ) -> Self {
        Self {
            worker,
            cancellation_factory,
        }
    }
}

impl TurnRunWakeNotifier for CompositeTurnRunWakeNotifier {
    fn notify_queued_run(
        &self,
        wake: TurnRunWake,
    ) -> Result<(), ironclaw_turns::TurnRunWakeNotifyError> {
        // Observe the wake on the cancellation factory FIRST so a retained
        // product run handle reflects the new status before any worker task
        // potentially terminates the run and clears local state.
        self.cancellation_factory.notify_run_wake(&wake);
        self.worker.notify_queued_run(wake)
    }
}

/// Default factory used until the host runtime wires real cancel observation.
pub struct AlwaysAliveRunCancellationFactory;

#[async_trait]
impl RunCancellationFactory for AlwaysAliveRunCancellationFactory {
    fn observation_kind(&self) -> RunCancellationObservationKind {
        RunCancellationObservationKind::InertFallback
    }

    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        _run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        Ok(RunCancellationHandle::default())
    }
}

fn remove_run_handles(
    handles: &RwLock<HashMap<TurnRunId, Vec<RunCancellationRequester>>>,
    run_id: TurnRunId,
) {
    handles.write().remove(&run_id);
}

#[cfg(test)]
mod tests;
