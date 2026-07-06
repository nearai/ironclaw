//! Tool-path analog of `ParkingModelGate`/`ParkingLlm`
//! (`support/scripted_provider.rs`). Parks a `HostRuntime` capability
//! dispatch until released, wrapping the SAME `HostRuntime` trait-object seam
//! `RecordingHostRuntime` already wraps in this file's sibling module — used
//! by `tests/integration/lease_wedge.rs` to cover lease-expiry recovery of a
//! wedged in-flight tool call (issue #5476).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityAuthResumeRequest,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
    RuntimeStatusRequest, VisibleCapabilityRequest as RuntimeVisibleCapabilityRequest,
    VisibleCapabilitySurface as RuntimeVisibleCapabilitySurface,
};
use tokio::sync::watch;

/// Synchronization handle for a [`ParkingHostRuntime`]: the test waits until the
/// capability dispatch parks, then releases it. Uses `watch` (not `oneshot`) so
/// both signals are level-triggered flags rather than one-shot consumables —
/// every waiter, including a second/concurrent `wait_until_parked()` call, sees
/// the same state instead of racing a single `.take()`.
#[derive(Clone)]
pub(crate) struct ParkingCapabilityGate(Arc<ParkingState>);

struct ParkingState {
    parked_tx: watch::Sender<bool>,
    parked_rx: watch::Receiver<bool>,
    release_tx: watch::Sender<bool>,
    release_rx: watch::Receiver<bool>,
}

impl ParkingCapabilityGate {
    pub(crate) fn new() -> Self {
        let (parked_tx, parked_rx) = watch::channel(false);
        let (release_tx, release_rx) = watch::channel(false);
        Self(Arc::new(ParkingState {
            parked_tx,
            parked_rx,
            release_tx,
            release_rx,
        }))
    }

    /// Await until the parked capability dispatch has signalled it is blocked.
    /// Idempotent and safe to call concurrently or repeatedly: each call clones
    /// its own receiver and checks the current flag before waiting on a change.
    pub(crate) async fn wait_until_parked(&self) {
        let mut rx = self.0.parked_rx.clone();
        while !*rx.borrow() {
            let _ = rx.changed().await;
        }
    }

    /// Release the parked capability dispatch so it delegates to the inner runtime.
    pub(crate) fn release(&self) {
        let _ = self.0.release_tx.send(true);
    }

    /// Guarantees `release()` runs even if a test assertion panics first, so a
    /// parked dispatch is never leaked for the rest of the process's lifetime.
    pub(crate) fn release_guard(&self) -> ParkingCapabilityGateReleaseGuard {
        ParkingCapabilityGateReleaseGuard(self.clone())
    }

    /// Runtime side: signal parked, then block until `release()` fires. A plain
    /// cooperative `.await` — no OS-thread blocking — so this stays safe under the
    /// default single-threaded `#[tokio::test]` flavor every sibling integration
    /// test uses. It is not meant to reproduce a blocked-worker-thread hazard;
    /// only to hold the tool call open long enough for the run's real, short-TTL
    /// (test-only) scheduler lease to expire before its own next heartbeat.
    async fn park(&self) {
        let _ = self.0.parked_tx.send(true);
        let mut rx = self.0.release_rx.clone();
        while !*rx.borrow() {
            let _ = rx.changed().await;
        }
    }
}

impl Default for ParkingCapabilityGate {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard returned by [`ParkingCapabilityGate::release_guard`]: releases
/// the gate on drop so a test that panics before an explicit `release()` call
/// never leaves the parked dispatch running for the rest of the process.
pub(crate) struct ParkingCapabilityGateReleaseGuard(ParkingCapabilityGate);

impl Drop for ParkingCapabilityGateReleaseGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// Wraps `inner` (the harness's already-composed `Arc<dyn HostRuntime>`, typically
/// `RecordingHostRuntime` over the real runtime) so parking sits outside the
/// existing recorder, at the same `HostRuntime` trait-object seam
/// `RecordingHostRuntime` uses. Only the two "fresh dispatch" entry points
/// (`invoke_capability`, `spawn_capability`) park — mirrors `ParkingLlm` parking
/// only `complete`/`complete_with_tools`, not any resume path. Every other method
/// forwards to `inner`, matching `RecordingHostRuntime`'s own forwarding.
pub(crate) struct ParkingHostRuntime {
    inner: Arc<dyn HostRuntime>,
    gate: ParkingCapabilityGate,
}

impl ParkingHostRuntime {
    pub(crate) fn new(inner: Arc<dyn HostRuntime>, gate: ParkingCapabilityGate) -> Self {
        Self { inner, gate }
    }
}

#[async_trait]
impl HostRuntime for ParkingHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.gate.park().await;
        self.inner.invoke_capability(request).await
    }

    async fn spawn_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.gate.park().await;
        self.inner.spawn_capability(request).await
    }

    async fn resume_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_capability(request).await
    }

    async fn auth_resume_capability(
        &self,
        request: RuntimeCapabilityAuthResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.auth_resume_capability(request).await
    }

    async fn resume_spawn_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_spawn_capability(request).await
    }

    async fn visible_capabilities(
        &self,
        request: RuntimeVisibleCapabilityRequest,
    ) -> Result<RuntimeVisibleCapabilitySurface, HostRuntimeError> {
        self.inner.visible_capabilities(request).await
    }

    async fn cancel_work(
        &self,
        request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        self.inner.cancel_work(request).await
    }

    async fn runtime_status(
        &self,
        request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        self.inner.runtime_status(request).await
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        self.inner.health().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;
    use ironclaw_host_api::{
        CapabilityId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId, MountView,
        ResourceEstimate, RuntimeKind, TrustClass, UserId,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use serde_json::Value;

    use super::*;

    /// Minimal `HostRuntime` stub: every method the test doesn't drive panics,
    /// so an accidental call to the wrong entry point fails loudly rather than
    /// silently returning a stub value.
    struct StubHostRuntime;

    #[async_trait]
    impl HostRuntime for StubHostRuntime {
        async fn invoke_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            panic!("test does not drive invoke_capability");
        }

        async fn spawn_capability(
            &self,
            _request: RuntimeCapabilityRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            Err(HostRuntimeError::unavailable("stub forwards after release"))
        }

        async fn resume_capability(
            &self,
            _request: RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            panic!("test does not drive resume_capability");
        }

        async fn auth_resume_capability(
            &self,
            _request: RuntimeCapabilityAuthResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            panic!("test does not drive auth_resume_capability");
        }

        async fn resume_spawn_capability(
            &self,
            _request: RuntimeCapabilityResumeRequest,
        ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
            panic!("test does not drive resume_spawn_capability");
        }

        async fn visible_capabilities(
            &self,
            _request: RuntimeVisibleCapabilityRequest,
        ) -> Result<RuntimeVisibleCapabilitySurface, HostRuntimeError> {
            panic!("test does not drive visible_capabilities");
        }

        async fn cancel_work(
            &self,
            _request: CancelRuntimeWorkRequest,
        ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
            panic!("test does not drive cancel_work");
        }

        async fn runtime_status(
            &self,
            _request: RuntimeStatusRequest,
        ) -> Result<HostRuntimeStatus, HostRuntimeError> {
            panic!("test does not drive runtime_status");
        }

        async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
            panic!("test does not drive health");
        }
    }

    fn stub_capability_request() -> RuntimeCapabilityRequest {
        let context = ExecutionContext::local_default(
            UserId::new("user").unwrap(),
            ExtensionId::new("caller").unwrap(),
            RuntimeKind::Wasm,
            TrustClass::UserTrusted,
            CapabilitySet::default(),
            MountView::default(),
        )
        .unwrap();
        RuntimeCapabilityRequest::new(
            context,
            CapabilityId::new("echo.say").unwrap(),
            ResourceEstimate::default(),
            Value::Null,
            TrustDecision {
                effective_trust: EffectiveTrustClass::user_trusted(),
                authority_ceiling: AuthorityCeiling {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    max_resource_ceiling: None,
                },
                provenance: TrustProvenance::Default,
                evaluated_at: Utc::now(),
            },
        )
    }

    /// `ParkingHostRuntime` parks both fresh-dispatch entry points
    /// (`invoke_capability`, `spawn_capability`), but only `invoke_capability`
    /// is exercised by the wedge integration test (`lease_wedge.rs` dispatches
    /// a tool call, never a subagent spawn). This drives `spawn_capability`
    /// directly: it must park until released, then forward to `inner`.
    #[tokio::test]
    async fn spawn_capability_parks_until_released_then_forwards_to_inner() {
        let gate = ParkingCapabilityGate::new();
        let runtime = ParkingHostRuntime::new(Arc::new(StubHostRuntime), gate.clone());

        let call =
            tokio::spawn(async move { runtime.spawn_capability(stub_capability_request()).await });

        tokio::time::timeout(Duration::from_secs(5), gate.wait_until_parked())
            .await
            .expect("spawn_capability must park before the timeout");
        gate.release();

        let outcome = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .expect("released spawn_capability must complete before the timeout")
            .expect("spawn_capability task must not panic");
        assert!(
            matches!(outcome, Err(HostRuntimeError::Unavailable { .. })),
            "released spawn_capability must forward to the inner runtime, got {outcome:?}"
        );
    }

    /// Covers the same two `ParkingCapabilityGate` guarantees
    /// `ParkingModelGate`'s own unit test covers (see `scripted_provider.rs`):
    /// release-before-park is not a lost wakeup, and a second `park()` call after
    /// the channels are consumed returns immediately. The wedge test itself never
    /// exercises the "release before park" ordering (it always parks first), so
    /// this is the only place that guarantee is checked.
    #[tokio::test]
    async fn parking_gate_release_before_await_and_second_call_do_not_block() {
        let gate = ParkingCapabilityGate::new();

        gate.release();
        tokio::time::timeout(Duration::from_secs(5), gate.park())
            .await
            .expect("release-before-park must not deadlock");

        tokio::time::timeout(Duration::from_secs(5), gate.park())
            .await
            .expect("second park() after channels are consumed must return immediately");
    }

    /// Pins the `watch`-based rewrite's idempotent-wait guarantee: unlike a
    /// `oneshot` + `.take()`, repeated `wait_until_parked` calls each check the
    /// current flag rather than consuming a channel, so every call — not just
    /// the first — correctly observes the park.
    #[tokio::test]
    async fn wait_until_parked_is_idempotent_across_repeated_calls() {
        let gate = ParkingCapabilityGate::new();
        let park_gate = gate.clone();
        tokio::spawn(async move { park_gate.park().await });

        tokio::time::timeout(Duration::from_secs(5), gate.wait_until_parked())
            .await
            .expect("first wait_until_parked call must observe the park");
        tokio::time::timeout(Duration::from_secs(5), gate.wait_until_parked())
            .await
            .expect("second wait_until_parked call must also observe the park");

        gate.release();
    }
}
