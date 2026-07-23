//! Scripted raw-provider seam for IronClaw integration tests. Reuses `TraceLlm`'s replay
//! engine: builds an in-memory `LlmTrace` from `IronClawScriptedReply`, canonicalizes
//! tool-call ids, and sits at the bottom of the real `ironclaw_llm` decorator chain
//! (design §3.1/§3.3).

// The parking provider (`ParkingModelGate`/`ParkingLlm`) is consumed only by the
// `ironclaw_integration_cancel` test binary, so it reads as dead in the
// `support_unit_tests` binary that compiles this tree without that consumer —
// matching the file-level allow every sibling support module already carries.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmError, LlmProvider,
    ToolCompletionRequest, ToolCompletionResponse,
};
use rust_decimal::Decimal;
use tokio::sync::oneshot;

use super::reply::IronClawScriptedReply;
use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceResponse, TraceStep, TraceTurn};

/// Model name surfaced by the scripted provider. Non-empty and not "default" so
/// the IronClaw model gateway's model-override resolution accepts it.
pub const SCRIPTED_MODEL_NAME: &str = "scripted/integration-test";

/// Build a `TraceLlm` that replays the given scripted replies in order.
pub fn scripted_trace_llm(replies: impl IntoIterator<Item = IronClawScriptedReply>) -> TraceLlm {
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

/// Synchronization handle for a [`ParkingLlm`]: the test waits until the model call parks,
/// then releases it. Uses `oneshot` (not `Notify`) so release-before-park and a second
/// `park()` call are both lost-wakeup-free — `Notify`'s single permit would deadlock the latter.
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

    /// Release the parked model call so it delegates to the inner trace.
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

/// A raw `LlmProvider` that parks the first model call until the test releases it, then
/// delegates to the inner scripted `TraceLlm`. Same vendor-SDK seam as `scripted_trace_llm`,
/// preserving the single-fake invariant.
pub struct ParkingLlm {
    inner: Arc<TraceLlm>,
    gate: ParkingModelGate,
}

/// Wraps `inner` (an `Arc<TraceLlm>`, not fresh-built) so the caller retains the same
/// trace handle the parked provider replays through.
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
// Recoverable model failures (E-GATEWAY seam) — model recovery coverage.
// ---------------------------------------------------------------------------

/// Distinctive provider-detail value used to prove context-overflow diagnostics
/// do not get copied into the model-visible recovery observation.
pub const CONTEXT_OVERFLOW_USED_TOKENS: usize = 987_654;

#[derive(Debug, Clone, Copy)]
pub enum RecoverableModelFailure {
    ContextOverflow,
    ContentFiltered,
    InvalidOutput,
}

#[derive(Debug, Clone, Copy)]
pub struct RecoverableModelFailureScript {
    pub failure: RecoverableModelFailure,
    pub successful_calls_before_failures: usize,
    pub failures: usize,
}

impl RecoverableModelFailureScript {
    pub fn new(failure: RecoverableModelFailure, failures: usize) -> Self {
        Self {
            failure,
            successful_calls_before_failures: 0,
            failures,
        }
    }

    pub fn after_successful_calls(mut self, calls: usize) -> Self {
        self.successful_calls_before_failures = calls;
        self
    }
}

#[derive(Default)]
struct ModelProviderCallRecords {
    interactive_requests: Vec<Vec<String>>,
    text_requests: Vec<Vec<String>>,
}

#[derive(Clone, Default)]
pub struct ModelProviderCallProbe(Arc<Mutex<ModelProviderCallRecords>>);

impl ModelProviderCallProbe {
    fn record(&self, messages: &[ironclaw_llm::ChatMessage], interactive: bool) {
        let contents = messages
            .iter()
            .map(|message| message.content.clone())
            .collect();
        let mut records = lock(&self.0);
        if interactive {
            records.interactive_requests.push(contents);
        } else {
            records.text_requests.push(contents);
        }
    }

    pub fn interactive_calls(&self) -> usize {
        lock(&self.0).interactive_requests.len()
    }

    pub fn text_calls(&self) -> usize {
        lock(&self.0).text_requests.len()
    }

    pub fn message_content_occurrences(&self, needle: &str) -> usize {
        let records = lock(&self.0);
        records
            .interactive_requests
            .iter()
            .chain(&records.text_requests)
            .flatten()
            .map(|content| content.matches(needle).count())
            .sum()
    }

    pub fn message_content_contains(&self, needle: &str) -> bool {
        let records = lock(&self.0);
        records
            .interactive_requests
            .iter()
            .chain(&records.text_requests)
            .flatten()
            .any(|content| content.contains(needle))
    }
}

/// A raw provider that reports a configured failure for interactive,
/// tool-capable calls a bounded number of times, then delegates to the scripted
/// provider. Text-only system inference still delegates normally so context
/// compaction can execute. The wrapper remains at the vendor-SDK seam so the
/// real decorator chain, model gateway, loop host, recovery strategy,
/// checkpointing, and prompt renderer all stay in the path.
pub struct RecoverableFailureLlm {
    inner: Arc<TraceLlm>,
    failure: RecoverableModelFailure,
    successful_calls_remaining: AtomicUsize,
    failures_remaining: AtomicUsize,
    calls: ModelProviderCallProbe,
}

pub fn recoverable_failure_trace_llm(
    failure: RecoverableModelFailure,
    successful_calls_before_failures: usize,
    failures: usize,
    inner: Arc<TraceLlm>,
) -> (RecoverableFailureLlm, ModelProviderCallProbe) {
    let calls = ModelProviderCallProbe::default();
    (
        RecoverableFailureLlm {
            inner,
            failure,
            successful_calls_remaining: AtomicUsize::new(successful_calls_before_failures),
            failures_remaining: AtomicUsize::new(failures),
            calls: calls.clone(),
        },
        calls,
    )
}

impl RecoverableFailureLlm {
    fn consume_scheduled_failure(&self) -> bool {
        if self
            .successful_calls_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return false;
        }
        self.failures_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
    }
}

#[async_trait]
impl LlmProvider for RecoverableFailureLlm {
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.calls.record(&request.messages, false);
        self.inner.complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.calls.record(&request.messages, true);
        if self.consume_scheduled_failure() {
            return match self.failure {
                RecoverableModelFailure::ContextOverflow => Err(LlmError::ContextLengthExceeded {
                    used: CONTEXT_OVERFLOW_USED_TOKENS,
                    limit: 1,
                }),
                RecoverableModelFailure::ContentFiltered => Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: Vec::new(),
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason: FinishReason::ContentFilter,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                    reasoning: None,
                    reasoning_details: None,
                }),
                RecoverableModelFailure::InvalidOutput => Ok(ToolCompletionResponse {
                    content: Some(String::new()),
                    tool_calls: Vec::new(),
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason: FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                    reasoning: None,
                    reasoning_details: None,
                }),
            };
        }
        self.inner.complete_with_tools(request).await
    }
}

// ---------------------------------------------------------------------------
// Fixed-error model provider (E-GATEWAY seam) — provider-`Err` failure
// category coverage (C-ERRORS).
// ---------------------------------------------------------------------------

/// Which fixed, non-retryable `LlmError` an [`ErrLlm`] provider fails with.
/// Both variants are excluded from `ironclaw_llm::retry::is_retryable` —
/// deliberately not `RequestFailed` (retryable, would add several seconds of
/// real backoff) — so the run fails promptly through the real decorator chain.
#[derive(Debug, Clone, Copy)]
pub enum ErrLlmKind {
    /// `LlmError::ContextLengthExceeded` — the batch-2 provider-fidelity
    /// `model_context_overflow` category arm.
    ContextLength,
    /// `LlmError::AuthFailed` — the credentials arm; maps through
    /// `map_provider_error` to `CredentialUnavailable` and must surface the
    /// pinned `model_credentials_unavailable` failure category.
    AuthFailed,
}

/// A raw `LlmProvider` that always fails with the fixed, non-retryable
/// `LlmError` selected by its [`ErrLlmKind`]. Same vendor-SDK seam as
/// `scripted_trace_llm`/`ParkingLlm`; proves the real decorator chain's
/// non-retryable-error mapping through to `TurnStatus::Failed`.
pub struct ErrLlm {
    kind: ErrLlmKind,
}

impl ErrLlm {
    pub fn new(kind: ErrLlmKind) -> Self {
        Self { kind }
    }

    fn make_error(&self) -> LlmError {
        match self.kind {
            ErrLlmKind::ContextLength => LlmError::ContextLengthExceeded { used: 1, limit: 1 },
            ErrLlmKind::AuthFailed => LlmError::AuthFailed {
                provider: "scripted".to_string(),
            },
        }
    }
}

#[async_trait]
impl LlmProvider for ErrLlm {
    fn model_name(&self) -> &str {
        SCRIPTED_MODEL_NAME
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(self.make_error())
    }

    async fn complete_with_tools(
        &self,
        _request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Err(self.make_error())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Covers two `ParkingModelGate` guarantees the committed cancel test doesn't exercise
    /// (it always releases after parking): (1) release-before-park is not a lost wakeup;
    /// (2) a second `park()` call after the channels are consumed returns immediately.
    #[tokio::test]
    async fn parking_llm_release_before_await_and_second_call_do_not_block() {
        let gate = ParkingModelGate::new();

        // Guarantee 1: release fires before park() exists to receive it.
        gate.release();
        tokio::time::timeout(Duration::from_secs(5), gate.park())
            .await
            .expect(
                "park() must resolve promptly when release() ran first \
                 (oneshot send is lost-wakeup free)",
            );

        // Guarantee 2: second park() call, after both channels are consumed.
        tokio::time::timeout(Duration::from_secs(5), gate.park())
            .await
            .expect("second park() call must return immediately, not block");
    }
}
