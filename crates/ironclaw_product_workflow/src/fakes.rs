//! In-memory fakes for contract tests and downstream integration tests.

use std::collections::HashMap;
use std::sync::Mutex;

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

// ---------------------------------------------------------------------------
// FakeConversationBindingService
// ---------------------------------------------------------------------------

/// In-memory fake that resolves all bindings to a default tenant/user/thread
/// unless programmed otherwise.
pub struct FakeConversationBindingService {
    state: Mutex<FakeBindingState>,
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
            state: Mutex::new(FakeBindingState::default()),
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
                "thread:{}",
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
        let state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        if let Some(prior) = state.settled.get(&fingerprint) {
            return Ok(IdempotencyDecision::Replay(prior.clone()));
        }
        Ok(IdempotencyDecision::New(ProductInboundAction::begin(
            fingerprint,
            received_at,
        )))
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let mut state = self.state.lock().expect("fake ledger state lock poisoned"); // safety: test-support fake
        if let Some(error) = state.fail_with.clone() {
            return Err(error);
        }
        if let Some(error) = state.settle_fail_with.clone() {
            return Err(error);
        }
        state.settled.insert(action.fingerprint.clone(), action);
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
