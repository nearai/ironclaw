//! In-memory fakes for contract tests and downstream integration tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{ProductInboundEnvelope, ProductRejection, UserMessagePayload};
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

use crate::action::{ActionFingerprintKey, ProductInboundAction};
use crate::binding::{ConversationBindingService, ResolveBindingRequest, ResolvedBinding};
use crate::error::ProductWorkflowError;
use crate::inbound_turn::{InboundTurnOutcome, InboundTurnService};
use crate::ledger::{IdempotencyDecision, IdempotencyLedger};
use crate::policy::{BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest};

// ---------------------------------------------------------------------------
// FakeConversationBindingService
// ---------------------------------------------------------------------------

/// In-memory fake that resolves all bindings to a default tenant/user/thread
/// unless programmed otherwise.
#[derive(Clone)]
pub struct FakeConversationBindingService {
    state: Arc<Mutex<FakeBindingState>>,
}

#[derive(Default)]
struct FakeBindingState {
    programmed: HashMap<String, ResolvedBinding>,
    fail_with: Option<ProductWorkflowError>,
    resolve_count: usize,
}

impl FakeConversationBindingService {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeBindingState::default())),
        }
    }

    /// Program a specific binding for a given source binding key.
    pub fn program_binding(&self, source_key: impl Into<String>, binding: ResolvedBinding) {
        let mut state = self.state.lock().expect("fake binding state lock poisoned"); // safety: test-support fake
        state.programmed.insert(source_key.into(), binding);
    }

    /// Force all resolutions to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self.state.lock().expect("fake binding state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// How many bindings have been resolved.
    pub fn resolve_count(&self) -> usize {
        let state = self.state.lock().expect("fake binding state lock poisoned"); // safety: test-support fake
        state.resolve_count
    }
}

impl Default for FakeConversationBindingService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConversationBindingService for FakeConversationBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let mut state = self.state.lock().expect("fake binding state lock poisoned"); // safety: test-support fake
        state.resolve_count += 1;
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        let key = request.external_conversation_ref.conversation_fingerprint();
        if let Some(binding) = state.programmed.get(&key).cloned() {
            return Ok(binding);
        }
        // Default: deterministic binding from external refs.
        Ok(ResolvedBinding {
            tenant_id: TenantId::new(format!("tenant:{}", request.installation_id.as_str()))
                .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                })?,
            user_id: UserId::new(format!("user:{}", request.external_actor_ref.id())).map_err(
                |e| ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                },
            )?,
            thread_id: ThreadId::new(format!(
                "thread:{}:{}",
                request.installation_id.as_str(),
                request.external_conversation_ref.conversation_fingerprint()
            ))
            .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                reason: e.to_string(),
            })?,
            agent_id: Some(AgentId::new("agent:fake").map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?),
            project_id: None,
        })
    }
}

// ---------------------------------------------------------------------------
// FakeIdempotencyLedger
// ---------------------------------------------------------------------------

/// In-memory idempotency ledger that deduplicates by fingerprint.
pub struct FakeIdempotencyLedger {
    state: Mutex<FakeIdempotencyState>,
}

#[derive(Default)]
struct FakeIdempotencyState {
    in_flight: HashMap<ActionFingerprintKey, ProductInboundAction>,
    settled: HashMap<ActionFingerprintKey, ProductInboundAction>,
    fail_with: Option<ProductWorkflowError>,
    settle_fail_with: Option<ProductWorkflowError>,
}

impl FakeIdempotencyLedger {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeIdempotencyState::default()),
        }
    }

    /// Force all operations to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// Force settle operations to fail while begin/replay still succeeds.
    pub fn force_settle_failure(&self, error: ProductWorkflowError) {
        let mut state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        state.settle_fail_with = Some(error);
    }

    /// How many actions are reserved but not settled.
    pub fn in_flight_count(&self) -> usize {
        let state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        state.in_flight.len()
    }

    /// Expire in-flight actions received before `cutoff`, simulating the
    /// durable-ledger recovery sweeper/TTL contract.
    pub fn expire_in_flight_before(&self, cutoff: DateTime<Utc>) -> usize {
        let mut state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        let before = state.in_flight.len();
        state
            .in_flight
            .retain(|_, action| action.received_at >= cutoff);
        before - state.in_flight.len()
    }

    /// How many actions have been settled.
    pub fn settled_count(&self) -> usize {
        let state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        state.settled.len()
    }

    /// Get all settled actions.
    pub fn settled_actions(&self) -> Vec<ProductInboundAction> {
        let state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        state.settled.values().cloned().collect()
    }
}

impl Default for FakeIdempotencyLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IdempotencyLedger for FakeIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let mut state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        if let Some(prior) = state.settled.get(&fingerprint) {
            return Ok(IdempotencyDecision::Replay(prior.clone()));
        }
        if state.in_flight.contains_key(&fingerprint) {
            return Err(ProductWorkflowError::Transient {
                reason: "idempotency fingerprint already in flight; retry after recovery lease"
                    .into(),
            });
        }
        let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
        state.in_flight.insert(fingerprint, action.clone());
        Ok(IdempotencyDecision::New(action))
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let mut state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        if let Some(error) = state.settle_fail_with.clone() {
            return Err(error);
        }
        state.in_flight.remove(&action.fingerprint);
        state.settled.insert(action.fingerprint.clone(), action);
        Ok(())
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let mut state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state.in_flight.remove(&action.fingerprint);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FakeBeforeInboundPolicy
// ---------------------------------------------------------------------------

/// In-memory fake for before-inbound policy checks.
#[derive(Clone)]
pub struct FakeBeforeInboundPolicy {
    state: Arc<Mutex<FakeBeforeInboundPolicyState>>,
}

#[derive(Default)]
struct FakeBeforeInboundPolicyState {
    requests: Vec<BeforeInboundPolicyRequest>,
    outcome: Option<Result<BeforeInboundPolicyOutcome, ProductWorkflowError>>,
}

impl FakeBeforeInboundPolicy {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeBeforeInboundPolicyState::default())),
        }
    }

    /// Allow all messages without modification.
    pub fn allow(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.outcome = Some(Ok(BeforeInboundPolicyOutcome::Allow));
    }

    /// Rewrite all user-message payloads.
    pub fn rewrite_user_message(&self, payload: UserMessagePayload) {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.outcome = Some(Ok(BeforeInboundPolicyOutcome::RewriteUserMessage(payload)));
    }

    /// Reject all user messages.
    pub fn reject(&self, rejection: ProductRejection) {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.outcome = Some(Ok(BeforeInboundPolicyOutcome::Reject(rejection)));
    }

    /// Fail all policy checks.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.outcome = Some(Err(error));
    }

    /// How many user messages reached policy checks.
    pub fn request_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.requests.len()
    }

    /// Recorded policy requests.
    pub fn requests(&self) -> Vec<BeforeInboundPolicyRequest> {
        let state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.requests.clone()
    }
}

impl Default for FakeBeforeInboundPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BeforeInboundPolicy for FakeBeforeInboundPolicy {
    async fn check_user_message(
        &self,
        request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.requests.push(request);
        state
            .outcome
            .clone()
            .unwrap_or(Ok(BeforeInboundPolicyOutcome::Allow))
    }
}

// ---------------------------------------------------------------------------
// FakeInboundTurnService
// ---------------------------------------------------------------------------

/// In-memory fake for the inbound turn service.
pub struct FakeInboundTurnService {
    state: Mutex<FakeInboundTurnState>,
}

#[derive(Default)]
struct FakeInboundTurnState {
    attempts: usize,
    replay_attempts: usize,
    accepted: Vec<ProductInboundEnvelope>,
    fail_with: Option<ProductWorkflowError>,
    programmed_replay: Option<InboundTurnOutcome>,
    programmed_outcome: Option<InboundTurnOutcome>,
}

impl FakeInboundTurnService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeInboundTurnState::default()),
        }
    }

    /// Program a specific outcome for all submissions.
    pub fn program_outcome(&self, outcome: InboundTurnOutcome) {
        let mut state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.programmed_outcome = Some(outcome);
    }

    /// Program an already-staged accepted-message replay outcome.
    pub fn program_replay_outcome(&self, outcome: InboundTurnOutcome) {
        let mut state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.programmed_replay = Some(outcome);
    }

    /// Force all submissions to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// How many submission attempts reached this fake.
    pub fn attempt_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.attempts
    }

    /// How many replay probes reached this fake.
    pub fn replay_attempt_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.replay_attempts
    }

    /// Get all envelopes that were accepted.
    pub fn accepted_envelopes(&self) -> Vec<ProductInboundEnvelope> {
        let state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.accepted.clone()
    }

    /// How many messages were accepted.
    pub fn accepted_count(&self) -> usize {
        self.accepted_envelopes().len()
    }
}

impl Default for FakeInboundTurnService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InboundTurnService for FakeInboundTurnService {
    async fn replay_accepted_user_message(
        &self,
        _envelope: &ProductInboundEnvelope,
    ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.replay_attempts += 1;
        Ok(state.programmed_replay.clone())
    }

    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake inbound turn state lock poisoned"); // safety: test-support fake
        state.attempts += 1;
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state.accepted.push(envelope.clone());
        if let Some(outcome) = state.programmed_outcome.clone() {
            return Ok(outcome);
        }
        // Default: successful submission.
        let binding = ResolvedBinding {
            tenant_id: TenantId::new("tenant:fake").map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?,
            user_id: UserId::new("user:fake").map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?,
            thread_id: ThreadId::new("thread:fake").map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?,
            agent_id: Some(AgentId::new("agent:fake").map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?),
            project_id: None,
        };
        let accepted_message_ref =
            AcceptedMessageRef::new(format!("msg:{}", envelope.external_event_id()))
                .map_err(|e: String| ProductWorkflowError::TurnSubmissionRejected { reason: e })?;
        Ok(InboundTurnOutcome::Submitted {
            accepted_message_ref,
            submitted_run_id: TurnRunId::new(),
            binding,
        })
    }
}
