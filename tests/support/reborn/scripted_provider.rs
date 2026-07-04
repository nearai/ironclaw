//! Scripted raw-provider seam for Reborn integration tests. Reuses `TraceLlm`'s
//! replay engine (no new provider): builds an in-memory `LlmTrace` from the
//! `RebornScriptedReply` façade, canonicalizes harness-local tool-call ids per
//! trace, and returns a `TraceLlm` to sit at the bottom of the real
//! `ironclaw_llm` decorator chain (design §3.1/§3.3).

// The parking provider (`ParkingModelGate`/`ParkingLlm`) is consumed only by the
// `reborn_integration_cancel` test binary, so it reads as dead in the
// `support_unit_tests` binary that compiles this tree without that consumer —
// matching the file-level allow every sibling support module already carries.
#![allow(dead_code)]

use std::collections::HashMap;
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
use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceResponse, TraceStep, TraceTurn};

/// Model name surfaced by the scripted provider. Non-empty and not "default" so
/// the Reborn model gateway's model-override resolution accepts it.
pub const SCRIPTED_MODEL_NAME: &str = "scripted/integration-test";

/// Build a `TraceLlm` that replays the given scripted replies in order.
pub fn scripted_trace_llm(replies: impl IntoIterator<Item = RebornScriptedReply>) -> TraceLlm {
    let mut tool_call_ids = ScriptedToolCallIds::default();
    let steps = replies
        .into_iter()
        .map(|reply| {
            let mut step = reply.into_step();
            tool_call_ids.canonicalize_step(&mut step);
            step
        })
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

#[derive(Default)]
struct ScriptedToolCallIds {
    next: u32,
    canonical_by_raw: HashMap<String, String>,
}

impl ScriptedToolCallIds {
    fn canonicalize_step(&mut self, step: &mut TraceStep) {
        if let TraceResponse::ToolCalls { tool_calls, .. } = &mut step.response {
            for tool_call in tool_calls {
                let canonical = self.next_id();
                self.canonical_by_raw
                    .insert(tool_call.id.clone(), canonical.clone());
                tool_call.id = canonical;
            }
        }

        for expected in &mut step.expected_tool_results {
            if let Some(canonical) = self.canonical_by_raw.get(&expected.tool_call_id) {
                expected.tool_call_id = canonical.clone();
            }
        }
    }

    fn next_id(&mut self) -> String {
        self.next += 1;
        format!("call-{}", self.next)
    }
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

// ---------------------------------------------------------------------------
// Fixed-error model provider (E-GATEWAY seam) — provider-`Err` failure
// category coverage (C-ERRORS).
// ---------------------------------------------------------------------------

/// A raw `LlmProvider` that always fails with a fixed, NON-retryable
/// `LlmError::ContextLengthExceeded`. Deliberately NOT the `LlmError::RequestFailed`
/// a naturally-exhausted `TraceLlm` returns (see `next_step` above) — `RequestFailed`
/// IS retryable (`ironclaw_llm::retry::is_retryable`), so scripting it would drive
/// several seconds of real exponential backoff (1s/2s/4s) before the run finally
/// failed. `ContextLengthExceeded` is excluded from `is_retryable`, so the run
/// fails on the first model call — fast and deterministic. Sits at the same
/// vendor-SDK seam `scripted_trace_llm`/`ParkingLlm` fill; the real `ironclaw_llm`
/// decorator chain still runs on top, so this proves the chain's non-retryable-error
/// mapping through to a terminal `TurnStatus::Failed`, not just the seam itself.
pub struct ErrLlm;

#[async_trait]
impl LlmProvider for ErrLlm {
    fn model_name(&self) -> &str {
        SCRIPTED_MODEL_NAME
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::ContextLengthExceeded { used: 1, limit: 1 })
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(LlmError::ContextLengthExceeded { used: 1, limit: 1 })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Enforces both concurrency guarantees documented on `ParkingModelGate`
    /// (module docs above) that the committed `reborn_integration_cancel`
    /// integration test does not exercise directly, since it always calls
    /// `release()` *after* the provider has already parked.
    ///
    /// Guarantee 1 — release-before-await ordering: `release()` sends on a
    /// `oneshot::Sender` and `oneshot::Sender::send` buffers the value
    /// regardless of whether a receiver is already awaiting, so calling
    /// `release()` before `park()` has ever run must not be a lost wakeup —
    /// the eventual `park()` call must still resolve promptly rather than
    /// hanging forever waiting on `release_rx`.
    ///
    /// Guarantee 2 — second call does not block: `parked_tx`/`release_tx` are
    /// `Mutex<Option<..>>` `take()`-based single-shot channels, so once the
    /// first `park()` call has consumed them, a second `park()` call (e.g.
    /// simulating a retry/failover hop in the real decorator chain) must
    /// return immediately instead of blocking on an already-consumed
    /// channel.
    #[tokio::test]
    async fn parking_llm_release_before_await_and_second_call_do_not_block() {
        let gate = ParkingModelGate::new();

        // Guarantee 1: release fires before any `park()` call exists to
        // receive it.
        gate.release();
        tokio::time::timeout(Duration::from_secs(5), gate.park())
            .await
            .expect(
                "park() must resolve promptly when release() ran first \
                 (oneshot send is lost-wakeup free)",
            );

        // Guarantee 2: a second park() call, after the first full
        // park+release cycle already consumed both channels, must not
        // block.
        tokio::time::timeout(Duration::from_secs(5), gate.park())
            .await
            .expect("second park() call must return immediately, not block");
    }
}
