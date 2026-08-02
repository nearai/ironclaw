//! The host-managed [`LoopModelPort`] implementation.
//!
//! This is an adapter, not a contract: it wraps a `LoopModelGateway` with the
//! idle watchdog, budget accounting, and milestone emission a real host needs.
//! It stays in the turn kernel only until the WS4 `loop_host` re-charter
//! absorbs it (CHECKLIST WS4, PROPOSAL §6.7.2); the contract it implements
//! lives in `ironclaw_loop_contracts`.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_loop_contracts::{
    AgentLoopHostError, LoopHostMilestoneEmitter, LoopHostMilestoneSink, LoopModelBudgetAccountant,
    LoopModelGateway, LoopModelGatewayError, LoopModelGatewayRequest, LoopModelPolicyGuard,
    LoopModelPort, LoopModelProgressSink, LoopModelRequest, LoopModelResponse, LoopRunContext,
    ModelCallOutcome, ModelWorkRequest, NoOpBudgetAccountant, NoOpPolicyGuard, ParentLoopOutput,
    sanitize_model_visible_text,
};

/// Maximum idle period for a primary assistant model call.
///
/// This is a defense-in-depth bound for every provider, not just NEAR AI. Text
/// progress resets the watchdog so a healthy long response is not cancelled.
/// It MUST stay below the process runner lease (90s by default) so a hung
/// provider is surfaced as a retryable `Unavailable` error before the lease
/// reclaims the runner mid-flight — the failure mode that wedged the Reborn
/// runtime on 2026-06-24. The invariant is enforced by
/// `primary_model_call_idle_timeout_is_below_runner_lease` below.
///
const PRIMARY_MODEL_CALL_IDLE_TIMEOUT: Duration = Duration::from_secs(75);
const FALLBACK_TEXT_DELTA_MILESTONE_STEP: usize = 15;

#[derive(Clone)]
pub struct HostManagedLoopModelPort<G, S>
where
    G: LoopModelGateway + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    context: LoopRunContext,
    gateway: Arc<G>,
    milestones: LoopHostMilestoneEmitter<S>,
    accountant: Arc<dyn LoopModelBudgetAccountant>,
    policy_guard: Arc<dyn LoopModelPolicyGuard>,
}

impl<G, S> HostManagedLoopModelPort<G, S>
where
    G: LoopModelGateway + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    pub fn new(context: LoopRunContext, gateway: Arc<G>, milestone_sink: Arc<S>) -> Self {
        let milestones = LoopHostMilestoneEmitter::new(context.clone(), milestone_sink);
        Self {
            context,
            gateway,
            milestones,
            accountant: Arc::new(NoOpBudgetAccountant),
            policy_guard: Arc::new(NoOpPolicyGuard),
        }
    }

    /// Create a port with a custom budget accountant injected.
    pub fn with_accountant(
        context: LoopRunContext,
        gateway: Arc<G>,
        milestone_sink: Arc<S>,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
    ) -> Self {
        let milestones = LoopHostMilestoneEmitter::new(context.clone(), milestone_sink);
        Self {
            context,
            gateway,
            milestones,
            accountant,
            policy_guard: Arc::new(NoOpPolicyGuard),
        }
    }

    /// Create a fully-configured port with policy guard and budget accountant.
    pub fn with_guards(
        context: LoopRunContext,
        gateway: Arc<G>,
        milestone_sink: Arc<S>,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
        policy_guard: Arc<dyn LoopModelPolicyGuard>,
    ) -> Self {
        let milestones = LoopHostMilestoneEmitter::new(context.clone(), milestone_sink);
        Self {
            context,
            gateway,
            milestones,
            accountant,
            policy_guard,
        }
    }
}

#[async_trait]
impl<G, S> LoopModelPort for HostManagedLoopModelPort<G, S>
where
    G: LoopModelGateway + ?Sized,
    S: LoopHostMilestoneSink + ?Sized + 'static,
{
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        let work_request = ModelWorkRequest::for_assistant(&self.context, &request);

        // Policy check — rejects before any provider or credential is touched.
        if let Err(policy_error) = self
            .policy_guard
            .check_model_work_policy(&self.context, &work_request)
            .await
        {
            return Err(policy_error.into_host_error());
        }

        // Pre-call budget check — rejects before touching the provider.
        if let Err(budget_error) = self
            .accountant
            .pre_model_work(&self.context, &work_request)
            .await
        {
            return Err(budget_error.into_host_error());
        }

        // From here forward, a reservation has been taken. The guard below
        // ensures it is released if `stream_model` is cancelled mid-await
        // (tokio drop, parent timeout) without ever reaching the explicit
        // `post_model_call` below.
        let mut release_guard =
            ReservationReleaseGuard::new(self.accountant.as_ref(), &self.context);

        log_milestone_failure(
            self.milestones
                .model_started(request.model_preference.clone())
                .await,
            "loop model_started milestone failed before model request",
        );

        // Bound inactivity, rather than total response time, so a hung gateway
        // fails before lease expiry without killing a healthy long stream.
        let (progress_generation, progress_updates) = tokio::sync::watch::channel(0_u64);
        let progress_sink = Arc::new(MilestoneModelProgressSink {
            milestones: self.milestones.clone(),
            emitted_text: AtomicBool::new(false),
            progress_generation,
        });
        let gateway_call = self.gateway.stream_model_with_progress(
            LoopModelGatewayRequest {
                context: self.context.clone(),
                request: request.clone(),
            },
            progress_sink.clone(),
        );
        let gateway_result = match await_with_progress_timeout(
            gateway_call,
            progress_updates,
            PRIMARY_MODEL_CALL_IDLE_TIMEOUT,
        )
        .await
        {
            Ok(result) => result.map(sanitize_model_response),
            Err(()) => Err(LoopModelGatewayError::timed_out()),
        };

        // Post-call accounting fires on BOTH success and failure. The
        // RAII guard stays armed across this await — if the future is
        // cancelled mid-`post_model_call`, the Drop path calls
        // `release_in_flight` to clean up. `release_in_flight` is
        // idempotent against a successful post-call (the in-flight
        // entry is already gone), so disarming after success isn't
        // strictly required — but we still disarm on the happy path so
        // the Drop log doesn't fire on every successful run.
        let outcome = match &gateway_result {
            Ok(response) => ModelCallOutcome::Success(response),
            Err(error) => ModelCallOutcome::Failure(error),
        };
        let post_result = self
            .accountant
            .post_model_call(&self.context, &request, outcome)
            .await;
        if let Err(post_error) = post_result {
            // Keep the guard armed when post-call accounting fails. Its Drop
            // retries the exact retained terminal action (reconcile with known
            // usage or release), rather than abandoning the reservation.
            drop(release_guard);
            let host_error = post_error.into_host_error();
            log_milestone_failure(
                self.milestones.model_failed(host_error.kind).await,
                "loop model_failed milestone failed after post-model accounting error",
            );
            return Err(host_error);
        }
        release_guard.disarm();

        let response = match gateway_result {
            Ok(response) => response,
            Err(error) => {
                let host_error = error.into_host_error();
                log_milestone_failure(
                    self.milestones.model_failed(host_error.kind).await,
                    "loop model_failed milestone failed after model error",
                );
                return Err(host_error);
            }
        };

        for safe_delta in &response.safe_reasoning_deltas {
            log_milestone_failure(
                self.milestones
                    .model_reasoning_delta(safe_delta.clone())
                    .await,
                "loop model reasoning milestone failed after successful model response",
            );
        }
        if matches!(response.output, ParentLoopOutput::AssistantReply(_))
            && !progress_sink.emitted_text()
        {
            let text_chunk_count = response
                .chunks
                .iter()
                .filter(|chunk| !chunk.safe_text_delta.is_empty())
                .count();
            let mut text_chunk_index = 0;
            let mut accumulated_text = String::new();
            for chunk in &response.chunks {
                if chunk.safe_text_delta.is_empty() {
                    continue;
                }
                accumulated_text.push_str(&chunk.safe_text_delta);
                text_chunk_index += 1;
                if !should_emit_fallback_text_delta(text_chunk_index, text_chunk_count) {
                    continue;
                }
                log_milestone_failure(
                    self.milestones
                        .model_text_delta(accumulated_text.clone())
                        .await,
                    "loop model text milestone failed after successful model response",
                );
            }
        }
        log_milestone_failure(
            self.milestones
                .model_completed(response.effective_model_profile_id.clone())
                .await,
            "loop model_completed milestone failed after successful model response",
        );
        Ok(response)
    }
}

struct MilestoneModelProgressSink<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    milestones: LoopHostMilestoneEmitter<S>,
    emitted_text: AtomicBool,
    progress_generation: tokio::sync::watch::Sender<u64>,
}

impl<S> MilestoneModelProgressSink<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    fn emitted_text(&self) -> bool {
        self.emitted_text.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl<S> LoopModelProgressSink for MilestoneModelProgressSink<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    async fn model_text_update(&self, safe_text: String) {
        self.emitted_text.store(true, Ordering::SeqCst);
        self.progress_generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        log_milestone_failure(
            self.milestones.model_text_delta(safe_text).await,
            "loop model text progress milestone failed during model stream",
        );
    }
}

async fn await_with_progress_timeout<F, T>(
    future: F,
    mut progress_updates: tokio::sync::watch::Receiver<u64>,
    idle_timeout: Duration,
) -> Result<T, ()>
where
    F: Future<Output = T>,
{
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            progress = tokio::time::timeout(idle_timeout, progress_updates.changed()) => {
                match progress {
                    Ok(Ok(())) => continue,
                    Err(_elapsed) => return Err(()),
                    Ok(Err(_closed)) => {
                        return tokio::time::timeout(idle_timeout, &mut future)
                            .await
                            .map_err(|_elapsed| ());
                    }
                }
            }
        }
    }
}

/// RAII guard that releases the in-flight reservation if the surrounding
/// future is cancelled before `post_model_call` runs.
///
/// On Drop, when still armed, the guard calls
/// [`LoopModelBudgetAccountant::release_in_flight`] — a synchronous
/// best-effort path that the accountant uses to finalize the retained action
/// without awaiting. Callers disarm the guard only after the async
/// `post_model_call` path succeeds.
struct ReservationReleaseGuard<'a> {
    accountant: &'a dyn LoopModelBudgetAccountant,
    context: &'a LoopRunContext,
    armed: bool,
}

impl<'a> ReservationReleaseGuard<'a> {
    fn new(accountant: &'a dyn LoopModelBudgetAccountant, context: &'a LoopRunContext) -> Self {
        Self {
            accountant,
            context,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ReservationReleaseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.accountant.release_in_flight(self.context);
        }
    }
}

fn sanitize_model_response(mut response: LoopModelResponse) -> LoopModelResponse {
    for chunk in &mut response.chunks {
        chunk.safe_text_delta =
            sanitize_model_visible_text(std::mem::take(&mut chunk.safe_text_delta));
    }
    for safe_delta in &mut response.safe_reasoning_deltas {
        *safe_delta = sanitize_model_visible_text(std::mem::take(safe_delta));
    }
    response
        .safe_reasoning_deltas
        .retain(|safe_delta| !safe_delta.is_empty());
    if let ParentLoopOutput::AssistantReply(reply) = &mut response.output {
        reply.content = sanitize_model_visible_text(std::mem::take(&mut reply.content));
    }
    response
}

fn should_emit_fallback_text_delta(chunk_index: usize, chunk_count: usize) -> bool {
    chunk_index == chunk_count || chunk_index.is_multiple_of(FALLBACK_TEXT_DELTA_MILESTONE_STEP)
}

/// Milestone emission is best-effort: a failed emit must never abort the model
/// call, only leave a diagnostic log. Every `stream_model` milestone site
/// shares this "log the kind on error, otherwise ignore" shape, so it lives
/// here once.
fn log_milestone_failure(result: Result<(), AgentLoopHostError>, message: &'static str) {
    if let Err(error) = result {
        tracing::debug!(
            kind = ?error.kind,
            "{}",
            message
        );
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_loop_contracts::AgentLoopHostErrorKind;

    use super::*;

    #[test]
    fn primary_model_call_idle_timeout_is_below_runner_lease() {
        assert!(
            PRIMARY_MODEL_CALL_IDLE_TIMEOUT < ironclaw_processes::DEFAULT_PROCESS_LEASE_DURATION
        );
    }

    #[test]
    fn fallback_model_text_delta_emission_is_throttled_and_final() {
        let emitted = (1..=32)
            .filter(|chunk_index| should_emit_fallback_text_delta(*chunk_index, 32))
            .collect::<Vec<_>>();

        assert_eq!(emitted, vec![15, 30, 32]);
        assert!(should_emit_fallback_text_delta(14, 14));
        assert!(!should_emit_fallback_text_delta(1, 14));
    }

    #[test]
    fn model_gateway_error_detail_round_trips_to_host_error() {
        let detail = "provider failed at /tmp/{response}";
        let gateway_error =
            LoopModelGatewayError::new(AgentLoopHostErrorKind::Unavailable, "model gateway failed")
                .expect("valid summary")
                .with_detail(detail);

        assert_eq!(
            gateway_error.into_host_error().detail.as_deref(),
            Some(detail)
        );
    }

    #[test]
    fn model_gateway_error_detail_stays_out_of_wire_shape() {
        let error =
            LoopModelGatewayError::new(AgentLoopHostErrorKind::Unavailable, "model gateway failed")
                .expect("valid summary")
                .with_detail("private diagnostic");

        let serialized = serde_json::to_value(&error).expect("gateway error serializes");
        assert!(serialized.get("detail").is_none());

        let decoded: LoopModelGatewayError = serde_json::from_value(serialized)
            .expect("older wire shape without detail still deserializes");
        assert_eq!(decoded.detail, None);
    }
}
