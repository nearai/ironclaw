//! Tool-path analog of `ParkingModelGate`/`ParkingLlm`
//! (`support/scripted_provider.rs`). Parks a `HostRuntime` capability
//! dispatch until released, wrapping the SAME `HostRuntime` trait-object seam
//! `RecordingHostRuntime` already wraps in this file's sibling module — used
//! by `tests/integration/lease_wedge.rs` to cover lease-expiry recovery of a
//! wedged in-flight tool call (issue #5476).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeCapabilityAuthResumeRequest,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest,
    RuntimeStatusRequest, VisibleCapabilityRequest as RuntimeVisibleCapabilityRequest,
    VisibleCapabilitySurface as RuntimeVisibleCapabilitySurface,
};
use tokio::sync::oneshot;

/// Synchronization handle for a [`ParkingHostRuntime`]: the test waits until the
/// capability dispatch parks, then releases it. Uses `oneshot` (not `Notify`) so
/// release-before-park and a second `park()` call are both lost-wakeup-free —
/// `Notify`'s single permit would deadlock the latter. Verbatim mirror of
/// `scripted_provider::ParkingModelGate`, renamed for the tool-dispatch seam.
#[derive(Clone)]
pub(crate) struct ParkingCapabilityGate(Arc<ParkingState>);

struct ParkingState {
    parked_tx: Mutex<Option<oneshot::Sender<()>>>,
    parked_rx: Mutex<Option<oneshot::Receiver<()>>>,
    release_tx: Mutex<Option<oneshot::Sender<()>>>,
    release_rx: Mutex<Option<oneshot::Receiver<()>>>,
}

impl ParkingCapabilityGate {
    pub(crate) fn new() -> Self {
        let (parked_tx, parked_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        Self(Arc::new(ParkingState {
            parked_tx: Mutex::new(Some(parked_tx)),
            parked_rx: Mutex::new(Some(parked_rx)),
            release_tx: Mutex::new(Some(release_tx)),
            release_rx: Mutex::new(Some(release_rx)),
        }))
    }

    /// Await until the parked capability dispatch has signalled it is blocked.
    /// Returns immediately on any subsequent call (the channel is consumed once).
    pub(crate) async fn wait_until_parked(&self) {
        let rx = lock(&self.0.parked_rx).take();
        if let Some(rx) = rx {
            rx.await
                .expect("parking host runtime dropped before signalling parked");
        }
    }

    /// Release the parked capability dispatch so it delegates to the inner runtime.
    pub(crate) fn release(&self) {
        if let Some(tx) = lock(&self.0.release_tx).take() {
            let _ = tx.send(());
        }
    }

    /// Runtime side: signal parked, then block until `release()` fires. A plain
    /// cooperative `.await` — no OS-thread blocking — so this stays safe under the
    /// default single-threaded `#[tokio::test]` flavor every sibling integration
    /// test uses. It is not meant to reproduce a blocked-worker-thread hazard;
    /// only to hold the tool call open long enough for the run's real, short-TTL
    /// (test-only) scheduler lease to expire before its own next heartbeat.
    async fn park(&self) {
        if let Some(tx) = lock(&self.0.parked_tx).take() {
            let _ = tx.send(());
        }
        let rx = lock(&self.0.release_rx).take();
        if let Some(rx) = rx {
            rx.await
                .expect("parking gate dropped before releasing the parked capability dispatch");
        }
    }
}

impl Default for ParkingCapabilityGate {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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

    use super::*;

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
}
