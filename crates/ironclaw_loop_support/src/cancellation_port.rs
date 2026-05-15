use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_turns::{
    TurnRunId,
    run_profile::{
        AgentLoopHostError, LoopCancelReasonKind, LoopCancellationPort, LoopCancellationSignal,
    },
};
use parking_lot::RwLock;

/// Snapshot handle the host runtime owns and flips on cancellation.
#[derive(Clone, Default)]
pub struct RunCancellationHandle {
    fired: Arc<AtomicBool>,
    signal: Arc<RwLock<Option<LoopCancellationSignal>>>,
}

impl RunCancellationHandle {
    pub fn request(&self, reason_kind: LoopCancelReasonKind) {
        if self.fired.load(Ordering::Acquire) {
            return;
        }
        let mut signal_lock = self.signal.write();
        if signal_lock.is_some() {
            return;
        }
        let signal = LoopCancellationSignal {
            reason_kind,
            requested_at: Utc::now(),
        };
        *signal_lock = Some(signal);
        self.fired.store(true, Ordering::Release);
    }

    pub fn is_requested(&self) -> bool {
        self.fired.load(Ordering::Acquire)
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

impl LoopCancellationPort for RunStateLoopCancellationPort {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        if !self.handle.fired.load(Ordering::Acquire) {
            return None;
        }
        self.handle.signal.read().clone()
    }
}

/// Always reports "not cancelled".
pub struct AlwaysAliveLoopCancellationPort;

impl LoopCancellationPort for AlwaysAliveLoopCancellationPort {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        None
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
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError>;
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

/// Default factory used until the host runtime wires real cancel observation.
pub struct AlwaysAliveRunCancellationFactory;

#[async_trait]
impl RunCancellationFactory for AlwaysAliveRunCancellationFactory {
    fn observation_kind(&self) -> RunCancellationObservationKind {
        RunCancellationObservationKind::InertFallback
    }

    async fn handle_for_run(
        &self,
        _run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        Ok(RunCancellationHandle::default())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_turns::run_profile::{LoopCancelReasonKind, LoopCancellationPort};
    use ironclaw_turns::{TurnRunId, run_profile::AgentLoopHostError};

    use super::{
        AlwaysAliveLoopCancellationPort, AlwaysAliveRunCancellationFactory, RunCancellationFactory,
        RunCancellationHandle, RunCancellationObservationKind, RunStateLoopCancellationPort,
    };

    struct TestLiveCancellationFactory;

    #[async_trait]
    impl RunCancellationFactory for TestLiveCancellationFactory {
        async fn handle_for_run(
            &self,
            _run_id: TurnRunId,
        ) -> Result<RunCancellationHandle, AgentLoopHostError> {
            Ok(RunCancellationHandle::default())
        }
    }

    #[test]
    fn observe_returns_none_when_not_requested() {
        let port = RunStateLoopCancellationPort::new(RunCancellationHandle::default());

        assert_eq!(port.observe_cancellation(), None);
    }

    #[test]
    fn observe_returns_signal_after_flip() {
        let handle = RunCancellationHandle::default();
        let port = RunStateLoopCancellationPort::new(handle.clone());

        handle.request(LoopCancelReasonKind::UserRequested);

        let signal = port.observe_cancellation().expect("signal");
        assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
        assert!(handle.is_requested());
    }

    #[test]
    fn observe_idempotent_after_first_read() {
        let handle = RunCancellationHandle::default();
        let port = RunStateLoopCancellationPort::new(handle.clone());

        handle.request(LoopCancelReasonKind::Superseded);

        let first = port.observe_cancellation();
        assert_eq!(port.observe_cancellation(), first);
        assert_eq!(port.observe_cancellation(), first);
    }

    #[test]
    fn duplicate_request_preserves_first_signal() {
        let handle = RunCancellationHandle::default();
        let port = RunStateLoopCancellationPort::new(handle.clone());

        handle.request(LoopCancelReasonKind::UserRequested);
        let first = port.observe_cancellation().expect("first signal");

        handle.request(LoopCancelReasonKind::Policy);

        let second = port.observe_cancellation().expect("second signal");
        assert_eq!(second, first);
        assert_eq!(second.reason_kind, LoopCancelReasonKind::UserRequested);
    }

    #[test]
    fn observe_payload_includes_requested_at() {
        let handle = RunCancellationHandle::default();
        let port = RunStateLoopCancellationPort::new(handle.clone());
        let before = Utc::now();

        handle.request(LoopCancelReasonKind::Policy);

        let after = Utc::now();
        let signal = port.observe_cancellation().expect("signal");
        assert!(signal.requested_at >= before);
        assert!(signal.requested_at <= after + chrono::Duration::seconds(5));
    }

    #[test]
    fn handle_signal_visible_after_atomic_load() {
        let handle = RunCancellationHandle::default();
        let port = Arc::new(RunStateLoopCancellationPort::new(handle.clone()));
        let observer = Arc::clone(&port);

        let join = std::thread::spawn(move || {
            for _ in 0..10_000 {
                if let Some(signal) = observer.observe_cancellation() {
                    return signal;
                }
                std::thread::sleep(Duration::from_micros(50));
            }
            panic!("observer did not see cancellation signal");
        });

        handle.request(LoopCancelReasonKind::UserRequested);

        let signal = join.join().expect("observer thread");
        assert_eq!(signal.reason_kind, LoopCancelReasonKind::UserRequested);
    }

    #[test]
    fn always_alive_port_returns_none() {
        let port = AlwaysAliveLoopCancellationPort;

        assert_eq!(port.observe_cancellation(), None);
    }

    #[tokio::test]
    async fn always_alive_factory_is_identified_as_inert_fallback() {
        let factory = AlwaysAliveRunCancellationFactory;

        assert_eq!(
            factory.observation_kind(),
            RunCancellationObservationKind::InertFallback
        );
        assert!(!factory.observation_kind().is_live_capable());

        let handle = factory.handle_for_run(TurnRunId::new()).await.unwrap();
        assert!(!handle.is_requested());
    }

    #[test]
    fn custom_run_cancellation_factory_defaults_to_live_capable() {
        let factory = TestLiveCancellationFactory;

        assert_eq!(
            factory.observation_kind(),
            RunCancellationObservationKind::LiveCapable
        );
        assert!(factory.observation_kind().is_live_capable());
    }
}
