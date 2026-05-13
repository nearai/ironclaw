//! In-memory fakes for contract tests and downstream integration tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    ApprovalDecision, AuthResolutionResult, ProductInboundEnvelope, ProjectionSubscriptionRequest,
};
use ironclaw_turns::{AcceptedMessageRef, LoopGateRef, TurnActor, TurnRunId, TurnScope};

use crate::action::{
    ActionFingerprintKey, AuthRequestRef, LinkedThreadActionId, ProductCommandName,
    ProductInboundAction,
};
use crate::binding::{ConversationBindingService, ResolveBindingRequest, ResolvedBinding};
use crate::error::ProductWorkflowError;
use crate::inbound_turn::{InboundTurnOutcome, InboundTurnService};
use crate::ledger::{IdempotencyDecision, IdempotencyLedger};
use crate::services::{
    ApprovalInteractionService, ApprovalResolutionOutcome, AuthInteractionService,
    AuthResolutionOutcome, BeforeInboundOutcome, BeforeInboundPolicy, BeforeInboundRequest,
    LinkedThreadActionOutcome, LinkedThreadActionService, MissionFireOutcome, MissionFireRef,
    MissionFireRejectionReason, MissionFireRequest, MissionFireSuppressionReason, MissionService,
    ProductCommandOutcome, ProductCommandRouter, ProjectionSubscriptionAuthority,
    ProjectionSubscriptionAuthorityRequest, SystemActionOutcome, SystemActionService,
};

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
// FakeInboundTurnService
// ---------------------------------------------------------------------------

/// In-memory fake for the inbound turn service.
pub struct FakeInboundTurnService {
    state: Mutex<FakeInboundTurnState>,
}

#[derive(Default)]
struct FakeInboundTurnState {
    attempts: usize,
    accepted: Vec<ProductInboundEnvelope>,
    fail_with: Option<ProductWorkflowError>,
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

// ---------------------------------------------------------------------------
// FakeBeforeInboundPolicy
// ---------------------------------------------------------------------------

/// In-memory fake for [`BeforeInboundPolicy`]. Records every evaluation request
/// and replays a programmed outcome (defaulting to `Continue` with no
/// rewriting).
pub struct FakeBeforeInboundPolicy {
    state: Mutex<FakeBeforeInboundState>,
}

#[derive(Default)]
struct FakeBeforeInboundState {
    evaluate_count: usize,
    next_outcome: Option<BeforeInboundOutcome>,
    fail_with: Option<ProductWorkflowError>,
    last_request: Option<BeforeInboundRequest>,
}

impl FakeBeforeInboundPolicy {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeBeforeInboundState::default()),
        }
    }

    /// Program the next evaluation to pass through (optionally rewriting text).
    pub fn program_continue(&self, rewritten: Option<String>) {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(BeforeInboundOutcome::Continue {
            rewritten_text: rewritten,
        });
    }

    /// Program the next evaluation to reject with a redacted reason.
    pub fn program_reject(&self, reason: impl Into<String>) {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(BeforeInboundOutcome::Reject {
            reason: ironclaw_product_adapters::RedactedString::new(reason.into()),
        });
    }

    /// Force all evaluations to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// How many evaluations have been performed.
    pub fn evaluate_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.evaluate_count
    }

    /// The most recent request observed (if any).
    pub fn last_request(&self) -> Option<BeforeInboundRequest> {
        let state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.last_request.clone()
    }
}

impl Default for FakeBeforeInboundPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BeforeInboundPolicy for FakeBeforeInboundPolicy {
    async fn evaluate_inbound(
        &self,
        request: BeforeInboundRequest,
    ) -> Result<BeforeInboundOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake before-inbound policy state lock poisoned"); // safety: test-support fake
        state.evaluate_count += 1;
        state.last_request = Some(request);
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        if let Some(outcome) = state.next_outcome.clone() {
            return Ok(outcome);
        }
        Ok(BeforeInboundOutcome::Continue {
            rewritten_text: None,
        })
    }
}

// ---------------------------------------------------------------------------
// FakeProductCommandRouter
// ---------------------------------------------------------------------------

/// In-memory fake for [`ProductCommandRouter`]. Records every routed command +
/// arguments pair and replays a programmed outcome (defaulting to echoing the
/// input command back as `Routed`).
pub struct FakeProductCommandRouter {
    state: Mutex<FakeProductCommandRouterState>,
}

#[derive(Default)]
struct FakeProductCommandRouterState {
    routed: Vec<(ProductCommandName, String)>,
    next_outcome: Option<ProductCommandOutcome>,
    /// When true, the next call returns `UnknownCommand` echoing the input
    /// command. Cleared after the call.
    next_is_unknown: bool,
    fail_with: Option<ProductWorkflowError>,
}

impl FakeProductCommandRouter {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeProductCommandRouterState::default()),
        }
    }

    /// Program the next routing call to report the command as unknown. The
    /// `UnknownCommand` outcome will echo whatever command was passed to
    /// `route_command`, since the router's job is to surface the *requested*
    /// command name in the rejection.
    pub fn program_unknown(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake product command router state lock poisoned"); // safety: test-support fake
        state.next_is_unknown = true;
        state.next_outcome = None;
    }

    /// Program the next routing call to report the command as routed.
    pub fn program_routed(&self, command: ProductCommandName) {
        let mut state = self
            .state
            .lock()
            .expect("fake product command router state lock poisoned"); // safety: test-support fake
        state.next_is_unknown = false;
        state.next_outcome = Some(ProductCommandOutcome::Routed { command });
    }

    /// Force all routing calls to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake product command router state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// All commands seen by this router, in order.
    pub fn routed(&self) -> Vec<(ProductCommandName, String)> {
        let state = self
            .state
            .lock()
            .expect("fake product command router state lock poisoned"); // safety: test-support fake
        state.routed.clone()
    }

    /// How many commands have been routed.
    pub fn routed_count(&self) -> usize {
        self.routed().len()
    }
}

impl Default for FakeProductCommandRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProductCommandRouter for FakeProductCommandRouter {
    async fn route_command(
        &self,
        _envelope: &ProductInboundEnvelope,
        command: ProductCommandName,
        arguments: String,
    ) -> Result<ProductCommandOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake product command router state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state.routed.push((command.clone(), arguments));
        if state.next_is_unknown {
            state.next_is_unknown = false;
            return Ok(ProductCommandOutcome::UnknownCommand { command });
        }
        if let Some(outcome) = state.next_outcome.clone() {
            return Ok(outcome);
        }
        Ok(ProductCommandOutcome::Routed { command })
    }
}

// ---------------------------------------------------------------------------
// FakeApprovalInteractionService
// ---------------------------------------------------------------------------

/// In-memory fake for [`ApprovalInteractionService`]. Records each
/// `(gate_ref, decision)` pair and replays a programmed outcome (defaulting to
/// `Handled { gate_ref }` echoing the input ref).
pub struct FakeApprovalInteractionService {
    state: Mutex<FakeApprovalState>,
}

#[derive(Default)]
struct FakeApprovalState {
    resolutions: Vec<(LoopGateRef, ApprovalDecision)>,
    next_outcome: Option<ApprovalResolutionOutcome>,
    fail_with: Option<ProductWorkflowError>,
}

impl FakeApprovalInteractionService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeApprovalState::default()),
        }
    }

    /// Program the next resolution to report the gate ref as stale/unknown.
    pub fn program_stale(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake approval interaction state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(ApprovalResolutionOutcome::StaleOrUnknown);
    }

    /// Program the next resolution to report success, echoing the gate ref.
    pub fn program_handled(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake approval interaction state lock poisoned"); // safety: test-support fake
        state.next_outcome = None; // fall back to default echo behaviour
        state.fail_with = None;
    }

    /// Force all resolutions to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake approval interaction state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// All resolutions seen by this service, in order.
    pub fn resolutions(&self) -> Vec<(LoopGateRef, ApprovalDecision)> {
        let state = self
            .state
            .lock()
            .expect("fake approval interaction state lock poisoned"); // safety: test-support fake
        state.resolutions.clone()
    }

    /// How many approvals have been resolved.
    pub fn resolution_count(&self) -> usize {
        self.resolutions().len()
    }
}

impl Default for FakeApprovalInteractionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ApprovalInteractionService for FakeApprovalInteractionService {
    async fn resolve_approval(
        &self,
        _envelope: &ProductInboundEnvelope,
        gate_ref: LoopGateRef,
        decision: ApprovalDecision,
    ) -> Result<ApprovalResolutionOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake approval interaction state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state.resolutions.push((gate_ref.clone(), decision));
        if let Some(outcome) = state.next_outcome.clone() {
            return Ok(outcome);
        }
        Ok(ApprovalResolutionOutcome::Handled { gate_ref })
    }
}

// ---------------------------------------------------------------------------
// FakeAuthInteractionService
// ---------------------------------------------------------------------------

/// In-memory fake for [`AuthInteractionService`]. Records each
/// `(auth_request_ref, result)` pair and replays a programmed outcome
/// (defaulting to `Handled { auth_request_ref }` echoing the input ref).
pub struct FakeAuthInteractionService {
    state: Mutex<FakeAuthState>,
}

#[derive(Default)]
struct FakeAuthState {
    resolutions: Vec<(AuthRequestRef, AuthResolutionResult)>,
    next_outcome: Option<AuthResolutionOutcome>,
    fail_with: Option<ProductWorkflowError>,
}

impl FakeAuthInteractionService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeAuthState::default()),
        }
    }

    /// Program the next resolution to report the auth ref as stale/unknown.
    pub fn program_stale(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake auth interaction state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(AuthResolutionOutcome::StaleOrUnknown);
    }

    /// Program the next resolution to report success, echoing the auth ref.
    pub fn program_handled(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake auth interaction state lock poisoned"); // safety: test-support fake
        state.next_outcome = None; // fall back to default echo behaviour
        state.fail_with = None;
    }

    /// Force all resolutions to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake auth interaction state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// All resolutions seen by this service, in order.
    pub fn resolutions(&self) -> Vec<(AuthRequestRef, AuthResolutionResult)> {
        let state = self
            .state
            .lock()
            .expect("fake auth interaction state lock poisoned"); // safety: test-support fake
        state.resolutions.clone()
    }

    /// How many auth resolutions have been seen.
    pub fn resolution_count(&self) -> usize {
        self.resolutions().len()
    }
}

impl Default for FakeAuthInteractionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthInteractionService for FakeAuthInteractionService {
    async fn resolve_auth(
        &self,
        _envelope: &ProductInboundEnvelope,
        auth_request_ref: AuthRequestRef,
        result: AuthResolutionResult,
    ) -> Result<AuthResolutionOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake auth interaction state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state.resolutions.push((auth_request_ref.clone(), result));
        if let Some(outcome) = state.next_outcome.clone() {
            return Ok(outcome);
        }
        Ok(AuthResolutionOutcome::Handled { auth_request_ref })
    }
}

// ---------------------------------------------------------------------------
// FakeLinkedThreadActionService
// ---------------------------------------------------------------------------

/// In-memory fake for [`LinkedThreadActionService`]. Records each action
/// (id + data + reply target) and always echoes back `Routed { action_id }`.
pub struct FakeLinkedThreadActionService {
    state: Mutex<FakeLinkedThreadActionState>,
}

#[derive(Default)]
struct FakeLinkedThreadActionState {
    actions: Vec<(LinkedThreadActionId, Option<String>, Option<String>)>,
    fail_with: Option<ProductWorkflowError>,
}

impl FakeLinkedThreadActionService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeLinkedThreadActionState::default()),
        }
    }

    /// Force all calls to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake linked thread action state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// All actions seen by this service, in order.
    pub fn actions(&self) -> Vec<(LinkedThreadActionId, Option<String>, Option<String>)> {
        let state = self
            .state
            .lock()
            .expect("fake linked thread action state lock poisoned"); // safety: test-support fake
        state.actions.clone()
    }

    /// How many actions have been routed.
    pub fn action_count(&self) -> usize {
        self.actions().len()
    }
}

impl Default for FakeLinkedThreadActionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LinkedThreadActionService for FakeLinkedThreadActionService {
    async fn handle_action(
        &self,
        _envelope: &ProductInboundEnvelope,
        action_id: LinkedThreadActionId,
        data: Option<String>,
        reply_target_message_id: Option<String>,
    ) -> Result<LinkedThreadActionOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake linked thread action state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state
            .actions
            .push((action_id.clone(), data, reply_target_message_id));
        Ok(LinkedThreadActionOutcome::Routed { action_id })
    }
}

// ---------------------------------------------------------------------------
// FakeMissionService
// ---------------------------------------------------------------------------

/// In-memory fake for [`MissionService`]. Records every fire request and
/// replays a programmed outcome (defaulting to a fresh `Submitted` envelope).
pub struct FakeMissionService {
    state: Mutex<FakeMissionState>,
}

#[derive(Default)]
struct FakeMissionState {
    fires: Vec<MissionFireRequest>,
    next_outcome: Option<MissionFireOutcome>,
    fail_with: Option<ProductWorkflowError>,
    fire_count: usize,
}

impl FakeMissionService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeMissionState::default()),
        }
    }

    /// Program the next fire to succeed with a fresh `Submitted` envelope.
    pub fn program_submitted(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(MissionFireOutcome::Submitted {
            mission_fire_ref: MissionFireRef::new(),
            run_id: TurnRunId::new(),
        });
    }

    /// Program the next fire to report a busy thread (deferred).
    pub fn program_deferred_busy(&self) {
        let mut state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(MissionFireOutcome::DeferredBusy {
            mission_fire_ref: MissionFireRef::new(),
            active_run_id: TurnRunId::new(),
        });
    }

    /// Program the next fire to be suppressed with the supplied reason.
    pub fn program_suppressed(&self, reason: MissionFireSuppressionReason) {
        let mut state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(MissionFireOutcome::Suppressed {
            mission_fire_ref: MissionFireRef::new(),
            reason,
        });
    }

    /// Program the next fire to be rejected with the supplied reason.
    pub fn program_rejected(&self, reason: MissionFireRejectionReason) {
        let mut state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.next_outcome = Some(MissionFireOutcome::Rejected { reason });
    }

    /// Force all fires to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// All fire requests seen by this service, in order.
    pub fn fires(&self) -> Vec<MissionFireRequest> {
        let state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.fires.clone()
    }

    /// How many fires have been requested.
    pub fn fire_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.fire_count
    }
}

impl Default for FakeMissionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MissionService for FakeMissionService {
    async fn fire_mission(
        &self,
        request: MissionFireRequest,
    ) -> Result<MissionFireOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake mission service state lock poisoned"); // safety: test-support fake
        state.fire_count += 1;
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state.fires.push(request);
        if let Some(outcome) = state.next_outcome.clone() {
            return Ok(outcome);
        }
        Ok(MissionFireOutcome::Submitted {
            mission_fire_ref: MissionFireRef::new(),
            run_id: TurnRunId::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// FakeSystemActionService
// ---------------------------------------------------------------------------

/// In-memory fake for [`SystemActionService`]. Records every typed system
/// action (actor + kind + optional scope + optional data) and always returns
/// `Routed`.
pub struct FakeSystemActionService {
    state: Mutex<FakeSystemActionState>,
}

#[derive(Default)]
struct FakeSystemActionState {
    actions: Vec<(String, String, Option<String>, Option<String>)>,
    fail_with: Option<ProductWorkflowError>,
}

impl FakeSystemActionService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeSystemActionState::default()),
        }
    }

    /// Force all calls to fail.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake system action state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// All actions seen by this service, in order.
    pub fn actions(&self) -> Vec<(String, String, Option<String>, Option<String>)> {
        let state = self
            .state
            .lock()
            .expect("fake system action state lock poisoned"); // safety: test-support fake
        state.actions.clone()
    }

    /// How many system actions have been routed.
    pub fn action_count(&self) -> usize {
        self.actions().len()
    }
}

impl Default for FakeSystemActionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SystemActionService for FakeSystemActionService {
    async fn handle_action(
        &self,
        _envelope: &ProductInboundEnvelope,
        system_actor_ref: String,
        kind: String,
        scope_thread_id: Option<String>,
        data: Option<String>,
    ) -> Result<SystemActionOutcome, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake system action state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state
            .actions
            .push((system_actor_ref, kind, scope_thread_id, data));
        Ok(SystemActionOutcome::Routed)
    }
}

// ---------------------------------------------------------------------------
// FakeProjectionSubscriptionAuthority
// ---------------------------------------------------------------------------

/// In-memory fake for [`ProjectionSubscriptionAuthority`]. Records every
/// authorization request and replays either a programmed response or a
/// deterministic default derived from the request (mirroring the
/// [`FakeConversationBindingService`] defaulting pattern).
pub struct FakeProjectionSubscriptionAuthority {
    state: Mutex<FakeProjectionSubscriptionAuthorityState>,
}

#[derive(Default)]
struct FakeProjectionSubscriptionAuthorityState {
    authorizations: Vec<ProjectionSubscriptionAuthorityRequest>,
    programmed_response: Option<ProjectionSubscriptionRequest>,
    fail_with: Option<ProductWorkflowError>,
}

impl FakeProjectionSubscriptionAuthority {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeProjectionSubscriptionAuthorityState::default()),
        }
    }

    /// Program a specific response for the next authorization. Subsequent
    /// requests reuse the same response until reprogrammed.
    pub fn program_response(&self, response: ProjectionSubscriptionRequest) {
        let mut state = self
            .state
            .lock()
            .expect("fake projection subscription authority state lock poisoned"); // safety: test-support fake
        state.programmed_response = Some(response);
    }

    /// Force the next authorization to fail with a transient workflow error.
    pub fn program_failure(&self, reason: impl Into<String>) {
        let mut state = self
            .state
            .lock()
            .expect("fake projection subscription authority state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(ProductWorkflowError::Transient {
            reason: reason.into(),
        });
    }

    /// Force all authorizations to fail with a caller-supplied error.
    pub fn force_failure(&self, error: ProductWorkflowError) {
        let mut state = self
            .state
            .lock()
            .expect("fake projection subscription authority state lock poisoned"); // safety: test-support fake
        state.fail_with = Some(error);
    }

    /// All requests seen by this service, in order.
    pub fn authorizations(&self) -> Vec<ProjectionSubscriptionAuthorityRequest> {
        let state = self
            .state
            .lock()
            .expect("fake projection subscription authority state lock poisoned"); // safety: test-support fake
        state.authorizations.clone()
    }

    /// How many authorizations have been performed.
    pub fn authorization_count(&self) -> usize {
        self.authorizations().len()
    }
}

impl Default for FakeProjectionSubscriptionAuthority {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProjectionSubscriptionAuthority for FakeProjectionSubscriptionAuthority {
    async fn authorize_subscription(
        &self,
        request: ProjectionSubscriptionAuthorityRequest,
    ) -> Result<ProjectionSubscriptionRequest, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .expect("fake projection subscription authority state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        state.authorizations.push(request.clone());
        if let Some(response) = state.programmed_response.clone() {
            return Ok(response);
        }

        // Default: deterministic mapping from the request — mirrors the
        // FakeConversationBindingService defaulting pattern. The cursor is
        // forwarded unchanged so contract tests can assert pass-through.
        let tenant_id = TenantId::new(format!("tenant:{}", request.installation_id.as_str()))
            .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                reason: e.to_string(),
            })?;
        let user_id =
            UserId::new(format!("user:{}", request.external_actor_ref.id())).map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?;
        let thread_id = ThreadId::new(format!(
            "thread:{}:{}",
            request.installation_id.as_str(),
            request.external_conversation_ref.conversation_fingerprint()
        ))
        .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
            reason: e.to_string(),
        })?;
        let agent_id = AgentId::new("agent:fake").map_err(|e| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: e.to_string(),
            }
        })?;
        let project_id = ProjectId::new("project:fake").map_err(|e| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: e.to_string(),
            }
        })?;

        let actor = TurnActor::new(user_id);
        let scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);

        Ok(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: request.after_cursor,
        })
    }
}
