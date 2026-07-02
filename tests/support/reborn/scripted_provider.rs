//! Scripted raw-provider seam for Reborn integration tests. Reuses `TraceLlm`'s
//! replay engine (no new provider): builds an in-memory `LlmTrace` from the
//! `RebornScriptedReply` façade and returns a `TraceLlm` to sit at the bottom of
//! the real `ironclaw_llm` decorator chain (design §3.1/§3.3).

// The parking provider (`ParkingModelGate`/`ParkingLlm`) is consumed only by the
// `reborn_integration_cancel` test binary, so it reads as dead in the
// `support_unit_tests` binary that compiles this tree without that consumer —
// matching the file-level allow every sibling support module already carries.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw::error::LlmError;
use ironclaw_llm::{
    CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};
use rust_decimal::Decimal;
use tokio::sync::oneshot;

use super::reply::RebornScriptedReply;
use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceTurn};

/// Model name surfaced by the scripted provider. Non-empty and not "default" so
/// the Reborn model gateway's model-override resolution accepts it.
pub const SCRIPTED_MODEL_NAME: &str = "scripted/integration-test";

/// Build a `TraceLlm` that replays the given scripted replies in order.
pub fn scripted_trace_llm(replies: impl IntoIterator<Item = RebornScriptedReply>) -> TraceLlm {
    let steps = replies
        .into_iter()
        .map(RebornScriptedReply::into_step)
        .collect();
    let trace = LlmTrace::new(
        SCRIPTED_MODEL_NAME,
        vec![TraceTurn {
            user_input: "(scripted)".to_string(),
            steps,
            expects: Default::default(),
        }],
    );
    TraceLlm::from_trace(trace)
}

// ---------------------------------------------------------------------------
// Parking model provider (E-GATEWAY seam) — mid-turn cancel coverage.
// ---------------------------------------------------------------------------

/// Synchronization handle for a [`ParkingLlm`]: the test waits until the model
/// call parks, then releases it. Cloneable (shares one [`ParkingState`] over an
/// `Arc`), so the test keeps a handle while a clone lives inside the provider.
///
/// Uses `oneshot` channels rather than `Notify` so signalling is lost-wakeup
/// free: `oneshot::Sender::send` stores the value regardless of whether the
/// receiver is already awaiting, so `release()` may run before or after the
/// provider reaches its `await` without racing. The first model call parks; a
/// second call (if any) delegates immediately.
///
/// The `take()`-based single-shot design is deliberate and idempotent: a second
/// `park()` (e.g. from a retry/failover hop in the real `ironclaw_llm` decorator
/// chain this provider sits under) finds its channel already consumed and
/// returns immediately rather than blocking. A plain `Notify` pair would
/// *deadlock* that second `park()` — `Notify` stores only one permit, so once
/// the single `release` permit is consumed there is nothing left to wake a
/// second waiter.
#[derive(Clone)]
pub struct ParkingModelGate(Arc<ParkingState>);

struct ParkingState {
    parked_tx: Mutex<Option<oneshot::Sender<()>>>,
    parked_rx: Mutex<Option<oneshot::Receiver<()>>>,
    release_tx: Mutex<Option<oneshot::Sender<()>>>,
    release_rx: Mutex<Option<oneshot::Receiver<()>>>,
}

impl ParkingModelGate {
    pub fn new() -> Self {
        let (parked_tx, parked_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        Self(Arc::new(ParkingState {
            parked_tx: Mutex::new(Some(parked_tx)),
            parked_rx: Mutex::new(Some(parked_rx)),
            release_tx: Mutex::new(Some(release_tx)),
            release_rx: Mutex::new(Some(release_rx)),
        }))
    }

    /// Await until the parked model call has signalled it is blocked. Returns
    /// immediately on any subsequent call (the channel is consumed once).
    pub async fn wait_until_parked(&self) {
        let rx = lock(&self.0.parked_rx).take();
        if let Some(rx) = rx {
            rx.await
                .expect("parking model provider dropped before signalling parked");
        }
    }

    /// Release the parked model call so it delegates to the inner trace and the
    /// turn proceeds.
    pub fn release(&self) {
        if let Some(tx) = lock(&self.0.release_tx).take() {
            let _ = tx.send(());
        }
    }

    /// Provider side: signal parked, then block until `release()` fires.
    async fn park(&self) {
        if let Some(tx) = lock(&self.0.parked_tx).take() {
            let _ = tx.send(());
        }
        let rx = lock(&self.0.release_rx).take();
        if let Some(rx) = rx {
            rx.await
                .expect("parking gate dropped before releasing the parked model call");
        }
    }
}

impl Default for ParkingModelGate {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A raw `LlmProvider` that parks the first model call until the test releases
/// it, then delegates to the inner scripted [`TraceLlm`]. Sits at the same
/// vendor-SDK seam `scripted_trace_llm` fills, preserving the tier's
/// single-fake invariant (the real `ironclaw_llm` decorator chain still runs
/// on top).
pub struct ParkingLlm {
    inner: Arc<TraceLlm>,
    gate: ParkingModelGate,
}

/// Build a parking provider wrapping the already-built `inner` trace, released
/// via `gate`. Takes an `Arc<TraceLlm>` (rather than building its own from raw
/// replies) so the caller retains the SAME trace handle the parked provider
/// replays through — parking mode is only a wrapper around the same scripted
/// provider, not a separate trace.
pub fn parking_trace_llm(gate: ParkingModelGate, inner: Arc<TraceLlm>) -> ParkingLlm {
    ParkingLlm { inner, gate }
}

#[async_trait]
impl LlmProvider for ParkingLlm {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.gate.park().await;
        self.inner.complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.gate.park().await;
        self.inner.complete_with_tools(request).await
    }
}
