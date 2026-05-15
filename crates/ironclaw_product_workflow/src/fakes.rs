//! In-memory fakes for contract tests and downstream integration tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::ProductInboundEnvelope;
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

use crate::action::{ActionFingerprintKey, ProductInboundAction};
use crate::binding::{ConversationBindingService, ResolveBindingRequest, ResolvedBinding};
use crate::error::ProductWorkflowError;
use crate::inbound_turn::{InboundTurnOutcome, InboundTurnService};
use crate::ledger::{IdempotencyDecision, IdempotencyLedger};
use crate::webui_service::{
    WebUiCancelRunCommand, WebUiCreateThreadCommand, WebUiGateResolved, WebUiGetTimelineCommand,
    WebUiMessageAccepted, WebUiMessageRunOutcome, WebUiResolveGateCommand, WebUiRunCancelled,
    WebUiSendMessageCommand, WebUiService, WebUiServiceError, WebUiThreadCreated,
    WebUiTimelineCursor, WebUiTimelineReplay, WebUiTimelineSnapshot,
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
// FakeWebUiService
// ---------------------------------------------------------------------------

/// In-memory fake for the WebUI service facade.
///
/// Records every command invocation and returns programmable outcomes, so
/// downstream route-handler tests can verify wiring without spinning up the
/// real thread service or turn coordinator.
pub struct FakeWebUiService {
    state: Mutex<FakeWebUiState>,
}

#[derive(Default)]
struct FakeWebUiState {
    create_thread_calls: Vec<WebUiCreateThreadCommand>,
    send_message_calls: Vec<WebUiSendMessageCommand>,
    cancel_run_calls: Vec<WebUiCancelRunCommand>,
    resolve_gate_calls: Vec<WebUiResolveGateCommand>,
    timeline_snapshot_calls: Vec<WebUiGetTimelineCommand>,
    timeline_updates_calls: Vec<WebUiGetTimelineCommand>,
    create_thread_outcome: Option<WebUiThreadCreated>,
    send_message_outcome: Option<WebUiMessageAccepted>,
    cancel_run_outcome: Option<WebUiRunCancelled>,
    resolve_gate_outcome: Option<WebUiGateResolved>,
    timeline_snapshot_outcome: Option<WebUiTimelineSnapshot>,
    timeline_updates_outcome: Option<WebUiTimelineReplay>,
    create_thread_error: Option<WebUiServiceError>,
    send_message_error: Option<WebUiServiceError>,
    cancel_run_error: Option<WebUiServiceError>,
    resolve_gate_error: Option<WebUiServiceError>,
    timeline_snapshot_error: Option<WebUiServiceError>,
    timeline_updates_error: Option<WebUiServiceError>,
}

impl FakeWebUiService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeWebUiState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeWebUiState> {
        self.state.lock().expect("fake webui state lock poisoned") // safety: test-support fake
    }

    pub fn program_create_thread(&self, outcome: WebUiThreadCreated) {
        self.lock().create_thread_outcome = Some(outcome);
    }

    pub fn program_send_message(&self, outcome: WebUiMessageAccepted) {
        self.lock().send_message_outcome = Some(outcome);
    }

    pub fn program_cancel_run(&self, outcome: WebUiRunCancelled) {
        self.lock().cancel_run_outcome = Some(outcome);
    }

    pub fn program_resolve_gate(&self, outcome: WebUiGateResolved) {
        self.lock().resolve_gate_outcome = Some(outcome);
    }

    pub fn program_timeline_snapshot(&self, outcome: WebUiTimelineSnapshot) {
        self.lock().timeline_snapshot_outcome = Some(outcome);
    }

    pub fn program_timeline_updates(&self, outcome: WebUiTimelineReplay) {
        self.lock().timeline_updates_outcome = Some(outcome);
    }

    pub fn fail_create_thread(&self, error: WebUiServiceError) {
        self.lock().create_thread_error = Some(error);
    }

    pub fn fail_send_message(&self, error: WebUiServiceError) {
        self.lock().send_message_error = Some(error);
    }

    pub fn fail_cancel_run(&self, error: WebUiServiceError) {
        self.lock().cancel_run_error = Some(error);
    }

    pub fn fail_resolve_gate(&self, error: WebUiServiceError) {
        self.lock().resolve_gate_error = Some(error);
    }

    pub fn fail_timeline_snapshot(&self, error: WebUiServiceError) {
        self.lock().timeline_snapshot_error = Some(error);
    }

    pub fn fail_timeline_updates(&self, error: WebUiServiceError) {
        self.lock().timeline_updates_error = Some(error);
    }

    pub fn create_thread_calls(&self) -> Vec<WebUiCreateThreadCommand> {
        self.lock().create_thread_calls.clone()
    }

    pub fn send_message_calls(&self) -> Vec<WebUiSendMessageCommand> {
        self.lock().send_message_calls.clone()
    }

    pub fn cancel_run_calls(&self) -> Vec<WebUiCancelRunCommand> {
        self.lock().cancel_run_calls.clone()
    }

    pub fn resolve_gate_calls(&self) -> Vec<WebUiResolveGateCommand> {
        self.lock().resolve_gate_calls.clone()
    }

    pub fn timeline_snapshot_calls(&self) -> Vec<WebUiGetTimelineCommand> {
        self.lock().timeline_snapshot_calls.clone()
    }

    pub fn timeline_updates_calls(&self) -> Vec<WebUiGetTimelineCommand> {
        self.lock().timeline_updates_calls.clone()
    }
}

impl Default for FakeWebUiService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WebUiService for FakeWebUiService {
    async fn create_thread(
        &self,
        command: WebUiCreateThreadCommand,
    ) -> Result<WebUiThreadCreated, WebUiServiceError> {
        let mut state = self.lock();
        state.create_thread_calls.push(command.clone());
        if let Some(error) = state.create_thread_error.clone() {
            return Err(error);
        }
        if let Some(outcome) = state.create_thread_outcome.clone() {
            return Ok(outcome);
        }
        let thread_id = command.requested_thread_id.unwrap_or_else(|| {
            ThreadId::new("thread:fake-webui").expect("fake thread id") // safety: test-support fake; literal id is always valid
        });
        Ok(WebUiThreadCreated { thread_id })
    }

    async fn send_message(
        &self,
        command: WebUiSendMessageCommand,
    ) -> Result<WebUiMessageAccepted, WebUiServiceError> {
        let mut state = self.lock();
        state.send_message_calls.push(command.clone());
        if let Some(error) = state.send_message_error.clone() {
            return Err(error);
        }
        if let Some(outcome) = state.send_message_outcome.clone() {
            return Ok(outcome);
        }
        let accepted_message_ref =
            AcceptedMessageRef::new(format!("msg:fake:{}", command.client_action_id.as_str()))
                .map_err(|_| WebUiServiceError::InvalidInput)?;
        Ok(WebUiMessageAccepted {
            thread_id: command.scope.thread_id.clone(),
            accepted_message_ref,
            run: WebUiMessageRunOutcome::Submitted {
                run_id: TurnRunId::new(),
            },
        })
    }

    async fn cancel_run(
        &self,
        command: WebUiCancelRunCommand,
    ) -> Result<WebUiRunCancelled, WebUiServiceError> {
        let mut state = self.lock();
        state.cancel_run_calls.push(command.clone());
        if let Some(error) = state.cancel_run_error.clone() {
            return Err(error);
        }
        if let Some(outcome) = state.cancel_run_outcome.clone() {
            return Ok(outcome);
        }
        Ok(WebUiRunCancelled {
            run_id: command.run_id,
            already_terminal: false,
        })
    }

    async fn resolve_gate(
        &self,
        command: WebUiResolveGateCommand,
    ) -> Result<WebUiGateResolved, WebUiServiceError> {
        let mut state = self.lock();
        state.resolve_gate_calls.push(command.clone());
        if let Some(error) = state.resolve_gate_error.clone() {
            return Err(error);
        }
        if let Some(outcome) = state.resolve_gate_outcome.clone() {
            return Ok(outcome);
        }
        Ok(WebUiGateResolved::Resumed {
            run_id: command.run_id,
        })
    }

    async fn get_timeline_snapshot(
        &self,
        command: WebUiGetTimelineCommand,
    ) -> Result<WebUiTimelineSnapshot, WebUiServiceError> {
        let mut state = self.lock();
        state.timeline_snapshot_calls.push(command.clone());
        if let Some(error) = state.timeline_snapshot_error.clone() {
            return Err(error);
        }
        if let Some(outcome) = state.timeline_snapshot_outcome.clone() {
            return Ok(outcome);
        }
        Ok(empty_timeline_snapshot(&command))
    }

    async fn get_timeline_updates(
        &self,
        command: WebUiGetTimelineCommand,
    ) -> Result<WebUiTimelineReplay, WebUiServiceError> {
        let mut state = self.lock();
        state.timeline_updates_calls.push(command.clone());
        if let Some(error) = state.timeline_updates_error.clone() {
            return Err(error);
        }
        if let Some(outcome) = state.timeline_updates_outcome.clone() {
            return Ok(outcome);
        }
        Ok(empty_timeline_replay(&command))
    }
}

fn empty_timeline_snapshot(command: &WebUiGetTimelineCommand) -> WebUiTimelineSnapshot {
    WebUiTimelineSnapshot {
        entries: Vec::new(),
        runs: Vec::new(),
        next_cursor: synthesize_cursor(command),
        truncated: false,
    }
}

fn empty_timeline_replay(command: &WebUiGetTimelineCommand) -> WebUiTimelineReplay {
    WebUiTimelineReplay {
        entries: Vec::new(),
        runs: Vec::new(),
        next_cursor: synthesize_cursor(command),
        truncated: false,
    }
}

fn synthesize_cursor(command: &WebUiGetTimelineCommand) -> WebUiTimelineCursor {
    use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
    use ironclaw_events::{EventStreamKey, ReadScope};
    let stream = EventStreamKey::new(
        command.caller.tenant_id.clone(),
        command.caller.user_id.clone(),
        command.caller.agent_id.clone(),
    );
    let read_scope = ReadScope {
        project_id: command.caller.project_id.clone(),
        mission_id: None,
        thread_id: Some(command.thread_id.clone()),
        process_id: None,
    };
    WebUiTimelineCursor::from_projection(ProjectionCursor::origin_for_scope(ProjectionScope {
        stream,
        read_scope,
    }))
}
